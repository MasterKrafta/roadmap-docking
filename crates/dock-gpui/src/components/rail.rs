use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockCommand, FlyoutState, Point, TabHeaderRect,
};
use gpui::{
    Entity, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, img,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::DockController;

use super::tab_strip::GroupPresentation;

/// Flattened icon-rail presentation. The item order and rectangles are supplied
/// by dock-core; this component never traverses or changes the model tree.
pub struct RailView;

impl RailView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        column: ColumnRect,
        mode: ColumnMode,
        items: &[ColumnDisplayItem],
        headers: &[TabHeaderRect],
        presentations: &std::collections::HashMap<dock_core::NodeId, GroupPresentation>,
        content_origin: Point,
        _pointer_offset: Point,
        active_flyout: Option<FlyoutState>,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut rail = div()
            .id(format!("dock-rail-{:?}", column.column))
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(column.rect.size.width))
            .h_full();

        let mut previous_group = None;
        for item in items {
            let Some(header) = headers
                .iter()
                .find(|header| header.group == item.group && header.panel == item.panel)
            else {
                continue;
            };
            let presentation = presentations
                .get(&item.group)
                .and_then(|group| group.tabs.iter().find(|tab| tab.panel == item.panel));
            let title: SharedString = presentation
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| format!("Panel {}", item.display_index + 1).into());
            let icon = presentation.and_then(|tab| tab.icon.clone());
            let is_active = active_flyout
                .is_some_and(|flyout| flyout.column == column.column && flyout.panel == item.panel);
            let group_changed = previous_group.is_some_and(|group| group != item.group);
            previous_group = Some(item.group);

            let row_top = header.rect.origin.y - content_origin.y;
            let row_left = header.rect.origin.x - content_origin.x;
            let row_width = header.rect.size.width;
            let row_height = header.rect.size.height;
            let is_label_mode = mode == ColumnMode::IconsAndLabels;
            let icon_size = if is_label_mode { 16.0 } else { 20.0 };
            let panel = item.panel;
            let owner_column = column.column;
            let mut icon_row = div()
                .id(format!("rail-item-{:?}-{:?}", owner_column, panel))
                .absolute()
                .left(px(row_left))
                .top(px(row_top))
                .w(px(row_width))
                .h(px(row_height))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(if is_label_mode { 7.0 } else { 0.0 }))
                .px(px(if is_label_mode { 8.0 } else { 2.0 }))
                .overflow_hidden()
                .cursor_pointer()
                .bg(rgb(if is_active {
                    theme.colors.panel_tab_active
                } else {
                    theme.colors.panel_tab_inactive
                }))
                .border_1()
                .border_color(rgb(theme.colors.group_frame_border))
                .text_color(rgb(theme.colors.primary_text));

            if let Some(icon) = icon {
                icon_row = icon_row.child(img(icon).size(px(icon_size)).flex_none());
            } else {
                let fallback = title.chars().next().unwrap_or('?').to_string();
                icon_row = icon_row.child(
                    div()
                        .w(px(icon_size))
                        .h(px(icon_size))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.0))
                        .bg(rgb(theme.colors.group_grip_background))
                        .text_size(px(theme.typography.tab_size))
                        .child(fallback),
                );
            }

            if is_label_mode {
                icon_row = icon_row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(theme.typography.tab_size))
                        .child(title),
                );
            }

            let click_controller = controller.clone();
            icon_row = icon_row.on_click(move |event, _, app| {
                if event.standard_click() {
                    click_controller.update(app, |controller, controller_cx| {
                        let selected = controller.layout().flyout().is_some_and(|flyout| {
                            flyout.column == owner_column && flyout.panel == panel
                        });
                        let command =
                            if selected && controller.layout().flyout_auto_collapse_enabled() {
                                DockCommand::CloseFlyout
                            } else {
                                DockCommand::OpenFlyout {
                                    column: owner_column,
                                    panel,
                                }
                            };
                        if let Err(error) = controller.dispatch_in(command, controller_cx) {
                            tracing::warn!(%error, "could not toggle rail flyout");
                        }
                    });
                }
                app.stop_propagation();
            });

            if group_changed {
                rail = rail.child(
                    div()
                        .absolute()
                        .left(px(row_left + 4.0))
                        .top(px(row_top - 1.0))
                        .w(px((row_width - 8.0).max(0.0)))
                        .h(px(1.0))
                        .bg(rgb(theme.colors.group_frame_border)),
                );
            }
            rail = rail.child(icon_row);
        }

        rail
    }
}
