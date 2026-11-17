use dock_core::{Axis, NodeId, Rect};

use super::ResizePreviewKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizePreview {
    pub rect: Rect,
    pub kind: ResizePreviewKind,
    /// A column scroll owner whose content coordinates must be translated.
    pub scroll_owner: Option<NodeId>,
    /// Identifies the original splitter whose static line is hidden during drag.
    pub splitter: Option<(NodeId, usize, Axis)>,
}
