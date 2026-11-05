//! Asset service wiring of the editor: opens the assets root, ticks the service once per frame,
//! applies the browser's requests and the diagnostics panel's actions, and keeps the browser fresh.

use std::time::{Duration, Instant};

use super::EditorApp;
use redox_editor_assets::{AssetChange, AssetService, DeleteOutcome};
use redox_editor_ui::EditorUiCtx;
use redox_editor_ui::asset_report::{AssetReportPanel, ReportAction, has_problems};
use redox_editor_ui::asset_browser::AssetRequest;

/// Assets root opened at startup, relative to the working directory.
pub(super) const ASSETS_ROOT: &str = "assets";

/// The browser is refreshed at least this often, so that import status changes show up.
pub(super) const BROWSER_REFRESH: Duration = Duration::from_millis(250);

/// Opens the assets root. A failure is logged and the editor runs without the asset service.
pub(super) fn open_assets(ui_ctx: &EditorUiCtx) -> (Option<AssetService>, AssetReportPanel) {
    match AssetService::open(ASSETS_ROOT) {
        Ok(service) => {
            let report = service.report();
            ui_ctx.log(format!(
                "[INFO] Assets: {} registered, {} sidecars created, {} GUIDs regenerated, {} damaged, {} missing references",
                service.registry().len(),
                report.created.len(),
                report.regenerated.len(),
                report.damaged.len(),
                report.missing_refs.len()
            ));
            let panel = AssetReportPanel {
                open: has_problems(report),
            };
            (Some(service), panel)
        }
        Err(err) => {
            ui_ctx.log(format!(
                "[ERROR] Assets root {ASSETS_ROOT} cannot be opened: {err}"
            ));
            (None, AssetReportPanel::default())
        }
    }
}

fn log_change(ui_ctx: &EditorUiCtx, change: &AssetChange) {
    let line = match change {
        AssetChange::Added { path, .. } => format!("[INFO] Asset added: {path}"),
        AssetChange::Missing { path, .. } => {
            format!("[WARN] Asset missing: {path}; references to it are now missing")
        }
        AssetChange::Moved { from, to, .. } => format!("[INFO] Asset moved: {from} -> {to}"),
        AssetChange::Outdated { path, .. } => format!("[INFO] Asset changed, importing: {path}"),
        AssetChange::Imported { path } => format!("[INFO] Imported {path}"),
        AssetChange::ImportFailed { path, message } => {
            format!("[ERROR] Import of {path} failed: {message}")
        }
    };
    ui_ctx.log(line);
}

impl EditorApp {
    /// Once per frame: applies file-system and import changes, the browser's requests, and
    /// refreshes the browser when something changed or the refresh interval has passed.
    pub(super) fn sync_assets(&mut self) {
        let changes = match self.assets.as_mut() {
            Some(service) => service.tick(),
            None => return,
        };
        let changed = !changes.is_empty();
        for change in &changes {
            log_change(&self.ui_ctx, change);
        }

        let requests = self.ui_ctx.assets.lock().take_requests();
        for request in requests {
            self.handle_asset_request(request);
        }

        let open_path = self
            .ui_ctx
            .assets
            .lock()
            .model
            .selected()
            .map(|entry| entry.path.clone());
        let refresh_due = self.last_asset_refresh.elapsed() >= BROWSER_REFRESH;
        if let Some(service) = self.assets.as_mut() {
            service.set_open_asset(open_path.as_deref());
            if changed || refresh_due {
                self.ui_ctx.assets.lock().refresh(service.registry());
                self.last_asset_refresh = Instant::now();
            }
        }
    }

