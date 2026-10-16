//! Plugin pattern: `App::new().add_plugin(PhysicsPlugin::default()).run()`.

use redox_ecs::{Events, World, transform_hierarchy_system};
use redox_math::Vec3;

use crate::app::AppBuilder;
use crate::config::EngineConfig;
use crate::dispatcher::Stage;
use crate::time::Time;

/// Installs systems and resources onto an [`AppBuilder`].
pub trait Plugin {
    fn build(&self, app: &mut AppBuilder);
}

/// `App::new()` → [`AppBuilder`] with default [`EngineConfig`].
pub struct App;

impl App {
    pub fn new() -> AppBuilder {
        AppBuilder::new(EngineConfig::default())
    }
}

/// Transform hierarchy (`Parent`/`Children` → `GlobalTransform`) on `PostUpdate`.
pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.add_system_mut(Stage::PostUpdate, |world, _| {
            transform_hierarchy_system(world);
        });
    }
}

/// Rapier world, KCC, trigger events, raycast results.
pub struct PhysicsPlugin {
    pub gravity: Vec3,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let gravity = self.gravity;
        app.on_setup(move |world| {
            world.insert_resource(redox_physics::PhysicsContext::new(gravity));
            world.insert_resource(Events::<redox_physics::RaycastRequest>::new());
            world.insert_resource(Events::<redox_physics::RaycastResult>::new());
            world.insert_resource(Events::<redox_physics::TriggerEnter>::new());
            world.insert_resource(Events::<redox_physics::TriggerExit>::new());
        });
        app.register_events::<redox_physics::RaycastRequest>();
        app.register_events::<redox_physics::RaycastResult>();
        app.register_events::<redox_physics::TriggerEnter>();
        app.register_events::<redox_physics::TriggerExit>();
        app.add_system_mut(Stage::PhysicsSync, |world, _| {
            redox_physics::sync_to_physics(world);
            let dt = world
                .get_resource::<Time>()
                .map(|t| t.fixed_delta_time)
                .unwrap_or(1.0 / 60.0);
            redox_physics::tick_physics(world, dt);
            redox_physics::sync_from_physics(world);
        });
    }
}

/// Audio context, occlusion raycasts, listener pose, lowpass.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.on_setup(|world| {
            world.insert_resource(redox_audio::AudioContext::new());
        });
        app.add_system_mut(Stage::PostUpdate, |world, _| {
            redox_audio::occlusion_raycast_system(world);
            sync_audio_listener(world);
            sync_audio_reverb(world);
        });
    }
}

/// Updates reverb zone weights from the listener pose and feeds the blended preset to kira.
fn sync_audio_reverb(world: &mut World) {
    redox_audio::reverb_listener_system(world);
    let preset = redox_audio::compute_active_reverb(world);
    if let Some(audio) = world.get_resource_mut::<redox_audio::AudioContext>() {
        audio.apply_reverb(preset.as_ref());
    }
}

fn sync_audio_listener(world: &mut World) {
    use redox_audio::AudioListener;
    use redox_math::Quat;

    let pose = world.all_entities().find_map(|e| {
        world.get_component::<AudioListener>(e).map(|l| {
            let forward = if l.forward.length_squared() > 1e-8 {
                l.forward.normalize()
            } else {
                redox_math::Vec3::NEG_Z
            };
            let orientation = Quat::from_rotation_arc(redox_math::Vec3::NEG_Z, forward);
            (l.position, orientation)
        })
    });
    let Some((position, orientation)) = pose else {
        return;
    };
    if let Some(audio) = world.get_resource_mut::<redox_audio::AudioContext>() {
        audio.set_listener_pose(position, orientation);
    }
}

/// GPU renderer is created in [`AppBuilder::run`]; this plugin is the explicit hook.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, _app: &mut AppBuilder) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_install_without_window() {
        let mut app = App::new();
        CorePlugin.build(&mut app);
        PhysicsPlugin::default().build(&mut app);
        AudioPlugin.build(&mut app);
        RenderPlugin.build(&mut app);
        let _ = app;
    }
}
