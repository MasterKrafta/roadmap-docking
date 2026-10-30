use std::rc::Rc;

use dock_core::{DockCommand, GroupRect, NodeId, PanelId};
use gpui::{App, Entity, IntoElement, ParentElement, Styled, div, px, rgb, prelude::InteractiveElement as _};
use ui::Theme;

use crate::DockController;

pub(crate) type GroupMenuToggle = Rc<dyn Fn(NodeId, &mut App)>;

/// Context menu for one Tabs group. The component receives Core state and
/// only emits semantic commands; it owns no layout or visibility state.
pub(crate) struct GroupContextMenu;

impl GroupContextMenu {
    pub(crate) fn render(
        group: GroupRect,
        collapsed: bool,
        active_panel: Option<PanelId>,
        top: f32,
        controller: Entity<DockController>,
        toggle_menu: GroupMenuToggle,
        theme: Theme,
    ) -> impl IntoElement {
        let mut menu = div()
            .id(format!("group-context-menu-{:?}", group.group))
            .absolute()
            .top(px(top))
            .right(px(4.0))
            .w(px(theme.metrics.popup_menu_width.min(group.rect.size.width)))
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(theme.colors.popup_menu_border))
            .bg(rgb(theme.colors.popup_menu_background));

        menu = menu.child(menu_entry(
            "Закрыть группу",
            DockCommand::CloseGroup { group: group.group },
            group.group,
            controller.clone(),
            toggle_menu.clone(),
            theme,
        ));

        if let Some(panel) = active_panel {
            menu = menu.child(menu_entry(
                "Закрыть вкладку",
                DockCommand::ClosePanel { panel },
                group.group,
                controller.clone(),
                toggle_menu.clone(),
                theme,
            ));
        }

        menu = menu.child(menu_entry(
            if collapsed { "Развернуть" } else { "Свернуть" },
            DockCommand::CollapseGroup {
                group: group.group,
                collapsed: !collapsed,
            },
            group.group,
            controller.clone(),
            toggle_menu.clone(),
            theme,
        ));

        menu.child(menu_entry(
            "Вынести в плавающее окно",
            DockCommand::Float {
                node: group.group,
                bounds: group.rect,
            },
            group.group,
            controller,
            toggle_menu,
            theme,
        ))
    }
}

fn menu_entry(
    label: &'static str,
    command: DockCommand,
    group: NodeId,
    controller: Entity<DockController>,
    toggle_menu: GroupMenuToggle,
    theme: Theme,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(theme.metrics.popup_menu_row_height))
        .flex()
        .items_center()
        .px(px(theme.metrics.popup_menu_horizontal_padding))
        .cursor_pointer()
        .text_color(rgb(theme.colors.primary_text))
        .child(label)
        .on_click(move |_, _, app| {
            crate::input::dispatch_command(&controller, command.clone(), app);
            (toggle_menu)(group, app);
        })
}
