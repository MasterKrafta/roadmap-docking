//! Tab strip, fixed-width active tab, compressible inactive tabs,
//! drag zone, and group menu for the next dock implementation.

use dock_core::{Point, Rect, Size};
use gpui::{
    App, IntoElement, MouseDownEvent, ParentElement, RenderOnce, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, MouseDownHandler, TabStrip};

use super::super::model::TabGroup;
use super::super::window::TabDragContext;
use super::group_menu_button::GroupMenuButton;
use super::panel_tab::PanelTab;

const TAB_HORIZONTAL_PADDING: f32 = 8.0;
const MIN_INACTIVE_TAB_WIDTH: f32 = 32.0;
const MIN_DRAG_ZONE_WIDTH: f32 = 40.0;

#[derive(IntoElement)]
pub(in crate::next_dock) struct GroupHeader {
    pub(in crate::next_dock) rect: Rect,
    pub(in crate::next_dock) group: TabGroup,
    pub(in crate::next_dock) theme: Theme,
    pub(in crate::next_dock) key: String,
    pub(in crate::next_dock) tab_drag_context: Option<TabDragContext>,
}

impl RenderOnce for GroupHeader {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let rect = self.rect;
        let theme = self.theme;
        let width = rect.size.width.max(0.0);
        let height = rect.size.height.max(0.0);

        let panel_count = self.group.panels.len();

        let active_index = self.group.active.min(panel_count.saturating_sub(1));

        let natural_widths: Vec<f32> = self
            .group
            .panels
            .iter()
            .map(|panel| {
                measure_text_width(window, panel.title, &theme) + TAB_HORIZONTAL_PADDING * 2.0
            })
            .collect();

        let menu_width = theme.metrics.group_control_button_width.max(0.0).min(width);
        let tab_area_width = (width - menu_width).max(0.0);

        let active_width = natural_widths.get(active_index).copied().unwrap_or(0.0);

        let inactive_count = panel_count.saturating_sub(1);

        let minimum_tabs_width = active_width + inactive_count as f32 * MIN_INACTIVE_TAB_WIDTH;

        // Оставляем место под drag-зону, если группа может вместить
        // активную вкладку, минимальные неактивные и саму drag-зону.
        let drag_reserve = if tab_area_width >= minimum_tabs_width + MIN_DRAG_ZONE_WIDTH {
            MIN_DRAG_ZONE_WIDTH
        } else {
            0.0
        };

        let tab_budget = (tab_area_width - drag_reserve).max(0.0);
        let inactive_budget = (tab_budget - active_width).max(0.0);

        let mut tab_widths = natural_widths.clone();

        shrink_inactive_widths(&mut tab_widths, active_index, inactive_budget);

        let mut result = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .child(TabStrip { rect, theme });

        let mut x = 0.0;

