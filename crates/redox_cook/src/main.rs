//! `redox_cook` - asset pipeline command line tool.
//!
//! ```text
//! redox_cook scan <assets-dir>             create/repair .meta sidecars (GUIDs)
//! redox_cook pack <assets-dir> <out.pak> [--compress] [--incremental] [--textures]
//!                                          pack a folder into a deterministic archive
//! redox_cook list <file.pak>               print the entries of an archive
//! redox_cook verify <file.pak>             read every entry and check its hash
//! redox_cook texture <out.ktx2> <images...> [--etc1s] [--linear] [--no-mips] [--array|--cube]
//!                                          encode images to KTX2 (UASTC or ETC1S, with mips)
//! redox_cook texture-info <file.ktx2>      print format, size, mips, layers, faces, compression
//! redox_cook bake-ibl <sky.hdr> <out-dir>  bake IBL maps to KTX2 (build with --features bake)
//! redox_cook bake-hlod <world.bin> <out-dir> [--triangles 2000]
//!                                          per-cell HLOD proxies (not yet possible: see hlod_bake)
//! ```
//!
//! `.meta` sidecars are not packed (they are editor data); everything else is.
//! `--compress` stores entries LZ4-compressed where that helps. `--incremental` keeps a
//! `<out.pak>.stamp` file (path, size and modification time of every input plus the options)
//! and skips the work when nothing changed since the last run.

#[cfg(feature = "bake")]
mod bake;
mod hlod_bake;
mod texture;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_asset::{AssetRegistry, PakReader, PakWriter};

