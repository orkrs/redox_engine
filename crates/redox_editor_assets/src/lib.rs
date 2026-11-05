//! Asset services of RedOx Studio (registry integration, import queue, thumbnails). Skeleton: filled by EDT-A6.

#![warn(missing_docs)]

/// Placeholder for the asset service; the real service arrives with the asset registry slice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AssetServiceStub;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_constructible() {
        assert_eq!(format!("{:?}", AssetServiceStub), "AssetServiceStub");
    }
}
