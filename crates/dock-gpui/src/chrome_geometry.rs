use std::collections::{HashMap, HashSet};

use crate::tree_query::{active_panel, floating_title_groups_in_tree};
use dock_core::{
    ColumnMode, DockMetrics, Layout, LayoutRects, Node, NodeId, PanelId, Rect, Size,
    VerticalOverflow,
};

pub(crate) const GROUP_DRAG_GRIP_HEIGHT: f32 = 4.0;

/// Height of the pinned control row placed above rail items.
pub(crate) fn rail_control_row_height(
    tab_strip_height: f32,
    collapsed_group_height: f32,
    column_height: f32,
) -> f32 {
    tab_strip_height
        .max(collapsed_group_height)
        .min(column_height.max(0.0))
}

#[derive(Clone, Copy)]
struct GroupChromeGeometry {
    vertical_shift: f32,
}

/// Adapts Core's generic header-plus-tabs geometry to Photoshop's tabs-only
/// anatomy. The title-bar strip that belongs to a floating window is preserved;
/// any additional Tabs group header is removed. The same adjusted snapshot is
/// used by rendering and drop resolution so their hit targets remain aligned.
///
/// Pass only a fresh snapshot returned by `Layout::compute_rects`; this
/// function mutates rectangles and is not idempotent. Never apply it to the
/// rectangles exposed by `FrameGeometry`, which are already normalized.
/// Application code should prefer `FrameGeometry::compute` so layout and drop
/// hit testing share the same once-normalized snapshot.
pub fn normalize_docked_group_geometry(
    layout: &Layout,
    metrics: &DockMetrics,
    rects: &mut LayoutRects,
) {
    let mut rail_groups = HashSet::<NodeId>::new();
    for column_rect in &rects.columns {
        let Some(Node::Column(column)) = layout.node(column_rect.column) else {
            continue;
        };
        if column.mode.is_rail() {
            if let Ok(items) = layout.column_display_order(column_rect.column) {
                rail_groups.extend(items.into_iter().map(|item| item.group));
            } else {
                rail_groups.extend(column.groups.iter().copied());
            }
        }
    }

    let floating_title_groups = floating_title_geometry_by_group(layout, metrics, rects);

    // Rail mode switches stay pinned; all affected group geometry is shifted together.
    let (rail_column_reservations, rail_group_shifts) =
        rail_reservations_by_column(layout, metrics, rects);

    let mode_button_groups = rects
        .columns
        .iter()
        .filter_map(|column_rect| match layout.node(column_rect.column) {
            Some(Node::Column(column)) if column.mode == ColumnMode::Expanded => {
                column.groups.first().copied()
            }
            _ => None,
        })
        .collect::<HashSet<_>>();

    let mut groups = HashMap::<NodeId, GroupChromeGeometry>::new();
    let mut active_panel_shifts = HashMap::<PanelId, f32>::new();
    let mut floating_collapsed_shifts = HashMap::<NodeId, f32>::new();
    for group_rect in &rects.groups {
        let group_id = group_rect.group;
        if rail_groups.contains(&group_id) {
            continue;
        }
        let Some(Node::Tabs(tabs)) = layout.node(group_id) else {
            continue;
        };
        if let Some(&(title_height, window_top)) = floating_title_groups.get(&group_id) {
            // Expanded groups already place the strip below the floating title
            // through Core's header geometry. A collapsed group in a composite
            // root may already be below that title because Core reserved the
            // space during stack allocation. Only offset geometry that still
            // starts inside the title row (the standalone Tabs-root case).
            if tabs.collapsed {
                let shift = (window_top + title_height - group_rect.rect.origin.y).max(0.0);
                if shift > 0.001 {
                    floating_collapsed_shifts.insert(group_id, shift);
                }
            }
            continue;
        }
        let grip_height = if tabs.collapsed {
            0.0
        } else {
            GROUP_DRAG_GRIP_HEIGHT.min(metrics.header_height)
        };
        let vertical_shift = if tabs.collapsed {
            0.0
        } else {
            (metrics.header_height - grip_height).max(0.0)
        };
        groups.insert(group_id, GroupChromeGeometry { vertical_shift });
        if vertical_shift > 0.0
            && let Some(panel) = active_panel(layout, tabs)
        {
            active_panel_shifts.insert(panel, vertical_shift);
        }
    }

    apply_group_chrome_shifts(
        rects,
        &groups,
        &active_panel_shifts,
        &floating_collapsed_shifts,
    );

    // Reserve the pinned rail row in both group and tab-header geometry. Grow
    // existing scroll extents, or create an overflow record if this row alone
    // pushes rail content beyond its viewport.
    for group in &mut rects.groups {
        if let Some(shift) = rail_group_shifts.get(&group.group) {
            group.rect.origin.y += *shift;
        }
    }
    for header in &mut rects.tab_headers {
        if let Some(shift) = rail_group_shifts.get(&header.group) {
            header.rect.origin.y += *shift;
        }
    }

    normalize_rail_overflow_extents(layout, rects, &rail_column_reservations);

    normalize_tab_strip_widths(layout, metrics, rects, &rail_groups, &mode_button_groups);
}

