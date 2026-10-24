use dock_core::{FloatingGroupId, Point, Rect, Size};
use gpui::{
    App, ClickEvent, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    RenderOnce, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::IconButton;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type DragStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Header of a floating dock window; optional controls are rendered when their handlers are supplied.
#[derive(IntoElement)]
pub struct FloatingTitle {
    /// Floating-window identity for the stable element ID.
    pub group: FloatingGroupId,
    /// Header bounds, local to its floating-window container.
    pub rect: Rect,
    /// Display title for the floating window.
    pub title: String,
    /// Shared application theme.
    pub theme: Theme,
    /// Starts dragging the floating window.
    pub on_drag_start: DragStartHandler,
    /// Optional close command; omitted until the host provides a safe close-window operation.
    pub on_close: Option<ClickHandler>,
    /// Optional collapse command for the floating window.
    pub on_collapse: Option<ClickHandler>,
}

impl RenderOnce for FloatingTitle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let button_width = self.rect.size.height.min(28.0).max(0.0);
        let button_count =
            usize::from(self.on_close.is_some()) + usize::from(self.on_collapse.is_some());
        let buttons_width = button_width * button_count as f32;
        let drag_width = (self.rect.size.width - buttons_width).max(0.0);

        let mut header = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.toolbar_background));

        let drag_area = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(drag_width))
            .h(px(self.rect.size.height))
            .text_color(rgb(self.theme.colors.primary_text))
            .child(self.title)
            .id(format!("dock-floating-title-{:?}", self.group))
            .on_mouse_down(MouseButton::Left, self.on_drag_start);
        header = header.child(drag_area);

        let mut button_right = self.rect.size.width;
        if let Some(on_close) = self.on_close {
            button_right -= button_width;
            header = header.child(
                IconButton::new(
                    format!("dock-floating-close-{:?}", self.group),
                    Rect::new(Point::new(button_right, 0.0), Size::new(button_width, self.rect.size.height)),
                    "×",
                    &self.theme,
                    on_close,
                )
                .into_any_element(),
            );
        }
        if let Some(on_collapse) = self.on_collapse {
            button_right -= button_width;
            header = header.child(
                IconButton::new(
                    format!("dock-floating-collapse-{:?}", self.group),
                    Rect::new(Point::new(button_right, 0.0), Size::new(button_width, self.rect.size.height)),
                    "−",
                    &self.theme,
                    on_collapse,
                )
                .into_any_element(),
            );
        }

        header
    }
}
