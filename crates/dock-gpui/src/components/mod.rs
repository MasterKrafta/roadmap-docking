//! Stateless GPUI building blocks used by the dock renderer.

pub mod focus_ring;
pub mod group_frame;
pub mod group_grip;
pub mod group_menu_button;
pub mod icon_button;
pub mod panel_tab;
pub mod rect_box;
pub mod tab_overflow_button;
pub mod tab_strip;

pub use focus_ring::with_focus_ring;
pub use group_frame::GroupFrame;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use icon_button::IconButton;
pub use panel_tab::PanelTab;
pub use rect_box::RectBox;
pub use tab_overflow_button::TabOverflowButton;
pub use tab_strip::TabStrip;
