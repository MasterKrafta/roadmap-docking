use dock_core::Point;

use super::super::context_menu::ContextMenuTarget;

#[derive(Clone)]
pub(super) struct FloatingColumnOpenMenu {
    pub(super) target: ContextMenuTarget,
    pub(super) anchor: Point,
    pub(super) open_submenu: Option<usize>,
}
