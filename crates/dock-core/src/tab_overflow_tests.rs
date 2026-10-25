use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockMetrics, DockSide, HitResult, PanelConstraints, PanelId, PanelKey, Point,
    Profile, Rect, Size,
};

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    )
}

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
}

fn two_or_more_tabs_layout(
    width: f32,
    tab_count: usize,
) -> (crate::Layout, crate::NodeId, Vec<PanelId>) {
    two_or_more_tabs_layout_with_active(width, tab_count, 0)
}

fn two_or_more_tabs_layout_with_active(
    width: f32,
    tab_count: usize,
    active_index: usize,
) -> (crate::Layout, crate::NodeId, Vec<PanelId>) {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut panel_ids = Vec::with_capacity(tab_count);
    let mut panel_nodes = Vec::with_capacity(tab_count);

    for index in 0..tab_count {
        let panel_key = format!("overflow-tab-{index}");
        let (panel, node) = builder.panel(key(&panel_key), constraints());
        panel_ids.push(panel);
        panel_nodes.push(node);
    }

    let group = builder.tabs(panel_nodes, active_index, false);
    let column = builder.column(width, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    (builder.build(), group, panel_ids)
}

fn viewport() -> Rect {
    Rect::new(Point::new(13.0, 17.0), Size::new(1200.0, 700.0))
}

#[test]
fn tab_headers_compress_before_hidden_tabs_move_to_overflow_menu() {
    let metrics = DockMetrics::default();
    let (layout, group, panels) = two_or_more_tabs_layout(200.0, 4);
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();

    assert_eq!(rects.tab_headers.len(), 2);
    assert_eq!(rects.tab_headers[0].group, group);
    assert_eq!(rects.tab_headers[0].panel, panels[0]);
    assert_eq!(rects.tab_headers[0].index, 0);
    assert_eq!(rects.tab_headers[0].rect.size.width, 86.0);
    assert_eq!(rects.tab_headers[1].panel, panels[1]);
    assert_eq!(rects.tab_headers[1].index, 1);
    assert_eq!(rects.tab_headers[1].rect.size.width, 86.0);

    assert_eq!(rects.tab_overflows.len(), 1);
    let overflow = &rects.tab_overflows[0];
    assert_eq!(overflow.group, group);
    assert_eq!(
        overflow.button_rect,
        Rect::new(
            Point::new(185.0, 45.0),
            Size::new(metrics.tab_overflow_button_width, metrics.tab_strip_height),
        ),
    );
    assert_eq!(overflow.hidden_tabs, vec![panels[2], panels[3]]);

    let hit = layout
        .hit_test(
            Point::new(
                overflow.button_rect.origin.x + overflow.button_rect.size.width * 0.5,
                overflow.button_rect.origin.y + overflow.button_rect.size.height * 0.5,
            ),
            &rects,
            &metrics,
        )
        .unwrap();
    assert_eq!(hit, HitResult::TabOverflowButton { group });
}

#[test]
fn active_tab_is_not_hidden_when_its_index_exceeds_visible_overflow_window() {
    let metrics = DockMetrics::default();
    let (layout, group, panels) = two_or_more_tabs_layout_with_active(200.0, 4, 3);
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();

    assert_eq!(rects.tab_headers.len(), 2);
    assert_eq!(rects.tab_headers[0].panel, panels[2]);
    assert_eq!(rects.tab_headers[0].index, 2);
    assert_eq!(rects.tab_headers[1].panel, panels[3]);
    assert_eq!(rects.tab_headers[1].index, 3);
    assert_eq!(rects.tab_headers.iter().all(|header| header.group == group), true);
    assert_eq!(rects.tab_overflows[0].hidden_tabs, vec![panels[0], panels[1]]);
}

#[test]
fn all_tabs_remain_visible_and_share_width_when_minimums_fit() {
    let metrics = DockMetrics::default();
    let (layout, group, panels) = two_or_more_tabs_layout(200.0, 2);
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();

    assert_eq!(rects.tab_headers.len(), 2);
    assert!(rects.tab_overflows.is_empty());
    assert_eq!(rects.tab_headers[0].panel, panels[0]);
    assert_eq!(rects.tab_headers[1].panel, panels[1]);
    assert!(rects.tab_headers.iter().all(|header| header.group == group));
    assert!(
        rects
            .tab_headers
            .iter()
            .all(|header| header.rect.size.width == 100.0)
    );
}

#[test]
fn narrow_strip_uses_full_width_for_overflow_button_when_no_tab_fits() {
    let metrics = DockMetrics {
        min_column_width: 80.0,
        ..DockMetrics::default()
    };
    let (layout, group, panels) = two_or_more_tabs_layout(80.0, 2);
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();

    assert!(rects.tab_headers.is_empty());
    assert_eq!(rects.tab_overflows.len(), 1);
    let overflow = &rects.tab_overflows[0];
    assert_eq!(overflow.group, group);
    assert_eq!(overflow.hidden_tabs, panels);
    assert_eq!(overflow.button_rect.origin.x, 13.0);
    assert_eq!(overflow.button_rect.size.width, 80.0);
}

#[cfg(feature = "serde")]
#[test]
fn tab_overflow_geometry_round_trips_with_hidden_tab_ids() {
    let metrics = DockMetrics::default();
    let (layout, _, _) = two_or_more_tabs_layout(200.0, 4);
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let overflow = rects.tab_overflows[0].clone();
    let encoded = serde_json::to_string(&overflow).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::TabOverflowRect>(&encoded).unwrap(),
        overflow
    );

    let encoded_snapshot = serde_json::to_string(&rects).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::LayoutRects>(&encoded_snapshot).unwrap(),
        rects
    );
}
