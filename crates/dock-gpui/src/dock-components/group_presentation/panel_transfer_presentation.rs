use std::collections::{HashMap, HashSet};

use dock_core::{
    DockMetrics, NodeId, PanelId, PanelRect, Point, Rect, Size, TabHeaderRect, TabOverflowRect,
    TabStripRect,
};

use super::group_presentation::GroupPresentation;

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
