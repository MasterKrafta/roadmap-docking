#[path = "context_menu_callbacks.rs"]
mod context_menu_callbacks;
#[path = "context_menu_entry.rs"]
mod context_menu_entry;
#[path = "context_menu_item.rs"]
mod context_menu_item;
#[path = "context_menu_target.rs"]
mod context_menu_target;

pub use context_menu_callbacks::{
    ContextMenuCallbacks, ContextMenuDismiss, ContextMenuSubmenuToggle,
};
pub use context_menu_entry::ContextMenuEntry;
pub use context_menu_item::ContextMenuItem;
pub(crate) use context_menu_target::{
    ContextMenuTarget, ContextMenuToggle, GroupMenuToggle,
};

use dock_core::{Point, Rect};
use gpui::{
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, img,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

/// Universal popup menu used by group, tab, column, and tab-overflow controls.
///
/// It owns no docking state. Its caller supplies a menu model, viewport-local
/// anchor, and callbacks for dismissal and first-level submenu selection.
pub struct ContextMenu;

impl ContextMenu {
    pub fn render(
        id: impl Into<SharedString>,
        viewport: Rect,
        anchor: Point,
        entries: &[ContextMenuEntry],
        open_submenu: Option<usize>,
        callbacks: ContextMenuCallbacks,
        theme: Theme,
    ) -> impl IntoElement {
        let ContextMenuCallbacks {
            on_submenu,
            on_dismiss,
        } = callbacks;
        let width = theme
            .metrics
            .popup_menu_width
            .min(viewport.size.width)
            .max(1.0);
        let full_height = entries_height(entries, theme.metrics.popup_menu_row_height);
        let height = full_height.min(viewport.size.height.max(1.0));
        let max_x = (viewport.origin.x + viewport.size.width - width).max(viewport.origin.x);
        let max_y = (viewport.origin.y + viewport.size.height - height).max(viewport.origin.y);
        let left = anchor.x.clamp(viewport.origin.x, max_x);
        let top = anchor.y.clamp(viewport.origin.y, max_y);

        let submenu_layout = open_submenu.and_then(|index| {
            let ContextMenuEntry::Submenu {
                entries: submenu_entries,
                disabled: false,
                ..
            } = entries.get(index)?
            else {
                return None;
            };
            let row_top = top
                + entries
                    .iter()
                    .take(index)
                    .map(|entry| entry_height(entry, theme.metrics.popup_menu_row_height))
                    .sum::<f32>();
            let submenu_height =
                entries_height(submenu_entries, theme.metrics.popup_menu_row_height)
                    .min(viewport.size.height.max(1.0));
            let right = viewport.origin.x + viewport.size.width;
            let submenu_left = if left + width + width <= right {
                left + width - 1.0
            } else {
                (left - width + 1.0).max(viewport.origin.x)
            };
            let submenu_top = row_top.min(
                (viewport.origin.y + viewport.size.height - submenu_height).max(viewport.origin.y),
            );
            Some((index, submenu_left, submenu_top, submenu_height))
        });

        // One hitbox covers both the main menu and its submenu. GPUI's
        // on_mouse_down_out is then precise: it never treats a click inside
        // either popup as an outside click.
        let mut region_left = left;
        let mut region_top = top;
        let mut region_right = left + width;
        let mut region_bottom = top + height;
        if let Some((_, submenu_left, submenu_top, submenu_height)) = submenu_layout {
            region_left = region_left.min(submenu_left);
            region_top = region_top.min(submenu_top);
            region_right = region_right.max(submenu_left + width);
            region_bottom = region_bottom.max(submenu_top + submenu_height);
        }

        let mut layer = div().id(id.into()).absolute().top_0().left_0().size_full();
        let escape_dismiss = on_dismiss.clone();
        layer = layer.capture_key_down(move |event, _, app| {
            if event.keystroke.key.eq_ignore_ascii_case("escape") {
                (escape_dismiss)(app);
                app.stop_propagation();
            }
        });

        let dismiss_outside = on_dismiss.clone();
        let mut region = div()
            .absolute()
            .left(px(region_left))
            .top(px(region_top))
            .w(px((region_right - region_left).max(1.0)))
            .h(px((region_bottom - region_top).max(1.0)))
            .on_mouse_down_out(move |_, _, app| {
                (dismiss_outside)(app);
                app.stop_propagation();
            });

        let mut menu = menu_surface(left - region_left, top - region_top, width, height, theme);
        for (index, entry) in entries.iter().enumerate() {
            menu = menu.child(render_entry(
                entry,
                index,
                open_submenu,
                on_submenu.clone(),
                on_dismiss.clone(),
                theme,
                true,
            ));
        }
        region = region.child(menu);

        if let Some((index, submenu_left, submenu_top, submenu_height)) = submenu_layout
            && let Some(ContextMenuEntry::Submenu {
                entries: submenu_entries,
                disabled: false,
                ..
            }) = entries.get(index)
        {
            let mut submenu = menu_surface(
                submenu_left - region_left,
                submenu_top - region_top,
                width,
                submenu_height,
                theme,
            );
            for (submenu_index, entry) in submenu_entries.iter().enumerate() {
                submenu = submenu.child(render_entry(
                    entry,
                    submenu_index,
                    None,
                    on_submenu.clone(),
                    on_dismiss.clone(),
                    theme,
                    false,
                ));
            }
            region = region.child(submenu);
        }

        layer.child(region)
    }
}

fn menu_surface(
    left: f32,
    top: f32,
    width: f32,
    max_height: f32,
    theme: Theme,
) -> impl IntoElement + ParentElement {
    div()
        .id(format!("dock-menu-{left}-{top}-{width}"))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .max_h(px(max_height.max(1.0)))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .border_1()
        .border_color(rgb(theme.colors.popup_menu_border))
        .bg(rgb(theme.colors.popup_menu_background))
        .shadow_lg()
}

fn entries_height(entries: &[ContextMenuEntry], row_height: f32) -> f32 {
    entries
        .iter()
        .map(|entry| entry_height(entry, row_height))
        .sum()
}

fn entry_height(entry: &ContextMenuEntry, row_height: f32) -> f32 {
    match entry {
        ContextMenuEntry::Separator => 7.0,
        ContextMenuEntry::Item(_) | ContextMenuEntry::Submenu { .. } => row_height,
    }
}

fn render_entry(
    entry: &ContextMenuEntry,
    index: usize,
    open_submenu: Option<usize>,
    on_submenu: ContextMenuSubmenuToggle,
    on_dismiss: ContextMenuDismiss,
    theme: Theme,
    allow_submenu: bool,
) -> impl IntoElement {
    match entry {
        ContextMenuEntry::Separator => div()
            .id(format!("dock-menu-separator-{index}"))
            .w_full()
            .h(px(7.0))
            .flex()
            .items_center()
            .child(
                div()
                    .mx(px(6.0))
                    .w_full()
                    .h(px(1.0))
                    .bg(rgb(theme.colors.popup_menu_border)),
            ),
        ContextMenuEntry::Item(item) => {
            let mut row = div()
                .id(format!("dock-menu-item-{index}"))
                .w_full()
                .h(px(theme.metrics.popup_menu_row_height))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(theme.metrics.popup_menu_horizontal_padding))
                .text_color(rgb(if item.is_disabled() {
                    theme.colors.muted_text
                } else {
                    theme.colors.primary_text
                }))
                .bg(rgb(if item.is_selected() {
                    theme.colors.popup_menu_selected
                } else {
                    theme.colors.popup_menu_background
                }));

            if let Some(icon) = item.icon().cloned() {
                row = row.child(img(icon).size(px(14.0)).flex_none());
            } else if item.is_selected() {
                row = row.child(
                    div()
                        .w(px(14.0))
                        .flex_none()
                        .text_color(rgb(theme.colors.primary_text))
                        .child("✓"),
                );
            }

            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(item.label().clone()),
            );

            if let Some(hotkey) = item.hotkey().cloned() {
                row = row.child(
                    div()
                        .flex_none()
                        .text_color(rgb(theme.colors.muted_text))
                        .child(hotkey),
                );
            }

            if item.is_disabled() {
                row.opacity(0.62).cursor_not_allowed()
            } else if let Some(action) = item.action() {
                let dismiss = on_dismiss.clone();
                row.cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.colors.popup_menu_hover)))
                    .on_click(move |_, _, app| {
                        action(app);
                        (dismiss)(app);
                        app.stop_propagation();
                    })
            } else {
                row
            }
        }
        ContextMenuEntry::Submenu {
            label,
            hotkey,
            disabled,
            ..
        } => {
            let selected = open_submenu == Some(index);
            let mut row = div()
                .id(format!("dock-menu-submenu-{index}"))
                .w_full()
                .h(px(theme.metrics.popup_menu_row_height))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(theme.metrics.popup_menu_horizontal_padding))
                .text_color(rgb(if *disabled {
                    theme.colors.muted_text
                } else {
                    theme.colors.primary_text
                }))
                .bg(rgb(if selected {
                    theme.colors.popup_menu_selected
                } else {
                    theme.colors.popup_menu_background
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label.clone()),
                );

            if let Some(hotkey) = hotkey.clone() {
                row = row.child(
                    div()
                        .flex_none()
                        .text_color(rgb(theme.colors.muted_text))
                        .child(hotkey),
                );
            }
            row = row.child("›");

            if *disabled || !allow_submenu {
                row.opacity(0.62).cursor_not_allowed()
            } else {
                let toggle = on_submenu.clone();
                row.cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.colors.popup_menu_hover)))
                    .on_click(move |_, _, app| {
                        (toggle)(if selected { None } else { Some(index) }, app);
                        app.stop_propagation();
                    })
            }
        }
    }
}
