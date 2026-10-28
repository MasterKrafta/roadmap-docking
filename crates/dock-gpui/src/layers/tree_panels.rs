use std::collections::HashMap;

use dock_core::{Axis, Layout, Node, NodeId, PanelId};
use gpui::{AnyElement, IntoElement};

use crate::components::{PanelSurface, SplitterHandle};
use crate::host::DockHost;
use crate::requests::ResizeHandle;
use crate::surface::RenderData;

use super::tree::TreeArea;

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let rects = area.rects;
    let nodes = area.nodes;
    let mut elements = Vec::new();

    let legacy_panel_nodes = data.frame.is_none().then(|| {
        layout.nodes().filter_map(|(node_id, node)| match node {
            Node::Panel(panel) => Some((panel.panel_id, node_id)),
            _ => None,
        }).collect::<HashMap<PanelId, NodeId>>()
    });

    for panel_rect in rects.panels.iter().filter(|entry| {
        let node_id = data.frame.and_then(|frame| frame.panel_node(entry.panel)).or_else(|| {
            legacy_panel_nodes.as_ref().and_then(|nodes| nodes.get(&entry.panel).copied())
        });
        node_id.is_some_and(|node_id| nodes.contains(&node_id))
    }) {
        let Some(instance) = layout.panel_instance(panel_rect.panel) else { continue; };
        elements.push(
            PanelSurface {
                rect: area.rect(panel_rect.rect),
                theme: *data.theme,
                content: host.render_panel(instance),
            }.into_any_element(),
        );
    }

    for splitter in rects.splitters.iter().filter(|entry| nodes.contains(&entry.owner)) {
        let handle = supported_splitter_handle(
            layout,
            splitter.owner,
            splitter.boundary_index,
            splitter.axis,
        );
        elements.push(
            SplitterHandle {
                id: format!("dock-splitter-{:?}-{}", splitter.owner, splitter.boundary_index),
                rect: area.rect(splitter.rect),
                axis: splitter.axis,
                theme: *data.theme,
                on_resize_start: handle.map(|handle| host.resize_start(handle)),
                on_resize_key: handle.map(|handle| host.resize_key(handle)),
            }.into_any_element(),
        );
    }

    elements
}

fn supported_splitter_handle(
    layout: &Layout,
    owner: NodeId,
    boundary_index: usize,
    axis: Axis,
) -> Option<ResizeHandle> {
    let supported = match (axis, layout.node(owner)) {
        (Axis::Y, Some(Node::Column(column))) => {
            boundary_index < column.groups.len().saturating_sub(1)
        }
        (Axis::X, Some(Node::Split(split))) => {
            split.axis == Axis::X
                && boundary_index < split.children.len().saturating_sub(1)
                && split
                    .children
                    .iter()
                    .all(|child| matches!(layout.node(*child), Some(Node::Column(_))))
        }
        _ => false,
    };

    supported.then_some(ResizeHandle::Splitter {
        owner,
        boundary_index,
        axis,
    })
}


#[cfg(test)]
mod tests {
    use super::supported_splitter_handle;
    use crate::dock::DockController;
    use crate::requests::ResizeHandle;
    use dock_core::{Axis, DockCommand, DockSide, Node, Point, Rect, Size};
    use ui::Theme;

    #[test]
    fn splitter_handles_reject_out_of_range_boundaries_on_both_axes() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout();

        let (column_id, group_count) = layout
            .nodes()
            .find_map(|(id, node)| match node {
                Node::Column(column) if column.groups.len() >= 2 => {
                    Some((id, column.groups.len()))
                }
                _ => None,
            })
            .expect("initial layout should contain a column with multiple groups");
        assert!(supported_splitter_handle(
            layout,
            column_id,
            group_count - 2,
            Axis::Y
        ).is_some());
        assert!(supported_splitter_handle(
            layout,
            column_id,
            group_count - 1,
            Axis::Y
        ).is_none());

        let (split_id, child_count) = layout
            .nodes()
            .find_map(|(id, node)| match node {
                Node::Split(split) if split.axis == Axis::X && split.children.len() >= 2 => {
                    Some((id, split.children.len()))
                }
                _ => None,
            })
            .expect("initial layout should contain a horizontal split");
        assert!(supported_splitter_handle(
            layout,
            split_id,
            child_count - 2,
            Axis::X
        ).is_some());
        assert!(supported_splitter_handle(
            layout,
            split_id,
            child_count - 1,
            Axis::X
        ).is_none());
    }

    #[test]
    fn floating_horizontal_split_exposes_resize_handle() {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let owner = controller.layout().main_area(DockSide::Right).unwrap().root;
        assert!(matches!(
            controller.layout().node(owner),
            Some(Node::Split(split)) if split.axis == Axis::X
        ));
        controller.apply(DockCommand::Float {
            node: owner,
            bounds: Rect::new(Point::new(80.0, 60.0), Size::new(700.0, 560.0)),
        }).unwrap();

        assert_eq!(
            supported_splitter_handle(controller.layout(), owner, 0, Axis::X),
            Some(ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::X,
            })
        );
    }

    #[test]
    fn horizontal_split_nodes_expose_resize_handles() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout();
        let owner = layout
            .nodes()
            .find_map(|(id, node)| match node {
                Node::Split(split) if split.axis == Axis::X => Some(id),
                _ => None,
            })
            .expect("initial dock layout should contain a horizontal split");

        assert_eq!(
            supported_splitter_handle(layout, owner, 0, Axis::X),
            Some(ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::X,
            })
        );
    }
}
