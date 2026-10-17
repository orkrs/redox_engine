//! `redox_cook` - asset pipeline command line tool.
//!
//! ```text
//! redox_cook scan <assets-dir>             create/repair .meta sidecars (GUIDs)
//! redox_cook pack <assets-dir> <out.pak>   pack a folder into a deterministic archive
//! redox_cook list <file.pak>               print the entries of an archive
//! redox_cook verify <file.pak>             read every entry and check its hash
//! ```
//!
//! `.meta` sidecars are not packed (they are editor data); everything else is.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_asset::{AssetRegistry, PakReader, PakWriter};

const USAGE: &str = "usage:\n  redox_cook scan <assets-dir>\n  redox_cook pack <assets-dir> <out.pak>\n  redox_cook list <file.pak>\n  redox_cook verify <file.pak>";

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Scan(PathBuf),
    Pack { dir: PathBuf, out: PathBuf },
    List(PathBuf),
    Verify(PathBuf),
}

fn parse(args: &[String]) -> Result<Command, String> {
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    match rest.as_slice() {
        ["scan", dir] => Ok(Command::Scan(dir.into())),
        ["pack", dir, out] => Ok(Command::Pack {
            dir: dir.into(),
            out: out.into(),
        }),
        ["list", pak] => Ok(Command::List(pak.into())),
        ["verify", pak] => Ok(Command::Verify(pak.into())),
        [] => Err("missing command".into()),
        [cmd, ..] if !["scan", "pack", "list", "verify"].contains(cmd) => {
            Err(format!("unknown command: {cmd}"))
        }
        _ => Err("wrong number of arguments".into()),
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Scan(dir) => {
            let (registry, report) = AssetRegistry::scan(&dir).map_err(|e| e.to_string())?;
            println!(
                "{} assets ({} new, {} duplicate guids fixed, {} damaged sidecars)",
                registry.len(),
                report.created.len(),
                report.regenerated.len(),
                report.damaged.len()
            );
            for path in &report.damaged {
                eprintln!("damaged .meta: {}", path.display());
            }
            if report.damaged.is_empty() {
                Ok(())
            } else {
                Err("some sidecars are damaged".into())
            }
        }
        Command::Pack { dir, out } => {
            let mut writer = PakWriter::new();
            add_cooked(&mut writer, &dir)?;
            writer.write_to(&out).map_err(|e| e.to_string())?;
            println!("packed {} files into {}", writer.len(), out.display());
            Ok(())
        }
        Command::List(pak) => {
            let reader = PakReader::open(&pak).map_err(|e| e.to_string())?;
            for name in reader.names() {
                println!("{name}");
            }
            Ok(())
        }
        Command::Verify(pak) => {
            let reader = PakReader::open(&pak).map_err(|e| e.to_string())?;
            let mut bad = 0usize;
            let mut total = 0usize;
            for name in reader.names() {
                total += 1;
                if let Err(e) = reader.read(name) {
                    eprintln!("{name}: {e}");
                    bad += 1;
                }
            }
            if bad == 0 {
                println!("{total} entries ok");
                Ok(())
            } else {
                Err(format!("{bad} of {total} entries are damaged"))
            }
        }
    }
}

/// Adds every file below `dir` except `.meta` sidecars.
fn add_cooked(writer: &mut PakWriter, dir: &Path) -> Result<(), String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_none_or(|e| e != "meta") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
    for file in files {
        let rel = file
            .strip_prefix(dir)
            .unwrap_or(&file)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        writer.add(&rel, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let command = match parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("redox_cook")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn parses_every_command() {
        assert_eq!(parse(&args(&["scan", "a"])), Ok(Command::Scan("a".into())));
        assert_eq!(
            parse(&args(&["pack", "a", "o.pak"])),
            Ok(Command::Pack {
                dir: "a".into(),
                out: "o.pak".into()
            })
        );
        assert_eq!(
            parse(&args(&["list", "o.pak"])),
            Ok(Command::List("o.pak".into()))
        );
        assert_eq!(
            parse(&args(&["verify", "o.pak"])),
            Ok(Command::Verify("o.pak".into()))
        );
    }

    #[test]
    fn rejects_bad_command_lines() {
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["explode"])).unwrap_err().contains("unknown"));
        assert!(parse(&args(&["pack", "only-one"])).is_err());
        assert!(parse(&args(&["scan", "a", "b"])).is_err());
    }

    #[test]
    fn pack_skips_sidecars_and_verifies() {
        let dir = std::env::temp_dir().join(format!("redox_cook_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/sub")).unwrap();
        std::fs::write(dir.join("assets/a.txt"), b"a").unwrap();
        std::fs::write(dir.join("assets/sub/b.txt"), b"b").unwrap();
        std::fs::write(dir.join("assets/a.txt.meta"), b"meta").unwrap();
        let pak = dir.join("out.pak");

        run(Command::Pack {
            dir: dir.join("assets"),
            out: pak.clone(),
        })
        .unwrap();
        let reader = PakReader::open(&pak).unwrap();
        assert_eq!(reader.names().collect::<Vec<_>>(), ["a.txt", "sub/b.txt"]);
        run(Command::Verify(pak)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
