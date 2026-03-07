//! Engine runner and Application builder.

use std::time::Instant;

use winit::event::{Event, WindowEvent};

use redox_ecs::World;
use redox_render::context::RenderContext;

use crate::config::EngineConfig;
use crate::dispatcher::{Dispatcher, Stage};
use crate::time::Time;
use crate::window;

/// A builder linking the context and core systems to launch the engine.
pub struct AppBuilder {
    config: EngineConfig,
    dispatcher: Dispatcher<RenderContext>,
}

impl AppBuilder {
    /// Creates a new `AppBuilder` configured with the supplied settings.
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            dispatcher: Dispatcher::new(),
        }
    }

    /// Appends a new system to the application loop.
    pub fn add_system<F>(mut self, stage: Stage, system: F) -> Self
    where
        F: FnMut(&mut World, &mut RenderContext) + 'static,
    {
        self.dispatcher.add_system(stage, system);
        self
    }

    /// Initializes logging, GPU context, and begins the main event loop.
    ///
    /// The event loop will never return (hence `-> !`).
    pub fn run(mut self) -> ! {
        // 1. Init logger
        env_logger::init();
        log::info!("RedOx Engine Starting up...");

        // 2. Create window & event loop
        let (event_loop, window_arc) = window::create_window(&self.config);

        // 3. Init RenderContext (async -> sync via pollster)
        let mut render_context = pollster::block_on(RenderContext::new(window_arc.clone()));

        // 4. Create ECS World
        let mut world = World::new();

        // 5. Create Time resource
        let mut time = Time::new(1.0 / 60.0);

        // Track frame times
        let mut last_frame_time = Instant::now();

        // 7. Event loop execution
        event_loop
            .run(move |event, elwt| {
                match event {
                    Event::WindowEvent {
                        event: WindowEvent::CloseRequested,
                        ..
                    } => {
                        log::info!("Close requested, shutting down.");
                        elwt.exit()
                    }
                    Event::WindowEvent {
                        event: WindowEvent::Resized(size),
                        ..
                    } => {
                        render_context.resize(size.width, size.height);
                    }
                    Event::AboutToWait => {
                        window_arc.request_redraw();
                    }
                    Event::WindowEvent {
                        event: WindowEvent::RedrawRequested,
                        ..
                    } => {
                        let now = Instant::now();
                        let delta = now.duration_since(last_frame_time).as_secs_f32();
                        last_frame_time = now;

                        time.tick(delta);

                        // 6. & 7. Execute all scheduled systems via dispatcher
                        self.dispatcher
                            .run(&mut world, &mut render_context, &mut time);

                        // If surface was lost during render execution, resizing resets it
                    }
                    _ => {}
                }
            })
            .unwrap();

        // Note: With winit v0.30, .run() returns Result<()>.
        // Since we want `-> !` we just exit. For standard return simply remove `-> !`.
        std::process::exit(0)
    }
}
