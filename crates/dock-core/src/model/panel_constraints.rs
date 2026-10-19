use crate::geometry::Size;

/// Size constraints copied from the app's panel descriptor when a panel opens.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelConstraints {
    min_size: Size,
    preferred_size: Size,
    max_size: Size,
    singleton: bool,
}

impl PanelConstraints {
    pub const fn new(min_size: Size, preferred_size: Size, max_size: Size, singleton: bool) -> Self {
        Self { min_size, preferred_size, max_size, singleton }
    }

    pub const fn min_size(&self) -> Size { self.min_size }
    pub const fn preferred_size(&self) -> Size { self.preferred_size }
    pub const fn max_size(&self) -> Size { self.max_size }
    pub const fn is_singleton(&self) -> bool { self.singleton }
}
