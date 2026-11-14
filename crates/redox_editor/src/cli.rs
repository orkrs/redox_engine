//! Command line parsing and the headless `--bake` mode.

pub(crate) const DEFAULT_WORLD_ROOT: &str = "worlds/arena";

pub(crate) const HELP_TEXT: &str = "RedOx Studio Editor\n  --help                  Show this help\n  --bake                  Bake meshlets + SDF and write a report (headless, then exit)\n  --world <path>          World root (default worlds/arena)\n  --validate-project <root>  Validate project.ron, assets and start scene; exit 0 on success\n  --run-sample <root>     Validate and load the start scene (add --frames N; GPU stepping 📌)";

/// Parsed command line. `--world <path>` selects the world root.
#[derive(Debug)]
pub(crate) struct CliArgs {
    pub(crate) help: bool,
    pub(crate) bake: bool,
    pub(crate) world_root: String,
    pub(crate) validate_project: Option<String>,
    pub(crate) run_sample: Option<String>,
    pub(crate) sample_frames: u32,
}

pub(crate) fn parse_cli(args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        help: false,
        bake: false,
        world_root: DEFAULT_WORLD_ROOT.to_string(),
        validate_project: None,
        run_sample: None,
        sample_frames: 0,
    };
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => cli.help = true,
            "--bake" => cli.bake = true,
            "--world" => {
                cli.world_root = iter
                    .next()
                    .ok_or_else(|| "--world requires a path".to_string())?
                    .clone();
            }
            "--validate-project" => {
                cli.validate_project = Some(
                    iter.next()
                        .ok_or_else(|| "--validate-project requires a path".to_string())?
                        .clone(),
                );
            }
            "--run-sample" => {
                cli.run_sample = Some(
                    iter.next()
                        .ok_or_else(|| "--run-sample requires a path".to_string())?
                        .clone(),
                );
            }
            "--frames" => {
                let text = iter
                    .next()
                    .ok_or_else(|| "--frames requires a number".to_string())?;
                cli.sample_frames = text
                    .parse()
                    .map_err(|_| format!("invalid --frames value: {text}"))?;
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
    fn rejects_unknown_arguments_and_missing_values() {
        assert!(
            parse_cli(&args(&["--nope"]))
                .unwrap_err()
                .contains("--nope")
        );
        assert!(parse_cli(&args(&["--world"])).is_err());
    }
}
