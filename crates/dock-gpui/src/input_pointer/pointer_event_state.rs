use std::{cell::Cell, rc::Rc};

use gpui::{Entity, HitboxId, ScrollHandle};

use crate::{DragHostManager, FloatingWindowManager};

use super::pointer_press::PointerPress;

/// Shared pointer state retained by DockHost across rerenders.
pub(crate) struct PointerEventState {
    pub(crate) manager: Entity<FloatingWindowManager>,
    pub(crate) drag_host_manager: Entity<DragHostManager>,
    pub(crate) pointer_capture: Rc<Cell<Option<HitboxId>>>,
    pub(crate) pointer_press: Rc<Cell<Option<PointerPress>>>,
}