        let mut tabs_viewport = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(tab_area_width))
            .h(px(height))
            .overflow_hidden();

        for (index, panel) in self.group.panels.iter().enumerate() {
            let tab_width = tab_widths[index];

            let tab_rect = Rect::new(Point::new(x, 0.0), Size::new(tab_width, height));

            let panel = *panel;

            let on_drag_start: MouseDownHandler =
                if let Some(context) = self.tab_drag_context.as_ref() {
                    let strip_bounds = context.strip_bounds;
                    let drag_tab_widths = tab_widths.clone();
                    let tab_left_x = tab_rect.origin.x;
                    let owner = context.owner.clone();
                    let composition_index = context.composition_index;
                    let path = context.path.clone();
                    let viewport_size = context.viewport_size;

                    Box::new(move |event: &MouseDownEvent, _window, app| {
                        let Some(owner) = owner.upgrade() else {
                            return;
                        };

                        let path = path.clone();
                        let tab_widths_for_drag = drag_tab_widths.clone();
                        let grab_offset_x =
                            event.position.x.as_f32() - (strip_bounds.origin.x + tab_left_x);

                        owner.update(app, |this, _cx| {
                            this.begin_tab_drag(
                                composition_index,
                                path,
                                index,
                                panel,
                                event.position,
                                viewport_size,
                                strip_bounds,
                                tab_widths_for_drag,
                                grab_offset_x,
                            );
                        });
                    })
                } else {
                    Box::new(|_, _, _| {})
                };

            let is_dragged_tab = self
                .tab_drag_context
                .as_ref()
                .and_then(|context| context.dragged_panel_index)
                == Some(index);

            if is_dragged_tab {
                tabs_viewport = tabs_viewport.child(
                    div()
                        .absolute()
                        .left(px(tab_rect.origin.x))
                        .top(px(tab_rect.origin.y))
                        .w(px(tab_rect.size.width))
                        .h(px(tab_rect.size.height))
                        .bg(rgb(theme.colors.tab_strip_background)),
                );
            } else {
                tabs_viewport = tabs_viewport.child(PanelTab {
                    id: format!("next-dock-tab-{}-{index}", self.key),
                    rect: tab_rect,
                    label: panel.title.to_owned(),
                    active: index == active_index,
                    theme,
                    on_activate: noop_click(),
                    on_drag_start,
                });
            }

            x += tab_width;
        }

        result = result.child(tabs_viewport);

        let drag_width = (tab_area_width - x).max(0.0);

        if drag_width > 0.0 {
            result = result.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(0.0))
                    .w(px(drag_width))
                    .h(px(height))
                    .bg(rgb(theme.colors.tab_strip_background)),
            );
        }

        let menu_rect = Rect::new(
            Point::new(tab_area_width, 0.0),
            Size::new(menu_width, height),
        );

        result.child(GroupMenuButton {
            id: format!("next-dock-group-menu-{}", self.key),
            rect: menu_rect,
            theme,
            on_click: noop_click(),
        })
    }
}

pub(in crate::next_dock) fn minimum_width_for_group(
    group: &TabGroup,
    window: &Window,
    theme: &Theme,
) -> f32 {
    let panel_count = group.panels.len();
    let menu_width = theme.metrics.group_control_button_width.max(0.0);

    if panel_count == 0 {
        return menu_width + MIN_DRAG_ZONE_WIDTH;
    }

    let active_index = group.active.min(panel_count - 1);

    let active_width = measure_text_width(window, group.panels[active_index].title, theme)
        + TAB_HORIZONTAL_PADDING * 2.0;

    menu_width
        + active_width
        + panel_count.saturating_sub(1) as f32 * MIN_INACTIVE_TAB_WIDTH
        + MIN_DRAG_ZONE_WIDTH
}

fn shrink_inactive_widths(widths: &mut [f32], active_index: usize, available_width: f32) {
    loop {
        let current_width: f32 = widths
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != active_index)
            .map(|(_, width)| *width)
            .sum();

        let overflow = current_width - available_width;
        if overflow <= 0.01 {
            break;
        }

        let candidates: Vec<usize> = (0..widths.len())
            .filter(|index| {
                *index != active_index && widths[*index] > MIN_INACTIVE_TAB_WIDTH + 0.01
            })
            .collect();

        if candidates.is_empty() {
            break;
        }

        let reduction_per_tab = overflow / candidates.len() as f32;
        let mut total_reduced = 0.0;

        for index in candidates {
            let reducible = widths[index] - MIN_INACTIVE_TAB_WIDTH;
            let reduction = reducible.min(reduction_per_tab);

            widths[index] -= reduction;
            total_reduced += reduction;
        }

        if total_reduced <= 0.001 {
            break;
        }
    }
}

fn measure_text_width(window: &Window, text: &str, theme: &Theme) -> f32 {
    let typography = theme.typography;

    window
        .text_system()
        .shape_line(
            text.to_owned().into(),
            px(typography.tab_size),
            &[gpui::TextRun {
                len: text.len(),
                font: gpui::font(typography.family),
                ..Default::default()
            }],
            None,
        )
        .width()
        .as_f32()
}

fn noop_click() -> ClickHandler {
    Box::new(|_, _, _| {})
}
