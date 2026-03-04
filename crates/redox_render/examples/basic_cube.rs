use redox_math::{Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::systems::RenderObject;
use std::sync::Arc;
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Render - Basic Cube")
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0))
            .build(&event_loop)
            .unwrap(),
    );

    // Initialize RenderContext (async, so use pollster for example)
    let mut context = pollster::block_on(RenderContext::new(window.clone()));

    // Create a cube mesh and a material
    let cube_mesh = create_cube();
    let mesh_index = context.upload_mesh(&cube_mesh);

    // Load texture
    let metal_grid_bytes = include_bytes!("../assets/metal_grid.png");
    let tex_idx = context
        .upload_texture(metal_grid_bytes, "metal_grid")
        .unwrap_or(0);

    let material = Material::textured(Vec3::new(0.8, 0.2, 0.3), tex_idx); // Reddish tint
    let material_index = context.add_material(material);

    // Setup camera
    let mut camera = Camera::new(45.0f32.to_radians(), 800.0 / 600.0, 0.1, 100.0);
    let camera_pos = Vec3::new(0.0, 2.0, 5.0);
    let camera_rot = Quat::from_rotation_x(-20.0f32.to_radians());

    // Setup light
    let light = DirectionalLight::default();
    let light_uniform = LightUniform::new(&light, Vec3::splat(0.1));
    context.update_light_buffer(&light_uniform);

    let mut rotation = 0.0f32;

    event_loop
        .run(move |event, elwt| {
            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => elwt.exit(),
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    context.resize(size.width, size.height);
                    camera.aspect_ratio = size.width as f32 / size.height as f32;
                }
                Event::AboutToWait => {
                    window.request_redraw();
                }
                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    rotation += 0.01;

                    // Update camera
                    context
                        .camera_uniform
                        .update(&camera, camera_pos, camera_rot);
                    context.update_camera_buffer();

                    // Build render object
                    let model_matrix = Mat4::from_rotation_y(rotation);
                    let render_obj = RenderObject {
                        model_matrix,
                        mesh_index,
                        material_index,
                    };

                    match context.render_frame(&[render_obj]) {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) => {
                            context.resize(context.config.width, context.config.height)
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                        Err(e) => eprintln!("{:?}", e),
                    }
                }
                _ => {}
            }
        })
        .unwrap();
}
