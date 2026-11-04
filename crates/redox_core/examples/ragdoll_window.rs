//! Ragdoll window demo (PHY-07, step 5).
//!
//! A humanoid figure stands on a floor. `R` drops it into a ragdoll, `T` returns it to the
//! animated pose (the ragdoll is removed and the figure snaps back, as the engine does: no blend).
//! The Fox model is a quadruped and does not fit the humanoid template, so the figure is drawn as
//! boxes: one box per body of the ragdoll template, placed at the body's transform.
//!
//! Run: `cargo run --example ragdoll_window -p redox_core --release`
//! Screenshot without a window (figure after it has fallen): `... -- --shot out.png`
//! Self-test without a window (exit code 0 when the transitions behave): `... -- --selftest`

use std::process::ExitCode;

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_anim::Skeleton;
use redox_core::config::EngineConfig;
use redox_core::ragdoll_system::{CharacterPoseMode, RagdollRig, enter_ragdoll, exit_ragdoll};
use redox_core::window::{RunClosure, create_window};
use redox_ecs::{Entity, World};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_physics::{CharacterController, PhysicsContext, RagdollShape, RagdollTemplate};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::{RenderObject, Renderer};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

const DT: f32 = 1.0 / 60.0;
/// The figure is lifted this far above the floor so that no body starts inside it.
const LIFT: f32 = 0.05;
/// Largest rise of the centre of mass allowed after activation from rest, in metres.
const MAX_RISE: f32 = 0.1;
/// Collision group of the ragdoll bodies (the same as the headless demo).
const RAGDOLL_GROUP: u32 = 2;
const FLOOR_HALF: f32 = 20.0;
const FIELD_OF_VIEW: f32 = 0.9;
const CAMERA_POS: Vec3 = Vec3::new(0.0, 1.2, 3.2);
const SHOT_WIDTH: u32 = 960;
const SHOT_HEIGHT: u32 = 540;
/// Physics steps before the figure is dropped in `--shot` (the animated figure stands still).
const SHOT_STANDING_STEPS: usize = 30;
/// Physics steps after the drop in `--shot`: about three seconds, long enough to come to rest.
const SHOT_FALLING_STEPS: usize = 180;

/// Bones of a standard Mixamo-style humanoid, in skeleton order, with their parents and the
/// translation of each bone from its parent in a standing rest pose.
const BONES: [(&str, Option<usize>, [f32; 3]); 15] = [
    ("mixamorig:Hips", None, [0.0, 1.0, 0.0]),
    ("mixamorig:Spine", Some(0), [0.0, 0.2, 0.0]),
    ("mixamorig:Head", Some(1), [0.0, 0.5, 0.0]),
    ("mixamorig:LeftArm", Some(1), [0.2, 0.45, 0.0]),
    ("mixamorig:LeftForeArm", Some(3), [0.0, -0.3, 0.0]),
    ("mixamorig:LeftHand", Some(4), [0.0, -0.25, 0.0]),
    ("mixamorig:RightArm", Some(1), [-0.2, 0.45, 0.0]),
    ("mixamorig:RightForeArm", Some(6), [0.0, -0.3, 0.0]),
    ("mixamorig:RightHand", Some(7), [0.0, -0.25, 0.0]),
    ("mixamorig:LeftUpLeg", Some(0), [0.1, -0.1, 0.0]),
    ("mixamorig:LeftLeg", Some(9), [0.0, -0.45, 0.0]),
    ("mixamorig:LeftFoot", Some(10), [0.0, -0.4, 0.0]),
    ("mixamorig:RightUpLeg", Some(0), [-0.1, -0.1, 0.0]),
    ("mixamorig:RightLeg", Some(12), [0.0, -0.45, 0.0]),
    ("mixamorig:RightFoot", Some(13), [0.0, -0.4, 0.0]),
];

/// A key that the demo reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    /// `R`: drop the figure into a ragdoll.
    Ragdoll,
    /// `T`: return the figure to the animated pose.
    Animate,
}

/// The physics change that a key press asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Enter,
    Exit,
}

/// Transitions between the animated pose and the ragdoll. A key that does not change the state
/// does nothing: `R` on a ragdoll and `T` on an animated figure are ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RagdollToggle {
    ragdoll: bool,
}

