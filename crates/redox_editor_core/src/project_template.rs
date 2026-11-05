//! Starter templates for new projects. A template is the text of the first scene file that
//! [`crate::project::ProjectContext::create`] writes into `scenes/`.

/// File name of the start scene written by every template.
pub const START_SCENE_FILE: &str = "main.scene.ron";

/// A template chosen when a project is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTemplate {
    /// A camera and a directional light.
    Empty,
    /// A ground slab, a cube, a camera and a directional light.
    ThreeDStarter,
}

impl ProjectTemplate {
    /// Every template, in menu order.
    pub const ALL: [ProjectTemplate; 2] = [ProjectTemplate::Empty, ProjectTemplate::ThreeDStarter];

    /// Name shown in menus and dialogs.
    pub fn label(self) -> &'static str {
        match self {
            ProjectTemplate::Empty => "Empty",
            ProjectTemplate::ThreeDStarter => "3D starter",
        }
    }

    /// Text of the start scene file.
    pub fn scene_text(self) -> &'static str {
        match self {
            ProjectTemplate::Empty => EMPTY_SCENE,
            ProjectTemplate::ThreeDStarter => THREE_D_STARTER_SCENE,
        }
    }
}

/// Start scene of the `empty` template.
const EMPTY_SCENE: &str = r#"(
    format_version: 1,
    name: "main",
    entities: [
        (
            guid: "00000000000000000000000000000001",
            components: [
                Struct(type_name: "Transform", fields: [("translation", List([F64(0.0), F64(2.0), F64(-6.0)])), ("rotation", List([F64(0.0), F64(0.0), F64(0.0), F64(1.0)])), ("scale", List([F64(1.0), F64(1.0), F64(1.0)]))]),
                Struct(type_name: "Camera", fields: []),
            ],
        ),
        (
            guid: "00000000000000000000000000000002",
            components: [
                Struct(type_name: "DirectionalLight", fields: []),
            ],
        ),
    ],
)
"#;

/// Start scene of the `3d_starter` template: ground, cube, camera and light.
const THREE_D_STARTER_SCENE: &str = r#"(
    format_version: 1,
    name: "main",
    entities: [
        (
            guid: "00000000000000000000000000000001",
            components: [
                Struct(type_name: "Name", fields: [("name", String("Ground"))]),
                Struct(type_name: "Transform", fields: [("translation", List([F64(0.0), F64(-0.5), F64(0.0)])), ("rotation", List([F64(0.0), F64(0.0), F64(0.0), F64(1.0)])), ("scale", List([F64(20.0), F64(1.0), F64(20.0)]))]),
            ],
        ),
        (
            guid: "00000000000000000000000000000002",
            components: [
                Struct(type_name: "Name", fields: [("name", String("Cube"))]),
                Struct(type_name: "Transform", fields: [("translation", List([F64(0.0), F64(0.5), F64(0.0)])), ("rotation", List([F64(0.0), F64(0.0), F64(0.0), F64(1.0)])), ("scale", List([F64(1.0), F64(1.0), F64(1.0)]))]),
            ],
        ),
        (
            guid: "00000000000000000000000000000003",
            components: [
                Struct(type_name: "Transform", fields: [("translation", List([F64(4.0), F64(4.0), F64(-8.0)])), ("rotation", List([F64(0.0), F64(0.0), F64(0.0), F64(1.0)])), ("scale", List([F64(1.0), F64(1.0), F64(1.0)]))]),
                Struct(type_name: "Camera", fields: []),
            ],
        ),
        (
            guid: "00000000000000000000000000000004",
            components: [
                Struct(type_name: "DirectionalLight", fields: []),
            ],
        ),
    ],
)
"#;
