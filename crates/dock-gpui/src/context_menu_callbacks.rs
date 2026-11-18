use std::rc::Rc;

use gpui::App;

/// Called when a menu is dismissed.
pub type ContextMenuDismiss = Rc<dyn Fn(&mut App)>;

/// Updates the single open first-level submenu.
pub type ContextMenuSubmenuToggle = Rc<dyn Fn(Option<usize>, &mut App)>;

/// Callbacks owned by the host while a context menu is open.
#[derive(Clone)]
pub struct ContextMenuCallbacks {
    pub on_submenu: ContextMenuSubmenuToggle,
    pub on_dismiss: ContextMenuDismiss,
}
