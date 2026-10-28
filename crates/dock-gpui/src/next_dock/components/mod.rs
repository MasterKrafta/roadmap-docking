//! Reusable components for the next dock implementation.

mod column_header;
mod group_header;
mod group_menu_button;
mod panel_tab;

pub(super) use column_header::ColumnHeader;
pub(super) use group_header::{GroupHeader, minimum_width_for_group};
