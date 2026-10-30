//! Reusable components used by the dock implementation.

mod column_header;
mod dock_column_view;
mod floating_composition_view;
mod group_header;
mod group_menu_button;
mod panel_tab;
mod tab_group_view;

pub(super) use dock_column_view::DockColumnView;
pub(super) use floating_composition_view::FloatingCompositionView;
pub(super) use group_header::minimum_width_for_group;
pub(super) use tab_group_view::render as render_tab_group;