    fn handle_asset_request(&mut self, request: AssetRequest) {
        let Some(service) = self.assets.as_mut() else {
            return;
        };
        match request {
            AssetRequest::Rescan => match service.rescan() {
                Ok(report) => {
                    self.ui_ctx.log(format!(
                        "[INFO] Rescan: {} damaged, {} missing references",
                        report.damaged.len(),
                        report.missing_refs.len()
                    ));
                    self.asset_report.open = has_problems(&report);
                }
                Err(err) => self.ui_ctx.log(format!("[ERROR] Rescan failed: {err}")),
            },
            AssetRequest::Move { path, to } => match service.move_asset(&path, &to) {
                Ok(report) => {
                    self.ui_ctx.log(format!("[INFO] Moved {path} -> {to}"));
                    if !report.scenes.is_empty() {
                        self.ui_ctx
                            .assets
                            .lock()
                            .ask_hint_update(report.guid, report.scenes);
                    }
                }
                Err(err) => self
                    .ui_ctx
                    .log(format!("[ERROR] Move of {path} failed: {err}")),
            },
            AssetRequest::Delete { path, force } => match service.delete_asset(&path, force) {
                Ok(DeleteOutcome::Blocked { dependents }) => {
                    self.ui_ctx.assets.lock().ask_force_delete(path, dependents);
                }
                Ok(DeleteOutcome::Deleted { dangling, .. }) => {
                    self.ui_ctx.log(format!(
                        "[WARN] Deleted {path}; {} references now point to a missing asset",
                        dangling.len()
                    ));
                }
                Err(err) => self
                    .ui_ctx
                    .log(format!("[ERROR] Delete of {path} failed: {err}")),
            },
            AssetRequest::Reimport { path } => {
                if service.import(&path) {
                    self.ui_ctx.log(format!("[INFO] Queued import of {path}"));
                } else {
                    self.ui_ctx.log(format!(
                        "[WARN] {path} is already queued or has no importer"
                    ));
                }
            }
            AssetRequest::CancelImport { path } => {
                if service.cancel_import(&path) {
                    self.ui_ctx.log(format!("[INFO] Cancelled import of {path}"));
                }
            }
            AssetRequest::Reveal { path } => {
                let abs = service.registry().root().join(&path);
                let spawned = std::process::Command::new("explorer")
                    .arg(format!("/select,{}", abs.display()))
                    .spawn();
                if let Err(err) = spawned {
                    self.ui_ctx
                        .log(format!("[ERROR] Cannot open Explorer: {err}"));
                }
            }
            AssetRequest::UpdateHints { guid } => match service.update_path_hints(guid) {
                Ok(count) => self
                    .ui_ctx
                    .log(format!("[INFO] Updated {count} path hint(s)")),
                Err(err) => self
                    .ui_ctx
                    .log(format!("[ERROR] Path hints not updated: {err}")),
            },
        }
    }

    /// Applies one action chosen in the diagnostics panel.
    pub(super) fn apply_report_action(&mut self, action: ReportAction) {
        let Some(service) = self.assets.as_mut() else {
            return;
        };
        match action {
            ReportAction::SelectPath(path) => {
                let guid = service.guid_of(&path);
                self.ui_ctx.assets.lock().model.select(guid);
            }
            ReportAction::OpenScene(path) => {
                let guid = service.guid_of(&path);
                self.ui_ctx.assets.lock().model.select(guid);
                self.ui_ctx.log(format!(
                    "[INFO] Scene {path} selected; opening scenes arrives with the scene document (EDT-A2)"
                ));
            }
            ReportAction::RepairSidecar(path) => match service.repair_sidecar(&path) {
                Ok(changes) => {
                    self.ui_ctx
                        .log(format!("[INFO] Sidecar of {path} recreated"));
                    for change in &changes {
                        log_change(&self.ui_ctx, change);
                    }
                }
                Err(err) => self
                    .ui_ctx
                    .log(format!("[ERROR] Sidecar of {path} not repaired: {err}")),
            },
            ReportAction::Fix { target, fix } => match service.fix_missing_ref(&target, &fix) {
                Ok(count) => self.ui_ctx.log(format!(
                    "[INFO] Fixed {count} reference(s) in {}",
                    target.scene
                )),
                Err(err) => self
                    .ui_ctx
                    .log(format!("[ERROR] Reference not fixed: {err}")),
            },
        }
    }
}
