use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, FlyoutState, FloatingGroupRect, GroupRect, NodeId,
    PanelRect, Point, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, App, Entity, IntoElement, ParentElement, Styled, WeakEntity, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{
    components::main_dock::render_column_content,
    dock_components::{
        CompositionHeader, DockGroupView, GroupPresentation,
    },
    DockController,
};
use super::super::context_menu::GroupMenuToggle;

/// Shared presentation helpers for a complete Floating Column.
pub(super) struct FloatingWindowContent;

impl FloatingWindowContent {
    /// Renders one complete Column presentation inside the caller's viewport.
    ///
    /// Core rectangles remain in composition-local coordinates. parent_origin is
    /// the Core-space origin represented by the native client area; a permanent
    /// Column host may begin half-way through an adjacent Axis::X splitter gap.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_column(
        floating: FloatingGroupRect,
        column: ColumnRect,
        parent_origin: Point,
        groups: &[GroupRect],
        panels: &[(PanelRect, Option<AnyView>)],
        dragged_group: Option<NodeId>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        rail_display_orders: &HashMap<NodeId, Vec<ColumnDisplayItem>>,
        splitters: &[SplitterRect],
        active_splitter: Option<(NodeId, usize, dock_core::Axis)>,
        vertical_overflow: &[VerticalOverflow],
        scroll_handles: &HashMap<NodeId, gpui::ScrollHandle>,
        active_flyout: Option<FlyoutState>,
        metrics: DockMetrics,
        show_header: bool,
        show_close_button: bool,
        close_action: Option<Box<dyn Fn(&mut App) + 'static>>,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let group_ids = column_groups
            .get(&column.column)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mode = column_modes
            .get(&column.column)
            .copied()
            .unwrap_or(ColumnMode::Expanded);
        let overflow = vertical_overflow
            .iter()
            .find(|overflow| overflow.owner == column.column)
            .copied();

        let mut root = div()
            .id(format!("floating-column-{:?}-{:?}", floating.group, column.column))
            .absolute()
            .left(px(-parent_origin.x))
            .top(px(-parent_origin.y))
            .size_full()
            .child(render_column_content(
                column,
                group_ids,
                groups,
                panels,
                mode,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                rail_display_orders
                    .get(&column.column)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                active_flyout,
                metrics,
                menu_group,
                context_menu,
                splitters,
                active_splitter,
                overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                Some(floating.group),
                show_header,
                dragged_group,
            ));

        if show_header {
            let header_rect = dock_core::Rect::new(
                column.rect.origin,
                dock_core::Size::new(column.rect.size.width, metrics.column_header_height),
            );
            root = root.child(CompositionHeader::render(
                format!("floating-column-header-{:?}", column.column),
                header_rect,
                std::slice::from_ref(&column),
                metrics,
                theme,
                show_close_button,
                close_action,
            ));
        }

        root
    }
}
