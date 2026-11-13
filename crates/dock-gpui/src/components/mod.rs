//! Stateless GPUI presentation components for dock-core geometry snapshots.

pub mod canvas_slot;
pub mod context_menu;
mod drop_overlay_geometry;
pub mod drop_overlay;
pub mod floating;
pub mod flyout;
pub mod main_dock;
pub mod panel_host;
pub mod rail;
pub mod splitter;
pub mod tab_overflow;

pub use canvas_slot::CanvasSlot;
pub use context_menu::{ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem};
pub use drop_overlay::DropOverlay;
pub use floating::FloatingHost;
pub use flyout::FlyoutView;
pub use main_dock::MainDockView;
pub use panel_host::PanelHost;
pub use rail::RailView;
pub use splitter::SplitterView;
pub use tab_overflow::TabOverflow;

pub(crate) use context_menu::{ContextMenuTarget, ContextMenuToggle};
