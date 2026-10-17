//! Window creation and management via `winit`.

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::error::EventLoopError;
use winit::event::{DeviceEvent, DeviceId, Event, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

use crate::config::EngineConfig;

/// Creates an event loop and a configured winit window.
pub fn create_window(config: &EngineConfig) -> (EventLoop<()>, Arc<Window>) {
    let event_loop = EventLoop::new().expect("failed to create the winit event loop");
    let mut builder = Window::default_attributes()
        .with_title(config.window_title.clone())
        .with_inner_size(LogicalSize::new(
            config.window_width as f64,
            config.window_height as f64,
        ))
        .with_resizable(config.resizable);

    if config.fullscreen {
        builder = builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    // winit 0.30 prefers creating windows inside `ApplicationHandler::resumed`. The engine and
    // its examples need the window (and the GPU context built from it) before the loop starts,
    // which is valid on Windows, so this one call opts out of the deprecation.
    #[allow(deprecated)]
    let window = event_loop
        .create_window(builder)
        .expect("Failed to create winit window");

    (event_loop, Arc::new(window))
}

/// Runs an event loop with a closure over winit [`Event`]s, through the non-deprecated
/// `ApplicationHandler` API.
pub trait RunClosure {
    /// Same contract as the deprecated `EventLoop::run`: blocks until the loop exits.
    fn run_closure<F>(self, handler: F) -> Result<(), EventLoopError>
    where
        F: FnMut(Event<()>, &ActiveEventLoop);
}

struct ClosureApp<F>(F);

impl<F: FnMut(Event<()>, &ActiveEventLoop)> ApplicationHandler for ClosureApp<F> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        (self.0)(Event::NewEvents(cause), event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        (self.0)(Event::Resumed, event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        (self.0)(Event::WindowEvent { window_id, event }, event_loop);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        (self.0)(Event::DeviceEvent { device_id, event }, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        (self.0)(Event::AboutToWait, event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        (self.0)(Event::Suspended, event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        (self.0)(Event::LoopExiting, event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        (self.0)(Event::MemoryWarning, event_loop);
    }
}

impl RunClosure for EventLoop<()> {
    fn run_closure<F>(self, handler: F) -> Result<(), EventLoopError>
    where
        F: FnMut(Event<()>, &ActiveEventLoop),
    {
        self.run_app(&mut ClosureApp(handler))
    }
}
