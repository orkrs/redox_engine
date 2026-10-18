//! GPU memory bookkeeping by category.
//!
//! A pure accounting structure with no GPU access and no dependencies. It
//! records how many bytes each [`MemoryCategory`] uses and, optionally, a
//! byte limit per category. It is not wired into the renderer yet.

const MIB: u64 = 1024 * 1024;

/// Bytes of a 2D texture: sum over `mip_levels` levels of max(w >> level, 1) * max(h >> level, 1)
/// * bytes_per_pixel. `mip_levels` 0 counts as 1. Saturating, never panics.
pub fn texture_size_bytes(width: u32, height: u32, bytes_per_pixel: u32, mip_levels: u32) -> u64 {
    let levels = mip_levels.max(1);
    let bpp = u64::from(bytes_per_pixel);
    let (mut w, mut h) = (u64::from(width.max(1)), u64::from(height.max(1)));
    let mut total = 0u64;
    for level in 0..levels {
        let bytes = w.saturating_mul(h).saturating_mul(bpp);
        total = total.saturating_add(bytes);
        if w == 1 && h == 1 {
            // Every remaining level is a 1x1 level of the same size.
            let remaining = u64::from(levels - level - 1);
            return total.saturating_add(bytes.saturating_mul(remaining));
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

/// A category of GPU memory tracked by [`MemoryBudget`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemoryCategory {
    /// Texture memory, including streamed mip levels.
    Textures,
    /// Vertex and index buffers for static meshes.
    Meshes,
    /// Shadow map memory.
    Shadows,
    /// Virtual geometry (meshlet) data.
    VirtualGeometry,
    /// Generic GPU buffers (uniforms, storage, staging).
    Buffers,
    /// Anything not covered by the other categories.
    Other,
}

impl MemoryCategory {
    /// Every category, in declaration order.
    pub const ALL: [MemoryCategory; 6] = [
        MemoryCategory::Textures,
        MemoryCategory::Meshes,
        MemoryCategory::Shadows,
        MemoryCategory::VirtualGeometry,
        MemoryCategory::Buffers,
        MemoryCategory::Other,
    ];

    /// Stable lowercase name for logs and UI.
    pub fn name(self) -> &'static str {
        match self {
            MemoryCategory::Textures => "textures",
            MemoryCategory::Meshes => "meshes",
            MemoryCategory::Shadows => "shadows",
            MemoryCategory::VirtualGeometry => "virtual_geometry",
            MemoryCategory::Buffers => "buffers",
            MemoryCategory::Other => "other",
        }
    }

    /// Index of this category into the fixed per-category arrays.
    fn index(self) -> usize {
        self as usize
    }
}

/// Usage snapshot of one category, as returned by [`MemoryBudget::snapshot`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CategoryUsage {
    /// The category this entry describes.
    pub category: MemoryCategory,
    /// Bytes currently in use.
    pub used: u64,
    /// Byte limit, or `None` when unlimited.
    pub limit: Option<u64>,
    /// `used / limit`, or `None` without a limit or when the limit is 0.
    pub fraction: Option<f32>,
}

/// Per-category GPU memory accounting with optional limits.
#[derive(Clone, Debug, Default)]
pub struct MemoryBudget {
    used: [u64; MemoryCategory::ALL.len()],
    limits: [Option<u64>; MemoryCategory::ALL.len()],
}

impl MemoryBudget {
    /// Creates a budget with zero usage and no limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets or clears the byte limit of `category`.
    pub fn set_limit(&mut self, category: MemoryCategory, bytes: Option<u64>) {
        self.limits[category.index()] = bytes;
    }

    /// Returns the byte limit of `category`, or `None` when unlimited.
    pub fn limit(&self, category: MemoryCategory) -> Option<u64> {
        self.limits[category.index()]
    }

    /// Returns the bytes currently used by `category`.
    pub fn used(&self, category: MemoryCategory) -> u64 {
        self.used[category.index()]
    }

