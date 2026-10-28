//! Tab strip, fixed-width active tab, compressible inactive tabs,
//! drag zone, and group menu for the next dock implementation.

use dock_core::{Point, Rect, Size};
use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb};
use ui::Theme;

use crate::components::{ClickHandler, TabStrip};

use super::super::model::TabGroup;
use super::group_menu_button::GroupMenuButton;
use super::panel_tab::PanelTab;

const PREFERRED_TAB_WIDTH: f32 = 100.0;
const MIN_INACTIVE_TAB_WIDTH: f32 = 32.0;
const MIN_DRAG_ZONE_WIDTH: f32 = 40.0;

#[derive(IntoElement)]
pub(in crate::next_dock) struct GroupHeader {
    pub(in crate::next_dock) rect: Rect,
    pub(in crate::next_dock) group: TabGroup,
    pub(in crate::next_dock) theme: Theme,
    pub(in crate::next_dock) key: String,
}

impl RenderOnce for GroupHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let rect = self.rect;
        let theme = self.theme;
        let width = rect.size.width.max(0.0);
        let height = rect.size.height.max(0.0);

        let menu_width = theme.metrics.group_control_button_width.max(0.0).min(width);

        // The menu never participates in tab-width calculations.
        let tab_area_width = (width - menu_width).max(0.0);
        let panel_count = self.group.panels.len();

        let active_index = self.group.active.min(panel_count.saturating_sub(1));

        let capacity = max_tab_count(width, &theme);

        // Defensive fallback if the window becomes narrower after the tabs
        // were created. In normal operation, the insertion rule prevents
        // the group from exceeding this capacity.
        let visible_count = if panel_count == 0 {
            0
        } else if can_accept_panel(width, panel_count.saturating_sub(1), &theme) {
            panel_count
        } else {
            capacity.min(panel_count)
        };

        // Keep the active tab visible when the viewport is narrowed.
        let start_index = if visible_count == 0 {
            0
        } else {
            active_index
                .saturating_sub(visible_count.saturating_sub(1))
                .min(panel_count.saturating_sub(visible_count))
        };

        // Preserve the preferred active width and reserve drag space when
        // the group is wide enough to do so.
        let drag_reserve = if tab_area_width > MIN_DRAG_ZONE_WIDTH {
            MIN_DRAG_ZONE_WIDTH
        } else {
            0.0
        };

        let active_width = if visible_count > 0 {
            PREFERRED_TAB_WIDTH.min((tab_area_width - drag_reserve).max(0.0))
        } else {
            0.0
        };

        let inactive_count = visible_count.saturating_sub(1);
        let inactive_space = (tab_area_width - active_width - drag_reserve).max(0.0);

        let inactive_width = if inactive_count > 0 {
            (inactive_space / inactive_count as f32)
                .min(PREFERRED_TAB_WIDTH)
                .max(MIN_INACTIVE_TAB_WIDTH)
        } else {
            0.0
        };

        let mut result = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .child(TabStrip { rect, theme });

        let mut x = 0.0;

        for index in start_index..(start_index + visible_count) {
            let is_active = index == active_index;
            let tab_width = if is_active {
                active_width
            } else {
                inactive_width
            };

            let tab_rect = Rect::new(Point::new(x, 0.0), Size::new(tab_width, height));

            result = result.child(PanelTab {
                id: format!("next-dock-tab-{}-{index}", self.key),
                rect: tab_rect,
                label: self.group.panels[index].title.to_owned(),
                active: is_active,
                theme,
                on_activate: noop_click(),
            });

            x += tab_width;
        }

        // Everything left between the last tab and the menu is the drag zone.
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

/// Maximum number of tabs that can fit while respecting the width constraints.
fn max_tab_count(width: f32, theme: &Theme) -> usize {
    let menu_width = theme
        .metrics
        .group_control_button_width
        .max(0.0)
        .min(width.max(0.0));

    let tab_area_width = (width - menu_width).max(0.0);

    if tab_area_width <= 0.0 {
        return 0;
    }

    let drag_reserve = if tab_area_width >= PREFERRED_TAB_WIDTH + MIN_DRAG_ZONE_WIDTH {
        MIN_DRAG_ZONE_WIDTH
    } else {
        0.0
    };

    let active_width = PREFERRED_TAB_WIDTH.min((tab_area_width - drag_reserve).max(0.0));

    if active_width <= 0.0 {
        return 0;
    }

    if active_width < PREFERRED_TAB_WIDTH {
        return 1;
    }

    let space_for_inactive = (tab_area_width - active_width - drag_reserve).max(0.0);

    1 + (space_for_inactive / MIN_INACTIVE_TAB_WIDTH).floor() as usize
}

pub(in crate::next_dock) fn minimum_width_for_group(panel_count: usize, theme: &Theme) -> f32 {
    let menu_width = theme.metrics.group_control_button_width.max(0.0);

    let tabs_width = if panel_count == 0 {
        0.0
    } else {
        PREFERRED_TAB_WIDTH + panel_count.saturating_sub(1) as f32 * MIN_INACTIVE_TAB_WIDTH
    };

    menu_width + tabs_width + MIN_DRAG_ZONE_WIDTH
}

/// Whether another panel can be added without exceeding the width-based limit.
pub(super) fn can_accept_panel(
    group_width: f32,
    current_panel_count: usize,
    theme: &Theme,
) -> bool {
    current_panel_count < max_tab_count(group_width, theme)
}

fn noop_click() -> ClickHandler {
    Box::new(|_, _, _| {})
}
