//! `texture`: encode images to KTX2 (UASTC or ETC1S with mips), and the `pack --textures` step.

use std::path::{Path, PathBuf};

use redox_asset::{EncodeMode, EncodeOptions, EncodeShape, encode_ktx2};

/// Options of the `texture` command.
#[derive(Debug, PartialEq, Eq)]
pub struct TextureJob {
    /// Output `.ktx2` file.
    pub out: PathBuf,
    /// Input images (one, or the layers / cube faces).
    pub inputs: Vec<PathBuf>,
    /// Basis format.
    pub etc1s: bool,
    /// Data textures (normals, masks) are linear instead of sRGB.
    pub linear: bool,
    /// Skip the mip chain.
    pub no_mips: bool,
    /// Skip Zstandard on UASTC.
    pub no_zstd: bool,
    /// Shape of several inputs.
    pub shape: Shape,
}

/// How several inputs combine.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Shape {
    /// One image (the default).
    Flat,
    /// Array layers.
    Array,
    /// Cube maps, 6 faces each.
    Cube,
}

/// Parses `texture <out.ktx2> <inputs...> [options]`.
pub fn parse_texture(args: &[&str]) -> Result<TextureJob, String> {
    let (out, rest) = args.split_first().ok_or("texture needs an output file")?;
    let mut job = TextureJob {
        out: (*out).into(),
        inputs: Vec::new(),
        etc1s: false,
        linear: false,
        no_mips: false,
        no_zstd: false,
        shape: Shape::Flat,
    };
    for arg in rest {
        match *arg {
            "--uastc" => job.etc1s = false,
            "--etc1s" => job.etc1s = true,
            "--linear" => job.linear = true,
            "--no-mips" => job.no_mips = true,
            "--no-zstd" => job.no_zstd = true,
            "--array" => job.shape = Shape::Array,
            "--cube" => job.shape = Shape::Cube,
            flag if flag.starts_with("--") => return Err(format!("unknown option: {flag}")),
            input => job.inputs.push(input.into()),
        }
    }
    if job.inputs.is_empty() {
        return Err("texture needs at least one input image".into());
    }
    if job.shape == Shape::Flat && job.inputs.len() > 1 {
        return Err("several inputs need --array or --cube".into());
    }
    Ok(job)
}

fn options(job: &TextureJob) -> EncodeOptions {
    EncodeOptions {
        mode: if job.etc1s {
            EncodeMode::Etc1s
        } else {
            EncodeMode::Uastc
        },
        srgb: !job.linear,
        mipmaps: !job.no_mips,
        shape: match job.shape {
            Shape::Flat => EncodeShape::Flat,
            Shape::Array => EncodeShape::Array,
            Shape::Cube => EncodeShape::Cube,
        },
        zstd: !job.no_zstd,
    }
}

fn read_image(path: &Path) -> Result<image::DynamicImage, String> {
    image::open(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Runs a `texture` job; returns the size of the written file.
pub fn run_texture(job: &TextureJob) -> Result<usize, String> {
    let images = job
        .inputs
        .iter()
        .map(|p| read_image(p))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes = encode_ktx2(&images, options(job)).map_err(|e| e.to_string())?;
    std::fs::write(&job.out, &bytes).map_err(|e| format!("{}: {e}", job.out.display()))?;
    Ok(bytes.len())
}

/// Whether `pack --textures` converts this file.
pub fn is_convertible(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

/// Data textures are stored linear: normal maps, metallic-roughness and similar masks.
pub fn looks_linear(path: &Path) -> bool {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    [
        "normal", "_nrm", "_n", "_mr", "rough", "metal", "_orm", "_ao", "mask",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}

/// Encodes one image for `pack --textures`: UASTC with mips, sRGB unless the name says it is data.
pub fn encode_for_pack(path: &Path) -> Result<Vec<u8>, String> {
    let image = read_image(path)?;
    encode_ktx2(
        &[image],
        EncodeOptions {
            srgb: !looks_linear(path),
            ..EncodeOptions::default()
        },
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_and_inputs() {
        let job = parse_texture(&["o.ktx2", "a.png", "--etc1s", "--linear", "--no-mips"]).unwrap();
        assert_eq!(job.out, PathBuf::from("o.ktx2"));
        assert_eq!(job.inputs, [PathBuf::from("a.png")]);
        assert!(job.etc1s && job.linear && job.no_mips && !job.no_zstd);
        let cube = parse_texture(&["o.ktx2", "1", "2", "3", "4", "5", "6", "--cube"]).unwrap();
        assert_eq!((cube.inputs.len(), cube.shape), (6, Shape::Cube));
    }

    #[test]
    fn rejects_bad_command_lines() {
        assert!(parse_texture(&[]).is_err());
        assert!(parse_texture(&["o.ktx2"]).is_err());
        assert!(parse_texture(&["o.ktx2", "a.png", "b.png"]).is_err());
        assert!(parse_texture(&["o.ktx2", "a.png", "--wat"]).is_err());
    }

    #[test]
    fn names_decide_linear_or_srgb() {
        assert!(looks_linear(Path::new("rock_normal.png")));
        assert!(looks_linear(Path::new("a/b/wall_MR.png")));
        assert!(!looks_linear(Path::new("rock_albedo.png")));
        assert!(is_convertible(Path::new("x.PNG")) && !is_convertible(Path::new("x.glb")));
    }

    #[test]
    fn a_texture_job_writes_a_loadable_file() {
        use redox_asset::{AssetLoader, Ktx2Loader};

        let dir = std::env::temp_dir().join(format!("redox_cook_tex_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("in.png");
        image::RgbaImage::from_fn(32, 32, |x, y| {
            image::Rgba([x as u8 * 8, y as u8 * 8, 99, 255])
        })
        .save(&png)
        .unwrap();
        let job = TextureJob {
            out: dir.join("out.ktx2"),
            inputs: vec![png],
            etc1s: false,
            linear: false,
            no_mips: false,
            no_zstd: false,
            shape: Shape::Flat,
        };
        let size = run_texture(&job).unwrap();
        assert!(size > 100);
        let tex = Ktx2Loader::default()
            .load(&std::fs::read(&job.out).unwrap())
            .unwrap();
        assert_eq!((tex.width, tex.height, tex.levels.len()), (32, 32, 6));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
