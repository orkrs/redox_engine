//! Build profiles, cook and packaging of RedOx Studio. Skeleton: filled by the build slice (EDT-C1).

#![warn(missing_docs)]

/// Placeholder for the build service; real profiles and cook steps arrive with the build slice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildServiceStub;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_constructible() {
        assert_eq!(BuildServiceStub, BuildServiceStub::default());
    }
}
