//! Reusable components for the next dock implementation.

mod column_header;
mod dock_column_view;
mod group_header;
mod group_menu_button;
mod floating_composition_view;
mod panel_tab;
mod tab_group_view;

pub(super) use column_header::ColumnHeader;
pub(super) use dock_column_view::DockColumnView;
pub(super) use floating_composition_view::FloatingCompositionView;
pub(super) use group_header::minimum_width_for_group;
pub(super) use tab_group_view::render as render_tab_group;
