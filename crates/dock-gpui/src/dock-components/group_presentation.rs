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

/// Source-side presentation changes while one or more cached PanelViews are transported.
#[derive(Clone, Default)]
pub(crate) struct PanelTransferPresentation {
    pub transferred_panels: HashSet<PanelId>,
    pub groups_without_visible_tabs: HashSet<NodeId>,
    pub panels_in_hidden_groups: HashSet<PanelId>,
}

impl PanelTransferPresentation {
    pub(crate) fn new(
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        transferred_panels: &[PanelId],
    ) -> Self {
        let transferred_panels = transferred_panels.iter().copied().collect::<HashSet<_>>();
        let groups_without_visible_tabs = group_presentations
            .iter()
            .filter_map(|(group, presentation)| {
                (presentation.tab_count() > 0
                    && presentation
                        .tabs
                        .iter()
                        .all(|tab| transferred_panels.contains(&tab.panel)))
                .then_some(*group)
            })
            .collect::<HashSet<_>>();
        let panels_in_hidden_groups = group_presentations
            .iter()
            .filter(|(group, _)| groups_without_visible_tabs.contains(group))
            .flat_map(|(_, presentation)| presentation.tabs.iter().map(|tab| tab.panel))
            .collect::<HashSet<_>>();

        Self {
            transferred_panels,
            groups_without_visible_tabs,
            panels_in_hidden_groups,
        }
    }
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


    #[test]
    fn panel_transfer_presentation_hides_only_groups_without_remaining_tabs() {
        use std::collections::HashMap;

        use dock_core::PanelId;
        use slotmap::SlotMap;

        use super::{GroupPresentation, PanelTransferPresentation, TabPresentation};

        let mut group_ids = SlotMap::<NodeId, ()>::with_key();
        let group = group_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let moved = panel_ids.insert(());
        let remaining = panel_ids.insert(());
        let presentations = HashMap::from([(
            group,
            GroupPresentation {
                active_panel: Some(moved),
                tabs: vec![
                    TabPresentation {
                        panel: moved,
                        title: "Moved".into(),
                        icon: None,
                        index: 0,
                    },
                    TabPresentation {
                        panel: remaining,
                        title: "Remaining".into(),
                        icon: None,
                        index: 1,
                    },
                ],
                ..GroupPresentation::default()
            },
        )]);

        let partial = PanelTransferPresentation::new(&presentations, &[moved]);
        assert!(partial.transferred_panels.contains(&moved));
        assert!(!partial.groups_without_visible_tabs.contains(&group));
        assert!(!partial.panels_in_hidden_groups.contains(&remaining));

        let complete = PanelTransferPresentation::new(&presentations, &[moved, remaining]);
        assert!(complete.groups_without_visible_tabs.contains(&group));
        assert!(complete.panels_in_hidden_groups.contains(&moved));
        assert!(complete.panels_in_hidden_groups.contains(&remaining));
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
