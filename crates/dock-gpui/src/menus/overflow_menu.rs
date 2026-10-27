use dock_core::{DockCommand, Layout, Node, NodeId};
use crate::frame::FrameGeometry;

use super::MenuItemModel;

pub(super) fn items(layout: &Layout, frame: &FrameGeometry, group: NodeId) -> Vec<MenuItemModel> {
    let hidden_tabs = frame
        .rects()
        .tab_overflows
        .iter()
        .find(|overflow| overflow.group == group)
        .map(|overflow| overflow.hidden_tabs.clone())
        .unwrap_or_default();

    let mut items = hidden_tabs
        .into_iter()
        .filter_map(|panel| {
            let title = layout.panel_instance(panel).map(crate::panels::panel_title)?;
            Some(MenuItemModel {
                id: format!("overflow-{:?}-{:?}", group, panel),
                label: title,
                checked: matches!(
                    layout.node(group),
                    Some(Node::Tabs(tabs))
                        if tabs.children.get(tabs.active_index).is_some_and(|child| matches!(
                            layout.node(*child),
                            Some(Node::Panel(node_panel)) if node_panel.panel_id == panel
                        ))
                ),
                enabled: true,
                command: Some(DockCommand::ActivateTab { group, panel }),
            })
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        items.push(MenuItemModel {
            id: format!("overflow-empty-{:?}", group),
            label: "Нет скрытых вкладок".to_owned(),
            checked: false,
            enabled: false,
            command: None,
        });
    }

    items
}