impl RagdollToggle {
    fn is_ragdoll(self) -> bool {
        self.ragdoll
    }

    /// Applies a key press and returns the change it asks the figure to make, if any.
    fn press(&mut self, key: Key) -> Option<Change> {
        match (key, self.ragdoll) {
            (Key::Ragdoll, false) => {
                self.ragdoll = true;
                Some(Change::Enter)
            }
            (Key::Animate, true) => {
                self.ragdoll = false;
                Some(Change::Exit)
            }
            _ => None,
        }
    }
}

fn skeleton() -> Skeleton {
    Skeleton::from_local_bind(
        BONES
            .iter()
            .map(|(name, parent, [x, y, z])| {
                (
                    (*name).to_owned(),
                    *parent,
                    Mat4::from_translation(Vec3::new(*x, *y, *z)),
                )
            })
            .collect(),
    )
}

/// Full size of a body's collision shape, for drawing it as a box.
fn box_size(shape: &RagdollShape) -> Vec3 {
    match *shape {
        RagdollShape::Capsule {
            radius,
            half_height,
        } => Vec3::new(2.0 * radius, 2.0 * (half_height + radius), 2.0 * radius),
        RagdollShape::Box {
            half_extents: [x, y, z],
        } => Vec3::new(2.0 * x, 2.0 * y, 2.0 * z),
        RagdollShape::Sphere { radius } => Vec3::splat(2.0 * radius),
    }
}

/// The standing figure, its physics world and the ragdoll state.
struct Figure {
    world: World,
    physics: PhysicsContext,
    skeleton: Skeleton,
    template: RagdollTemplate,
    /// The character entity: a kinematic capsule that owns the controller and the pose mode.
    entity: Entity,
    /// Body poses of the animated figure, in template order.
    animated_bodies: Vec<Mat4>,
    /// Model-space bind pose of the skeleton, lifted by [`LIFT`].
    bind_model: Vec<Mat4>,
    toggle: RagdollToggle,
}

impl Figure {
    fn new() -> Result<Self, String> {
        let skeleton = skeleton();
        let template = RagdollTemplate::humanoid(&skeleton)
            .map_err(|error| format!("cannot build the ragdoll template: {error:?}"))?;
        let bind_model: Vec<Mat4> = skeleton
            .global_matrices(&skeleton.bind_local_pose())
            .into_iter()
            .map(|m| Mat4::from_translation(Vec3::Y * LIFT) * m)
            .collect();
        let animated_bodies = template
            .bone_poses(&skeleton, &bind_model)
            .map_err(|error| format!("cannot pose the figure: {error:?}"))?;

        let mut world = World::new();
        let mut physics = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
        let floor = world.spawn();
        physics
            .add_rigid_body(
                floor,
                RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
                vec![ColliderBuilder::cuboid(FLOOR_HALF, 0.5, FLOOR_HALF)],
            )
            .map_err(|error| format!("cannot add the floor: {error:?}"))?;

        // The character's own capsule encloses the standing figure, as a real controller does.
        let entity = world.spawn();
        physics
            .add_rigid_body(
                entity,
                RigidBodyBuilder::kinematic_position_based().translation(Vec3::new(0.0, 0.6, 0.0)),
                vec![ColliderBuilder::capsule_y(0.5, 0.3)],
            )
            .map_err(|error| format!("cannot add the character: {error:?}"))?;
        world.add_component(entity, CharacterController::default());
        world.add_component(entity, CharacterPoseMode::Animated);

        Ok(Self {
            world,
            physics,
            skeleton,
            template,
            entity,
            animated_bodies,
            bind_model,
            toggle: RagdollToggle::default(),
        })
    }

    fn is_ragdoll(&self) -> bool {
        self.toggle.is_ragdoll()
    }

