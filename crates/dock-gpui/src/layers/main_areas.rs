use std::collections::{HashMap, HashSet};

use dock_core::{DockSide, Node, NodeId, Point, Rect, Size};
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::chrome_geometry::rail_control_row_height;
use crate::components::ColumnModeButton;
use crate::dock::cycle_column_mode;
use crate::geometry::translate_rect_to_local;
use crate::host::DockHost;
use crate::surface::RenderData;

use super::{
    rect_element,
    tree::{self, TreeArea},
};

/// A scroll viewport and the nodes inside its column, reused for drop-zone clipping.
pub(crate) struct ScrollOverflow {
    pub owner: NodeId,
    pub viewport: Rect,
    pub nodes: HashSet<NodeId>,
}

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
) -> (Vec<AnyElement>, Vec<ScrollOverflow>) {
    let layout = data.layout;
    let rects = data.rects;
    let frame = data.frame;
    let theme = data.theme;
    let mut elements = Vec::new();
    let mut main_column_overflows = Vec::new();

    for side in [DockSide::Left, DockSide::Right] {
        let Some(area) = layout.main_area(side) else {
            continue;
        };
        let nodes = tree::nodes_for_root(layout, area.root, frame);
        let overflow_columns: Vec<_> = rects
            .vertical_overflow
            .iter()
            .filter(|overflow| {
                nodes.contains(&overflow.owner)
                    && matches!(layout.node(overflow.owner), Some(Node::Column(_)))
            })
            .copied()
            .collect();

        let mut non_scrolling_nodes = nodes;
        for overflow in &overflow_columns {
            let column_nodes = tree::nodes_for_root(layout, overflow.owner, frame);
            for node in &column_nodes {
                non_scrolling_nodes.remove(node);
            }
            main_column_overflows.push(ScrollOverflow {
                owner: overflow.owner,
                viewport: overflow.viewport,
                nodes: column_nodes,
            });
        }

        if !non_scrolling_nodes.is_empty() {
            elements.extend(tree::elements(
                data,
                host,
                TreeArea {
                    rects,
                    nodes: &non_scrolling_nodes,
                    origin: Point::ZERO,
                    content_column: None,
                },
            ));
        }

        for overflow in overflow_columns {
            let Some(scroll_handle) = scroll_handles.get(&overflow.owner) else {
                continue;
            };
            let column_nodes = tree::nodes_for_root(layout, overflow.owner, frame);
            let content_extent =
                translate_rect_to_local(overflow.content_extent, overflow.viewport.origin);
            let mut content = div()
                .relative()
                .w(px(overflow.content_extent.size.width))
                .h(px(overflow.content_extent.size.height));

            for element in tree::elements(
                data,
                host,
                TreeArea {
                    rects,
                    nodes: &column_nodes,
                    origin: overflow.viewport.origin,
                    content_column: Some((overflow.owner, content_extent)),
                },
            ) {
                content = content.child(element);
            }

            elements.push(
                rect_element(overflow.viewport, theme.colors.dock_background)
                    .id(format!("dock-column-scroll-{:?}", overflow.owner))
                    .overflow_y_scroll()
                    .track_scroll(scroll_handle)
                    .child(content)
                    .into_any_element(),
            );

            // Keep rail-mode controls fixed to the viewport, outside the scrollable
            // content whose tab headers were shifted down to reserve this row.
            if let Some(Node::Column(column)) = layout.node(overflow.owner)
                && column.mode.is_rail()
            {
                let button_height = rail_control_row_height(
                    theme.metrics.tab_strip_height,
                    theme.metrics.collapsed_group_height,
                    overflow.viewport.size.height,
                );
                if button_height > 0.0 {
                    elements.push(
                        ColumnModeButton {
                            column: overflow.owner,
                            mode: column.mode,
                            rect: Rect::new(
                                overflow.viewport.origin,
                                Size::new(overflow.viewport.size.width, button_height),
                            ),
                            theme: *theme,
                            on_toggle: host.command(cycle_column_mode(overflow.owner, column.mode)),
                        }
                        .into_any_element(),
                    );
                }
            }
        }
    }

    (elements, main_column_overflows)
}
