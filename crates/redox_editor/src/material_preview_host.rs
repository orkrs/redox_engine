//! Material editor preview on [`RenderHost`] (MAT-10 step 2).

use redox_editor_assets::{
    MaterialGraphRegistry, PreviewMesh, SimplePreviewLight, graph_id_for_guid,
};
use redox_editor_ui::MaterialEditorSession;
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::material::MaterialGraphId;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::{Material, RenderObject, Renderer};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::shadow::ShadowClass;

use crate::render_host::RenderHost;

const PREVIEW_SIZE: u32 = 128;
const UNTITLED_GRAPH_ID: u32 = 0xffff_0001;

pub struct MaterialPreviewRig {
    pub material_slot: usize,
    sphere_mesh: usize,
    cube_mesh: usize,
    plane_mesh: usize,
    last_revision: u64,
    color_tex: wgpu::Texture,
    color_view: wgpu::TextureView,
}

pub fn init_material_preview(renderer: &mut Renderer) -> MaterialPreviewRig {
    let material_slot = renderer.add_material(Material::solid(Vec3::ONE));
    let sphere_mesh = renderer.upload_mesh(&create_sphere(1.0, 24));
    let cube_mesh = renderer.upload_mesh(&create_cube());
    let mut plane = create_cube();
    for v in &mut plane.vertices {
        v.position.y *= 0.05;
        v.position.x *= 3.0;
        v.position.z *= 3.0;
    }
    let plane_mesh = renderer.upload_mesh(&plane);
    let color_tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mat_preview_color"),
        size: wgpu::Extent3d {
            width: PREVIEW_SIZE,
            height: PREVIEW_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    MaterialPreviewRig {
        material_slot,
        sphere_mesh,
        cube_mesh,
        plane_mesh,
        last_revision: u64::MAX,
        color_view: color_tex.create_view(&Default::default()),
        color_tex,
    }
}

impl RenderHost {
    pub fn sync_material_preview(
        &mut self,
        session: Option<&MaterialEditorSession>,
        registry: &mut MaterialGraphRegistry,
    ) {
        let Some(rig) = self.material_preview.as_mut() else {
            return;
        };
        let Some(session) = session else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if session.preview_revision == rig.last_revision || !session.compile.errors.is_empty() {
            return;
        }
        let graph = session.document.graph.clone();
        let graph_id = session
            .document
            .asset_guid
            .map(|g| registry.register_guid(g, graph.clone()))
            .unwrap_or_else(|| {
                registry.register(UNTITLED_GRAPH_ID, graph.clone());
                UNTITLED_GRAPH_ID
            });
        renderer.register_material_graph(MaterialGraphId(graph_id), graph);
        let _ = renderer.set_material_graph(rig.material_slot, &session.document.graph, &[]);
        rig.last_revision = session.preview_revision;
        self.draw_material_preview(session);
    }

    fn draw_material_preview(&mut self, session: &MaterialEditorSession) {
        let Some(rig) = self.material_preview.as_ref() else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let mesh = match session.preview.mesh {
            PreviewMesh::Sphere => rig.sphere_mesh,
            PreviewMesh::Cube => rig.cube_mesh,
            PreviewMesh::Plane => rig.plane_mesh,
        };
        renderer.enable_taa(false);
        let camera = Camera::new(1.0, 1.0, 0.1, 50.0);
        renderer
            .camera_uniform
            .update(&camera, Vec3::new(0.0, 0.0, 3.2), Quat::IDENTITY);
        renderer.update_camera_buffer();
        let light = SimplePreviewLight::studio_fallback();
        let mut lights = LightScene::new(DirectionalLight::new(
            Vec3::new(light.direction[0], light.direction[1], light.direction[2]),
            Vec3::new(light.colour[0], light.colour[1], light.colour[2]),
            2.2,
        ));
        lights.ambient = Vec3::new(light.ambient[0], light.ambient[1], light.ambient[2]);
        renderer.update_light_buffer(&lights.to_legacy_uniform());
        let object = RenderObject {
            id: 1,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::IDENTITY,
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: rig.material_slot,
            bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::ONE),
        };
        let _ = renderer.render_frame_view(
            &FrameView::new(&camera, std::slice::from_ref(&object), &lights),
            &rig.color_view,
        );
    }

    pub fn material_preview_view(&self) -> Option<&wgpu::TextureView> {
        Some(&self.material_preview.as_ref()?.color_view)
    }
}

pub fn register_open_material(registry: &mut MaterialGraphRegistry, session: &MaterialEditorSession) {
    if let Some(guid) = session.document.asset_guid {
        registry.register_guid(guid, session.document.graph.clone());
    }
}