/// Applies vertical shifts consistently to group surfaces, tab hit targets, and active content.
fn apply_group_chrome_shifts(
    rects: &mut LayoutRects,
    groups: &HashMap<NodeId, GroupChromeGeometry>,
    active_panel_shifts: &HashMap<PanelId, f32>,
    floating_collapsed_shifts: &HashMap<NodeId, f32>,
) {
    // Keep collapsed floating tabs below the independent window title row.
    // Shift the group rectangle and hit targets together so the visual surface
    // and the Core drop geometry continue to describe the same area.
    for group in &mut rects.groups {
        if let Some(shift) = floating_collapsed_shifts.get(&group.group) {
            group.rect.origin.y += *shift;
        }
    }

    // Shift normalized docked strips into their former header row. Grow active
    // content by the same amount to preserve its bottom edge. Floating collapsed
    // strips instead move down to avoid being covered by the title bar.
    for strip in &mut rects.tab_strips {
        if let Some(group) = groups.get(&strip.group) {
            strip.rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&strip.group) {
            strip.rect.origin.y += *shift;
        }
    }
    for header in &mut rects.tab_headers {
        if let Some(group) = groups.get(&header.group) {
            header.rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&header.group) {
            header.rect.origin.y += *shift;
        }
    }
    for overflow in &mut rects.tab_overflows {
        if let Some(group) = groups.get(&overflow.group) {
            overflow.button_rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&overflow.group) {
            overflow.button_rect.origin.y += *shift;
        }
    }
    for panel in &mut rects.panels {
        if let Some(shift) = active_panel_shifts.get(&panel.panel) {
            panel.rect.origin.y -= *shift;
            panel.rect.size.height += *shift;
        }
    }
}

/// Extends rail-column scroll extents to account for the pinned control-row reservation.
fn normalize_rail_overflow_extents(
    layout: &Layout,
    rects: &mut LayoutRects,
    rail_column_reservations: &HashMap<NodeId, f32>,
) {
    for (column_id, reserved) in rail_column_reservations {
        let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == *column_id) else {
            continue;
        };
        let group_ids = layout
            .column_display_order(*column_id)
            .map(|items| items.into_iter().map(|item| item.group).collect::<HashSet<_>>())
            .unwrap_or_else(|_| match layout.node(*column_id) {
                Some(Node::Column(column)) => column.groups.iter().copied().collect(),
                _ => HashSet::new(),
            });
        let required_bottom = rects
            .groups
            .iter()
            .filter(|entry| group_ids.contains(&entry.group))
            .map(|entry| entry.rect.origin.y + entry.rect.size.height)
            .fold(column_rect.rect.origin.y, f32::max);
        let required_height = (required_bottom - column_rect.rect.origin.y)
            .max(column_rect.rect.size.height);

        if let Some(overflow) = rects
            .vertical_overflow
            .iter_mut()
            .find(|entry| entry.owner == *column_id)
        {
            overflow.content_extent.size.height =
                (overflow.content_extent.size.height + *reserved).max(required_height);
        } else if required_height > column_rect.rect.size.height {
            rects.vertical_overflow.push(VerticalOverflow::new(
                *column_id,
                column_rect.rect,
                Rect::new(
                    column_rect.rect.origin,
                    Size::new(column_rect.rect.size.width, required_height),
                ),
            ));
        }
    }
}

