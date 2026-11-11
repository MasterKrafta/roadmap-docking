use std::collections::{HashMap, HashSet};

use dock_core::{Layout, LayoutRects, Node, NodeId, PanelId};
use gpui::SharedString;

use crate::{IconSource, PanelRegistry};

#[derive(Clone)]
pub(crate) struct TabPresentation {
    pub panel: PanelId,
    pub title: SharedString,
    pub icon: Option<SharedString>,
    pub index: usize,
}

#[derive(Clone, Default)]
pub(crate) struct GroupPresentation {
    /// Effective state after Column overflow compaction.
    pub collapsed: bool,
    /// Persistent user-selected collapsed state in the Tabs model.
    pub model_collapsed: bool,
    /// This Group is temporarily collapsed by its overflowing Column.
    pub auto_collapsed: bool,
    /// This Group is the one temporarily expanded by user choice.
    pub auto_expanded: bool,
    pub active_panel: Option<PanelId>,
    pub tabs: Vec<TabPresentation>,
}

/// Builds presentation metadata for Tabs nodes without retaining another tree.
pub(crate) fn collect_group_presentations(
    layout: &Layout,
    registry: &PanelRegistry,
    node_ids: &HashSet<NodeId>,
    rects: Option<&LayoutRects>,
) -> HashMap<NodeId, GroupPresentation> {
    let mut groups = HashMap::new();
    for group in node_ids {
        let Some(Node::Tabs(tabs)) = layout.node(*group) else {
            continue;
        };

        let tab_presentations = tabs
            .children
            .iter()
            .enumerate()
            .filter_map(|(index, child)| {
                let Some(Node::Panel(panel)) = layout.node(*child) else {
                    return None;
                };
                let instance = layout.panel_instance(panel.panel_id)?;
                let descriptor = registry.get(&instance.key);
                let title = descriptor
                    .map(|descriptor| descriptor.title().clone())
                    .unwrap_or_else(|| instance.key.to_string().into());
                let icon = descriptor.and_then(|descriptor| match descriptor.icon() {
                    IconSource::Asset(path) => Some(path.clone()),
                    IconSource::None => None,
                });
                Some(TabPresentation {
                    panel: panel.panel_id,
                    title,
                    icon,
                    index,
                })
            })
            .collect();

        let active_panel = tabs
            .active_child()
            .and_then(|child| match layout.node(child) {
                Some(Node::Panel(panel)) => Some(panel.panel_id),
                _ => None,
            });

        let auto_collapsed =
            rects.is_some_and(|rects| rects.auto_collapsed_groups.contains(group));
        let auto_expanded =
            rects.is_some_and(|rects| rects.auto_expanded_groups.contains(group));
        groups.insert(
            *group,
            GroupPresentation {
                collapsed: tabs.collapsed || auto_collapsed,
                model_collapsed: tabs.collapsed,
                auto_collapsed,
                auto_expanded,
                active_panel,
                tabs: tab_presentations,
            },
        );
    }
    groups
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use dock_core::{Layout, NodeId, Profile};
    use slotmap::Key;

    use super::collect_group_presentations;

    #[test]
    fn empty_layout_has_no_group_presentations() {
        let groups = collect_group_presentations(
            &Layout::new(Profile::Photoshop),
            &crate::PanelRegistry::new(),
            &HashSet::<NodeId>::new(),
            None,
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn presentation_collection_ignores_non_tabs_nodes() {
        let groups = collect_group_presentations(
            &Layout::new(Profile::Generic),
            &crate::PanelRegistry::new(),
            &HashSet::from([NodeId::null()]),
            None,
        );
        assert!(groups.is_empty());
    }
}

impl GroupPresentation {
    /// Number of tabs currently belonging to this Group.
    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Whether this Group contains exactly one tab.
    pub(crate) fn is_single_tab(&self) -> bool {
        self.tab_count() == 1
    }
}
