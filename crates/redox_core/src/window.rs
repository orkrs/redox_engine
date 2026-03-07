//! Window creation and management via `winit`.

use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event_loop::EventLoop;
use winit::window::{Fullscreen, Window, WindowBuilder};

use crate::config::EngineConfig;

/// Creates an event loop and a configured winit window.
pub fn create_window(config: &EngineConfig) -> (EventLoop<()>, Arc<Window>) {
    let event_loop = EventLoop::new().unwrap();
    let mut builder = WindowBuilder::new()
        .with_title(config.window_title.clone())
        .with_inner_size(LogicalSize::new(
            config.window_width as f64,
            config.window_height as f64,
        ))
        .with_resizable(config.resizable);

    if config.fullscreen {
        builder = builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let window = builder
        .build(&event_loop)
        .expect("Failed to create winit window");

    (event_loop, Arc::new(window))
}
