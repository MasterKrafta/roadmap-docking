use super::DockController;
use dock_core::{
    CommandError, DockCommand, DockMetrics, LayoutRects, PanelId, Point, Profile, Rect, Size,
};
use slotmap::Key;

use crate::registry::PanelRegistry;

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

    assert_eq!(controller.layout.update_canvas_bounds(Some(canvas)), Ok(false));

    let measured = Rect::new(
        Point::new(10.0, 20.0),
        Size::new(800.0, 600.0),
    );
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
        controller.set_viewport(Rect::new(
            Point::ZERO,
            Size::new(1280.0, 800.0),
        )),
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

    assert!(controller
        .dispatch_transient(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::AllHidden,
        })
        .is_err());

    assert_eq!(controller.layout.version(), version);
    assert!(!controller.can_undo());
}
