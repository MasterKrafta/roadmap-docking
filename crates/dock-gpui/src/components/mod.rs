//! Small GPUI building blocks shared by the dock implementation.

use gpui::{App, ClickEvent, KeyDownEvent, MouseDownEvent, Window};

/// Boxed callback invoked by a GPUI click event.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Boxed callback invoked by a GPUI key-down event.
pub type KeyDownHandler = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App) + 'static>;

/// Boxed callback invoked by a GPUI mouse-down event.
pub type MouseDownHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

pub mod focus_ring;
pub mod group_frame;
pub mod icon_button;
pub mod panel_surface;
pub mod rect_box;
pub mod splitter_handle;
pub mod tab_strip;

pub use focus_ring::with_focus_ring;
pub use group_frame::GroupFrame;
pub use icon_button::IconButton;
pub use panel_surface::PanelSurface;
pub use rect_box::RectBox;
pub use splitter_handle::SplitterHandle;
pub use tab_strip::TabStrip;