const USAGE: &str = "usage:\n  redox_cook scan <assets-dir>\n  redox_cook pack <assets-dir> <out.pak> [--compress] [--incremental] [--textures]\n  redox_cook texture <out.ktx2> <images...> [--etc1s] [--linear] [--no-mips] [--no-zstd] [--array|--cube]\n  redox_cook list <file.pak>\n  redox_cook verify <file.pak>\n  redox_cook texture-info <file.ktx2>\n  redox_cook bake-ibl <sky.hdr> <out-dir>\n  redox_cook bake-hlod <world.bin> <out-dir> [--triangles 2000]";

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Scan(PathBuf),
    Pack {
        dir: PathBuf,
        out: PathBuf,
        compress: bool,
        incremental: bool,
        textures: bool,
    },
    Texture(texture::TextureJob),
    List(PathBuf),
    Verify(PathBuf),
    TextureInfo(PathBuf),
    BakeIbl {
        input: PathBuf,
        out: PathBuf,
    },
    BakeHlod {
        world: PathBuf,
        out: PathBuf,
        triangles: usize,
    },
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
                textures: false,
            };
            for flag in flags {
                let Command::Pack {
                    compress,
                    incremental,
                    textures,
                    ..
                } = &mut command
                else {
                    unreachable!("constructed as Pack above");
                };
                match *flag {
                    "--compress" => *compress = true,
                    "--incremental" => *incremental = true,
                    "--textures" => *textures = true,
                    other => return Err(format!("unknown option: {other}")),
                }
            }
            Ok(command)
        }
        ["list", pak] => Ok(Command::List(pak.into())),
        ["texture", args @ ..] => texture::parse_texture(args).map(Command::Texture),
        ["verify", pak] => Ok(Command::Verify(pak.into())),
        ["texture-info", file] => Ok(Command::TextureInfo(file.into())),
        ["bake-ibl", input, out] => Ok(Command::BakeIbl {
            input: input.into(),
            out: out.into(),
        }),
        ["bake-hlod", world, out, flags @ ..] => Ok(Command::BakeHlod {
            world: world.into(),
            out: out.into(),
            triangles: hlod_bake::parse_triangles(flags)?,
        }),
        [] => Err("missing command".into()),
        [cmd, ..]
            if ![
                "scan",
                "pack",
                "list",
                "verify",
                "texture",
                "texture-info",
                "bake-ibl",
                "bake-hlod",
            ]
            .contains(cmd) =>
        {
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
            textures,
        } => {
            let stamp_path = PathBuf::from(format!("{}.stamp", out.display()));
            let stamp = if incremental {
                Some(input_stamp(&dir, compress, textures)?)
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
            add_cooked(&mut writer, &dir, textures)?;
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
        Command::Texture(job) => {
            let size = texture::run_texture(&job)?;
            println!("wrote {} ({size} bytes)", job.out.display());
            Ok(())
        }
        Command::TextureInfo(file) => {
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            println!("{}", texture_info(&bytes)?);
            Ok(())
        }
        Command::BakeIbl { input, out } => run_bake_ibl(&input, &out),
        Command::BakeHlod {
            world,
            out,
            triangles,
        } => hlod_bake::bake_hlod(&world, &out, triangles),
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

#[cfg(feature = "bake")]
fn run_bake_ibl(input: &Path, out: &Path) -> Result<(), String> {
    let written = bake::bake_ibl(input, out)?;
    println!("wrote {} to {}", written.join(", "), out.display());
    Ok(())
}

#[cfg(not(feature = "bake"))]
fn run_bake_ibl(_input: &Path, _out: &Path) -> Result<(), String> {
    Err(
        "bake-ibl needs the GPU renderer: cargo run -p redox_cook --features bake -- bake-ibl ..."
            .into(),
    )
}

/// Human-readable summary of a KTX2 file (what `texture-info` prints).
fn texture_info(bytes: &[u8]) -> Result<String, String> {
    use redox_asset::{AssetLoader, Ktx2Loader};

    let reader = ktx2::Reader::new(bytes).map_err(|e| format!("not a KTX2 file: {e:?}"))?;
    let header = reader.header();
    let scheme = header
        .supercompression_scheme
        .map_or_else(|| "none".to_string(), |s| format!("{s:?}"));
    let stored: usize = reader.levels().map(|l| l.data.len()).sum();
    let mut text = format!(
        "{}x{}, {} mips, {} layers, {} faces, vkFormat {}, supercompression {scheme}, {stored} bytes stored",
        header.pixel_width,
        header.pixel_height,
        header.level_count.max(1),
        header.layer_count.max(1),
        header.face_count,
        header.format.map_or(0, |f| f.value()),
    );
    match Ktx2Loader::default().load(bytes) {
        Ok(tex) => {
            let decoded: usize = tex.levels.iter().map(Vec::len).sum();
            text.push_str(&format!(
                "\nloads as {:?}, {decoded} bytes decoded",
                tex.format
            ));
        }
        Err(e) => text.push_str(&format!("\ncannot be loaded by the engine: {e}")),
    }
    Ok(text)
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
fn input_stamp(dir: &Path, compress: bool, textures: bool) -> Result<String, String> {
    let mut stamp = format!("compress={compress} textures={textures}\n");
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
fn add_cooked(writer: &mut PakWriter, dir: &Path, textures: bool) -> Result<(), String> {
    for file in cooked_files(dir)? {
        let rel = file
            .strip_prefix(dir)
            .unwrap_or(&file)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let (rel, bytes) = if textures && texture::is_convertible(&file) {
            let ktx2 = Path::new(&rel).with_extension("ktx2");
            (
                ktx2.to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
                texture::encode_for_pack(&file)?,
            )
        } else {
            (
                rel,
                std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?,
            )
        };
        // Textures are already compressed and are read in pieces by the streamer: keep them raw.
        let keep_raw = rel.ends_with(".ktx2") || rel.ends_with(".basis");
        if keep_raw {
            writer.add_uncompressed(&rel, bytes)
        } else {
            writer.add(&rel, bytes)
        }
        .map_err(|e| e.to_string())?;
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
                textures: false,
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
                textures: false,
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
    fn texture_info_describes_supported_and_unsupported_files() {
        use redox_asset::TextureFormat;
        use redox_asset::testing::{Ktx2Builder, Supercompression};

        let file = Ktx2Builder::new(TextureFormat::Bc7Unorm, 8, 8)
            .cube()
            .supercompression(Supercompression::Zstd)
            .build();
        let info = texture_info(&file).unwrap();
        assert!(info.contains("8x8, 4 mips, 1 layers, 6 faces"), "{info}");
        assert!(
            info.contains("Zstandard") && info.contains("loads as Bc7Unorm"),
            "{info}"
        );
        let odd = Ktx2Builder::new(TextureFormat::Bc7Unorm, 4, 4)
            .vk_format(1000)
            .build();
        assert!(texture_info(&odd).unwrap().contains("cannot be loaded"));
        assert!(texture_info(b"junk").is_err());
        assert_eq!(
            parse(&args(&["bake-ibl", "s.hdr", "out"])),
            Ok(Command::BakeIbl {
                input: "s.hdr".into(),
                out: "out".into()
            })
        );
        assert_eq!(
            parse(&args(&["texture-info", "a.ktx2"])),
            Ok(Command::TextureInfo("a.ktx2".into()))
        );
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
            textures: false,
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
            textures: false,
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

    #[test]
    fn pack_textures_converts_images_and_keeps_them_uncompressed() {
        use redox_asset::{AssetLoader, Ktx2Loader};

        let dir = std::env::temp_dir().join(format!("redox_cook_pt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/tex")).unwrap();
        image::RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba([x as u8 * 16, y as u8 * 16, 7, 255])
        })
        .save(dir.join("assets/tex/rock_albedo.png"))
        .unwrap();
        std::fs::write(dir.join("assets/readme.txt"), "x".repeat(2000)).unwrap();
        let pak = dir.join("out.pak");
        run(Command::Pack {
            dir: dir.join("assets"),
            out: pak.clone(),
            compress: true,
            incremental: false,
            textures: true,
        })
        .unwrap();
        let reader = PakReader::open(&pak).unwrap();
        assert_eq!(
            reader.names().collect::<Vec<_>>(),
            ["readme.txt", "tex/rock_albedo.ktx2"]
        );
        let ktx2 = reader.read("tex/rock_albedo.ktx2").unwrap();
        let tex = Ktx2Loader::default().load(&ktx2).unwrap();
        assert_eq!((tex.width, tex.levels.len()), (16, 5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
