//! Individual validation rules, each in its own function.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::{AssetGuid, AssetMeta, AssetRegistry};
use redox_ecs::{Entity, Parent, World};
use redox_editor_assets::import::probe_texture_header;
use redox_editor_assets::{MissingRef, ReferenceIndex};
use redox_math::Transform;
use redox_reflect::{Reflect, TypeRegistry};
use redox_render::camera::ActiveCamera;
use redox_render::Camera;
use redox_scene::{AssetRef, AssetRefs, EntityGuid};

use super::issue::{Severity, ValidationFix, ValidationIssue, ValidationRule};
use super::settings::ValidationSettings;
use crate::{LoadReport, LoadWarning};

/// Inputs shared by every rule.
pub struct RuleContext<'a> {
    /// ECS world of the open level.
    pub world: &'a World,
    /// Reflected component types.
    pub registry: &'a TypeRegistry,
    /// Assets folder, when known.
    pub assets_root: Option<&'a Path>,
    /// Registry of imported assets, when known.
    pub asset_registry: Option<&'a AssetRegistry>,
    /// Scene reference index, when known.
    pub reference_index: Option<&'a ReferenceIndex>,
    /// Path of the scene file being validated (relative to project root), when any.
    pub scene_path: Option<&'a str>,
    /// Bounds and editor policy.
    pub settings: &'a ValidationSettings,
    /// Warnings from the last load, if re-validating without reloading.
    pub load_report: Option<&'a LoadReport>,
}

/// **Rule 1 — missing assets and broken references.**
pub fn check_missing_and_broken_assets(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if let Some(report) = ctx.load_report {
        for warning in &report.warnings {
            if let LoadWarning::MissingAsset {
                entity,
                type_name,
                path,
            } = warning
            {
                issues.push(ValidationIssue {
                    rule: ValidationRule::MissingAsset,
                    severity: Severity::Error,
                    message: format!("{type_name} points to missing asset `{path}`"),
                    entity: Some(*entity),
                    asset_path: Some(path.clone()),
                    scene_path: ctx.scene_path.map(str::to_owned),
                    fix: None,
                    select: Some(*entity),
                    open: Some(path.clone()),
                });
            }
        }
    }
    if let Some(index) = ctx.reference_index {
        if let Some(registry) = ctx.asset_registry {
            for missing in index.missing_refs(registry) {
                issues.push(missing_ref_issue(&missing, ctx.scene_path));
            }
        }
    }
    if let Some(root) = ctx.assets_root {
        for entity in ctx.world.all_entities() {
            let Some(guid) = ctx.world.get_component::<EntityGuid>(entity).copied() else {
                continue;
            };
            if let Some(refs) = ctx.world.get_component::<AssetRefs>(entity) {
                for asset_ref in &refs.0 {
                    if let Some(path) = asset_ref.path.as_deref() {
                        if !root.join(path).exists() {
                            issues.push(ValidationIssue {
                                rule: ValidationRule::MissingAsset,
                                severity: Severity::Error,
                                message: format!(
                                    "{} points to missing asset `{path}`",
                                    asset_ref.component
                                ),
                                entity: Some(guid),
                                asset_path: Some(path.to_owned()),
                                scene_path: ctx.scene_path.map(str::to_owned),
                                fix: None,
                                select: Some(guid),
                                open: Some(path.to_owned()),
                            });
                            continue;
                        }
                    }
                    if broken_ref(asset_ref, ctx.asset_registry) {
                        issues.push(ValidationIssue {
                            rule: ValidationRule::BrokenAssetRef,
                            severity: Severity::Error,
                            message: format!(
                                "{} reference is broken (guid {:?}, path {:?})",
                                asset_ref.component, asset_ref.guid, asset_ref.path
                            ),
                            entity: Some(guid),
                            asset_path: asset_ref.path.clone(),
                            scene_path: ctx.scene_path.map(str::to_owned),
                            fix: None,
                            select: Some(guid),
                            open: asset_ref.path.clone(),
                        });
                    }
                }
            }
        }
    }
    issues
}

/// **Rule 2 — unknown components from load warnings.**
pub fn check_unknown_components(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let Some(report) = ctx.load_report else {
        return Vec::new();
    };
    report
        .warnings
        .iter()
        .filter_map(|warning| match warning {
            LoadWarning::UnknownComponent { entity, type_name } => Some(ValidationIssue {
                rule: ValidationRule::UnknownComponent,
                severity: Severity::Warning,
                message: format!("unknown component type `{type_name}`"),
                entity: Some(*entity),
                asset_path: None,
                scene_path: ctx.scene_path.map(str::to_owned),
                fix: None,
                select: Some(*entity),
                open: None,
            }),
            _ => None,
        })
        .collect()
}

