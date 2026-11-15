use std::collections::{HashMap, HashSet};

use dock_core::{
    DockMetrics, Layout, LayoutRects, Node, NodeId, PanelId, PanelRect, Point, Rect, Size,
    TabHeaderRect, TabOverflowRect, TabStripRect,
  };
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
    groups_with_partial_transfer: HashSet<NodeId>,
    original_active_panels: HashMap<NodeId, PanelId>,
    replacement_active_panels: HashMap<NodeId, PanelId>,
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
        let groups_with_partial_transfer = group_presentations
            .iter()
            .filter_map(|(group, presentation)| {
                let transferred_count = presentation
                    .tabs
                    .iter()
                    .filter(|tab| transferred_panels.contains(&tab.panel))
                    .count();
                (transferred_count > 0 && transferred_count < presentation.tab_count())
                    .then_some(*group)
            })
            .collect::<HashSet<_>>();
        let original_active_panels = group_presentations
            .iter()
            .filter_map(|(group, presentation)| {
                presentation.active_panel.map(|panel| (*group, panel))
            })
            .collect::<HashMap<_, _>>();
        let replacement_active_panels = group_presentations
            .iter()
            .filter_map(|(group, presentation)| {
                let active_panel = presentation.active_panel?;
                if !transferred_panels.contains(&active_panel) {
                    return None;
                }
                let active_index = presentation
                    .tabs
                    .iter()
                    .find(|tab| tab.panel == active_panel)
                    .map(|tab| tab.index)
                    .unwrap_or(0);
                let replacement = presentation
                    .tabs
                    .iter()
                    .filter(|tab| !transferred_panels.contains(&tab.panel))
                    .min_by_key(|tab| {
                        (tab.index < active_index, tab.index.abs_diff(active_index))
                    })?
                    .panel;
                Some((*group, replacement))
            })
            .collect::<HashMap<_, _>>();

        Self {
            transferred_panels,
            groups_without_visible_tabs,
            panels_in_hidden_groups,
            groups_with_partial_transfer,
            original_active_panels,
            replacement_active_panels,
        }
    }

    /// Selects a remaining tab for presentation when the active tab is being dragged.
    /// Core's active index remains unchanged until the user releases the drag.
    pub(crate) fn apply_to_group_presentations(
        &self,
        group_presentations: &mut HashMap<NodeId, GroupPresentation>,
    ) {
        for (group, replacement) in &self.replacement_active_panels {
            if let Some(presentation) = group_presentations.get_mut(group) {
                presentation.active_panel = Some(*replacement);
            }
        }
    }

    /// Keeps the source content mounted under the remaining tab while its original
    /// active PanelView is being transported by DragHost.
    pub(crate) fn panel_rects_with_active_replacements(
        &self,
        panel_rects: &[PanelRect],
    ) -> Vec<PanelRect> {
        let mut result = panel_rects.to_vec();
        for (group, replacement) in &self.replacement_active_panels {
            let Some(original_active) = self.original_active_panels.get(group) else {
                continue;
            };
            if let Some(rect) = panel_rects
                .iter()
                .find(|rect| rect.panel == *original_active)
            {
                result.push(PanelRect::new(*replacement, rect.rect));
            }
        }
        result
    }

    /// Removes transported headers and packs remaining visible headers into their
    /// source strip, matching the geometry Core would produce after removal.
    pub(crate) fn visible_tab_headers(
        &self,
        headers: &[TabHeaderRect],
        strips: &[TabStripRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_groups: &HashSet<NodeId>,
        metrics: DockMetrics,
    ) -> Vec<TabHeaderRect> {
        let mut result = headers
            .iter()
            .filter(|header| {
                !self.transferred_panels.contains(&header.panel)
                    && !self.groups_without_visible_tabs.contains(&header.group)
            })
            .copied()
            .collect::<Vec<_>>();

        for group in &self.groups_with_partial_transfer {
            // Repacking a partially visible overflow strip requires Core's complete
            // overflow selection, so retain its measured rectangles unchanged.
            if tab_overflows.iter().any(|overflow| {
                overflow.group == *group && !overflow.hidden_tabs.is_empty()
            }) {
                continue;
            }
            let Some(strip) = strips.iter().find(|strip| strip.group == *group) else {
                continue;
            };
            let mut header_indices = result
                .iter()
                .enumerate()
                .filter_map(|(index, header)| (header.group == *group).then_some(index))
                .collect::<Vec<_>>();
            header_indices.sort_by_key(|index| result[*index].index);
            if header_indices.is_empty() {
                continue;
            }

            let compact_column_group = column_groups.contains(group)
                && group_presentations
                    .get(group)
                    .is_some_and(|presentation| !presentation.collapsed);
            let count = header_indices.len();
            let tab_width = if compact_column_group {
                metrics.min_tab_width
            } else {
                strip.rect.size.width / count as f32
            };
            if !tab_width.is_finite() || tab_width <= 0.0 {
                continue;
            }

            for (visible_index, header_index) in header_indices.into_iter().enumerate() {
                let header = result[header_index];
                let x = strip.rect.origin.x + tab_width * visible_index as f32;
                let width = if compact_column_group || visible_index + 1 < count {
                    tab_width
                } else {
                    (strip.rect.origin.x + strip.rect.size.width - x).max(0.0)
                };
                result[header_index] = TabHeaderRect::new(
                    header.group,
                    header.panel,
                    header.index,
                    Rect::new(
                        Point::new(x, strip.rect.origin.y),
                        Size::new(width, strip.rect.size.height),
                    ),
                );
            }
        }

        result
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
    fn partial_tab_transfer_reflows_the_strip_and_activates_the_remaining_panel() {
        use std::collections::HashMap;

        use dock_core::{
            DockMetrics, PanelId, PanelRect, Point, Rect, Size, TabHeaderRect, TabStripRect,
        };
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
        let transfer = PanelTransferPresentation::new(&presentations, &[moved]);
        let mut rendered_presentations = presentations.clone();
        transfer.apply_to_group_presentations(&mut rendered_presentations);
        assert_eq!(rendered_presentations[&group].active_panel, Some(remaining));

        let content_rect = Rect::new(Point::new(120.0, 48.0), Size::new(360.0, 240.0));
        let panel_rects =
            transfer.panel_rects_with_active_replacements(&[PanelRect::new(moved, content_rect)]);
        assert!(panel_rects
            .iter()
            .any(|rect| rect.panel == remaining && rect.rect == content_rect));

        let metrics = DockMetrics::default();
        let strip = TabStripRect::new(
            group,
            Rect::new(Point::new(80.0, 24.0), Size::new(300.0, 28.0)),
        );
        let headers = [
            TabHeaderRect::new(
                group,
                moved,
                0,
                Rect::new(Point::new(80.0, 24.0), Size::new(120.0, 28.0)),
            ),
            TabHeaderRect::new(
                group,
                remaining,
                1,
                Rect::new(Point::new(200.0, 24.0), Size::new(120.0, 28.0)),
            ),
        ];
        let visible = transfer.visible_tab_headers(
            &headers,
            &[strip],
            &[],
            &rendered_presentations,
            &HashSet::from([group]),
            metrics,
        );
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].panel, remaining);
        assert_eq!(visible[0].index, 1);
        assert_eq!(visible[0].rect.origin, strip.rect.origin);
        assert_eq!(visible[0].rect.size.width, metrics.min_tab_width);
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
