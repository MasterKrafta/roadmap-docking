use super::*;

#[path = "node_layout/column_layout.rs"]
mod column_layout;
#[path = "node_layout/tabs_layout.rs"]
mod tabs_layout;

use self::column_layout::layout_column;
use self::tabs_layout::layout_tabs;

pub(super) fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayoutState)?;

    // Columns and vertical splits manage their own stack overflow. Other nodes
    // need an expanded content rect when the parent viewport is shorter than
    // their minimum geometry, with a separate viewport/content extent record.
    let manages_vertical_stack = matches!(node, Node::Column(_))
        || matches!(node, Node::Split(split) if split.axis == Axis::Y);
    let mut content_rect = rect;
    if !manages_vertical_stack {
        let minimum = minimum_node_size(layout, id, metrics, minimum_sizes)?;
        if content_rect.size.height < minimum.height {
            push_vertical_overflow(id, rect, minimum.height, output)?;
            content_rect.size.height = minimum.height;
            content_rect
                .validate()
                .map_err(ComputeRectsError::Geometry)?;
        }
    }

    match node {
        Node::Column(column) => layout_column(
            layout,
            id,
            column,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Tabs(tabs) => layout_tabs(
            layout,
            id,
            tabs,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Panel(panel) => {
            let minimum = layout
                .panel_instance(panel.panel_id)
                .ok_or(ComputeRectsError::InvalidLayoutState)?
                .constraints
                .min_size;
            if content_rect.size.width < minimum.width {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::X,
                    available: content_rect.size.width,
                    required: minimum.width,
                });
            }
            if content_rect.size.height < minimum.height {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::Y,
                    available: content_rect.size.height,
                    required: minimum.height,
                });
            }
            output
                .panels
                .push(PanelRect::new(panel.panel_id, content_rect));
            Ok(())
        }
        Node::Split(split) => layout_split(
            layout,
            id,
            split,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
    }
}
