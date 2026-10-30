use std::rc::Rc;

use dock_core::{DockCommand, NodeId, PanelId, TabOverflowRect};
use gpui::{
    App, Entity, IntoElement, ParentElement, Styled, div, img, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::DockController;

use super::tab_strip::TabPresentation;
use crate::input::dispatch_command;

pub(crate) type TabOverflowToggle = Rc<dyn Fn(NodeId, &mut App)>;

/// Overflow affordance and popup list for tabs omitted by Core geometry.
pub struct TabOverflow;

impl TabOverflow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group_rect: dock_core::Rect,
        strip_rect: dock_core::Rect,
        overflow: TabOverflowRect,
        hidden_tabs: &[TabPresentation],
        active_panel: Option<PanelId>,
        controller: Entity<DockController>,
        theme: Theme,
        menu_open: bool,
        toggle_overflow: TabOverflowToggle,
    ) -> impl IntoElement {
        let left = overflow.button_rect.origin.x - strip_rect.origin.x;
        let top = overflow.button_rect.origin.y - strip_rect.origin.y;
        let button_width = overflow.button_rect.size.width;
        let button_height = overflow.button_rect.size.height;
        let mut root = div()
            .id(format!("tab-overflow-{:?}", overflow.group))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(button_width))
            .h(px(button_height));

        let button_toggle = toggle_overflow.clone();
        let button = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.tab_overflow_button))
            .hover(|style| style.bg(rgb(theme.colors.tab_overflow_button_hover)))
            .text_color(rgb(theme.colors.primary_text))
            .child("»»")
            .on_click(move |_, _, app| {
                (button_toggle)(overflow.group, app);
                app.stop_propagation();
            });
        root = root.child(button);

        if menu_open {
            let menu_width = theme.metrics.popup_menu_width.min(group_rect.size.width).max(1.0);
            let popup_left = (left + button_width - menu_width)
                .clamp(0.0, (strip_rect.size.width - menu_width).max(0.0));
            let mut menu = div()
                .id(format!("tab-overflow-menu-{:?}", overflow.group))
                .absolute()
                .left(px(popup_left - left))
                .top(px(button_height))
                .w(px(menu_width))
                .flex()
                .flex_col()
                .border_1()
                .border_color(rgb(theme.colors.popup_menu_border))
                .bg(rgb(theme.colors.popup_menu_background));

            for tab in hidden_tabs {
                let is_active = active_panel == Some(tab.panel);
                menu = menu.child(overflow_menu_entry(
                    tab.clone(),
                    is_active,
                    overflow.group,
                    controller.clone(),
                    toggle_overflow.clone(),
                    theme,
                ));
            }
            root = root.child(menu);
        }

        root
    }
}

fn overflow_menu_entry(
    tab: TabPresentation,
    active: bool,
    group: NodeId,
    controller: Entity<DockController>,
    toggle_overflow: TabOverflowToggle,
    theme: Theme,
) -> impl IntoElement {
    let mut row = div()
        .w_full()
        .h(px(theme.metrics.popup_menu_row_height))
        .flex()
        .items_center()
        .px(px(theme.metrics.popup_menu_horizontal_padding))
        .cursor_pointer()
        .text_color(rgb(theme.colors.primary_text))
        .bg(rgb(if active {
            theme.colors.popup_menu_selected
        } else {
            theme.colors.popup_menu_background
        }))
        .hover(|style| style.bg(rgb(theme.colors.popup_menu_hover)));

    if let Some(icon) = tab.icon.clone() {
        row = row.child(img(icon).size(px(14.0)).flex_none());
    }
    row = row.child(
        div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(tab.title.clone()),
    );

    row.on_click(move |_, _, app| {
        dispatch_command(
            &controller,
            DockCommand::ActivateTab {
                group,
                panel: tab.panel,
            },
            app,
        );
        (toggle_overflow)(group, app);
        app.stop_propagation();
    })
}