    /// Applies a key press. A failed activation leaves the figure as it was.
    fn press(&mut self, key: Key) -> Result<(), String> {
        let before = self.toggle;
        match self.toggle.press(key) {
            Some(Change::Enter) => enter_ragdoll(
                &mut self.physics,
                &mut self.world,
                self.entity,
                &self.skeleton,
                &self.template,
                &self.bind_model,
                Vec3::ZERO,
                RAGDOLL_GROUP,
            )
            .map_err(|error| {
                self.toggle = before;
                format!("cannot enter the ragdoll: {error:?}")
            }),
            Some(Change::Exit) => {
                exit_ragdoll(&mut self.physics, &mut self.world, self.entity);
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn step(&mut self) {
        self.physics.step(DT);
    }

    /// World transform of every template body, in template order: the physics bodies while
    /// ragdolled, the animated body poses otherwise.
    fn body_transforms(&self) -> Vec<Mat4> {
        let Some(rig) = self.world.get_component::<RagdollRig>(self.entity) else {
            return self.animated_bodies.clone();
        };
        rig.ragdoll
            .bodies()
            .iter()
            .map(|entity| {
                self.physics
                    .get_body_handle(*entity)
                    .and_then(|handle| self.physics.get_rigid_body_transform(handle))
                    .map_or(Mat4::IDENTITY, |(translation, rotation)| {
                        Mat4::from_rotation_translation(rotation, translation)
                    })
            })
            .collect()
    }

    /// Mean height of the template bodies: the figure's centre of mass, taking bodies as equal.
    fn mean_height(&self) -> f32 {
        let bodies = self.body_transforms();
        bodies.iter().map(|m| m.w_axis.y).sum::<f32>() / bodies.len() as f32
    }

    /// Height of the hips body, which stays low while the figure lies down.
    fn hips_height(&self) -> f32 {
        self.body_transforms().first().map_or(0.0, |m| m.w_axis.y)
    }
}

/// GPU resources of the scene: one cube mesh shared by the floor and the boxes.
struct Stage {
    cube: usize,
    floor: usize,
    figure: usize,
}

impl Stage {
    fn new(r: &mut Renderer) -> Self {
        let cube = r.upload_mesh(&create_cube());
        let floor = r.add_material(Material::solid(Vec3::splat(0.3)).roughness(0.9));
        let figure = r.add_material(Material::solid(Vec3::new(0.85, 0.45, 0.2)).roughness(0.7));
        Self {
            cube,
            floor,
            figure: figure,
        }
    }

    fn objects(&self, figure: &Figure) -> Vec<RenderObject> {
        let floor_size = Vec3::new(2.0 * FLOOR_HALF, 1.0, 2.0 * FLOOR_HALF);
        let mut objects = vec![cube_object(
            self.cube,
            self.floor,
            0,
            Mat4::from_scale_rotation_translation(
                floor_size,
                Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            ),
            floor_size,
        )];
        for (index, (transform, bone)) in figure
            .body_transforms()
            .iter()
            .zip(&figure.template.bones)
            .enumerate()
        {
            let size = box_size(&bone.shape);
            let model = *transform * Mat4::from_scale(size);
            objects.push(cube_object(
                self.cube,
                self.figure,
                index as u32 + 1,
                model,
                size,
            ));
        }
        objects
    }
}

/// A unit cube drawn with `model`; `size` is its full extent, used for culling bounds.
fn cube_object(mesh: usize, material: usize, id: u32, model: Mat4, size: Vec3) -> RenderObject {
    let center = model.w_axis.truncate();
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: model,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        // Conservative bounds: the box may be rotated by the physics.
        bounds: Aabb::from_center_size(center, Vec3::splat(size.max_element() * 1.5)),
    }
}

fn scene_lights() -> LightScene {
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.4, 1.0, 0.6),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.3);
    lights
}

/// Draws the figure and the floor into `view`.
fn render_objects(
    r: &mut Renderer,
    stage: &Stage,
    figure: &Figure,
    aspect: f32,
    view: &wgpu::TextureView,
) -> Result<(), String> {
    let camera = Camera::new(FIELD_OF_VIEW, aspect, 0.1, 80.0);
    r.camera_uniform.update(&camera, CAMERA_POS, Quat::IDENTITY);
    r.update_camera_buffer();
    let objects = stage.objects(figure);
    let lights = scene_lights();
    let frame = FrameView::new(&camera, &objects, &lights);
    r.render_frame_view(&frame, view)
        .map_err(|error| format!("frame: {error:?}"))
}

