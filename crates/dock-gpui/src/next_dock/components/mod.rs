//! Reusable components for the next dock implementation.

mod column_header;
mod group_header;
mod group_menu_button;
mod panel_tab;
mod tab_group_view;

pub(super) use column_header::ColumnHeader;
pub(super) use group_header::{GroupHeader, minimum_width_for_group};
pub(super) use tab_group_view::render as render_tab_group;
