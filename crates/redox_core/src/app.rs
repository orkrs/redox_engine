//! Engine runner and Application builder.

// winit 0.30 deprecates the closure event loop; migrating to `ApplicationHandler` is tracked in the roadmap.
#![allow(deprecated)]

use std::sync::Arc;
use std::time::Instant;

use winit::event::{Event, WindowEvent};
use winit::window::Window;

use redox_ecs::World;
use redox_input::InputState;
use redox_render::context::RenderContext;

use crate::config::EngineConfig;
use crate::dispatcher::{Dispatcher, Stage};
use crate::plugin::Plugin;
use crate::time::Time;
use crate::window;

type SetupHook = Box<dyn FnOnce(&mut World)>;
type UiSetupHook = Box<dyn FnOnce(&mut World, &mut RenderContext, Arc<Window>)>;

/// A builder linking the context and core systems to launch the engine.
pub struct AppBuilder {
    config: EngineConfig,
    dispatcher: Dispatcher<RenderContext>,
    /// Hooks executed once after `World` creation, before the main loop.
    setup_hooks: Vec<SetupHook>,
    /// Hooks executed after renderer creation (UI contexts need device/format).
    post_render_hooks: Vec<UiSetupHook>,
}

impl AppBuilder {
    /// Creates a new `AppBuilder` configured with the supplied settings.
    pub fn new(config: EngineConfig) -> Self {
        let mut this = Self {
            config,
            dispatcher: Dispatcher::new(),
            setup_hooks: Vec::new(),
            post_render_hooks: Vec::new(),
        };
        crate::plugin::CorePlugin.build(&mut this);
        this
    }

    /// Appends a new system to the application loop.
    pub fn add_system<F>(mut self, stage: Stage, system: F) -> Self
    where
        F: FnMut(&mut World, &mut RenderContext) + 'static,
    {
        self.add_system_mut(stage, system);
        self
    }

    /// Same as [`Self::add_system`] but for [`Plugin::build`](crate::plugin::Plugin).
    pub fn add_system_mut<F>(&mut self, stage: Stage, system: F)
    where
        F: FnMut(&mut World, &mut RenderContext) + 'static,
    {
        self.dispatcher.add_system(stage, system);
    }

    /// Applies a [`crate::plugin::Plugin`].
    pub fn add_plugin(mut self, plugin: impl crate::plugin::Plugin) -> Self {
        plugin.build(&mut self);
        self
    }

    /// One-shot world setup after the ECS world is created.
    pub fn on_setup<F>(&mut self, hook: F)
    where
        F: FnOnce(&mut World) + 'static,
    {
        self.setup_hooks.push(Box::new(hook));
    }

    /// Registers an event queue for per-frame aging.
    ///
    /// Call once per event type; [`redox_ecs::Events::update`] is then invoked
    /// automatically at the end of every frame.
    pub fn add_events<T>(mut self) -> Self
    where
        T: 'static + Send + Sync,
    {
        self.register_events::<T>();
        self
    }

    pub fn register_events<T>(&mut self)
    where
        T: 'static + Send + Sync,
    {
        self.dispatcher.add_frame_end_hook(|world| {
            if let Some(events) = world.get_resource_mut::<redox_ecs::Events<T>>() {
                events.update();
            }
        });
    }

    /// Enables physics: inserts a [`redox_physics::PhysicsContext`] resource and
    /// registers the ECS↔Rapier sync + fixed-step systems on `Stage::PhysicsSync`.
    pub fn with_physics(self, gravity: redox_math::Vec3) -> Self {
        self.add_plugin(crate::plugin::PhysicsPlugin { gravity })
    }

    /// Enables audio: inserts a [`redox_audio::AudioContext`] resource.
    ///
    /// The context degrades to no-ops when no audio device is available.
    pub fn with_audio(self) -> Self {
        self.add_plugin(crate::plugin::AudioPlugin)
    }

    /// Enables the egui debug UI: creates a [`redox_ui::UiContext`] after the
    /// renderer is initialised (it needs the device and surface format).
    pub fn with_ui(mut self) -> Self {
        self.post_render_hooks.push(Box::new(|world, ctx, window| {
            let ui = redox_ui::UiContext::new(&window, ctx.device(), ctx.surface_format());
            world.insert_resource(ui);
        }));
        self
    }

    /// Convenience: registers a one-shot World setup hook, then runs the app.
    pub fn run_with_setup<F>(mut self, setup: F) -> !
    where
        F: FnOnce(&mut World) + 'static,
    {
        self.setup_hooks.push(Box::new(setup));
        self.run()
    }

    /// Initializes logging, GPU context, and begins the main event loop.
    ///
    /// The event loop will never return (hence `-> !`).
    pub fn run(mut self) -> ! {
        // 1. Init logger
        // try_init: a host (editor, tests) may have installed a logger already.
        let _ = env_logger::try_init();
        log::info!("RedOx Engine Starting up...");

        // 2. Create window & event loop
        let (event_loop, window_arc) = window::create_window(&self.config);

        // 3. Init RenderContext (async -> sync via pollster), honoring config
        let mut render_context = pollster::block_on(RenderContext::new(
            window_arc.clone(),
            self.config.wgpu_present_mode(),
        ));
        render_context.set_lighting_mode(self.config.lighting_mode.into());
        render_context.set_aa_sharp(self.config.aa_sharp);
        render_context.set_render_scale(self.config.render_scale);

        // 4. Create ECS World with core resources
        let mut world = World::new();
        world.insert_resource(InputState::new());
        world.insert_resource(Time::new(self.config.fixed_timestep));

        for hook in self.setup_hooks.drain(..) {
            hook(&mut world);
        }
        for hook in self.post_render_hooks.drain(..) {
            hook(&mut world, &mut render_context, window_arc.clone());
        }

        // 5. Event loop execution
        let mut last_frame_time = Instant::now();

        event_loop
            .run(move |event, elwt| {
                match event {
                    Event::NewEvents(_) => {}
                    Event::WindowEvent { event, .. } => {
                        if let Some(input) = world.get_resource_mut::<InputState>() {
                            input.process_window_event(&event);
                        }

                        match event {
                            WindowEvent::CloseRequested => {
                                log::info!("Close requested, shutting down.");
                                elwt.exit()
                            }
                            WindowEvent::Resized(size) => {
                                render_context.resize(size.width, size.height);
                            }
                            WindowEvent::RedrawRequested => {
                                let now = Instant::now();
                                let delta = now.duration_since(last_frame_time).as_secs_f32();
                                last_frame_time = now;

                                // Tick time in its resource slot (delta clamped inside).
                                if let Some(time) = world.get_resource_mut::<Time>() {
                                    time.tick(delta);
                                }

                                self.dispatcher.run(&mut world, &mut render_context);

                                // After systems consume `just_pressed`, so the next frame's
                                // keyboard events aren't wiped by NewEvents-before-Redraw order.
                                if let Some(input) = world.get_resource_mut::<InputState>() {
                                    input.begin_frame();
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::AboutToWait => {
                        window_arc.request_redraw();
                    }
                    _ => {}
                }
            })
            .unwrap();

        std::process::exit(0)
    }
}