/// **Rule 3 — missing parents and hierarchy cycles.**
pub fn check_hierarchy(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if let Some(report) = ctx.load_report {
        for warning in &report.warnings {
            if let LoadWarning::MissingParent {
                entity,
                parent_guid,
            } = warning
            {
                issues.push(ValidationIssue {
                    rule: ValidationRule::MissingParent,
                    severity: Severity::Error,
                    message: format!("parent {parent_guid} is missing for entity {entity}"),
                    entity: Some(*entity),
                    asset_path: None,
                    scene_path: ctx.scene_path.map(str::to_owned),
                    fix: None,
                    select: Some(*entity),
                    open: None,
                });
            }
        }
    }
    let mut parent_of: HashMap<EntityGuid, EntityGuid> = HashMap::new();
    for entity in ctx.world.all_entities() {
        let Some(guid) = ctx.world.get_component::<EntityGuid>(entity).copied() else {
            continue;
        };
        if let Some(parent_entity) = ctx
            .world
            .get_component::<Parent>(entity)
            .map(|parent| parent.0)
        {
            match ctx.world.get_component::<EntityGuid>(parent_entity).copied() {
                Some(parent_guid) => {
                    parent_of.insert(guid, parent_guid);
                }
                None => {
                    issues.push(ValidationIssue {
                        rule: ValidationRule::MissingParent,
                        severity: Severity::Error,
                        message: format!("entity {guid} has a parent without a guid"),
                        entity: Some(guid),
                        asset_path: None,
                        scene_path: ctx.scene_path.map(str::to_owned),
                        fix: None,
                        select: Some(guid),
                        open: None,
                    });
                }
            }
        }
    }
    for cycle in find_cycles(&parent_of) {
        let first = cycle[0];
        issues.push(ValidationIssue {
            rule: ValidationRule::HierarchyCycle,
            severity: Severity::Error,
            message: format!(
                "hierarchy cycle: {}",
                cycle
                    .iter()
                    .map(|g| format!("{g:?}"))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
            entity: Some(first),
            asset_path: None,
            scene_path: ctx.scene_path.map(str::to_owned),
            fix: Some(ValidationFix::Unparent { entity: first }),
            select: Some(first),
            open: None,
        });
    }
    issues
}

/// **Rule 4 — no active camera in the scene.**
pub fn check_active_camera(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let mut cameras = Vec::new();
    for entity in ctx.world.all_entities() {
        if ctx.world.get_component::<Camera>(entity).is_some() {
            if let Some(guid) = ctx.world.get_component::<EntityGuid>(entity).copied() {
                cameras.push(guid);
            }
        }
    }
    if cameras.is_empty() {
        return vec![ValidationIssue {
            rule: ValidationRule::NoActiveCamera,
            severity: Severity::Error,
            message: "the scene has no camera".into(),
            entity: None,
            asset_path: None,
            scene_path: ctx.scene_path.map(str::to_owned),
            fix: Some(ValidationFix::AddDefaultCamera),
            select: None,
            open: None,
        }];
    }
    let active_ok = ctx
        .world
        .get_resource::<ActiveCamera>()
        .and_then(|active| ctx.world.get_component::<Camera>(active.0))
        .is_some();
    if !active_ok {
        vec![ValidationIssue {
            rule: ValidationRule::NoActiveCamera,
            severity: Severity::Warning,
            message: "no active camera is set; the viewport may not render".into(),
            entity: Some(cameras[0]),
            asset_path: None,
            scene_path: ctx.scene_path.map(str::to_owned),
            fix: None,
            select: Some(cameras[0]),
            open: None,
        }]
    } else {
        Vec::new()
    }
}

/// **Rule 5 — material graph compile.** Skipped (📌) until a headless matgraph hook exists.
pub fn check_material_compile(_ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    Vec::new()
}

/// **Rule 6 — large textures without mips (heuristic on `.meta` settings and headers).**
pub fn check_texture_mips(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let Some(registry) = ctx.asset_registry else {
        return Vec::new();
    };
    let threshold = ctx.settings.large_texture_threshold;
    let mut issues = Vec::new();
    for (path, _) in registry.iter() {
        let lower = path.to_ascii_lowercase();
        if !(lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".ktx2"))
        {
            continue;
        }
        let abs = registry.root().join(path);
        let Ok(bytes) = fs::read(&abs) else {
            continue;
        };
        let Ok(header) = probe_texture_header(&bytes) else {
            continue;
        };
        let large = header.width.max(header.height) > threshold;
        if !large {
            continue;
        }
        let sidecar = meta_sidecar_path(&abs);
        let settings = fs::read_to_string(&sidecar)
            .ok()
            .and_then(|text| ron::from_str::<AssetMeta>(&text).ok())
            .map(|meta| meta.settings)
            .unwrap_or_default();
        let mips_ok = meta_mentions_mips(&settings)
            || (lower.ends_with(".ktx2") && ktx2_level_count(&bytes).unwrap_or(1) > 1);
        if !mips_ok {
            issues.push(ValidationIssue {
                rule: ValidationRule::TextureNoMips,
                severity: Severity::Warning,
                message: format!(
                    "texture `{path}` is {}x{} but has no mip configuration",
                    header.width, header.height
                ),
                entity: None,
                asset_path: Some(path.clone()),
                scene_path: ctx.scene_path.map(str::to_owned),
                fix: None,
                select: None,
                open: Some(path.clone()),
            });
        }
    }
    issues
}

