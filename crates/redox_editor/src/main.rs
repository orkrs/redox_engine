//! RedOx Studio editor binary: CLI entry point; the window loop lives in [`app`].

mod app;
mod cli;
mod commands;
mod render_host;

use cli::{HELP_TEXT, parse_cli, run_bake};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = match parse_cli(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}\n{HELP_TEXT}");
            std::process::exit(2);
        }
    };
    if cli.help {
        println!("{HELP_TEXT}");
        return;
    }
    if cli.bake {
        if let Err(e) = run_bake(&cli.world_root) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    env_logger::init();
    log::info!("RedOx Studio Editor starting...");

    let event_loop = winit::event_loop::EventLoop::new().unwrap();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("RedOx Studio — Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1600, 900))
            .build(&event_loop)
            .unwrap(),
    );

    let mut app = app::EditorApp::new(window, &cli);
    let _ = event_loop.run(move |event, elwt| app.handle_event(event, elwt));
}
