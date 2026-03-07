//! Core subsystem linking the window, engine time, and system dispatcher.
//!
//! # Main Example Implementation
//!
//! ```no_run
//! use redox_core::config::EngineConfig;
//! use redox_core::app::AppBuilder;
//! use redox_core::dispatcher::Stage;
//! use redox_render::systems::RenderObject;
//!
//! // Assuming your RenderContext has an internal buffer to push render objects in RenderPrep stage:
//! // fn main() {
//! //     let config = EngineConfig::default();
//! //     let app = AppBuilder::new(config)
//! //         .add_system(Stage::Update, |world, context| {
//! //             // Iterate over components, rotate Transforms, etc.
//! //         })
//! //         .add_system(Stage::RenderPrep, |world, context| {
//! //             // Fetch all Entities with Transform, MeshHandle, MaterialHandle
//! //             // Build RenderObject instances
//! //             // context.clear_render_objects();
//! //             // context.push_render_object(RenderObject { ... });
//! //         })
//! //         .add_system(Stage::Render, |world, context| {
//! //             // context.render_frame(&context.render_objects).unwrap();
//! //         });
//! //
//! //     app.run();
//! // }
//! ```
//!

pub mod app;
pub mod config;
pub mod dispatcher;
pub mod time;
pub mod window;

pub use app::AppBuilder;
pub use config::EngineConfig;
pub use dispatcher::{Dispatcher, Stage};
pub use time::Time;
