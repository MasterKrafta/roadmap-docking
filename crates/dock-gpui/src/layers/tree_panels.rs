use std::collections::HashMap;

use dock_core::{Axis, DockSide, Layout, Node, NodeId, PanelId};
use gpui::{AnyElement, IntoElement};

use crate::components::{PanelSurface, SplitterHandle};
use crate::host::DockHost;
use crate::requests::ResizeHandle;
use crate::surface::RenderData;
use crate::tree_query::main_column_ids;

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
    match axis {
        Axis::Y if matches!(layout.node(owner), Some(Node::Column(_))) => {
            Some(ResizeHandle::Splitter { owner, boundary_index, axis })
        }
        Axis::X if [DockSide::Left, DockSide::Right].into_iter().any(|side| {
            layout.main_area(side).is_some_and(|area| area.root == owner)
                && !main_column_ids(layout, side).is_empty()
        }) => Some(ResizeHandle::Splitter { owner, boundary_index, axis }),
        _ => None,
    }
}