/// Mean and highest centre of mass during two seconds after a drop from rest.
fn fall_from_rest(figure: &mut Figure) -> Result<(f32, f32), String> {
    figure.press(Key::Ragdoll)?;
    let start = figure.mean_height();
    let mut highest = start;
    for _ in 0..(2.0 / DT) as usize {
        figure.step();
        highest = highest.max(figure.mean_height());
    }
    Ok((start, highest))
}

/// Runs the transitions without a window: a drop from rest must not lift the figure, the return
/// must restore the animated state, and a second drop must behave like the first.
fn selftest() -> Result<(), String> {
    let mut figure = Figure::new()?;
    for _ in 0..SHOT_STANDING_STEPS {
        figure.step();
    }
    if figure.is_ragdoll() {
        return Err("the figure starts ragdolled".into());
    }
    for cycle in 1..=2 {
        let (start, highest) = fall_from_rest(&mut figure)?;
        if !figure.is_ragdoll() {
            return Err(format!("cycle {cycle}: R did not enter the ragdoll"));
        }
        if highest - start > MAX_RISE {
            return Err(format!(
                "cycle {cycle}: the figure rose {:.3} m after activation",
                highest - start
            ));
        }
        figure.press(Key::Animate)?;
        if figure.is_ragdoll() {
            return Err(format!("cycle {cycle}: T did not leave the ragdoll"));
        }
        let entity = figure.entity;
        if figure.world.get_component::<RagdollRig>(entity).is_some() {
            return Err(format!("cycle {cycle}: the ragdoll rig was not removed"));
        }
        if figure
            .world
            .get_component::<CharacterController>(entity)
            .is_none()
        {
            return Err(format!("cycle {cycle}: the controller was not restored"));
        }
        if figure.world.get_component::<CharacterPoseMode>(entity)
            != Some(&CharacterPoseMode::Animated)
        {
            return Err(format!("cycle {cycle}: the pose mode is not Animated"));
        }
    }
    // Let the last drop come to rest on the floor (the hips end low).
    figure.press(Key::Ragdoll)?;
    for _ in 0..SHOT_FALLING_STEPS {
        figure.step();
    }
    let hips = figure.hips_height();
    if hips >= 0.4 {
        return Err(format!("the hips are still {hips:.2} m high after 3 s"));
    }
    Ok(())
}

