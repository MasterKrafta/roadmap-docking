use gpui::Div;

/// Compatibility shim while the window implementation is migrated into the dock-gpui crate.
pub(super) fn render(
    theme: ui::Theme,
    selected_tab: &'static str,
    width: f32,
    height: f32,
) -> Div {
    dock_gpui::panels::content::render(theme, selected_tab, width, height)
}
