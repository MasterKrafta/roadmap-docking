//! Stateless GPUI presentation components for dock-core geometry snapshots.

#[path = "canvas_slot.rs"]

pub mod canvas_slot;
#[path = "context_menu.rs"]
pub mod context_menu;
#[path = "drop_overlay_geometry.rs"]
mod drop_overlay_geometry;
#[path = "drop_overlay.rs"]
pub mod drop_overlay;
#[path = "floating.rs"]
pub mod floating;
#[path = "flyout.rs"]
pub mod flyout;
#[path = "main_dock.rs"]
pub mod main_dock;
#[path = "panel_host.rs"]
pub mod panel_host;
#[path = "rail.rs"]
pub mod rail;
#[path = "splitter.rs"]
pub mod splitter;
#[path = "tab_overflow.rs"]
pub mod tab_overflow;

pub use canvas_slot::CanvasSlot;
pub use context_menu::{ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem};
pub use drop_overlay::DropOverlay;
pub use flyout::FlyoutView;
pub use main_dock::MainDockView;
pub use panel_host::PanelHost;
pub use rail::RailView;
pub use splitter::SplitterView;
pub use tab_overflow::TabOverflow;

pub(crate) use context_menu::{ContextMenuTarget, ContextMenuToggle};
