//! Geometry calculation for the independent dock tree.

use ui::Theme;

use super::model::{DockNode, SplitAxis};

pub(super) const HEADER_HEIGHT: f32 = 36.0;

#[derive(Clone, Debug)]
pub(super) struct NodeGeometry {
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) kind: GeometryKind,
}

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

/// Calculates the tree geometry using the current theme.
pub(super) fn compute(root: &DockNode, width: f32, height: f32, theme: &Theme) -> NodeGeometry {
    let grip_height =
        (theme.metrics.column_header_height - theme.metrics.tab_strip_height).max(0.0);
    let tab_height = theme.metrics.tab_strip_height.max(0.0);
    let splitter_thickness = theme.metrics.splitter_thickness.max(0.0);

    compute_node(
        root,
        width.max(1.0),
        height.max(1.0),
        grip_height,
        tab_height,
        splitter_thickness,
    )
}

fn compute_node(
    node: &DockNode,
    width: f32,
    height: f32,
    grip_height: f32,
    tab_height: f32,
    splitter_thickness: f32,
) -> NodeGeometry {
    let kind = match node {
        DockNode::Tabs(_) => {
            let grip_height = grip_height.min(height);
            let tab_height = tab_height.min((height - grip_height).max(0.0));

            GeometryKind::Tabs {
                tab_height,
                content_height: (height - grip_height - tab_height).max(0.0),
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let splitter_extent = match axis {
                SplitAxis::Horizontal => splitter_thickness.min(width),
                SplitAxis::Vertical => splitter_thickness.min(height),
            };

            let available = match axis {
                SplitAxis::Horizontal => (width - splitter_extent).max(0.0),
                SplitAxis::Vertical => (height - splitter_extent).max(0.0),
            };

            let first_extent = (available * ratio).clamp(0.0, available);
            let second_extent = available - first_extent;

            let (first_width, first_height, second_width, second_height) = match axis {
                SplitAxis::Horizontal => (first_extent, height, second_extent, height),
                SplitAxis::Vertical => (width, first_extent, width, second_extent),
            };

            GeometryKind::Split {
                splitter_extent,
                first: Box::new(compute_node(
                    first,
                    first_width,
                    first_height,
                    grip_height,
                    tab_height,
                    splitter_thickness,
                )),
                second: Box::new(compute_node(
                    second,
                    second_width,
                    second_height,
                    grip_height,
                    tab_height,
                    splitter_thickness,
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
