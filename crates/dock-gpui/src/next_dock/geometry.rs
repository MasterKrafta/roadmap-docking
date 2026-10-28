//! Geometry calculation for the independent dock tree.

use ui::Theme;

use super::components::minimum_width_for_group;
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

pub(super) fn compute(root: &DockNode, width: f32, height: f32, theme: &Theme) -> NodeGeometry {
    let tab_height = theme.metrics.tab_strip_height.max(0.0);
    let splitter_thickness = theme.metrics.splitter_thickness.max(0.0);

    compute_node(
        root,
        width.max(1.0),
        height.max(1.0),
        tab_height,
        splitter_thickness,
        theme,
    )
}

/// Computes the minimum width required to display every tab in a subtree.
fn minimum_width(node: &DockNode, splitter_thickness: f32, theme: &Theme) -> f32 {
    match node {
        DockNode::Tabs(group) => minimum_width_for_group(group.panels.len(), theme),
        DockNode::Split {
            axis,
            first,
            second,
            ..
        } => {
            let first_width = minimum_width(first, splitter_thickness, theme);
            let second_width = minimum_width(second, splitter_thickness, theme);

            match axis {
                SplitAxis::Horizontal => first_width + splitter_thickness + second_width,
                SplitAxis::Vertical => first_width.max(second_width),
            }
        }
    }
}

fn compute_node(
    node: &DockNode,
    width: f32,
    height: f32,
    tab_height: f32,
    splitter_thickness: f32,
    theme: &Theme,
) -> NodeGeometry {
    let kind = match node {
        DockNode::Tabs(_) => {
            let tab_height = tab_height.min(height);

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
                SplitAxis::Horizontal => splitter_thickness.min(width),
                SplitAxis::Vertical => splitter_thickness.min(height),
            };

            let available = match axis {
                SplitAxis::Horizontal => (width - splitter_extent).max(0.0),
                SplitAxis::Vertical => (height - splitter_extent).max(0.0),
            };

            let first_extent = match axis {
                SplitAxis::Horizontal => {
                    let min_first = minimum_width(first, splitter_thickness, theme);
                    let min_second = minimum_width(second, splitter_thickness, theme);

                    if available >= min_first + min_second {
                        (available * ratio).clamp(min_first, available - min_second)
                    } else {
                        // The whole tree is too narrow to satisfy every
                        // minimum. Retain the requested split proportion.
                        (available * ratio).clamp(0.0, available)
                    }
                }

                SplitAxis::Vertical => (available * ratio).clamp(0.0, available),
            };

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
                    tab_height,
                    splitter_thickness,
                    theme,
                )),
                second: Box::new(compute_node(
                    second,
                    second_width,
                    second_height,
                    tab_height,
                    splitter_thickness,
                    theme,
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
