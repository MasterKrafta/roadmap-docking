use dock_core::{ColumnMode, NodeId, Rect};
use gpui::{App, IntoElement, RenderOnce, Window};
use ui::Theme;

use crate::components::{ClickHandler, IconButton};

/// Stateless control for cycling a column between expanded and rail presentations.
///
/// The caller owns the column state and supplies a handler that sends the next
/// SetColumnMode command through DockController.
#[derive(IntoElement)]
pub struct ColumnModeButton {
    /// Stable identity of the column controlled by this button.
    pub column: NodeId,
    /// Current mode, used to choose the direction glyph.
    pub mode: ColumnMode,
    /// Button bounds computed by the frame/layer adapter.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
    /// Requests the next column mode through the owning controller.
    pub on_toggle: ClickHandler,
}

impl RenderOnce for ColumnModeButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let background_color = self.theme.colors.column_mode_button;
        let hover_color = self.theme.colors.column_mode_button_hover;
        IconButton::new(
            format!("dock-column-mode-{:?}", self.column),
            self.rect,
            mode_glyph(self.mode),
            &self.theme,
            self.on_toggle,
        )
        .with_colors(background_color, hover_color)
        .with_border()
    }
}

fn mode_glyph(mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Expanded => ">>",
        ColumnMode::IconsAndLabels | ColumnMode::IconsOnly => "<<",
    }
}

#[cfg(test)]
mod tests {
    use super::mode_glyph;
    use dock_core::ColumnMode;

    #[test]
    fn direction_glyph_matches_expanded_and_rail_modes() {
        assert_eq!(mode_glyph(ColumnMode::Expanded), ">>");
        assert_eq!(mode_glyph(ColumnMode::IconsAndLabels), "<<");
        assert_eq!(mode_glyph(ColumnMode::IconsOnly), "<<");
    }
}
