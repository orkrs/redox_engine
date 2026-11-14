//! TZ §23.17 (minimal): enable the hello plugin from the project manifest.

use super::common::EditorHarness;
use redox_editor_core::project::{ProjectManifest, ProjectOpenOptions, MANIFEST_FILE};
use redox_editor_plugins::{HELLO_COMMAND_ID, HELLO_PLUGIN_NAME, PluginHost};

#[test]
fn tz_23_17_hello_plugin_from_manifest() {
    let mut h = EditorHarness::new();
    let dir = h.new_project("plugin_hello", "empty").expect("create project");
    let manifest_path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let mut manifest: ProjectManifest = ron::from_str(&text).expect("parse manifest");
    manifest.plugins = vec![HELLO_PLUGIN_NAME.to_string()];
    let body = ron::ser::to_string_pretty(&manifest, ron::ser::PrettyConfig::default())
        .expect("serialize");
    std::fs::write(&manifest_path, body).expect("write manifest");

    let options = ProjectOpenOptions {
        loadable_plugins: vec![HELLO_PLUGIN_NAME.to_string()],
    };
    let project =
        redox_editor_core::project::ProjectContext::open_with_options(&dir, options).expect("open");
    assert!(project.warnings().is_empty());

    let mut host = PluginHost::new();
    host.apply_project_plugins(&manifest.plugins, &mut h.ctx.commands, &h.ctx.log);
    assert!(h.ctx.commands.contains(HELLO_COMMAND_ID));

    host.set_enabled(HELLO_PLUGIN_NAME, false, &mut h.ctx.commands, &h.ctx.log)
        .expect("disable");
    assert!(!h.ctx.commands.contains(HELLO_COMMAND_ID));
}