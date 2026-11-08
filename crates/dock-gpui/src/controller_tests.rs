use super::{DockController, DockEvent, ResolverCacheKey};
use dock_core::{
    CommandError, DockCommand, DockMetrics, DragHandle, DragPayload, DragState, DropSource,
    HitResult, InitialLayoutError, LayoutRects, PanelConstraints, PanelId, PanelKey, Point,
    Node, Profile, Rect, Size,
};
use gpui::WindowId;
use slotmap::Key;

use crate::registry::{IconSource, PanelDescriptor, PanelRegistry};

fn baseline_registry() -> PanelRegistry {
    let mut registry = PanelRegistry::new();
    for key in ["layers", "color", "properties", "history"] {
        let descriptor = PanelDescriptor::new(
            PanelKey::new(key).unwrap(),
            key,
            IconSource::None,
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false),
            |_, _| unreachable!("panel factories are lazy"),
        );
        registry.register(descriptor).unwrap();
    }
    registry
}

fn controller() -> DockController {
    DockController::empty(
        Profile::Photoshop,
        DockMetrics::default(),
        PanelRegistry::new(),
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("empty Photoshop controller should be valid")
}

#[test]
fn empty_layout_computes_canvas_geometry() {
    let mut controller = controller();
    let rects = controller.geometry().unwrap();

    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
    );
    assert!(rects.columns.is_empty());
    assert!(rects.groups.is_empty());
    assert!(rects.panels.is_empty());
    assert!(rects.floating_groups.is_empty());
}