/// **Rule 7 — entities outside the configured world AABB.**
pub fn check_world_bounds(ctx: &RuleContext<'_>) -> Vec<ValidationIssue> {
    let bounds = ctx.settings.world_bounds;
    let mut issues = Vec::new();
    for entity in ctx.world.all_entities() {
        let Some(transform) = ctx.world.get_component::<Transform>(entity) else {
            continue;
        };
        let pos = transform.translation;
        if !point_in_aabb(pos, bounds) {
            let guid = ctx.world.get_component::<EntityGuid>(entity).copied();
            issues.push(ValidationIssue {
                rule: ValidationRule::OutOfWorldBounds,
                severity: Severity::Warning,
                message: format!(
                    "entity at ({:.1}, {:.1}, {:.1}) is outside world bounds",
                    pos.x, pos.y, pos.z
                ),
                entity: guid,
                asset_path: None,
                scene_path: ctx.scene_path.map(str::to_owned),
                fix: None,
                select: guid,
                open: None,
            });
        }
    }
    issues
}

fn missing_ref_issue(missing: &MissingRef, scene_path: Option<&str>) -> ValidationIssue {
    let entity = missing
        .entity
        .as_deref()
        .and_then(|text| text.parse::<EntityGuid>().ok());
    ValidationIssue {
        rule: ValidationRule::MissingAsset,
        severity: Severity::Error,
        message: format!(
            "{} in scene `{}` points to a missing asset (guid {:?}, path {:?})",
            missing.component, missing.scene, missing.guid, missing.path
        ),
        entity,
        asset_path: missing.path.clone(),
        scene_path: scene_path.map(str::to_owned).or(Some(missing.scene.clone())),
        fix: None,
        select: entity,
        open: missing.path.clone(),
    }
}

fn broken_ref(asset_ref: &AssetRef, registry: Option<&AssetRegistry>) -> bool {
    if let Some(text) = asset_ref.guid.as_deref() {
        if text.parse::<AssetGuid>().is_err() {
            return true;
        }
        if let Some(reg) = registry {
            if let Ok(guid) = text.parse::<AssetGuid>()
                && reg.path_of(guid).is_none()
            {
                return true;
            }
        }
    }
    false
}

fn find_cycles(parent_of: &HashMap<EntityGuid, EntityGuid>) -> Vec<Vec<EntityGuid>> {
    let mut cycles = Vec::new();
    let mut state: HashMap<EntityGuid, u8> = HashMap::new();
    for &start in parent_of.keys() {
        let mut stack = Vec::new();
        visit_cycle(start, parent_of, &mut state, &mut stack, &mut cycles);
    }
    cycles
}

fn visit_cycle(
    node: EntityGuid,
    parent_of: &HashMap<EntityGuid, EntityGuid>,
    state: &mut HashMap<EntityGuid, u8>,
    stack: &mut Vec<EntityGuid>,
    cycles: &mut Vec<Vec<EntityGuid>>,
) {
    match state.get(&node).copied().unwrap_or(0) {
        1 => {
            if let Some(pos) = stack.iter().position(|&g| g == node) {
                cycles.push(stack[pos..].to_vec());
            }
            return;
        }
        2 => return,
        _ => {}
    }
    state.insert(node, 1);
    stack.push(node);
    if let Some(&parent) = parent_of.get(&node) {
        visit_cycle(parent, parent_of, state, stack, cycles);
    }
    stack.pop();
    state.insert(node, 2);
}

fn point_in_aabb(point: redox_math::Vec3, bounds: redox_math::Aabb) -> bool {
    point.x >= bounds.min.x
        && point.y >= bounds.min.y
        && point.z >= bounds.min.z
        && point.x <= bounds.max.x
        && point.y <= bounds.max.y
        && point.z <= bounds.max.z
}

fn meta_sidecar_path(asset: &Path) -> PathBuf {
    let mut sidecar = asset.as_os_str().to_os_string();
    sidecar.push(".meta");
    PathBuf::from(sidecar)
}

fn meta_mentions_mips(settings: &str) -> bool {
    settings.contains("generate_mips: true")
        || settings.contains("mip_levels:")
        || settings.contains("mip_count:")
}

fn ktx2_level_count(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 44 {
        return None;
    }
    Some(u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]))
}