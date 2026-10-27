use dock_core::{FloatingGroupId, Point, Rect, Size};
use gpui::{
    App, ClickEvent, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    RenderOnce, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::IconButton;

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
pub type DragStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Header of a floating dock window with separate move and dock-capture surfaces.
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
    /// Starts moving the floating window without resolving dock targets.
    pub on_drag_start: DragStartHandler,
    /// Starts dragging the entire floating group toward a docking target, when supported.
    pub on_capture_drag_start: Option<DragStartHandler>,
    /// Optional close command; omitted until the host provides a safe close-window operation.
    pub on_close: Option<ClickHandler>,
    /// Optional collapse command for the floating window.
    pub on_collapse: Option<ClickHandler>,
}

impl RenderOnce for FloatingTitle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let button_count = self.on_capture_drag_start.is_some() as usize
            + self.on_close.is_some() as usize
            + self.on_collapse.is_some() as usize;
        let max_button_width = if button_count == 0 {
            0.0
        } else {
            self.rect.size.width / button_count as f32
        };
        let button_width = self.rect.size.height.min(28.0).min(max_button_width).max(0.0);
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
                    Rect::new(
                        Point::new(button_right, 0.0),
                        Size::new(button_width, self.rect.size.height),
                    ),
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
                    Rect::new(
                        Point::new(button_right, 0.0),
                        Size::new(button_width, self.rect.size.height),
                    ),
                    "−",
                    &self.theme,
                    on_collapse,
                )
                .into_any_element(),
            );
        }

        if let Some(on_capture_drag_start) = self.on_capture_drag_start {
            button_right -= button_width;
            let capture_area = div()
                .absolute()
                .left(px(button_right))
                .top(px(0.0))
                .w(px(button_width))
                .h(px(self.rect.size.height))
                .bg(rgb(self.theme.colors.toolbar_background))
                .text_color(rgb(self.theme.colors.primary_text))
                .child("⇲")
                .id(format!("dock-floating-capture-{:?}", self.group))
                .on_mouse_down(MouseButton::Left, on_capture_drag_start);
            header = header.child(capture_area);
        }

        header
    }
}