    /// Returns the bytes used across all categories (saturating).
    pub fn total_used(&self) -> u64 {
        self.used.iter().fold(0u64, |acc, &b| acc.saturating_add(b))
    }

    /// Adds `bytes` to `category` unconditionally (saturating).
    ///
    /// Real resources exist even when the category is over budget, so this
    /// never refuses.
    pub fn add(&mut self, category: MemoryCategory, bytes: u64) {
        let slot = &mut self.used[category.index()];
        *slot = slot.saturating_add(bytes);
    }

    /// Removes `bytes` from `category`, saturating at zero.
    pub fn sub(&mut self, category: MemoryCategory, bytes: u64) {
        let slot = &mut self.used[category.index()];
        *slot = slot.saturating_sub(bytes);
    }

    /// Adds `bytes` only if the category stays within its limit.
    ///
    /// A category without a limit always accepts. Returns `true` when the
    /// bytes were added. A refusal (including an overflowing request) leaves the usage unchanged.
    pub fn try_reserve(&mut self, category: MemoryCategory, bytes: u64) -> bool {
        let fits = match self.limit(category) {
            None => true,
            Some(limit) => self
                .used(category)
                .checked_add(bytes)
                .is_some_and(|total| total <= limit),
        };
        if fits {
            self.add(category, bytes);
        }
        fits
    }

    /// Returns the bytes left before the limit.
    ///
    /// `None` without a limit. Returns `0` when the category is over budget.
    pub fn remaining(&self, category: MemoryCategory) -> Option<u64> {
        self.limit(category)
            .map(|limit| limit.saturating_sub(self.used(category)))
    }

    /// Returns `true` when usage exceeds the limit. Always `false` without a limit.
    pub fn is_over_budget(&self, category: MemoryCategory) -> bool {
        self.limit(category)
            .is_some_and(|limit| self.used(category) > limit)
    }

    /// Returns one [`CategoryUsage`] per category, in [`MemoryCategory::ALL`] order.
    pub fn snapshot(&self) -> Vec<CategoryUsage> {
        MemoryCategory::ALL
            .iter()
            .map(|&category| {
                let used = self.used(category);
                let limit = self.limit(category);
                let fraction = limit.filter(|&l| l > 0).map(|l| used as f32 / l as f32);
                CategoryUsage {
                    category,
                    used,
                    limit,
                    fraction,
                }
            })
            .collect()
    }

    /// The categories whose usage exceeds their limit, in [`MemoryCategory::ALL`] order.
    pub fn over_budget_categories(&self) -> Vec<MemoryCategory> {
        MemoryCategory::ALL
            .into_iter()
            .filter(|&category| self.is_over_budget(category))
            .collect()
    }

    /// Text for a debug overlay, one line per category that has usage or a limit, in
    /// [`MemoryCategory::ALL`] order, then a `total:` line. Sizes are MiB with one decimal.
    /// Lines have no trailing newline.
    pub fn report(&self) -> String {
        let mut lines: Vec<String> = self
            .snapshot()
            .into_iter()
            .filter(|usage| usage.used > 0 || usage.limit.is_some())
            .map(|usage| report_line(&usage))
            .collect();
        lines.push(format!("total: {}", format_mib(self.total_used())));
        lines.join("\n")
    }
}

/// Formats one category line of [`MemoryBudget::report`].
fn report_line(usage: &CategoryUsage) -> String {
    let name = usage.category.name();
    let used = format_mib(usage.used);
    let over = if usage.limit.is_some_and(|limit| usage.used > limit) {
        " OVER"
    } else {
        ""
    };
    match usage.limit {
        None => format!("{name}: {used}"),
        Some(0) => format!("{name}: {used} / {}{over}", format_mib(0)),
        Some(limit) => format!(
            "{name}: {used} / {} ({}%){over}",
            format_mib(limit),
            percent_rounded(usage.used, limit)
        ),
    }
}

