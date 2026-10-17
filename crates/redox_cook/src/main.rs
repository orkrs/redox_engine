//! `redox_cook` - asset pipeline command line tool.
//!
//! ```text
//! redox_cook scan <assets-dir>             create/repair .meta sidecars (GUIDs)
//! redox_cook pack <assets-dir> <out.pak> [--compress] [--incremental]
//!                                          pack a folder into a deterministic archive
//! redox_cook list <file.pak>               print the entries of an archive
//! redox_cook verify <file.pak>             read every entry and check its hash
//! ```
//!
//! `.meta` sidecars are not packed (they are editor data); everything else is.
//! `--compress` stores entries LZ4-compressed where that helps. `--incremental` keeps a
//! `<out.pak>.stamp` file (path, size and modification time of every input plus the options)
//! and skips the work when nothing changed since the last run.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_asset::{AssetRegistry, PakReader, PakWriter};

const USAGE: &str = "usage:\n  redox_cook scan <assets-dir>\n  redox_cook pack <assets-dir> <out.pak> [--compress] [--incremental]\n  redox_cook list <file.pak>\n  redox_cook verify <file.pak>";

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Scan(PathBuf),
    Pack {
        dir: PathBuf,
        out: PathBuf,
        compress: bool,
        incremental: bool,
    },
    List(PathBuf),
    Verify(PathBuf),
}

fn parse(args: &[String]) -> Result<Command, String> {
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    match rest.as_slice() {
        ["scan", dir] => Ok(Command::Scan(dir.into())),
        ["pack", dir, out, flags @ ..] => {
            let mut command = Command::Pack {
                dir: dir.into(),
                out: out.into(),
                compress: false,
                incremental: false,
            };
            for flag in flags {
                let Command::Pack {
                    compress,
                    incremental,
                    ..
                } = &mut command
                else {
                    unreachable!("constructed as Pack above");
                };
                match *flag {
                    "--compress" => *compress = true,
                    "--incremental" => *incremental = true,
                    other => return Err(format!("unknown option: {other}")),
                }
            }
            Ok(command)
        }
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
        Command::Pack {
            dir,
            out,
            compress,
            incremental,
        } => {
            let stamp_path = PathBuf::from(format!("{}.stamp", out.display()));
            let stamp = if incremental {
                Some(input_stamp(&dir, compress)?)
            } else {
                None
            };
            if let Some(stamp) = &stamp
                && out.exists()
                && std::fs::read_to_string(&stamp_path).is_ok_and(|old| old == *stamp)
            {
                println!("{} is up to date", out.display());
                return Ok(());
            }
            let mut writer = PakWriter::new();
            writer.set_compression(compress);
            add_cooked(&mut writer, &dir)?;
            writer.write_to(&out).map_err(|e| e.to_string())?;
            if let Some(stamp) = stamp {
                std::fs::write(&stamp_path, stamp).map_err(|e| e.to_string())?;
            }
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

/// Every packable file below `dir` (`.meta` sidecars excluded), sorted.
fn cooked_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
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
    files.sort();
    Ok(files)
}

/// Fingerprint of the inputs: path, size and modification time of each file plus the options.
fn input_stamp(dir: &Path, compress: bool) -> Result<String, String> {
    let mut stamp = format!("compress={compress}\n");
    for file in cooked_files(dir)? {
        let meta = std::fs::metadata(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        stamp.push_str(&format!("{}\t{}\t{modified}\n", file.display(), meta.len()));
    }
    Ok(stamp)
}

/// Adds every file below `dir` except `.meta` sidecars.
fn add_cooked(writer: &mut PakWriter, dir: &Path) -> Result<(), String> {
    for file in cooked_files(dir)? {
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
                out: "o.pak".into(),
                compress: false,
                incremental: false,
            })
        );
        assert_eq!(
            parse(&args(&[
                "pack",
                "a",
                "o.pak",
                "--compress",
                "--incremental"
            ])),
            Ok(Command::Pack {
                dir: "a".into(),
                out: "o.pak".into(),
                compress: true,
                incremental: true,
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
        assert!(parse(&args(&["pack", "a", "o", "--bogus"])).is_err());
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
            compress: false,
            incremental: false,
        })
        .unwrap();
        let reader = PakReader::open(&pak).unwrap();
        assert_eq!(reader.names().collect::<Vec<_>>(), ["a.txt", "sub/b.txt"]);
        run(Command::Verify(pak)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incremental_pack_skips_unchanged_inputs_and_repacks_on_change() {
        let dir = std::env::temp_dir().join(format!("redox_cook_inc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/a.txt"), "a".repeat(1000)).unwrap();
        let pak = dir.join("out.pak");
        let pack = |compress| Command::Pack {
            dir: dir.join("assets"),
            out: pak.clone(),
            compress,
            incremental: true,
        };

        run(pack(true)).unwrap();
        let first = std::fs::metadata(&pak).unwrap().modified().unwrap();
        assert!(std::fs::metadata(&pak).unwrap().len() < 1000, "compressed");

        run(pack(true)).unwrap();
        assert_eq!(
            std::fs::metadata(&pak).unwrap().modified().unwrap(),
            first,
            "unchanged inputs are not repacked"
        );

        std::fs::write(dir.join("assets/b.txt"), "b").unwrap();
        run(pack(true)).unwrap();
        let reader = PakReader::open(&pak).unwrap();
        assert_eq!(reader.names().collect::<Vec<_>>(), ["a.txt", "b.txt"]);

        run(pack(false)).unwrap();
        assert!(
            std::fs::metadata(&pak).unwrap().len() > 1000,
            "option change repacks"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