/// Computes the pinned control-row reservation for each rail column and the
/// corresponding vertical shift for every group displayed inside those columns.
fn rail_reservations_by_column(
    layout: &Layout,
    metrics: &DockMetrics,
    rects: &LayoutRects,
) -> (HashMap<NodeId, f32>, HashMap<NodeId, f32>) {
    let mut rail_column_reservations = HashMap::<NodeId, f32>::new();
    let mut rail_group_shifts = HashMap::<NodeId, f32>::new();

    for column_rect in &rects.columns {
        let Some(Node::Column(column)) = layout.node(column_rect.column) else {
            continue;
        };
        if !column.mode.is_rail() {
            continue;
        }

        let reserved = rail_control_row_height(
            metrics.tab_strip_height,
            metrics.collapsed_group_height,
            column_rect.rect.size.height,
        );
        if reserved <= 0.0 {
            continue;
        }

        rail_column_reservations.insert(column_rect.column, reserved);
        if let Ok(items) = layout.column_display_order(column_rect.column) {
            for item in items {
                rail_group_shifts.entry(item.group).or_insert(reserved);
            }
        } else {
            for group in &column.groups {
                rail_group_shifts.entry(*group).or_insert(reserved);
            }
        }
    }

    (rail_column_reservations, rail_group_shifts)
}

/// Reserves horizontal space for group controls and distributes the remaining strip
/// width equally across visible tab headers.
fn normalize_tab_strip_widths(
    layout: &Layout,
    metrics: &DockMetrics,
    rects: &mut LayoutRects,
    rail_groups: &HashSet<NodeId>,
    mode_button_groups: &HashSet<NodeId>,
) {
    // This applies to floating Tabs groups too, so controls never cover a tab.
    for group_rect in &rects.groups {
        let group_id = group_rect.group;
        if rail_groups.contains(&group_id) || !matches!(layout.node(group_id), Some(Node::Tabs(_))) {
            continue;
        }
        let width = group_rect.rect.size.width.max(0.0);
        let menu_width = metrics.group_control_button_width.min(width);
        let mode_width = if mode_button_groups.contains(&group_id) {
            metrics.group_control_button_width.min((width - menu_width).max(0.0))
        } else {
            0.0
        };
        let has_overflow = rects.tab_overflows.iter().any(|entry| entry.group == group_id);
        let overflow_width = if has_overflow {
            metrics
                .tab_overflow_button_width
                .min((width - menu_width - mode_width).max(0.0))
        } else {
            0.0
        };
        let tabs_width = (width - menu_width - mode_width - overflow_width).max(0.0);

        let mut header_indices = rects
            .tab_headers
            .iter()
            .enumerate()
            .filter_map(|(index, header)| (header.group == group_id).then_some(index))
            .collect::<Vec<_>>();
        header_indices.sort_by_key(|index| rects.tab_headers[*index].index);
        let visible_count = header_indices.len();
        if visible_count > 0 {
            for (visible_index, header_index) in header_indices.into_iter().enumerate() {
                let left = group_rect.rect.origin.x
                    + tabs_width * visible_index as f32 / visible_count as f32;
                let right = if visible_index + 1 == visible_count {
                    group_rect.rect.origin.x + tabs_width
                } else {
                    group_rect.rect.origin.x
                        + tabs_width * (visible_index + 1) as f32 / visible_count as f32
                };
                let header = &mut rects.tab_headers[header_index];
                header.rect.origin.x = left;
                header.rect.size.width = (right - left).max(0.0);
            }
        }

        if has_overflow
            && let Some(overflow) = rects
                .tab_overflows
                .iter_mut()
                .find(|entry| entry.group == group_id)
        {
            overflow.button_rect.origin.x = group_rect.rect.origin.x + tabs_width;
            overflow.button_rect.size.width = overflow_width;
        }
    }
}

