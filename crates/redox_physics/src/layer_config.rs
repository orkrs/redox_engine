//! Collision layer configuration: named layers and a symmetric collision matrix, loaded from RON.
//!
//! The RON shape is `(layers: ["default", "player"], collide: [("player", "enemy")])`. Layer `i` in
//! `layers` is bit `1 << i`; at most 32 layers are allowed.

use std::collections::HashSet;

use serde::Deserialize;
use thiserror::Error;

use crate::shapes::CollisionLayers;

/// Maximum number of named layers, one per bit of the `u32` masks.
const MAX_LAYERS: usize = 32;

/// Errors produced by [`LayerConfig::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayerConfigError {
    /// The text is not valid RON of the expected shape.
    #[error("invalid layer configuration: {0}")]
    Parse(String),
    /// The same layer name is listed twice.
    #[error("layer `{0}` is declared more than once")]
    DuplicateLayer(String),
    /// A collision pair names a layer that is not listed.
    #[error("collision pair names unknown layer `{0}`")]
    UnknownLayer(String),
    /// More than 32 layers were declared.
    #[error("{0} layers declared, at most 32 are allowed")]
    TooManyLayers(usize),
}

/// The RON document as written by the user, before validation.
#[derive(Deserialize)]
struct RawLayerConfig {
    layers: Vec<String>,
    collide: Vec<(String, String)>,
}

/// A validated set of named collision layers and their collision pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerConfig {
    /// Layer names in declaration order; the index of a name is its bit position.
    names: Vec<String>,
    /// Collision pairs resolved to layer bits, as written (not yet symmetric).
    pairs: Vec<(u32, u32)>,
}

impl LayerConfig {
    /// Parses and validates a RON layer configuration.
    ///
    /// RON: `(layers: ["default", "player", ...], collide: [("player", "enemy"), ...])`. Each pair makes
    /// the two layers interact symmetrically; a layer may be paired with itself. A layer with no pair
    /// collides with nothing.
    ///
    /// # Errors
    /// Returns [`LayerConfigError::Parse`] for text that is not valid RON of that shape,
    /// [`LayerConfigError::TooManyLayers`] for more than 32 layers,
    /// [`LayerConfigError::DuplicateLayer`] for a name listed twice, and
    /// [`LayerConfigError::UnknownLayer`] for a pair naming an unlisted layer.
    pub fn parse(ron_text: &str) -> Result<Self, LayerConfigError> {
        let raw: RawLayerConfig =
            ron::from_str(ron_text).map_err(|e| LayerConfigError::Parse(e.to_string()))?;
        Self::validate(raw)
    }

    /// Number of named layers.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no layers are declared.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The single bit of a layer, or `None` when the name is not declared.
    pub fn bit(&self, name: &str) -> Option<u32> {
        let index = self.names.iter().position(|n| n == name)?;
        Some(1u32 << index)
    }

    /// Collision layers of a named layer: membership is its own bit, and the filter is the bits of
    /// every layer it is paired with. `None` when the name is not declared.
    pub fn layers_of(&self, name: &str) -> Option<CollisionLayers> {
        let member_of = self.bit(name)?;
        let collides_with = self
            .pairs
            .iter()
            .fold(0u32, |acc, &(a, b)| acc | partner_bits(member_of, a, b));
        Some(CollisionLayers {
            member_of,
            collides_with,
        })
    }

    /// Query mask for shape casts and overlaps: the OR of the named layers' bits. `None` when any
    /// name is not declared.
    pub fn mask(&self, names: &[&str]) -> Option<u32> {
        names
            .iter()
            .try_fold(0u32, |acc, name| Some(acc | self.bit(name)?))
    }

    /// Checks the raw document and resolves names to bits.
    fn validate(raw: RawLayerConfig) -> Result<Self, LayerConfigError> {
        let count = raw.layers.len();
        if count > MAX_LAYERS {
            return Err(LayerConfigError::TooManyLayers(count));
        }
        check_unique(&raw.layers)?;
        let pairs = raw
            .collide
            .iter()
            .map(|(a, b)| Ok((resolve(&raw.layers, a)?, resolve(&raw.layers, b)?)))
            .collect::<Result<Vec<_>, LayerConfigError>>()?;
        Ok(Self {
            names: raw.layers,
            pairs,
        })
    }
}

/// Bits that `member` collides with, given one pair `(a, b)`: the partner of `member` in the pair.
fn partner_bits(member: u32, a: u32, b: u32) -> u32 {
    let mut bits = 0;
    if a == member {
        bits |= b;
    }
    if b == member {
        bits |= a;
    }
    bits
}

/// Fails on the first name that appears twice in `names`.
fn check_unique(names: &[String]) -> Result<(), LayerConfigError> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            return Err(LayerConfigError::DuplicateLayer(name.clone()));
        }
    }
    Ok(())
}

/// The bit of a declared layer name inside a validated list.
fn resolve(names: &[String], name: &str) -> Result<u32, LayerConfigError> {
    names
        .iter()
        .position(|n| n == name)
        .map(|index| 1u32 << index)
        .ok_or_else(|| LayerConfigError::UnknownLayer(name.to_owned()))
}
