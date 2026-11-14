//! Meshlet bake source paths (OFPA has no meshes — asset browser glTF selection).

use redox_editor_assets::AssetKind;
use redox_editor_ui::{EditorMeshProxy, EditorUiCtx};
use std::path::PathBuf;

/// glTF paths to cluster: selected asset browser entry, else the selected entity's mesh proxy.
pub fn collect_meshlet_bake_sources(ui_ctx: &EditorUiCtx, assets_root: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(entry) = ui_ctx.assets.lock().model.selected() {
        if matches!(entry.kind, AssetKind::Mesh) {
            let abs = assets_root
                .map(|root| root.join(&entry.path))
                .unwrap_or_else(|| PathBuf::from(&entry.path));
            if abs.is_file() {
                out.push(abs);
            }
        }
    }
    if out.is_empty() {
        if let Some(entity) = ui_ctx.core.selection.lock().primary {
            let world = ui_ctx.core.world.lock();
            if let Some(proxy) = world.get_component::<EditorMeshProxy>(entity) {
                let p = &proxy.path;
                if p.is_file() {
                    out.push(p.clone());
                } else if let Some(root) = assets_root {
                    let joined = root.join(p);
                    if joined.is_file() {
                        out.push(joined);
                    }
                }
            }
        }
    }
    out
}