/// `bytes` as MiB with one decimal place, e.g. `"3.5 MiB"`.
fn format_mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB as f64)
}

/// `used / limit` as a whole percentage, rounded half up. `limit` must be non-zero.
fn percent_rounded(used: u64, limit: u64) -> u128 {
    let (used, limit) = (u128::from(used), u128::from(limit));
    (used * 100 + limit / 2) / limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_used_saturates_instead_of_overflowing() {
        let mut b = MemoryBudget::new();
        b.add(MemoryCategory::Textures, u64::MAX);
        b.add(MemoryCategory::Meshes, 1);
        assert_eq!(b.total_used(), u64::MAX);
    }

    #[test]
    fn try_reserve_at_max_limit_does_not_overflow() {
        let mut b = MemoryBudget::new();
        b.set_limit(MemoryCategory::Buffers, Some(u64::MAX));
        b.add(MemoryCategory::Buffers, u64::MAX - 1);
        assert!(b.try_reserve(MemoryCategory::Buffers, 1));
        assert!(!b.try_reserve(MemoryCategory::Buffers, 1));
        assert_eq!(b.used(MemoryCategory::Buffers), u64::MAX);
    }

    #[test]
    fn over_budget_category_reserves_nothing_but_keeps_usage() {
        let mut b = MemoryBudget::new();
        b.set_limit(MemoryCategory::Shadows, Some(10));
        b.add(MemoryCategory::Shadows, 50);
        assert!(!b.try_reserve(MemoryCategory::Shadows, 0));
        assert_eq!(b.remaining(MemoryCategory::Shadows), Some(0));
        assert_eq!(b.used(MemoryCategory::Shadows), 50);
    }

    #[test]
    fn fraction_can_exceed_one_when_over_budget() {
        let mut b = MemoryBudget::new();
        b.set_limit(MemoryCategory::Meshes, Some(100));
        b.add(MemoryCategory::Meshes, 150);
        let usage = b.snapshot();
        let meshes = usage.iter().find(|u| u.category == MemoryCategory::Meshes);
        assert_eq!(meshes.and_then(|u| u.fraction), Some(1.5));
    }

    #[test]
    fn report_rounds_small_sizes_and_percentages() {
        let mut b = MemoryBudget::new();
        // 1 byte rounds down to 0.0 MiB; 18.75% rounds half up to 19%.
        b.set_limit(MemoryCategory::Textures, Some(64 * MIB));
        b.add(MemoryCategory::Textures, 12 * MIB);
        b.add(MemoryCategory::Other, 1);
        assert_eq!(
            b.report(),
            "textures: 12.0 MiB / 64.0 MiB (19%)\nother: 0.0 MiB\ntotal: 12.0 MiB"
        );
    }

    #[test]
    fn report_limit_without_usage_is_listed_as_zero_percent() {
        let mut b = MemoryBudget::new();
        b.set_limit(MemoryCategory::Meshes, Some(MIB));
        assert_eq!(b.report(), "meshes: 0.0 MiB / 1.0 MiB (0%)\ntotal: 0.0 MiB");
    }

    #[test]
    fn over_budget_categories_is_empty_when_all_within_limits() {
        let mut b = MemoryBudget::new();
        b.set_limit(MemoryCategory::Textures, Some(10));
        b.add(MemoryCategory::Textures, 10);
        assert!(b.over_budget_categories().is_empty());
    }

    #[test]
    fn texture_size_handles_zero_levels_and_zero_dimensions() {
        assert_eq!(texture_size_bytes(0, 0, 4, 0), 4);
        assert_eq!(texture_size_bytes(1, 1, 4, 40), 40 * 4);
    }

    #[test]
    fn names_match_all_order_length() {
        assert_eq!(MemoryCategory::ALL.len(), 6);
        assert_eq!(MemoryCategory::ALL[0].name(), "textures");
        assert_eq!(MemoryCategory::ALL[5].name(), "other");
    }
}
