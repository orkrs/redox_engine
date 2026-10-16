//! Generate an OFPA world (`world.ron` + `cells/<cx>_<cz>/*.ron`).
//!
//! ```text
//! cargo run -p redox_scene --bin pcg -- --out worlds/arena --kind arena --width 8 --height 8 --seed 42
//! cargo run -p redox_scene --bin pcg -- --out worlds/dungeon --kind dungeon --width 12 --height 12
//! ```

use std::env;
use std::process::ExitCode;

use redox_scene::{generate_to, PcgKind, PcgParams};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut params = PcgParams::default();
    let mut out: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                out = Some(args.next().ok_or("--out needs a path")?);
            }
            "--kind" | "-k" => {
                let v = args.next().ok_or("--kind needs arena|dungeon")?;
                params.kind = PcgKind::parse(&v).ok_or_else(|| format!("unknown kind `{v}`"))?;
            }
            "--name" => {
                params.name = args.next().ok_or("--name needs a value")?;
            }
            "--width" | "-w" => {
                params.width = parse_i32(&next(&mut args, "--width")?)?;
            }
            "--height" | "-h" => {
                params.height = parse_i32(&next(&mut args, "--height")?)?;
            }
            "--cell-size" => {
                params.cell_size = parse_f32(&next(&mut args, "--cell-size")?)?;
            }
            "--seed" | "-s" => {
                params.seed = parse_u64(&next(&mut args, "--seed")?)?;
            }
            "--help" => {
                print_help();
                return Ok(());
            }
            other => return Err(format!("unknown arg `{other}`")),
        }
    }

    let out = out.ok_or("missing --out <dir>")?;
    if params.name == "world" {
        if let Some(stem) = std::path::Path::new(&out).file_name().and_then(|s| s.to_str()) {
            params.name = stem.to_string();
        }
    }
    if params.width < 1 || params.height < 1 {
        return Err("width/height must be >= 1".into());
    }

    let generated = generate_to(&out, &params).map_err(|e| e.to_string())?;
    let actors: usize = generated.cells.iter().map(|(_, a)| a.len()).sum();
    println!(
        "wrote {} ({} cells, {} actors, seed {}, kind {:?})",
        out,
        generated.cells.len(),
        actors,
        params.seed,
        params.kind
    );
    Ok(())
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_i32(s: &str) -> Result<i32, String> {
    s.parse().map_err(|_| format!("not an i32: {s}"))
}

fn parse_f32(s: &str) -> Result<f32, String> {
    s.parse().map_err(|_| format!("not an f32: {s}"))
}

fn parse_u64(s: &str) -> Result<u64, String> {
    s.parse().map_err(|_| format!("not a u64: {s}"))
}

fn print_help() {
    eprintln!(
        "\
pcg — RedOx world-partition generator

USAGE:
    cargo run -p redox_scene --bin pcg -- --out <dir> [options]

OPTIONS:
    --out, -o DIR        Output directory (writes world.ron + cells/)
    --kind, -k KIND      arena | dungeon          (default: arena)
    --name NAME          World name in world.ron  (default: output folder name)
    --width, -w N        Grid width in cells      (default: 8)
    --height, -h N       Grid height in cells     (default: 8)
    --cell-size F        Cell size in world units (default: 64)
    --seed, -s N         RNG seed                 (default: 1)
    --help               This text

Runtime load:
    WorldPartition::open(\"worlds/arena\")
    tick_streaming(world, viewer_pos);  // each frame"
    );
}
