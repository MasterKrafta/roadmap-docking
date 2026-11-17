use std::rc::Rc;

use dock_core::{NodeId, PanelId, Point};
use gpui::App;

/// The source of a DockHost context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextMenuTarget {
    Group(NodeId),
    Tab { group: NodeId, panel: PanelId },
    Column(NodeId),
    Overflow(NodeId),
}

/// Opens or toggles one of the DockHost context menus.
pub(crate) type ContextMenuToggle = Rc<dyn Fn(ContextMenuTarget, Point, &mut App)>;

/// Callback shared by the existing group and overflow component interfaces.
pub(crate) type GroupMenuToggle = ContextMenuToggle;
