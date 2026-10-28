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
    /// Optional action that closes all tab groups contained in this floating window.
    pub on_close: Option<ClickHandler>,
    /// Optional collapse/expand command for the floating window's single tab group.
    pub on_collapse: Option<ClickHandler>,
    /// Displays '+' for a collapsed group and '−' for an expanded group.
    pub collapse_label: &'static str,
}

impl RenderOnce for FloatingTitle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let button_count = self.on_capture_drag_start.is_some() as usize
            + self.on_close.is_some() as usize
            + self.on_collapse.is_some() as usize;
        let button_gap = self.theme.metrics.button_gap;
        let button_width = floating_title_button_width(
            self.rect.size.height,
            self.theme.metrics.button_size,
            self.rect.size.width,
            button_count,
            button_gap,
        );
        let buttons_width =
            button_width * button_count as f32 + button_gap * button_count.saturating_sub(1) as f32;
        let drag_width = (self.rect.size.width - buttons_width).max(0.0);

        let typography = self.theme.typography;
        let mut header = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.toolbar_background))
            .font_family(typography.family)
            .text_size(px(typography.title_size))
            .line_height(px(typography.line_height));

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
        let mut controls_remaining = button_count;
        if let Some(on_close) = self.on_close {
            controls_remaining -= 1;
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
            if controls_remaining > 0 {
                button_right -= button_gap;
            }
        }
        if let Some(on_collapse) = self.on_collapse {
            controls_remaining -= 1;
            button_right -= button_width;
            header = header.child(
                IconButton::new(
                    format!("dock-floating-collapse-{:?}", self.group),
                    Rect::new(
                        Point::new(button_right, 0.0),
                        Size::new(button_width, self.rect.size.height),
                    ),
                    self.collapse_label,
                    &self.theme,
                    on_collapse,
                )
                .into_any_element(),
            );
            if controls_remaining > 0 {
                button_right -= button_gap;
            }
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

fn floating_title_button_width(
    header_height: f32,
    button_size: f32,
    header_width: f32,
    button_count: usize,
    button_gap: f32,
) -> f32 {
    if button_count == 0 {
        return 0.0;
    }

    let total_gap_width = button_gap * button_count.saturating_sub(1) as f32;
    let max_button_width = (header_width - total_gap_width).max(0.0) / button_count as f32;
    header_height
        .min(button_size)
        .min(max_button_width)
        .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::floating_title_button_width;

    #[test]
    fn floating_title_button_width_uses_the_theme_button_size() {
        assert_eq!(floating_title_button_width(36.0, 24.0, 200.0, 2, 4.0), 24.0);
    }

    #[test]
    fn floating_title_button_width_reserves_theme_gaps() {
        assert_eq!(floating_title_button_width(36.0, 24.0, 30.0, 2, 4.0), 13.0);
    }

    #[test]
    fn floating_title_button_width_respects_available_header_space() {
        assert_eq!(floating_title_button_width(36.0, 24.0, 30.0, 2, 0.0), 15.0);
    }

    #[test]
    fn floating_title_button_width_is_zero_without_controls() {
        assert_eq!(floating_title_button_width(36.0, 24.0, 200.0, 0, 4.0), 0.0);
    }
}
