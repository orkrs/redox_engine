//! Helpers for tests: build KTX2 containers in memory.
//!
//! Available to other crates through the `testing` feature
//! (`redox_asset = { ..., features = ["testing"] }` in `dev-dependencies`).

use crate::loader::TextureFormat;

/// How the level data is compressed in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Supercompression {
    /// Stored as is.
    None,
    /// Each level is a Zstandard frame.
    Zstd,
}

/// Byte `i` of the test pattern for sub-image `sub` of mip `mip`.
pub fn pattern_byte(mip: usize, sub: u32, i: usize) -> u8 {
    (mip as u32 * 31 + sub * 7 + i as u32 * 3) as u8
}

/// Builds a KTX2 file in memory.
#[derive(Clone, Debug)]
pub struct Ktx2Builder {
    format: TextureFormat,
    width: u32,
    height: u32,
    layers: u32,
    faces: u32,
    scheme: Supercompression,
    levels: Vec<Vec<u8>>,
    declared_sizes: Vec<Option<u64>>,
    vk_override: Option<u32>,
}

impl Ktx2Builder {
    /// A texture with the full mip chain filled with the test pattern ([`pattern_byte`]).
    pub fn new(format: TextureFormat, width: u32, height: u32) -> Self {
        let mut builder = Self {
            format,
            width,
            height,
            layers: 0,
            faces: 1,
            scheme: Supercompression::None,
            levels: Vec::new(),
            declared_sizes: Vec::new(),
            vk_override: None,
        };
        builder.refill();
        builder
    }

    fn mip_count(&self) -> usize {
        (32 - self.width.max(self.height).max(1).leading_zeros()) as usize
    }

    fn refill(&mut self) {
        let subs = self.layers.max(1) * self.faces;
        self.levels = (0..self.mip_count())
            .map(|mip| {
                let (w, h) = ((self.width >> mip).max(1), (self.height >> mip).max(1));
                let size = self.format.level_size(w, h);
                let mut data = Vec::with_capacity(size * subs as usize);
                for sub in 0..subs {
                    data.extend((0..size).map(|i| pattern_byte(mip, sub, i)));
                }
                data
            })
            .collect();
        self.declared_sizes = vec![None; self.levels.len()];
    }

    /// Makes it a cube map (6 faces).
    pub fn cube(mut self) -> Self {
        self.faces = 6;
        self.refill();
        self
    }

    /// Makes it an array of `layers` images (layer count 0 in the file means "not an array").
    pub fn layers(mut self, layers: u32) -> Self {
        self.layers = layers;
        self.refill();
        self
    }

    /// Keeps only the first `count` mip levels.
    pub fn mips(mut self, count: usize) -> Self {
        self.levels.truncate(count);
        self.declared_sizes.truncate(count);
        self
    }

    /// Chooses the supercompression scheme.
    pub fn supercompression(mut self, scheme: Supercompression) -> Self {
        self.scheme = scheme;
        self
    }

    /// Replaces the data of one level (before compression).
    pub fn level_data(mut self, mip: usize, data: Vec<u8>) -> Self {
        self.levels[mip] = data;
        self
    }

    /// Writes a wrong uncompressed size for one level into the level index.
    pub fn declare_size(mut self, mip: usize, size: u64) -> Self {
        self.declared_sizes[mip] = Some(size);
        self
    }

    /// Writes this `vkFormat` value instead of the real one.
    pub fn vk_format(mut self, code: u32) -> Self {
        self.vk_override = Some(code);
        self
    }

    /// The uncompressed data of every level.
    pub fn raw_levels(&self) -> &[Vec<u8>] {
        &self.levels
    }

    /// Serializes the file.
    pub fn build(&self) -> Vec<u8> {
        crate::loader::ktx2_container(
            self.vk_override,
            self.format,
            (self.width, self.height),
            (self.layers, self.faces),
            &self.levels,
            self.scheme == Supercompression::Zstd,
            &self.declared_sizes,
        )
    }
}
