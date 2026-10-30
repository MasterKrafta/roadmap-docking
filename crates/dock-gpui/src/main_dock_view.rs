use std::collections::{HashMap, HashSet};

use crate::group_view::{GroupMenuToggle, GroupPresentation, GroupView};

use dock_core::{
    Axis, ColumnMode, ColumnRect, DockCommand, DockMetrics, GroupRect, NodeId, Point,
    SplitterRect, TabHeaderRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    Entity, IntoElement, ParentElement, ScrollHandle, Styled, div, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::DockController;

/// Main-dock presentation layer. All geometry comes from a Core snapshot;
/// this component stores no second copy of the docking tree.
pub struct MainDockView;

impl MainDockView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        columns: &[ColumnRect],
        groups: &[GroupRect],
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
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

            layer = layer.child(ColumnView::render(
                *column,
                mode,
                &group_rects,
                tab_strips,
                tab_headers,
                group_presentations,
                metrics,
                menu_group,
                toggle_group_menu.clone(),
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

/// One main-dock column. Core supplies the group rectangles and the scroll
/// extent; GPUI supplies only the clipping and ephemeral scroll position.
pub struct ColumnView;

impl ColumnView {
    #[allow(clippy::too_many_arguments)]
    fn render(
        column: ColumnRect,
        mode: ColumnMode,
        groups: &[GroupRect],
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
        splitters: &[SplitterRect],
        overflow: Option<VerticalOverflow>,
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let viewport = overflow.map_or(column.rect, |value| value.viewport);
        let content_extent = overflow.map_or(column.rect, |value| value.content_extent);
        let content_origin = content_extent.origin;

        let mut content = div()
            .relative()
            .w(px(content_extent.size.width))
            .h(px(content_extent.size.height));

        if mode == ColumnMode::Expanded {
            let mut ordered_groups = groups.to_vec();
            ordered_groups.sort_by_key(|group| menu_group == Some(group.group));
            for group in &ordered_groups {
                let strip = tab_strips
                    .iter()
                    .find(|strip| strip.group == group.group)
                    .copied();
                let presentation = group_presentations.get(&group.group);
                if let (Some(strip), Some(presentation)) = (strip, presentation) {
                    let headers = tab_headers
                        .iter()
                        .filter(|header| header.group == group.group)
                        .copied()
                        .collect::<Vec<_>>();
                    let is_first_group = group.rect.origin.y == column.rect.origin.y;
                    let reserved_grip_left = if is_first_group {
                        metrics.group_control_button_width + 8.0
                    } else {
                        0.0
                    };
                    content = content.child(GroupView::render(
                        *group,
                        strip,
                        &headers,
                        presentation,
                        content_origin,
                        metrics,
                        reserved_grip_left,
                        controller.clone(),
                        theme,
                        menu_group == Some(group.group),
                        toggle_group_menu.clone(),
                    ));
                } else {
                    content = content.child(group_surface(*group, content_origin, theme));
                }
            }
            for splitter in splitters {
                content = content.child(splitter_surface(*splitter, content_origin, theme));
            }
        } else {
            // Rail entries and icons are implemented by §5.7 RailView.
            // Keep the Core-computed extent and scroll behavior ready for it.
        }

        let mut scroll_viewport = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(viewport.size.width))
            .h(px(viewport.size.height))
            .bg(rgb(theme.colors.dock_background));

        if let Some(handle) = scroll_handle.as_ref() {
            scroll_viewport = scroll_viewport
                .overflow_y_scroll()
                .track_scroll(handle)
                .child(content);
        } else {
            scroll_viewport = scroll_viewport.child(content);
        }

        let mut root = div()
            .id(format!("dock-column-{:?}", column.column))
            .absolute()
            .left(px(column.rect.origin.x))
            .top(px(column.rect.origin.y))
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .child(scroll_viewport);

        // The mode control sits in the column's top chrome band. It overlays
        // the first group grip rather than changing Core's authoritative rects.
        let next_mode = next_column_mode(mode);
        let mode_label = column_mode_label(mode);
        let controller_for_click = controller.clone();
        let column_id = column.column;
        let mode_button = div()
            .id(format!("column-mode-{:?}", column.column))
            .absolute()
            .top(px(3.0))
            .left(px(4.0))
            .w(px(28.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.column_mode_button))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .child(mode_label)
            .on_click(move |_, _, app| {
                controller_for_click.update(app, |controller, controller_cx| {
                    if let Err(error) = controller.dispatch_in(
                        DockCommand::SetColumnMode {
                            column: column_id,
                            mode: next_mode,
                        },
                        controller_cx,
                    ) {
                        tracing::warn!(%error, "could not change dock column mode");
                    }
                });
            });

        root = root.child(mode_button);
        root
    }
}

/// Converts a point in the visible column viewport to the unscrolled Core
/// coordinate space. Core rectangles never move when GPUI scrolls the content.
pub fn scroll_adjusted_hit_test_point(point: Point, scroll_offset_y: f32) -> Point {
    Point::new(point.x, point.y + scroll_offset_y.max(0.0))
}

fn next_column_mode(mode: ColumnMode) -> ColumnMode {
    match mode {
        ColumnMode::Expanded => ColumnMode::IconsAndLabels,
        ColumnMode::IconsAndLabels => ColumnMode::IconsOnly,
        ColumnMode::IconsOnly => ColumnMode::Expanded,
    }
}

fn column_mode_label(mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Expanded => "⟨⟨",
        ColumnMode::IconsAndLabels => "⟨",
        ColumnMode::IconsOnly => "⟩⟩",
    }
}

fn group_surface(group: GroupRect, content_origin: Point, theme: Theme) -> impl IntoElement {
    let left = group.rect.origin.x - content_origin.x;
    let top = group.rect.origin.y - content_origin.y;
    let header_height = theme
        .metrics
        .column_header_height
        .min(group.rect.size.height);

    div()
        .id(format!("dock-group-{:?}", group.group))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(group.rect.size.width))
        .h(px(group.rect.size.height))
        .bg(rgb(theme.colors.group_frame_background))
        .border_1()
        .border_color(rgb(theme.colors.group_frame_border))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(header_height))
                .bg(rgb(theme.colors.group_grip_background)),
        )
}