/// Renders the figure standing, drops it, and saves the picture after it has fallen.
fn screenshot(path: &str) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|error| format!("no GPU adapter: {error:?}"))?;
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::PIPELINE_CACHE
        | wgpu::Features::INDIRECT_FIRST_INSTANCE
        | redox_render::profiler::PROFILER_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        required_limits: redox_render::renderer::required_device_limits(&adapter.limits()),
        ..Default::default()
    }))
    .map_err(|error| format!("no device: {error:?}"))?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: SHOT_WIDTH,
        height: SHOT_HEIGHT,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    let mut r = Renderer::from_shared_device(device, queue, &adapter, None, config);
    let stage = Stage::new(&mut r);
    let mut figure = Figure::new()?;
    for _ in 0..SHOT_STANDING_STEPS {
        figure.step();
    }
    figure.press(Key::Ragdoll)?;
    for _ in 0..SHOT_FALLING_STEPS {
        figure.step();
    }

    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ragdoll_shot"),
        size: wgpu::Extent3d {
            width: SHOT_WIDTH,
            height: SHOT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    // A few frames let the renderer's caches settle before the picture is read back.
    for _ in 0..3 {
        render_objects(
            &mut r,
            &stage,
            &figure,
            SHOT_WIDTH as f32 / SHOT_HEIGHT as f32,
            &view,
        )?;
        let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let pixels = read_texture_ktx2(&r.device, &r.queue, &target, false)
        .ok_or("read back: no pixels")?
        .levels
        .swap_remove(0);
    image::save_buffer(
        path,
        &pixels,
        SHOT_WIDTH,
        SHOT_HEIGHT,
        image::ColorType::Rgba8,
    )
    .map_err(|error| format!("save {path}: {error}"))?;
    println!("saved {path}: hips at {:.2} m", figure.hips_height());
    Ok(())
}

/// Opens the window; `R` drops the figure, `T` stands it up again.
fn play() -> Result<(), String> {
    let config = EngineConfig {
        window_title: "RedOx - Ragdoll (R: drop, T: stand up)".to_string(),
        ..Default::default()
    };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    let stage = Stage::new(&mut ctx);
    let mut figure = Figure::new()?;

    event_loop
        .run_closure(move |event, elwt| match event {
            winit::event::Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput { event: key, .. }
                    if key.state == ElementState::Pressed && !key.repeat =>
                {
                    let pressed = match key.physical_key {
                        PhysicalKey::Code(KeyCode::KeyR) => Some(Key::Ragdoll),
                        PhysicalKey::Code(KeyCode::KeyT) => Some(Key::Animate),
                        _ => None,
                    };
                    if let Some(pressed) = pressed {
                        if let Err(error) = figure.press(pressed) {
                            eprintln!("ragdoll_window: {error}");
                            elwt.exit();
                        }
                    }
                }
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                WindowEvent::RedrawRequested => {
                    figure.step();
                    if let Err(error) = draw_window(&mut ctx, &stage, &figure) {
                        eprintln!("ragdoll_window: {error}");
                    }
                }
                _ => {}
            },
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .map_err(|error| format!("event loop: {error:?}"))
}

/// Acquires the surface and draws one frame into it.
fn draw_window(ctx: &mut RenderContext, stage: &Stage, figure: &Figure) -> Result<(), String> {
    let output = match redox_render::acquire_surface_texture(ctx.surface()) {
        Ok(output) => output,
        Err(redox_render::SurfaceError::Outdated | redox_render::SurfaceError::Lost) => {
            let (width, height) = (ctx.config.width, ctx.config.height);
            ctx.resize(width, height);
            return Ok(());
        }
        Err(redox_render::SurfaceError::Timeout | redox_render::SurfaceError::Occluded) => {
            return Ok(());
        }
        Err(redox_render::SurfaceError::Validation) => return Err("surface validation".into()),
    };
    let view = output.texture.create_view(&Default::default());
    let aspect = ctx.config.width as f32 / ctx.config.height.max(1) as f32;
    render_objects(ctx, stage, figure, aspect, &view)?;
    ctx.queue().present(output);
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--selftest") {
        return match selftest() {
            Ok(()) => {
                println!("ragdoll_window: selftest ok");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("ragdoll_window: selftest failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(at) = args.iter().position(|arg| arg == "--shot") {
        let out = args
            .get(at + 1)
            .map_or("ragdoll_window.png", String::as_str);
        return match screenshot(out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ragdoll_window: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match play() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ragdoll_window: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_drops_an_animated_figure_once() {
        let mut toggle = RagdollToggle::default();
        assert_eq!(toggle.press(Key::Ragdoll), Some(Change::Enter));
        assert!(toggle.is_ragdoll());
        assert_eq!(
            toggle.press(Key::Ragdoll),
            None,
            "a second R changes nothing"
        );
        assert!(toggle.is_ragdoll());
    }

    #[test]
    fn t_stands_a_ragdolled_figure_up_and_ignores_an_animated_one() {
        let mut toggle = RagdollToggle::default();
        assert_eq!(
            toggle.press(Key::Animate),
            None,
            "T on a standing figure does nothing"
        );
        assert!(!toggle.is_ragdoll());
        toggle.press(Key::Ragdoll);
        assert_eq!(toggle.press(Key::Animate), Some(Change::Exit));
        assert!(!toggle.is_ragdoll());
    }

    #[test]
    fn the_toggle_alternates_over_many_presses() {
        let mut toggle = RagdollToggle::default();
        let changes: Vec<Option<Change>> = [
            Key::Ragdoll,
            Key::Ragdoll,
            Key::Animate,
            Key::Animate,
            Key::Ragdoll,
        ]
        .into_iter()
        .map(|key| toggle.press(key))
        .collect();
        assert_eq!(
            changes,
            vec![
                Some(Change::Enter),
                None,
                Some(Change::Exit),
                None,
                Some(Change::Enter)
            ]
        );
    }

    #[test]
    fn a_figure_stays_put_while_animated() {
        let mut figure = Figure::new().unwrap();
        let start = figure.mean_height();
        for _ in 0..60 {
            figure.step();
        }
        assert!((figure.mean_height() - start).abs() < 1e-6);
        assert!(!figure.is_ragdoll());
    }

    #[test]
    fn the_selftest_passes() {
        selftest().unwrap();
    }
}
