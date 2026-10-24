use dock_core::{
    create_initial_layout, DockMetrics, DockSide, DropSource, DropTarget, DropZone, Layout,
    Node, PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size,
};
use dock_gpui::FrameGeometry;

fn fixture() -> (Layout, DockMetrics, Rect) {
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
    let layout =
        create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 700.0));

    (layout, metrics, viewport)
}

fn panel_id(layout: &Layout, key: &str) -> PanelId {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
        .unwrap_or_else(|| panic!("initial layout should contain the {key} panel"))
}

fn second_right_group(layout: &Layout) -> dock_core::NodeId {
    let right = layout
        .main_area(DockSide::Right)
        .expect("initial layout should have a right main area");
    let Node::Column(column) = layout
        .node(right.root)
        .expect("right main area root should exist")
    else {
        panic!("initial right main area should be a column");
    };
    *column
        .groups
        .get(1)
        .expect("initial right column should contain a second tab group")
}

#[test]
fn compute_is_idempotent_for_an_unchanged_layout() {
    let (layout, metrics, viewport) = fixture();

    let first = FrameGeometry::compute(&layout, viewport, &metrics)
        .expect("first frame geometry calculation should succeed");
    let second = FrameGeometry::compute(&layout, viewport, &metrics)
        .expect("second frame geometry calculation should succeed");

    assert_eq!(first, second);
    assert_eq!(first.rects().viewport, Some(viewport));
}

#[test]
fn drop_tab_insertion_zones_match_the_normalized_headers() {
    let (layout, metrics, viewport) = fixture();
    let source = DropSource::Panel {
        panel: panel_id(&layout, "layers"),
    };
    let target_group = second_right_group(&layout);

    let unnormalized_rects = layout
        .compute_rects(viewport, &metrics)
        .expect("raw Core geometry should be valid");
    let unnormalized_zones = layout
        .compute_drop_zones(source, &unnormalized_rects, &metrics)
        .expect("drop zones for raw Core geometry should be valid");

    let frame = FrameGeometry::compute(&layout, viewport, &metrics)
        .expect("normalized frame geometry should be valid");
    let frame_zones = frame
        .drop_zones(&layout, source, &metrics)
        .expect("drop zones for frame geometry should be valid");
    let same_snapshot_zones = layout
        .compute_drop_zones(source, frame.rects(), &metrics)
        .expect("drop zones for the frame snapshot should be valid");

    assert_eq!(
        frame_zones, same_snapshot_zones,
        "FrameGeometry must resolve drop zones from its own normalized rectangle snapshot"
    );

    let target_headers = frame
        .rects()
        .tab_headers
        .iter()
        .filter(|header| header.group == target_group)
        .collect::<Vec<_>>();
    assert!(
        !target_headers.is_empty(),
        "the target tab group should expose visible headers"
    );

    for header in target_headers {
        // The left-half insertion zone for this header has the same left edge
        // and vertical bounds as the header rendered from FrameGeometry.
        let frame_zone = frame_zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::TabInsertion
                    && zone.rect.origin.x == header.rect.origin.x
                    && zone.rect.origin.y == header.rect.origin.y
                    && matches!(
                        zone.target,
                        DropTarget::Tabs { group, index }
                            if group == target_group && index == header.index
                    )
            })
            .unwrap_or_else(|| panic!("missing insertion zone aligned with header {:?}", header.panel));

        assert_eq!(frame_zone.rect.size.height, header.rect.size.height);

        // Core's unnormalized header row is higher. The UI must not use drop
        // zones made from that raw snapshot, or hit targets would miss the tabs.
        let raw_header = unnormalized_rects
            .tab_headers
            .iter()
            .find(|raw| raw.group == target_group && raw.panel == header.panel)
            .expect("raw Core geometry should include the same target header");
        assert_ne!(
            header.rect.origin.y, raw_header.rect.origin.y,
            "the fixture should exercise the header-row normalization"
        );

        let raw_zone = unnormalized_zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::TabInsertion
                    && zone.rect.origin.x == raw_header.rect.origin.x
                    && matches!(
                        zone.target,
                        DropTarget::Tabs { group, index }
                            if group == target_group && index == raw_header.index
                    )
            })
            .expect("raw Core geometry should expose the left insertion half");
        assert_eq!(raw_zone.rect.origin.y, raw_header.rect.origin.y);
        assert_ne!(
            frame_zone.rect.origin.y, raw_zone.rect.origin.y,
            "frame drop zone must be aligned to normalized, rendered tab geometry"
        );
    }
}
