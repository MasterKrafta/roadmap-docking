//! Stateless GPUI building blocks used by the dock renderer.

pub mod column_mode_button;
pub mod drop_zone;
pub mod floating_title;
pub mod flyout_header;
pub mod focus_ring;
pub mod group_frame;
pub mod group_grip;
pub mod group_menu_button;
pub mod icon_button;
pub mod panel_surface;
pub mod popup_menu;
pub mod panel_tab;
pub mod rail_entry;
pub mod rect_box;
pub mod resize_edge_handle;
pub mod splitter_handle;
pub mod tab_overflow_button;
pub mod tab_strip;

pub use column_mode_button::ColumnModeButton;
pub use drop_zone::DropZone;
pub use floating_title::FloatingTitle;
pub use flyout_header::FlyoutHeader;
pub use focus_ring::with_focus_ring;
pub use group_frame::GroupFrame;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use icon_button::IconButton;
pub use panel_surface::PanelSurface;
pub use popup_menu::{PopupMenu, PopupMenuItem};
pub use panel_tab::PanelTab;
pub use rail_entry::RailEntry;
pub use rect_box::RectBox;
pub use resize_edge_handle::{ResizeEdgeHandle, ResizeEdgeKind};
pub use splitter_handle::SplitterHandle;
pub use tab_overflow_button::TabOverflowButton;
pub use tab_strip::TabStrip;
