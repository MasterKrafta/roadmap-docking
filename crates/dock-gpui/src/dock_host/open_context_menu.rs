use dock_core::Point;

use crate::components::ContextMenuTarget;

#[derive(Clone)]
pub(super) struct OpenContextMenu {
    pub(super) target: ContextMenuTarget,
    pub(super) anchor: Point,
    pub(super) open_submenu: Option<usize>,
}
