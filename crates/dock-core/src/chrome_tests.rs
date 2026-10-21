use crate::layout_builder::LayoutBuilder;
use crate::{
    ChromeMode, ColumnMode, DockCommand, DockMetrics, DockSide, Layout, PanelConstraints, PanelKey,
    Point, Profile, Rect, Size,
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

fn layout_with_two_main_docks() -> Layout {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    for (side, id) in [(DockSide::Left, "chrome-left"), (DockSide::Right, "chrome-right")] {
        let (_, panel) = builder.panel(key(id), constraints());
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(240.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(side, column);
    }

    builder.build()
}

fn node_snapshot(layout: &Layout) -> Vec<(crate::NodeId, String)> {
    layout
        .nodes()
        .map(|(id, node)| (id, format!("{node:?}")))
        .collect()
}

fn viewport() -> Rect {
    Rect::new(Point::new(13.0, 17.0), Size::new(1200.0, 700.0))
}

#[test]
fn hidden_chrome_gives_canvas_the_full_viewport_without_deleting_dock_nodes() {
    let metrics = DockMetrics::default();
    let mut layout = layout_with_two_main_docks();
    let before_nodes = node_snapshot(&layout);
    let visible = layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(visible.columns.len(), 2);
    assert_eq!(visible.panels.len(), 2);

    for mode in [ChromeMode::DockHidden, ChromeMode::AllHidden] {
        let outcome = layout
            .apply(DockCommand::SetChrome { mode }, &metrics)
            .unwrap();

        assert_eq!(layout.chrome(), mode);
        assert!(outcome.changed_nodes.is_empty());
        assert_eq!(outcome.inverse, Some(DockCommand::SetChrome { mode: if mode == ChromeMode::DockHidden { ChromeMode::Visible } else { ChromeMode::DockHidden } }));
        assert_eq!(node_snapshot(&layout), before_nodes);

        let hidden = layout.compute_rects(viewport(), &metrics).unwrap();
        assert_eq!(hidden.canvas, Some(viewport()));
        assert!(hidden.columns.is_empty());
        assert!(hidden.groups.is_empty());
        assert!(hidden.tab_strips.is_empty());
        assert!(hidden.tab_headers.is_empty());
        assert!(hidden.panels.is_empty());
        assert!(hidden.splitters.is_empty());
        assert!(hidden.flyout.is_none());
    }

    layout
        .apply(DockCommand::SetChrome { mode: ChromeMode::Visible }, &metrics)
        .unwrap();
    let restored = layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(restored.columns.len(), 2);
    assert_eq!(restored.panels.len(), 2);
    assert_eq!(node_snapshot(&layout), before_nodes);
}

#[test]
fn tab_shortcut_modes_can_be_applied_without_mutating_layout_structure() {
    let metrics = DockMetrics::default();
    let mut layout = layout_with_two_main_docks();
    let before_nodes = node_snapshot(&layout);

    let tab_mode = layout.chrome().after_tab_shortcut();
    layout
        .apply(DockCommand::SetChrome { mode: tab_mode }, &metrics)
        .unwrap();
    assert_eq!(layout.chrome(), ChromeMode::AllHidden);
    assert!(!layout.chrome().dock_visible());
    assert!(!layout.chrome().toolbar_visible());
    assert!(!layout.chrome().options_bar_visible());

    let restored_mode = layout.chrome().after_tab_shortcut();
    layout
        .apply(DockCommand::SetChrome { mode: restored_mode }, &metrics)
        .unwrap();
    assert_eq!(layout.chrome(), ChromeMode::Visible);

    let shift_tab_mode = layout.chrome().after_shift_tab_shortcut();
    layout
        .apply(DockCommand::SetChrome { mode: shift_tab_mode }, &metrics)
        .unwrap();
    assert_eq!(layout.chrome(), ChromeMode::DockHidden);
    assert!(!layout.chrome().dock_visible());
    assert!(layout.chrome().toolbar_visible());
    assert!(layout.chrome().options_bar_visible());

    let restored_mode = layout.chrome().after_shift_tab_shortcut();
    layout
        .apply(DockCommand::SetChrome { mode: restored_mode }, &metrics)
        .unwrap();
    assert_eq!(layout.chrome(), ChromeMode::Visible);
    assert_eq!(node_snapshot(&layout), before_nodes);
}

#[cfg(feature = "serde")]
#[test]
fn set_chrome_round_trips_every_visibility_mode() {
    for mode in [ChromeMode::Visible, ChromeMode::DockHidden, ChromeMode::AllHidden] {
        let command = DockCommand::SetChrome { mode };
        let encoded = serde_json::to_string(&command).unwrap();
        assert_eq!(serde_json::from_str::<DockCommand>(&encoded).unwrap(), command);
    }
}
