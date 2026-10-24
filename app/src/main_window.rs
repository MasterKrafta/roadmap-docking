use gpui::{
    App, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

/// Root application window.
pub(crate) struct MainWindow {
    theme: ui::Theme,
    dockers_menu_open: bool,
    checked_panels: [bool; 4],
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        Ok(Self {
            theme,
            dockers_menu_open: false,
            checked_panels: [false; 4],
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);
        let menu_bar_height = self.theme.metrics.column_header_height.max(1.0);
        let dropdown_width = self.theme.metrics.default_column_width.max(1.0);
        let menu_item_height = 30.0;

        let dropdown = if self.dockers_menu_open {
            div()
                .flex()
                .flex_col()
                .w(px(dropdown_width))
                .bg(rgb(self.theme.colors.toolbar_background))
                .border_1()
                .border_color(rgb(self.theme.colors.border))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 1",
                    self.checked_panels[0],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[0] = !this.checked_panels[0];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 2",
                    self.checked_panels[1],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[1] = !this.checked_panels[1];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 3",
                    self.checked_panels[2],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[2] = !this.checked_panels[2];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 4",
                    self.checked_panels[3],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[3] = !this.checked_panels[3];
                        cx.notify();
                    }),
                ))
        } else {
            div().w(px(0.0)).h(px(0.0))
        };

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.canvas_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_bar_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(100.0))
                            .h(px(menu_bar_height))
                            .bg(rgb(self.theme.colors.toolbar_background))
                            .text_color(rgb(self.theme.colors.primary_text))
                            .border_1()
                            .border_color(rgb(self.theme.colors.border))
                            .child("Dockers ▾")
                            .id("dockers-menu-button")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dockers_menu_open = !this.dockers_menu_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(dropdown)
            .child(
                div()
                    .flex_1()
                    .w(px(width))
                    .bg(rgb(self.theme.colors.canvas_background)),
            )
    }
}

/// A menu row with a right-side checkmark that can be toggled independently.
fn render_menu_item<F>(
    theme: ui::Theme,
    width: f32,
    height: f32,
    label: &'static str,
    is_checked: bool,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .text_color(rgb(theme.colors.muted_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(
            div()
                .flex_1()
                .h(px(height))
                .items_center()
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.0))
                .h(px(height))
                .text_color(rgb(theme.colors.primary_text))
                .child(if is_checked { "✓" } else { "" }),
        )
        .id(label)
        .on_click(on_click)
}
