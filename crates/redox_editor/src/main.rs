//! RedOx Studio editor binary: CLI entry point; the window loop lives in [`app`].

mod app;
mod cli;
mod commands;
mod headless;
mod render_host;

use cli::{HELP_TEXT, parse_cli, run_bake};
use redox_core::window::RunClosure;
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cli = match parse_cli(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}\n{HELP_TEXT}");
            return ExitCode::from(2);
        }
    };
    if cli.help {
        println!("{HELP_TEXT}");
        return ExitCode::SUCCESS;
    }
    if let Some(root) = &cli.validate_project {
        return ExitCode::from(headless::validate_project_root(root.as_ref()));
    }
    if let Some(root) = &cli.run_sample {
        return ExitCode::from(headless::run_sample(root.as_ref(), cli.sample_frames));
    }
    if cli.bake {
        if let Err(e) = run_bake(&cli.world_root) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    env_logger::init();
    log::info!("RedOx Studio Editor starting...");

    let event_loop =
        winit::event_loop::EventLoop::new().expect("failed to create the winit event loop");
    let window = Arc::new(
        #[allow(deprecated)]
        event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_title("RedOx Studio — Editor")
                    .with_inner_size(winit::dpi::LogicalSize::new(1600, 900)),
            )
            .expect("failed to create the editor window"),
    );

    let mut app = app::EditorApp::new(window, &cli);
    let _ = event_loop.run_closure(move |event, elwt| app.handle_event(event, elwt));
    ExitCode::SUCCESS
}
