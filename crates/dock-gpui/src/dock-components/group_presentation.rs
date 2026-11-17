mod group_presentation;
mod panel_transfer_presentation;
mod tab_presentation;

pub(crate) use group_presentation::GroupPresentation;
pub(crate) use panel_transfer_presentation::PanelTransferPresentation;
pub(crate) use tab_presentation::TabPresentation;

use std::collections::{HashMap, HashSet};

use dock_core::{Layout, LayoutRects, Node, NodeId, PanelId};

use crate::{IconSource, PanelRegistry};

/// Returns active panel ids that should be mounted by the host.
///
/// A Group being rendered as a DragHost placeholder must not leave its
/// replacement active tab mounted underneath that placeholder.
pub(crate) fn active_panels_for_groups(
    group_ids: impl IntoIterator<Item = NodeId>,
    group_presentations: &HashMap<NodeId, GroupPresentation>,
    dragged_group: Option<NodeId>,
) -> HashSet<PanelId> {
    group_ids
        .into_iter()
        .filter(|group| Some(*group) != dragged_group)
        .filter_map(|group| {
            group_presentations
                .get(&group)
                .and_then(|presentation| presentation.active_panel)
        })
        .collect()
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
    fn dragged_group_does_not_keep_a_replacement_panel_mounted() {
        use std::collections::HashMap;

        use dock_core::{NodeId, PanelId};
        use slotmap::SlotMap;

        use super::{GroupPresentation, TabPresentation, active_panels_for_groups};

        let mut group_ids = SlotMap::<NodeId, ()>::with_key();
        let dragged_group = group_ids.insert(());
        let other_group = group_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let transferred_panel = panel_ids.insert(());
        let replacement_panel = panel_ids.insert(());
        let other_active_panel = panel_ids.insert(());

        let presentations = HashMap::from([
            (
                dragged_group,
                GroupPresentation {
                    active_panel: Some(replacement_panel),
                    tabs: vec![
                        TabPresentation {
                            panel: transferred_panel,
                            title: "Transferred".into(),
                            icon: None,
                            index: 0,
                        },
                        TabPresentation {
                            panel: replacement_panel,
                            title: "Replacement".into(),
                            icon: None,
                            index: 1,
                        },
                    ],
                    ..GroupPresentation::default()
                },
            ),
            (
                other_group,
                GroupPresentation {
                    active_panel: Some(other_active_panel),
                    tabs: Vec::new(),
                    ..GroupPresentation::default()
                },
            ),
        ]);

        let active_panels = active_panels_for_groups(
            [dragged_group, other_group],
            &presentations,
            Some(dragged_group),
        );

        assert!(!active_panels.contains(&transferred_panel));
        assert!(!active_panels.contains(&replacement_panel));
        assert!(active_panels.contains(&other_active_panel));
    }

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
