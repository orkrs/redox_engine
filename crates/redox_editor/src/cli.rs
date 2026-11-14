//! Command line parsing and the headless `--bake` mode.

pub(crate) const DEFAULT_WORLD_ROOT: &str = "worlds/arena";

pub(crate) const HELP_TEXT: &str = "RedOx Studio Editor\n  --help            Show this help\n  --bake            Bake meshlets + SDF and write a report (headless, then exit)\n  --play <project>  Headless PIE host for standalone play (stub; no window)\n  --bench <project> --frames N --out FILE   Write benchmark JSON (headless)\n  --world <path>    World root (default worlds/arena)";

/// Parsed command line. `--world <path>` selects the world root.
#[derive(Debug)]
pub(crate) struct CliArgs {
    pub(crate) help: bool,
    pub(crate) bake: bool,
    /// Project directory for `--play` standalone host.
    pub(crate) play_project: Option<String>,
    /// Benchmark project path.
    pub(crate) bench_project: Option<String>,
    /// Frames to record for `--bench`.
    pub(crate) bench_frames: u32,
    /// Output path for `--bench`.
    pub(crate) bench_out: Option<String>,
    pub(crate) world_root: String,
}

pub(crate) fn parse_cli(args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        help: false,
        bake: false,
        play_project: None,
        bench_project: None,
        bench_frames: 120,
        bench_out: None,
        world_root: DEFAULT_WORLD_ROOT.to_string(),
    };
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => cli.help = true,
            "--bake" => cli.bake = true,
            "--play" => {
                cli.play_project = Some(
                    iter.next()
                        .ok_or_else(|| "--play requires a project path".to_string())?
                        .clone(),
                );
            }
            "--bench" => {
                cli.bench_project = Some(
                    iter
                        .next()
                        .ok_or_else(|| "--bench requires a project path".to_string())?
                        .clone(),
                );
            }
            "--frames" => {
                cli.bench_frames = iter
                    .next()
                    .ok_or_else(|| "--frames requires a number".to_string())?
                    .parse()
                    .map_err(|_| "--frames must be an integer".to_string())?;
            }
            "--out" => {
                cli.bench_out = Some(
                    iter.next()
                        .ok_or_else(|| "--out requires a path".to_string())?
                        .clone(),
                );
            }
            "--world" => {
                cli.world_root = iter
                    .next()
                    .ok_or_else(|| "--world requires a path".to_string())?
                    .clone();
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(cli)
}

/// Headless bake: meshlet cache, SDF field and the OFPA scan report. No window or GPU.
pub(crate) fn run_bake(world_root: &str) -> Result<(), String> {
    let mut packager = redox_editor_ui::packager::PackagerPanel::new(world_root);
    let steps: [(&str, Result<String, anyhow::Error>); 3] = [
        ("meshlets", packager.bake_meshlets()),
        ("sdf", packager.bake_sdf()),
        ("report", packager.write_bake_report()),
    ];
    let mut failed = false;
    for (name, result) in steps {
        match result {
            Ok(msg) => println!("[bake:{name}] {msg}"),
            Err(e) => {
                eprintln!("[bake:{name}] FAILED: {e:#}");
                failed = true;
            }
        }
    }
    if failed {
        Err("one or more bake steps failed".to_string())
    } else {
        Ok(())
    }
}

/// Headless PIE host stub for standalone play (no game binary in-tree).
/// Headless benchmark: writes JSON with dummy frame samples (coordinator runs live GPU bench).
pub(crate) fn run_bench(project: &str, frames: u32, out: &str) -> Result<(), String> {
    let path = std::path::Path::new(project);
    if !path.is_dir() {
        return Err(format!("{} is not a project directory", path.display()));
    }
    let mut report = redox_editor_debugger::BenchReport::from_dummy_frames(project, frames);
    if let Ok(hash) = std::fs::read_to_string(path.join(".git/HEAD")) {
        report.meta.git_commit = Some(hash.trim().to_string());
    }
    let out_path = std::path::Path::new(out);
    report
        .write_json(out_path)
        .map_err(|e| format!("write bench json: {e}"))?;
    if out_path.extension().and_then(|s| s.to_str()) == Some("html") {
        report
            .write_html_stub(out_path)
            .map_err(|e| format!("write bench html: {e}"))?;
    }
    println!(
        "[bench] wrote {} ({} frames, avg {:.1} FPS)",
        out_path.display(),
        frames,
        report.avg_fps
    );
    Ok(())
}

pub(crate) fn run_play_host(project: &str) -> Result<(), String> {
    let path = std::path::Path::new(project);
    if !path.is_dir() {
        return Err(format!("{} is not a project directory", path.display()));
    }
    println!(
        "[play] standalone host stub: would run PIE on {} (coordinator wires real loop)",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("redox_editor")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn defaults_without_arguments() {
        let cli = parse_cli(&args(&[])).unwrap();
        assert!(!cli.help && !cli.bake);
        assert_eq!(cli.world_root, DEFAULT_WORLD_ROOT);
    }

    #[test]
    fn parses_flags_and_world_path() {
        let cli = parse_cli(&args(&["--bake", "--world", "worlds/test"])).unwrap();
        assert!(cli.bake);
        assert_eq!(cli.world_root, "worlds/test");
        assert!(parse_cli(&args(&["-h"])).unwrap().help);
    }

    #[test]
    fn parses_bench_flags() {
        let cli = parse_cli(&args(&[
            "--bench",
            "projects/demo",
            "--frames",
            "240",
            "--out",
            "bench.json",
        ]))
        .unwrap();
        assert_eq!(cli.bench_project.as_deref(), Some("projects/demo"));
        assert_eq!(cli.bench_frames, 240);
        assert_eq!(cli.bench_out.as_deref(), Some("bench.json"));
    }

    #[test]
    fn rejects_unknown_arguments_and_missing_values() {
        assert!(
            parse_cli(&args(&["--nope"]))
                .unwrap_err()
                .contains("--nope")
        );
        assert!(parse_cli(&args(&["--world"])).is_err());
    }
}
