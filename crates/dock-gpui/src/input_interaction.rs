use std::{cell::Cell, rc::Rc};

use dock_core::DragState;
use gpui::{App, Entity, HitboxId, Window};

use crate::DockController;

/// Shared pointer/capture lifecycle for MainDockHost and FloatingHost.
///
/// Hosts keep their own semantic press payloads and drop-target resolution,
/// while capture validation, cancellation and Core interaction state live here.
pub(crate) struct DockInteractionCoordinator;

impl DockInteractionCoordinator {
    pub(crate) fn capture(
        window: &mut Window,
        hitbox: HitboxId,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
    ) -> bool {
        window.capture_pointer(hitbox);
        let captured = window.captured_hitbox() == Some(hitbox);
        capture_state.set(captured.then_some(hitbox));
        captured
    }

    pub(crate) fn release(
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
    ) {
        window.release_pointer();
        capture_state.set(None);
    }

    pub(crate) fn has_capture(capture_state: &Rc<Cell<Option<HitboxId>>>) -> bool {
        capture_state.get().is_some()
    }

    pub(crate) fn window_has_capture(window: &Window) -> bool {
        window.captured_hitbox().is_some()
    }

    pub(crate) fn capture_was_lost(
        window: &Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
    ) -> bool {
        capture_state
            .get()
            .is_some_and(|captured| window.captured_hitbox() != Some(captured))
    }

    pub(crate) fn ensure_current(
        controller: &Entity<DockController>,
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
        app: &mut App,
    ) -> bool {
        let (current, active) = controller.update(app, |controller, cx| {
            let current = controller.ensure_pointer_interaction_current();
            if !current {
                cx.notify();
            }
            (current, controller.has_active_pointer_interaction())
        });
        if !current || (!active && Self::has_capture(capture_state)) {
            Self::release(window, capture_state);
        }
        current
    }

    pub(crate) fn active_interactions(
        controller: &Entity<DockController>,
        app: &mut App,
    ) -> (bool, bool, bool) {
        controller.update(app, |controller, _| {
            let state = controller.drag().session().state();
            (
                matches!(state, DragState::Pressed { .. } | DragState::Dragging { .. }),
                matches!(state, DragState::Dragging { .. }),
                controller.resize_interaction().is_some(),
            )
        })
    }

    /// Releases stale host capture after a command or layout change cancelled
    /// the Core interaction. Returns true while a Core gesture is still active.
    pub(crate) fn release_if_inactive(
        controller: &Entity<DockController>,
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
        app: &mut App,
    ) -> bool {
        let active = controller.update(app, |controller, _| {
            controller.has_active_pointer_interaction()
        });
        if !active && Self::has_capture(capture_state) {
            Self::release(window, capture_state);
        }
        active
    }

    pub(crate) fn cancel(
        controller: &Entity<DockController>,
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
        app: &mut App,
    ) {
        if let Err(error) = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction();
            cx.notify();
            result
        }) {
            tracing::debug!(%error, "could not cancel dock pointer interaction");
        }
        Self::release(window, capture_state);
    }

    pub(crate) fn cancel_on_focus_loss(
        controller: &Entity<DockController>,
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
        app: &mut App,
    ) {
        if let Err(error) = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction_on_focus_loss();
            cx.notify();
            result
        }) {
            tracing::warn!(%error, "could not cancel dock pointer interaction after focus loss");
        }
        Self::release(window, capture_state);
    }

    pub(crate) fn cancel_on_escape(
        controller: &Entity<DockController>,
        window: &mut Window,
        capture_state: &Rc<Cell<Option<HitboxId>>>,
        app: &mut App,
    ) {
        if let Err(error) = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            cx.notify();
            result
        }) {
            tracing::debug!(%error, "could not cancel dock pointer interaction with Escape");
        }
        Self::release(window, capture_state);
    }
}