/// Maps floating tab groups to their title-row height and window top.
fn floating_title_geometry_by_group(
    layout: &Layout,
    metrics: &DockMetrics,
    rects: &LayoutRects,
) -> HashMap<NodeId, (f32, f32)> {
    // A floating title is shared by every branch along the top edge of a
    // horizontal composition, not just the single group returned by min_by(y).
    // Index the computed groups once so composite floating roots can resolve
    // their title-bearing branches without rescanning all group rectangles.
    let group_rect_indices = rects
        .groups
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.group, index))
        .collect::<HashMap<NodeId, usize>>();

    // Core reserves title space for collapsed branches in composite roots;
    // standalone Tabs roots retain the adapter's simple local offset.
    let mut floating_title_groups = HashMap::<NodeId, (f32, f32)>::new();
    for (floating_id, floating) in layout.floating_groups_in_paint_order() {
        let Some(area) = layout.dock_area(floating.area) else {
            continue;
        };
        let Some(bounds) = rects
            .floating_groups
            .iter()
            .find(|entry| entry.group == floating_id)
            .map(|entry| entry.rect)
        else {
            continue;
        };
        if bounds.size.height <= 0.0 {
            continue;
        }
        let title_height = metrics.header_height.min(bounds.size.height);
        for top_group in floating_title_groups_in_tree(layout, area.root) {
            if let Some(&group_index) = group_rect_indices.get(&top_group) {
                let group_id = rects.groups[group_index].group;
                floating_title_groups.insert(group_id, (title_height, bounds.origin.y));
            }
        }
    }

    floating_title_groups
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_docked_group_geometry, rail_control_row_height, GROUP_DRAG_GRIP_HEIGHT,
    };
    use dock_core::{
        create_initial_layout, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget,
        Layout, Node, NodeId, PanelConstraints, PanelKey, Point, Profile, Rect, Size,
    };

    fn layout_with_tab_count(tab_count: usize, metrics: DockMetrics) -> (Layout, NodeId) {
        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();

        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.node(column_id).unwrap() else {
            panic!("Photoshop main area should use a column");
        };
        let group_id = column.groups[0];

        match tab_count {
            1 => {
                let panel = layout
                    .panel_instances()
                    .find_map(|(id, instance)| (instance.key.as_str() == "color").then_some(id))
                    .unwrap();
                layout
                    .apply(DockCommand::ClosePanel { panel }, &metrics)
                    .unwrap();
            }
            2 => {}
            5 => {
                for (offset, key) in ["extra-three", "extra-four", "extra-five"]
                    .into_iter()
                    .enumerate()
                {
                    layout
                        .apply(
                            DockCommand::OpenPanel {
                                key: PanelKey::new(key).unwrap(),
                                constraints,
                                target: Some(DropTarget::Tabs {
                                    group: group_id,
                                    index: 2 + offset,
                                }),
                            },
                            &metrics,
                        )
                        .unwrap();
                }
            }
            _ => panic!("the fixture supports 1, 2, or 5 tabs"),
        }

        // Give all five tabs enough room to remain visible instead of testing overflow.
        layout
            .apply(
                DockCommand::ResizeColumn {
                    column: column_id,
                    width: 700.0,
                },
                &metrics,
            )
            .unwrap();

        (layout, group_id)
    }

    #[test]
    fn visible_tab_widths_are_equal_for_one_two_and_five_tabs() {
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let viewport = Rect::new(Point::ZERO, Size::new(1600.0, 900.0));

        for tab_count in [1, 2, 5] {
            let (layout, group_id) = layout_with_tab_count(tab_count, metrics);
            let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
            let group_rect = rects
                .groups
                .iter()
                .find(|entry| entry.group == group_id)
                .unwrap()
                .rect;

            assert!(
                !rects.tab_overflows.iter().any(|entry| entry.group == group_id),
                "the {tab_count}-tab fixture should not overflow"
            );

            normalize_docked_group_geometry(&layout, &metrics, &mut rects);

            let mut headers = rects
                .tab_headers
                .iter()
                .filter(|header| header.group == group_id)
                .collect::<Vec<_>>();
            headers.sort_by_key(|header| header.index);
            assert_eq!(headers.len(), tab_count);

            let tabs_width =
                (group_rect.size.width - metrics.group_control_button_width * 2.0).max(0.0);
            let expected_tab_width = tabs_width / tab_count as f32;
            let mut expected_left = group_rect.origin.x;
            for header in headers {
                assert!(
                    (header.rect.origin.x - expected_left).abs() < 0.001,
                    "tab {}/{} should start at the next equal-width slot",
                    header.index + 1,
                    tab_count
                );
                assert!(
                    (header.rect.size.width - expected_tab_width).abs() < 0.001,
                    "tab {}/{} should be {expected_tab_width}px wide, got {}",
                    header.index + 1,
                    tab_count,
                    header.rect.size.width
                );
                expected_left += expected_tab_width;
            }

            let expected_tabs_right = group_rect.origin.x + tabs_width;
            assert!(
                (expected_left - expected_tabs_right).abs() < 0.001,
                "tabs must end before the reserved group-menu area"
            );
        }
    }

    #[test]
    fn group_control_width_metric_controls_reserved_tab_strip_space() {
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            group_control_button_width: 40.0,
            ..DockMetrics::default()
        };
        let (layout, group_id) = layout_with_tab_count(2, metrics);
        let viewport = Rect::new(Point::ZERO, Size::new(1600.0, 900.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        let group_width = rects
            .groups
            .iter()
            .find(|entry| entry.group == group_id)
            .unwrap()
            .rect
            .size
            .width;

        assert!(
            !rects.tab_overflows.iter().any(|entry| entry.group == group_id),
            "the two-tab fixture should not overflow"
        );

        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let visible_tab_width: f32 = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group_id)
            .map(|header| header.rect.size.width)
            .sum();
        let expected_tab_width = (group_width - 2.0 * metrics.group_control_button_width).max(0.0);

        assert!(
            (visible_tab_width - expected_tab_width).abs() < 0.001,
            "the menu and mode button widths should come from DockMetrics: expected total tab width {expected_tab_width}, got {visible_tab_width}"
        );
    }

    #[test]
    fn collapsed_floating_root_tabs_are_positioned_below_the_window_title() {
        use dock_core::DockCommand;

        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.node(root).unwrap() else {
            panic!("Photoshop main area should use a column");
        };
        let group = column.groups[0];
        layout
            .apply(
                DockCommand::Float {
                    node: group,
                    bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 240.0)),
                },
                &metrics,
            )
            .unwrap();
        layout
            .apply(
                DockCommand::CollapseGroup {
                    group,
                    collapsed: true,
                },
                &metrics,
            )
            .unwrap();

        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 700.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let floating = rects.floating_groups.first().unwrap().rect;
        let group_rect = rects.groups.iter().find(|entry| entry.group == group).unwrap().rect;
        let strip = rects.tab_strips.iter().find(|entry| entry.group == group).unwrap().rect;
        let expected_y = floating.origin.y + metrics.header_height;
        assert!((group_rect.origin.y - expected_y).abs() < 0.001);
        assert!((strip.origin.y - expected_y).abs() < 0.001);
        for header in rects.tab_headers.iter().filter(|entry| entry.group == group) {
            assert!((header.rect.origin.y - expected_y).abs() < 0.001);
        }
    }

    #[test]
    fn docked_tabs_replace_header_row_and_reserve_distinct_menu_and_drag_space() {
        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let viewport = Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 700.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        let before = rects.clone();

        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.node(column_id).unwrap() else {
            panic!("Photoshop main area should use a column");
        };
        let group_id = column.groups[0];
        let group_rect = before.groups.iter().find(|entry| entry.group == group_id).unwrap().rect;
        let before_strip = before.tab_strips.iter().find(|entry| entry.group == group_id).unwrap().rect;
        assert_eq!(before_strip.origin.y, group_rect.origin.y + metrics.header_height);
        let Node::Tabs(tabs) = layout.node(group_id).unwrap() else {
            panic!("column children should be tab groups");
        };
        let active_panel_id = super::active_panel(&layout, tabs).unwrap();
        let before_panel = before.panels.iter().find(|entry| entry.panel == active_panel_id).unwrap().rect;

        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let after_strip = rects.tab_strips.iter().find(|entry| entry.group == group_id).unwrap().rect;
        assert_eq!(
            after_strip.origin.y,
            group_rect.origin.y + GROUP_DRAG_GRIP_HEIGHT
        );
        let after_panel = rects.panels.iter().find(|entry| entry.panel == active_panel_id).unwrap().rect;
        assert_eq!(
            after_panel.origin.y,
            group_rect.origin.y + GROUP_DRAG_GRIP_HEIGHT + metrics.tab_strip_height
        );
        let before_bottom = before_panel.origin.y + before_panel.size.height;
        let after_bottom = after_panel.origin.y + after_panel.size.height;
        assert!((before_bottom - after_bottom).abs() < 0.001);

        let headers = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group_id)
            .collect::<Vec<_>>();
        let tab_right = headers
            .iter()
            .map(|header| header.rect.origin.x + header.rect.size.width)
            .fold(group_rect.origin.x, f32::max);
        let menu_left = group_rect.origin.x + group_rect.size.width - metrics.group_control_button_width;
        let mode_button_left = menu_left - metrics.group_control_button_width;
        assert!(
            (mode_button_left - tab_right).abs() < 0.001,
            "the mode button should sit directly after the tabs, without sharing their hit area"
        );
        assert!(
            (mode_button_left + metrics.group_control_button_width - menu_left).abs() < 0.001,
            "the mode button and group menu should have adjacent, distinct hit areas"
        );
    }

    #[test]
    fn rail_mode_reserves_a_pinned_control_row_and_shifts_item_geometry() {
        let metrics = DockMetrics {
            tab_strip_height: 24.0,
            collapsed_group_height: 20.0,
            ..DockMetrics::default()
        };
        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        layout
            .apply(
                DockCommand::SetColumnMode {
                    column: column_id,
                    mode: ColumnMode::IconsOnly,
                },
                &metrics,
            )
            .unwrap();

        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 500.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        let column_rect = rects
            .columns
            .iter()
            .find(|entry| entry.column == column_id)
            .unwrap()
            .rect;
        let first_header_before = rects.tab_headers[0].rect;
        let first_group_before = rects.groups[0].rect;
        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let reserved = rail_control_row_height(
            metrics.tab_strip_height,
            metrics.collapsed_group_height,
            column_rect.size.height,
        );
        assert_eq!(reserved, 24.0);
        assert_eq!(
            rects.tab_headers[0].rect.origin.y,
            first_header_before.origin.y + reserved
        );
        assert_eq!(
            rects.groups[0].rect.origin.y,
            first_group_before.origin.y + reserved
        );
        assert!(
            rects.tab_headers.iter().all(|header| {
                header.rect.origin.y >= column_rect.origin.y + reserved
            }),
            "every rail item must start below the pinned mode-control row"
        );
    }

    #[test]
    fn rail_control_row_extends_existing_overflow_content() {
        let metrics = DockMetrics::default();
        let constraints = PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        layout
            .apply(
                DockCommand::SetColumnMode {
                    column: column_id,
                    mode: ColumnMode::IconsOnly,
                },
                &metrics,
            )
            .unwrap();

        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 50.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        let original_overflow = rects
            .vertical_overflow
            .iter()
            .find(|entry| entry.owner == column_id)
            .copied()
            .expect("the rail items should overflow the short viewport");
        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let updated = rects
            .vertical_overflow
            .iter()
            .find(|entry| entry.owner == column_id)
            .expect("the rail control row must preserve the scroll region");
        let row_height = rail_control_row_height(
            metrics.tab_strip_height,
            metrics.collapsed_group_height,
            original_overflow.viewport.size.height,
        );
        assert_eq!(
            updated.content_extent.size.height,
            original_overflow.content_extent.size.height + row_height
        );
    }

}
