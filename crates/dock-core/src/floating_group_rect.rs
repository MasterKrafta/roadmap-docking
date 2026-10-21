use crate::{FloatingGroupId, Rect};

/// On-screen logical bounds for a floating dock group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroupRect {
    pub group: FloatingGroupId,
    pub rect: Rect,
}

impl FloatingGroupRect {
    pub const fn new(group: FloatingGroupId, rect: Rect) -> Self {
        Self { group, rect }
    }
}
