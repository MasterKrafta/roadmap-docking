use std::rc::Rc;

use dock_core::Point;
use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::InteractiveElement as _, px, rgb, StatefulInteractiveElement,
};

use crate::{
    DockController, DockHost, DockTheme,
    components::{ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem},
};

/// Minimal application shell that owns the main menu above the docking host.
///
/// Panel menu rows are generated from PanelRegistry; their checked state is
/// derived from the current Layout on every render.
pub struct AppShell {
    controller: Entity<DockController>,
    dock_host: Entity<DockHost>,
    theme: DockTheme,
    panels_menu_open: bool,
    controller_observed: bool,
}

impl AppShell {
    pub fn new(
        controller: Entity<DockController>,
        dock_host: Entity<DockHost>,
        theme: DockTheme,
    ) -> Self {
        Self {
            controller,
            dock_host,
            theme,
            panels_menu_open: false,
            controller_observed: false,
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, shell_cx| shell_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        let controller_for_actions = self.controller.clone();
        let entries = self.controller.update(cx, move |controller, _| {
            let layout = controller.layout();
            controller
                .registry()
                .descriptors()
                .into_iter()
                .map(|descriptor| {
                    let key = descriptor.key().clone();
                    let title = descriptor.title().clone();
                    let checked = layout
                        .panel_instances()
                        .any(|(_, instance)| instance.key == key);
                    let side = match key.as_str() {
                        "layers" => dock_core::DockSide::Left,
                        _ => dock_core::DockSide::Right,
                    };
                    let action_controller = controller_for_actions.clone();
                    ContextMenuItem::new(title, move |app| {
                        crate::actions::toggle_registered_panel(
                            &action_controller,
                            key.clone(),
                            side,
                            app,
                        );
                    })
                    .selected(checked)
                    .into()
                })
                .collect::<Vec<ContextMenuEntry>>()
        });

        let shell_for_trigger = cx.entity().clone();
        let menu_trigger = div()
            .id("application-panels-menu-trigger")
            .h_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .text_color(rgb(self.theme.colors.primary_text))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(self.theme.colors.popup_menu_hover)))
            .on_click(move |_, _, app| {
                let _ = shell_for_trigger.update(app, |shell, shell_cx| {
                    shell.panels_menu_open = true;
                    shell_cx.notify();
                });
            })
            .child("Панели");

        let menu_bar = div()
            .w_full()
            .h(px(32.0))
            .flex()
            .items_center()
            .px(px(4.0))
            .bg(rgb(self.theme.colors.toolbar_background))
            .child(menu_trigger);

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(self.theme.colors.window_background))
            .child(menu_bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.dock_host.clone()),
            );

        if self.panels_menu_open {
            let mut viewport = self.controller.update(cx, |controller, _| controller.viewport());
            viewport.size.height += 32.0;
            let dismiss_shell = cx.entity().clone();
            let on_dismiss = Rc::new(move |app: &mut App| {
                let _ = dismiss_shell.update(app, |shell, shell_cx| {
                    shell.panels_menu_open = false;
                    shell_cx.notify();
                });
            });
            root = root.child(ContextMenu::render(
                "application-panels-menu",
                viewport,
                Point::new(4.0, 32.0),
                &entries,
                None,
                ContextMenuCallbacks {
                    on_submenu: Rc::new(|_, _| {}),
                    on_dismiss,
                },
                self.theme,
            ));
        }

        root
    }
}