fn splitter_surface(
    splitter: SplitterRect,
    content_origin: Point,
    theme: Theme,
) -> impl IntoElement {
    let hit_rect = splitter.rect;
    let thickness = theme.metrics.splitter_thickness;
    let (left, top, width, height) = match splitter.axis {
        Axis::X => (
            hit_rect.origin.x + (hit_rect.size.width - thickness) * 0.5 - content_origin.x,
            hit_rect.origin.y - content_origin.y,
            thickness,
            hit_rect.size.height,
        ),
        Axis::Y => (
            hit_rect.origin.x - content_origin.x,
            hit_rect.origin.y + (hit_rect.size.height - thickness) * 0.5 - content_origin.y,
            hit_rect.size.width,
            thickness,
        ),
    };

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width.max(0.0)))
        .h(px(height.max(0.0)))
        .bg(rgb(theme.colors.splitter_handle))
}

#[cfg(test)]
mod tests {
    use dock_core::{ColumnMode, Point};

    use super::{next_column_mode, scroll_adjusted_hit_test_point};

    #[test]
    fn column_mode_button_cycles_through_all_three_modes() {
        assert_eq!(
            next_column_mode(ColumnMode::Expanded),
            ColumnMode::IconsAndLabels
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsAndLabels),
            ColumnMode::IconsOnly
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsOnly),
            ColumnMode::Expanded
        );
    }

    #[test]
    fn scroll_offset_maps_viewport_pointer_back_to_core_content_coordinates() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(120.0, 180.0), 240.0),
            Point::new(120.0, 420.0)
        );
    }

    #[test]
    fn negative_scroll_offsets_do_not_move_hit_test_points() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(10.0, 20.0), -12.0),
            Point::new(10.0, 20.0)
        );
    }
}
