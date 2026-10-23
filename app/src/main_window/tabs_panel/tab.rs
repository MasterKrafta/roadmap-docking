use gpui::{
    App, ClickEvent, Div, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div, px, rgb,
};
use ui::Theme;

/// One reusable tab header. Tab data is independent of the number of tabs in a group.
pub(crate) struct Tab {
    id: String,
    title: String,
}

impl Tab {
    pub(crate) fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn render<F>(
        &self,
        theme: Theme,
        width: f32,
        height: f32,
        active: bool,
        on_click: F,
    ) -> Div
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        let colors = theme.colors;
        let background = if active {
            colors.active_tab
        } else {
            colors.toolbar_background
        };
        let foreground = if active {
            colors.primary_text
        } else {
            colors.muted_text
        };

        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(width.max(0.0)))
            .h(px(height.max(1.0)))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .border_1()
            .border_color(rgb(colors.border))
            .id(self.id.clone())
            .on_click(on_click)
            .child(self.title.clone())
    }
}