#[test]
fn repeated_geometry_request_reuses_the_same_snapshot() {
    let mut controller = controller();

    let first = controller.geometry().unwrap() as *const LayoutRects;
    let second = controller.geometry().unwrap() as *const LayoutRects;

    assert_eq!(first, second);
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn canvas_bounds_update_is_idempotent_and_does_not_advance_layout_version() {
    let mut controller = controller();
    let canvas = controller.geometry().unwrap().canvas.unwrap();
    let version = controller.layout.version();

    assert_eq!(
        controller.layout.update_canvas_bounds(Some(canvas)),
        Ok(false)
    );

    let measured = Rect::new(Point::new(10.0, 20.0), Size::new(800.0, 600.0));
    assert_eq!(
        controller.layout.update_canvas_bounds(Some(measured)),
        Ok(true)
    );
    assert_eq!(controller.layout.version(), version);
}

#[test]
fn transient_commands_use_history_facade_without_adding_undo_entries() {
    let mut controller = controller();
    let version = controller.layout.version();

    let outcome = controller
        .dispatch_transient(DockCommand::SetFlyoutAutoCollapse { enabled: false })
        .unwrap();

    assert_eq!(outcome.new_version, version);
    assert!(!controller.layout.flyout_auto_collapse_enabled());
    assert!(!controller.can_undo());
    assert!(!controller.can_redo());
}

#[test]
fn rejected_command_does_not_mutate_layout_history_or_geometry_cache() {
    let mut controller = controller();
    controller.geometry().unwrap();
    let cached_ptr = controller.cached_geometry().unwrap() as *const LayoutRects;
    let version = controller.layout.version();

    let result = controller.dispatch(DockCommand::ClosePanel {
        panel: PanelId::null(),
    });

    assert!(matches!(result, Err(CommandError::MissingPanel { .. })));
    assert_eq!(controller.layout.version(), version);
    assert!(!controller.can_undo());
    assert!(!controller.can_redo());
    assert!(controller.last_error().is_some());
    assert!(controller.cached_geometry_is_current());
    assert_eq!(
        controller.cached_geometry().unwrap() as *const LayoutRects,
        cached_ptr
    );
}

#[test]
fn viewport_change_invalidates_geometry_without_mutating_layout() {
    let mut controller = controller();
    controller.geometry().unwrap();
    let version = controller.layout.version();

    assert_eq!(
        controller.set_viewport(Rect::new(Point::ZERO, Size::new(1280.0, 800.0),)),
        Ok(true)
    );
    assert!(!controller.cached_geometry_is_current());
    assert_eq!(controller.layout.version(), version);

    let updated = controller.geometry().unwrap();
    assert_eq!(updated.viewport.unwrap().size.width, 1280.0);
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn metric_replacement_invalidates_geometry_and_advances_metric_revision() {
    let mut controller = controller();
    controller.geometry().unwrap();
    let version = controller.layout.version();
    let initial_revision = controller.metrics_revision;

    let metrics = DockMetrics {
        min_tab_width: 80.0,
        ..DockMetrics::default()
    };
    assert!(controller.set_metrics(metrics).unwrap());
    assert_eq!(controller.metrics_revision, initial_revision + 1);
    assert!(!controller.cached_geometry_is_current());
    assert_eq!(controller.layout.version(), version);

    controller.geometry().unwrap();
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn invalid_metric_replacement_leaves_controller_unchanged() {
    let mut controller = controller();
    controller.geometry().unwrap();
    let current_metrics = controller.metrics;
    let current_revision = controller.metrics_revision;
    let cached_ptr = controller.cached_geometry().unwrap() as *const LayoutRects;

    let invalid_metrics = DockMetrics {
        drag_threshold: f32::NAN,
        ..current_metrics
    };

    assert!(controller.set_metrics(invalid_metrics).is_err());
    assert_eq!(controller.metrics, current_metrics);
    assert_eq!(controller.metrics_revision, current_revision);
    assert_eq!(
        controller.cached_geometry().unwrap() as *const LayoutRects,
        cached_ptr
    );
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn transient_dispatch_rejects_persistent_commands() {
    let mut controller = controller();
    let version = controller.layout.version();

    assert!(
        controller
            .dispatch_transient(DockCommand::SetChrome {
                mode: dock_core::ChromeMode::AllHidden,
            })
            .is_err()
    );

    assert_eq!(controller.layout.version(), version);
    assert!(!controller.can_undo());
}

#[test]
fn reset_layout_restores_core_baseline_and_clears_history() {
    let mut controller = DockController::empty(
        Profile::Photoshop,
        DockMetrics::default(),
        baseline_registry(),
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .unwrap();
    controller.geometry().unwrap();

    controller
        .dispatch(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::AllHidden,
        })
        .unwrap();
    assert!(controller.can_undo());
    controller.geometry().unwrap();

    controller.reset_layout().unwrap();

    assert_eq!(controller.layout.panel_instances().count(), 4);
    assert!(!controller.can_undo());
    assert!(!controller.can_redo());
    assert!(!controller.cached_geometry_is_current());

    let rects = controller.geometry().unwrap();
    assert_eq!(rects.columns.len(), 1);
    assert_eq!(rects.groups.len(), 2);
}

#[test]
fn reset_layout_without_required_descriptors_keeps_current_layout() {
    let mut controller = controller();
    controller.geometry().unwrap();
    let version = controller.layout.version();
    let cached_ptr = controller.cached_geometry().unwrap() as *const LayoutRects;

    let result = controller.reset_layout();

    assert!(matches!(
        result,
        Err(InitialLayoutError::MissingRequiredPanel { key: "layers" })
    ));
    assert_eq!(controller.layout.version(), version);
    assert_eq!(controller.layout.panel_instances().count(), 0);
    assert_eq!(
        controller.cached_geometry().unwrap() as *const LayoutRects,
        cached_ptr
    );
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn singleton_panel_is_reused_while_open_but_can_be_closed_and_reopened() {
    let metrics = DockMetrics::default();
    let mut registry = PanelRegistry::new();
    for key in ["layers", "color", "properties", "history"] {
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            key == "layers",
        );
        registry
            .register(PanelDescriptor::new(
                PanelKey::new(key).unwrap(),
                key,
                IconSource::None,
                constraints,
                |_, _| unreachable!("panel factories are lazy"),
            ))
            .unwrap();
    }

    let descriptors = registry.descriptors_for_core();
    let layout = dock_core::create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("baseline layout should be valid");
    let mut controller = DockController::new(
        layout,
        metrics,
        registry,
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("controller should accept the baseline layout");
    let key = PanelKey::new("layers").unwrap();

    let original_id = controller
        .layout()
        .panel_instances()
        .find_map(|(id, instance)| (instance.key == key).then_some(id))
        .expect("baseline layout should contain layers");

    controller
        .open_panel(&key)
        .expect("opening an already-open singleton should focus/reuse it");

    let open_layers = controller
        .layout()
        .panel_instances()
        .filter(|(_, instance)| instance.key == key)
        .count();
    assert_eq!(open_layers, 1);
    assert!(controller.layout().panel_instance(original_id).is_some());

    controller
        .close_panel(original_id)
        .expect("singleton panels must remain closable");
    assert_eq!(
        controller
            .layout()
            .panel_instances()
            .filter(|(_, instance)| instance.key == key)
            .count(),
        0
    );

    controller
        .open_panel(&key)
        .expect("a closed singleton should be openable again");
    let reopened_id = controller
        .layout()
        .panel_instances()
        .find_map(|(id, instance)| (instance.key == key).then_some(id))
        .expect("layers should be restored as a new runtime instance");

    assert_ne!(reopened_id, original_id);
}

#[test]
fn stale_layout_revision_cancels_drag_and_discards_resolver() {
    let metrics = DockMetrics::default();
    let registry = baseline_registry();
    let descriptors = registry.descriptors_for_core();
    let layout = dock_core::create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("baseline layout should be valid");
    let mut controller = DockController::new(
        layout,
        metrics,
        registry,
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("controller should accept the baseline layout");
    let panel = controller
        .layout()
        .panel_instances()
        .next()
        .map(|(panel, _)| panel)
        .expect("baseline layout contains panels");

    controller
        .begin_drag_interaction(
            DragPayload::Panel { panel },
            DragHandle::PanelTab,
            Point::ZERO,
        )
        .unwrap();
    controller
        .drag_mut()
        .session_mut()
        .pointer_move(Point::new(8.0, 0.0))
        .unwrap();
    controller
        .prepare_drop_resolver(DropSource::Panel { panel })
        .unwrap();
    assert!(controller.drag().resolver().is_some());

    controller
        .layout
        .apply(DockCommand::ClosePanel { panel }, &metrics)
        .unwrap();
    assert!(!controller.ensure_pointer_interaction_current());
    assert_eq!(controller.drag().session().state(), DragState::Idle);
    assert!(controller.drag().resolver().is_none());
    assert!(controller.layout().panel_instance(panel).is_none());
}

#[test]
fn viewport_change_cancels_drag_and_discards_resolver() {
    let metrics = DockMetrics::default();
    let registry = baseline_registry();
    let descriptors = registry.descriptors_for_core();
    let layout = dock_core::create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("baseline layout should be valid");
    let mut controller = DockController::new(
        layout,
        metrics,
        registry,
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("controller should accept the baseline layout");
    let panel = controller
        .layout()
        .panel_instances()
        .next()
        .map(|(panel, _)| panel)
        .expect("baseline layout contains panels");

    controller
        .begin_drag_interaction(
            DragPayload::Panel { panel },
            DragHandle::PanelTab,
            Point::ZERO,
        )
        .unwrap();
    controller
        .drag_mut()
        .session_mut()
        .pointer_move(Point::new(8.0, 0.0))
        .unwrap();
    controller
        .prepare_drop_resolver(DropSource::Panel { panel })
        .unwrap();
    assert!(controller.drag().resolver().is_some());

    assert!(
        controller
            .set_viewport(Rect::new(Point::ZERO, Size::new(1280.0, 800.0)))
            .unwrap()
    );
    assert_eq!(controller.drag().session().state(), DragState::Idle);
    assert!(controller.drag().resolver().is_none());
}

#[test]
fn resize_preview_does_not_mutate_layout_until_pointer_up() {
    let metrics = DockMetrics::default();
    let registry = baseline_registry();
    let descriptors = registry.descriptors_for_core();
    let layout = dock_core::create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("baseline layout should be valid");
    let mut controller = DockController::new(
        layout,
        metrics,
        registry,
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("controller should accept the baseline layout");
    let splitter = *controller
        .geometry()
        .unwrap()
        .splitters
        .first()
        .expect("baseline layout should contain a splitter");
    let origin = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    let hit = controller.hit(origin).unwrap();
    assert!(matches!(hit, HitResult::Splitter { .. }));

    let layout_before = format!("{:?}", controller.layout());
    let version_before = controller.layout().version();
    controller.begin_resize_interaction(hit, origin).unwrap();

    assert!(controller.set_resize_preview_point(Point::new(origin.x + 64.0, origin.y + 32.0,)));
    assert_eq!(format!("{:?}", controller.layout()), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(
        controller.resize_interaction().unwrap().preview_point,
        Some(Point::new(origin.x + 64.0, origin.y + 32.0))
    );

    controller.cancel_pointer_interaction().unwrap();
    assert_eq!(format!("{:?}", controller.layout()), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert!(controller.resize_interaction().is_none());
}

#[test]
fn resize_sequence_uses_core_actual_values_and_one_undo_checkpoint() {
    let metrics = DockMetrics::default();
    let registry = baseline_registry();
    let descriptors = registry.descriptors_for_core();
    let layout = dock_core::create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("baseline layout should be valid");
    let mut controller = DockController::new(
        layout,
        metrics,
        registry,
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .expect("controller should accept the baseline layout");

    let rects = controller.geometry().unwrap().clone();
    let splitter = *rects
        .splitters
        .first()
        .expect("baseline layout should contain a splitter");
    let origin = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    let hit = controller.hit(origin).unwrap();
    assert!(matches!(hit, HitResult::Splitter { .. }));

    let (column, group_count) = controller
        .layout()
        .nodes()
        .find_map(|(id, node)| match node {
            dock_core::Node::Column(column) if column.groups.len() >= 2 => {
                Some((id, column.groups.len()))
            }
            _ => None,
        })
        .expect("baseline layout should contain a multi-group column");
    let total_height = rects
        .vertical_overflow
        .iter()
        .find(|overflow| overflow.owner == column)
        .map(|overflow| overflow.content_extent.size.height)
        .or_else(|| {
            rects
                .columns
                .iter()
                .find(|rect| rect.column == column)
                .map(|rect| rect.rect.size.height)
        })
        .expect("column geometry should be available");
    let available_height =
        total_height - metrics.splitter_thickness * group_count.saturating_sub(1) as f32;
    let ratios_before = match controller.layout().node(column).unwrap() {
        dock_core::Node::Column(value) => value.group_ratios.clone(),
        _ => unreachable!(),
    };
    let version_before = controller.layout().version();

    controller.begin_resize_interaction(hit, origin).unwrap();
    let outcomes = controller
        .dispatch_sequence(vec![
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 4.0,
                available_height,
            },
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 3.0,
                available_height,
            },
        ])
        .expect("resize-only sequence should remain owned by the gesture");

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome.actual_value,
        Some(dock_core::CommandValue::GroupBoundaryDelta(delta)) if delta > 0.0
    )));
    assert!(controller.ensure_pointer_interaction_current());
    assert_eq!(controller.layout().version(), version_before + 2);
    assert!(controller.pending_events.iter().any(|event| matches!(
        event,
        DockEvent::ResizeApplied { hit: Some(event_hit), actual_values, .. }
            if *event_hit == hit && actual_values.len() == 2
    )));

    controller.end_resize_interaction();
    assert!(controller.can_undo());
    controller.undo().unwrap();
    assert!(!controller.can_undo());
    assert!(matches!(
        controller.layout().node(column),
        Some(dock_core::Node::Column(value)) if value.group_ratios == ratios_before
    ));
}

#[test]
fn main_geometry_cache_is_bound_to_its_window_id() {
    let mut controller = controller();
    let first_window = WindowId::from(71);
    let second_window = WindowId::from(72);

    controller.set_main_window_id(first_window);
    controller.geometry().unwrap();
    assert!(controller.cached_geometry_is_current());

    controller.set_main_window_id(second_window);
    assert!(!controller.cached_geometry_is_current());
    controller.geometry().unwrap();
    assert!(controller.cached_geometry_is_current());
}

#[test]
fn floating_geometry_hit_test_and_drop_resolver_are_window_local() {
    let mut controller = DockController::empty(
        Profile::Photoshop,
        DockMetrics::default(),
        baseline_registry(),
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .unwrap();
    controller.reset_layout().unwrap();

    let source_nodes = controller
        .layout()
        .nodes()
        .filter_map(|(id, node)| matches!(node, Node::Tabs(_)).then_some(id))
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(source_nodes.len(), 2, "the baseline workspace has two tab groups");

    for (index, node) in source_nodes.iter().copied().enumerate() {
        controller
            .dispatch(DockCommand::Float {
                node,
                bounds: Rect::new(
                    Point::new(280.0 + index as f32 * 40.0, 120.0),
                    Size::new(440.0, 360.0),
                ),
            })
            .unwrap();
    }

    let floating_groups = controller
        .layout()
        .floating_groups()
        .map(|(group, _)| group)
        .collect::<Vec<_>>();
    assert_eq!(floating_groups.len(), 2);

    let first_group = floating_groups[0];
    let second_group = floating_groups[1];
    let first_window = WindowId::from(81);
    let second_window = WindowId::from(82);

    controller.floating_geometry(first_window, first_group).unwrap();
    controller.floating_geometry(second_window, second_group).unwrap();
    assert_eq!(controller.floating_rects.len(), 2);
    assert_eq!(controller.floating_rects[&first_window].key.group, first_group);
    assert_eq!(controller.floating_rects[&second_window].key.group, second_group);

    assert!(controller.geometry().unwrap().floating_groups.is_empty());
    let first_rects = controller.floating_geometry(first_window, first_group).unwrap();
    assert_eq!(first_rects.floating_groups.len(), 1);
    assert_eq!(first_rects.floating_groups[0].rect.origin, Point::ZERO);
    let first_root = controller
        .layout()
        .floating_group(first_group)
        .and_then(|floating| controller.layout().dock_area(floating.area))
        .map(|area| area.root)
        .unwrap();

    // The floating title no longer has a dedicated Core hit result.
    // Its top strip belongs to the floating Column; GPUI handles title dragging.
    assert!(matches!(
        controller.hit_floating_window(first_window, first_group, Point::new(10.0, 10.0)),
        Ok(HitResult::Column { column }) if column == first_root
    ));

    // A hidden PopUp can be rebound to another group without retaining its old snapshot.
    controller.floating_geometry(first_window, second_group).unwrap();
    assert_eq!(controller.floating_rects[&first_window].key.group, second_group);
    assert_eq!(controller.floating_rects[&second_window].key.group, second_group);
    let first_cache = &controller.floating_rects[&first_window].rects as *const LayoutRects;
    let second_cache = &controller.floating_rects[&second_window].rects as *const LayoutRects;
    assert_ne!(first_cache, second_cache);

    let floating_root = controller
        .layout()
        .floating_group(second_group)
        .and_then(|floating| controller.layout().dock_area(floating.area))
        .map(|area| area.root)
        .unwrap();
    let source = DropSource::Group { group: floating_root };
    controller
        .begin_drag_interaction(
            DragPayload::Group { group: floating_root },
            DragHandle::GroupGrip,
            Point::new(40.0, 20.0),
        )
        .unwrap();
    controller
        .drag_mut()
        .session_mut()
        .pointer_move(Point::new(120.0, 100.0))
        .unwrap();
    assert!(matches!(controller.drag().session().state(), DragState::Dragging { .. }));

    controller
        .prepare_floating_window_drop_resolver(first_window, second_group, source)
        .unwrap();
    assert!(matches!(controller.drag.resolver_key, Some(ResolverCacheKey::Floating(_))));
    assert!(controller.main_drop_resolver().is_none());
    assert!(controller.main_drag_preview().is_none());
    assert!(controller
        .floating_window_drop_resolver(first_window, second_group)
        .is_some());
    assert!(controller
        .floating_window_drag_preview(first_window, second_group)
        .is_some());

    controller.prepare_drop_resolver(source).unwrap();
    assert!(matches!(controller.drag.resolver_key, Some(ResolverCacheKey::Main(_))));
    assert!(controller.main_drop_resolver().is_some());
    assert!(controller.main_drag_preview().is_some());
    assert!(controller
        .floating_window_drop_resolver(first_window, second_group)
        .is_none());
}

#[test]
fn native_tearoff_replaces_source_drag_with_floating_capture_and_escape_rolls_back() {
    let mut controller = DockController::empty(
        Profile::Photoshop,
        DockMetrics::default(),
        baseline_registry(),
        Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
    )
    .unwrap();
    controller.reset_layout().unwrap();

    let source_group = controller
        .layout()
        .nodes()
        .find_map(|(id, node)| matches!(node, Node::Tabs(_)).then_some(id))
        .unwrap();
    let press = Point::new(40.0, 20.0);
    controller
        .begin_drag_interaction(
            dock_core::DragPayload::Group { group: source_group },
            DragHandle::GroupGrip,
            press,
        )
        .unwrap();
    controller
        .drag_mut()
        .session_mut()
        .pointer_move(Point::new(60.0, 20.0))
        .unwrap();
    assert!(matches!(controller.drag().session().state(), DragState::Dragging { .. }));
    assert!(controller.begin_tearoff_transaction());

    let outcome = controller
        .history
        .apply(
            &mut controller.layout,
            DockCommand::Float {
                node: source_group,
                bounds: Rect::new(Point::new(1500.0, 500.0), Size::new(440.0, 360.0)),
            },
            &controller.metrics,
        )
        .unwrap();
    let Some(dock_core::CommandValue::FloatingGroup(floating_group)) = outcome.actual_value else {
        panic!("Float must return the newly created floating group id");
    };
    controller.drag.layout_version = Some(controller.layout.version());

    controller
        .continue_tearoff_as_floating_capture(floating_group)
        .unwrap();
    assert!(matches!(
        controller.drag().session().state(),
        DragState::Dragging { payload: dock_core::DragPayload::FloatingGroup { group }, .. }
            if group == floating_group
    ));
    assert_eq!(controller.drag().session().handle(), Some(DragHandle::FloatingCapture));
    assert!(controller.tearoff_transaction_active);

    controller.cancel_pointer_interaction_on_escape().unwrap();
    assert_eq!(controller.layout.floating_groups().count(), 0);
    assert!(!controller.tearoff_transaction_active);
    assert_eq!(controller.drag().session().state(), DragState::Idle);
}
