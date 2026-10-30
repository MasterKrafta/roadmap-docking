use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnMode, ColumnRect, DockMetrics, GroupRect, NodeId, PanelRect, Point,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{AnyView, Entity, IntoElement, ParentElement, ScrollHandle, Styled, div};
use ui::Theme;

use crate::DockController;

use super::{
    column::ColumnView,
    context_menu::GroupMenuToggle,
    splitter::splitter_surface,
    tab_overflow::TabOverflowToggle,
    tab_strip::GroupPresentation,
};

/// Main-dock presentation layer. All geometry comes from a Core snapshot;
/// this component stores no second copy of the docking tree.
pub struct MainDockView;

impl MainDockView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: &[(PanelRect, AnyView)],
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
        overflow_group: Option<NodeId>,
        toggle_tab_overflow: TabOverflowToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        vertical_overflow: &[VerticalOverflow],
        main_node_ids: &HashSet<NodeId>,
        main_column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        scroll_handles: &HashMap<NodeId, ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut layer = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full();

        for column in columns {
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);
            let group_ids = main_column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let group_rects = groups
                .iter()
                .filter(|group| group_ids.contains(&group.group))
                .copied()
                .collect::<Vec<_>>();
            let group_splitters = splitters
                .iter()
                .filter(|splitter| {
                    splitter.owner == column.column && splitter.axis == Axis::Y
                })
                .copied()
                .collect::<Vec<_>>();
            let overflow = vertical_overflow
                .iter()
                .find(|overflow| overflow.owner == column.column)
                .copied();
            let scroll_handle = scroll_handles.get(&column.column).cloned();
            let active_panels = group_ids
                .iter()
                .filter_map(|group| {
                    group_presentations
                        .get(group)
                        .and_then(|presentation| presentation.active_panel)
                })
                .collect::<HashSet<_>>();
            let column_panels = panels
                .iter()
                .filter(|(panel, _)| active_panels.contains(&panel.panel))
                .cloned()
                .collect::<Vec<_>>();

            layer = layer.child(ColumnView::render(
                *column,
                mode,
                &group_rects,
                &column_panels,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                metrics,
                menu_group,
                toggle_group_menu.clone(),
                overflow_group,
                toggle_tab_overflow.clone(),
                middle_click_closes_tabs,
                &group_splitters,
                overflow,
                scroll_handle,
                controller.clone(),
                theme,
            ));
        }

        // Axis::X represents vertical dividers between main-dock columns.
        // They are painted after columns so the handles remain visible.
        for splitter in splitters.iter().filter(|splitter| {
            splitter.axis == Axis::X && main_node_ids.contains(&splitter.owner)
        }) {
            layer = layer.child(splitter_surface(*splitter, Point::ZERO, theme));
        }

        layer
    }
}

