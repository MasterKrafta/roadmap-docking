use crate::{NodeId, Rect};

/// Scroll metadata for a vertical layout stack whose minimum content is taller
/// than the viewport allocated by its parent.
///
/// The content extent starts at the viewport origin and keeps its width, but
/// extends downward to the full minimum-size allocation. Adapters may scroll or
/// clip this content without shrinking any group below its minimum height. For
/// main-dock columns, the eventual GPUI adapter should use `viewport` as the
/// clipping/scrolling region and `content_extent` as the total scrollable size.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VerticalOverflow {
    /// Column or Split node that owns this vertical stack.
    pub owner: NodeId,
    /// Visible rectangle allocated to the stack by its parent.
    pub viewport: Rect,
    /// Full scrollable rectangle needed to lay out all children at minimum size.
    pub content_extent: Rect,
}

impl VerticalOverflow {
    pub const fn new(owner: NodeId, viewport: Rect, content_extent: Rect) -> Self {
        Self {
            owner,
            viewport,
            content_extent,
        }
    }
}
