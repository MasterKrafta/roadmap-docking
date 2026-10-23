mod hamburger_button;
mod panel_content;
mod tab;

use gpui::{Context, Div, ParentElement, Styled, div, px, rgb};
use hamburger_button::HamburgerButton;
use panel_content::PanelContent;
use super::MainWindow;
use tab::Tab;
use ui::Theme;

const MIN_TAB_SPACER_WIDTH: f32 = 24.0;

/// Composes a dynamic collection of tab headers, a menu button, and a content surface.
pub(crate) struct TabsPanel {
    theme: Theme,
    tabs: Vec<Tab>,
    active_tab_id: String,
    hamburger_button: HamburgerButton,
    content: PanelContent,
}

impl TabsPanel {
    pub(crate) fn new(theme: Theme) -> Self {
        let tabs = vec![
            Tab::new("tab-layers", "Layers"),
            Tab::new("tab-color", "Color"),
        ];

        Self {
            theme,
            active_tab_id: "tab-layers".to_owned(),
            tabs,
            hamburger_button: HamburgerButton::new(theme),
            content: PanelContent::new(theme),
        }
    }

    pub(crate) fn set_active_tab_id(&mut self, tab_id: String) {
        if self.tabs.iter().any(|tab| tab.id() == tab_id) {
            self.active_tab_id = tab_id;
        }
    }

    pub(crate) fn render(
        &self,
        cx: &mut Context<MainWindow>,
        height: f32,
        group_index: usize,
    ) -> Div {
        let metrics = self.theme.metrics;
        let colors = self.theme.colors;
        let width = metrics.default_column_width.max(1.0);
        let height = height.max(1.0);
        let strip_height = metrics.tab_strip_height.min(height).max(1.0);
        let button_width = metrics.button_size.min(width).max(1.0);
        let (tab_width, spacer_width) = tab_strip_geometry(
            width,
            button_width,
            metrics.column_tab_width,
            self.tabs.len(),
        );
        let content_height = (height - strip_height).max(0.0);

        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(width))
            .h(px(strip_height))
            .bg(rgb(colors.toolbar_background))
            .border_1()
            .border_color(rgb(colors.border));

        for tab in &self.tabs {
            let tab_id = tab.id().to_owned();
            let active = self.active_tab_id == tab.id();
            strip = strip.child(tab.render(
                self.theme,
                tab_width,
                strip_height,
                active,
                cx.listener(move |this, _, _, cx| {
                    this.column.set_active_tab_id(group_index, tab_id.clone());
                    cx.notify();
                }),
            ));
        }

        strip = strip
            // The empty region separates the last tab from the menu button.
            .child(div().w(px(spacer_width)).h(px(strip_height)))
            .child(self.hamburger_button.render(button_width, strip_height));

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(strip)
            .child(self.content.render(width, content_height))
    }
}

/// Computes equal tab widths while preserving a small empty drag/menu gap.
fn tab_strip_geometry(
    width: f32,
    button_width: f32,
    desired_tab_width: f32,
    tab_count: usize,
) -> (f32, f32) {
    let width = width.max(0.0);
    let button_width = button_width.max(0.0).min(width);
    let minimum_spacer = MIN_TAB_SPACER_WIDTH.min((width - button_width).max(0.0));
    let available_for_tabs = (width - button_width - minimum_spacer).max(0.0);
    let tab_width = if tab_count == 0 {
        0.0
    } else {
        desired_tab_width
            .max(0.0)
            .min(available_for_tabs / tab_count as f32)
    };
    let spacer_width =
        (width - button_width - tab_width * tab_count as f32).max(minimum_spacer);

    (tab_width, spacer_width)
}

#[cfg(test)]
mod tests {
    use super::tab_strip_geometry;

    #[test]
    fn default_width_allocates_two_dynamic_tabs_and_a_gap() {
        let (tab_width, spacer_width) = tab_strip_geometry(280.0, 24.0, 104.0, 2);

        assert!((tab_width - 104.0).abs() < 0.001);
        assert!((spacer_width - 48.0).abs() < 0.001);
    }

    #[test]
    fn more_tabs_share_available_width_instead_of_overflowing() {
        let (tab_width, spacer_width) = tab_strip_geometry(280.0, 24.0, 104.0, 4);

        assert!((tab_width - 58.0).abs() < 0.001);
        assert!((tab_width * 4.0 + spacer_width + 24.0 - 280.0).abs() < 0.001);
    }

    #[test]
    fn empty_tab_list_keeps_the_menu_gap() {
        let (tab_width, spacer_width) = tab_strip_geometry(280.0, 24.0, 104.0, 0);

        assert_eq!(tab_width, 0.0);
        assert!((spacer_width - 256.0).abs() < 0.001);
    }
}
