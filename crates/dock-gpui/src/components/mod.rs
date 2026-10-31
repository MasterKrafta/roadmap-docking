//! Stateless GPUI presentation components for dock-core geometry snapshots.

pub mod canvas_slot;
pub mod column;
pub mod context_menu;
pub mod drop_overlay;
pub mod group;
pub mod flyout;
pub mod floating;
pub mod main_dock;
pub mod panel_host;
pub mod rail;
pub mod splitter;
pub mod tab_overflow;
pub mod tab_strip;

pub use canvas_slot::CanvasSlot;
pub use context_menu::{ContextMenu, ContextMenuEntry, ContextMenuItem};
pub use column::ColumnView;
pub use drop_overlay::{DragGhost, DropOverlay};
pub use group::GroupView;
pub use flyout::FlyoutView;
pub use floating::FloatingWindowView;
pub use main_dock::MainDockView;
pub use panel_host::PanelHost;
pub use rail::RailView;
pub use tab_overflow::TabOverflow;
pub use splitter::SplitterView;
pub use tab_strip::{TabHeaderView, TabStripView};

pub(crate) use context_menu::{ContextMenuTarget, ContextMenuToggle};
pub(crate) use tab_strip::collect_group_presentations;
