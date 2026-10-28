//! Geometry calculation for the independent dock tree.

use gpui::Window;
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
    Column {
        header_height: f32,
        content: Box<NodeGeometry>,
    },
    Split {
        splitter_extent: f32,
        children: Vec<NodeGeometry>,
    },
}

pub(super) fn compute(
    root: &DockNode,
    width: f32,
    height: f32,
    window: &Window,
    theme: &Theme,
) -> NodeGeometry {
    let tab_height = theme.metrics.tab_strip_height.max(0.0);
    let splitter_thickness = theme.metrics.splitter_thickness.max(0.0);

    compute_node(
        root,
        width.max(1.0),
        height.max(1.0),
        tab_height,
        splitter_thickness,
        window,
        theme,
    )
}

/// Computes the minimum width required to display every tab in a subtree.
pub(super) fn minimum_width(
    node: &DockNode,
    splitter_thickness: f32,
    window: &Window,
    theme: &Theme,
) -> f32 {
    match node {
        DockNode::Tabs(group) => minimum_width_for_group(group, window, theme),
        DockNode::Column { content } => minimum_width(content, splitter_thickness, window, theme),
        DockNode::Split { axis, children, .. } => {
            let widths: Vec<f32> = children
                .iter()
                .map(|child| minimum_width(child, splitter_thickness, window, theme))
                .collect();

            match axis {
                SplitAxis::Horizontal => {
                    widths.iter().sum::<f32>()
                        + splitter_thickness * children.len().saturating_sub(1) as f32
                }
                SplitAxis::Vertical => widths.into_iter().fold(0.0, f32::max),
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
    window: &Window,
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

        DockNode::Column { content } => {
            let header_height = theme.metrics.column_header_height.max(0.0).min(height);

            GeometryKind::Column {
                header_height,
                content: Box::new(compute_node(
                    content,
                    width,
                    (height - header_height).max(0.0),
                    tab_height,
                    splitter_thickness,
                    window,
                    theme,
                )),
            }
        }

        DockNode::Split {
            axis,
            weights,
            children,
        } => {
            let child_count = children.len();
            let splitter_count = child_count.saturating_sub(1);
            let axis_extent = match axis {
                SplitAxis::Horizontal => width,
                SplitAxis::Vertical => height,
            };

            let splitter_extent = if splitter_count == 0 {
                0.0
            } else {
                splitter_thickness.min(axis_extent / splitter_count as f32)
            };

            let available = (axis_extent - splitter_extent * splitter_count as f32).max(0.0);

            let extents = match axis {
                SplitAxis::Horizontal => {
                    let minimums: Vec<f32> = children
                        .iter()
                        .map(|child| minimum_width(child, splitter_thickness, window, theme))
                        .collect();

                    distribute_with_minimums(available, weights, &minimums)
                }
                SplitAxis::Vertical => distribute_by_weights(available, weights),
            };

            let child_geometries = children
                .iter()
                .zip(extents)
                .map(|(child, extent)| {
                    let (child_width, child_height) = match axis {
                        SplitAxis::Horizontal => (extent, height),
                        SplitAxis::Vertical => (width, extent),
                    };

                    compute_node(
                        child,
                        child_width,
                        child_height,
                        tab_height,
                        splitter_thickness,
                        window,
                        theme,
                    )
                })
                .collect();

            GeometryKind::Split {
                splitter_extent,
                children: child_geometries,
            }
        }
    };

    NodeGeometry {
        width,
        height,
        kind,
    }
}

fn distribute_by_weights(available: f32, weights: &[f32]) -> Vec<f32> {
    if weights.is_empty() {
        return Vec::new();
    }

    let total_weight: f32 = weights.iter().map(|weight| weight.max(0.0)).sum();

    let mut extents = if total_weight <= f32::EPSILON {
        vec![available / weights.len() as f32; weights.len()]
    } else {
        weights
            .iter()
            .map(|weight| available * weight.max(0.0) / total_weight)
            .collect()
    };

    let allocated: f32 = extents.iter().sum();

    if let Some(last) = extents.last_mut() {
        *last += available - allocated;
    }

    extents
}

fn distribute_with_minimums(available: f32, weights: &[f32], minimums: &[f32]) -> Vec<f32> {
    if weights.is_empty() {
        return Vec::new();
    }

    let minimum_total: f32 = minimums.iter().sum();

    if minimum_total > available {
        // The tree does not fit: retain weighted proportions as a fallback.
        return distribute_by_weights(available, weights);
    }

    let mut extents = vec![0.0; weights.len()];
    let mut fixed = vec![false; weights.len()];

    loop {
        let fixed_total: f32 = extents
            .iter()
            .enumerate()
            .filter(|(index, _)| fixed[*index])
            .map(|(_, extent)| *extent)
            .sum();

        let remaining = (available - fixed_total).max(0.0);
        let remaining_weight: f32 = weights
            .iter()
            .enumerate()
            .filter(|(index, _)| !fixed[*index])
            .map(|(_, weight)| weight.max(0.0))
            .sum();

        let active_count = fixed.iter().filter(|is_fixed| !**is_fixed).count();

        if active_count == 0 {
            break;
        }

        if remaining_weight <= f32::EPSILON {
            let equal_extent = remaining / active_count as f32;

            for (index, extent) in extents.iter_mut().enumerate() {
                if !fixed[index] {
                    *extent = equal_extent;
                }
            }

            break;
        }

        let undersized = weights.iter().enumerate().find_map(|(index, weight)| {
            if fixed[index] {
                return None;
            }

            let proposed = remaining * weight.max(0.0) / remaining_weight;

            (proposed + 0.001 < minimums[index]).then_some(index)
        });

        if let Some(index) = undersized {
            extents[index] = minimums[index];
            fixed[index] = true;
        } else {
            for (index, extent) in extents.iter_mut().enumerate() {
                if !fixed[index] {
                    *extent = remaining * weights[index].max(0.0) / remaining_weight;
                }
            }

            break;
        }
    }

    let allocated: f32 = extents.iter().sum();

    if let Some(last) = extents.last_mut() {
        *last += available - allocated;
    }

    extents
}
