//! Geometry calculation for the independent dock tree.

use super::model::{DockNode, SplitAxis};

pub(super) const HEADER_HEIGHT: f32 = 36.0;
pub(super) const TAB_HEIGHT: f32 = 32.0;
pub(super) const SPLITTER_SIZE: f32 = 6.0;

/// Geometry for one node in the dock tree.
#[derive(Clone, Debug)]
pub(super) struct NodeGeometry {
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) kind: GeometryKind,
}

/// Geometry details for a tab group or a split.
#[derive(Clone, Debug)]
pub(super) enum GeometryKind {
    Tabs {
        tab_height: f32,
        content_height: f32,
    },
    Split {
        splitter_extent: f32,
        first: Box<NodeGeometry>,
        second: Box<NodeGeometry>,
    },
}

/// Calculates geometry for the complete tree using the available area.
pub(super) fn compute(
    root: &DockNode,
    width: f32,
    height: f32,
) -> NodeGeometry {
    compute_node(root, width.max(1.0), height.max(1.0))
}

fn compute_node(
    node: &DockNode,
    width: f32,
    height: f32,
) -> NodeGeometry {
    let kind = match node {
        DockNode::Tabs(_) => {
            let tab_height = TAB_HEIGHT.min(height);

            GeometryKind::Tabs {
                tab_height,
                content_height: (height - tab_height).max(0.0),
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let splitter_extent = match axis {
                SplitAxis::Horizontal => SPLITTER_SIZE.min(width),
                SplitAxis::Vertical => SPLITTER_SIZE.min(height),
            };

            let available = match axis {
                SplitAxis::Horizontal => (width - splitter_extent).max(0.0),
                SplitAxis::Vertical => (height - splitter_extent).max(0.0),
            };

            let first_extent = (available * ratio).clamp(0.0, available);
            let second_extent = available - first_extent;

            let (first_width, first_height, second_width, second_height) =
                match axis {
                    SplitAxis::Horizontal => (
                        first_extent,
                        height,
                        second_extent,
                        height,
                    ),
                    SplitAxis::Vertical => (
                        width,
                        first_extent,
                        width,
                        second_extent,
                    ),
                };

            GeometryKind::Split {
                splitter_extent,
                first: Box::new(compute_node(
                    first,
                    first_width,
                    first_height,
                )),
                second: Box::new(compute_node(
                    second,
                    second_width,
                    second_height,
                )),
            }
        }
    };

    NodeGeometry {
        width,
        height,
        kind,
    }
}