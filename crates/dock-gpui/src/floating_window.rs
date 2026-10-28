use crate::host::{
    CommandClickHandler, CommandKeyHandler, DockHost, DragStartHandler, NoopHost,
    ResizeStartHandler, SurfaceMouseMoveHandler, SurfaceMouseUpHandler,
};
use crate::layers::floating::FloatingGroupRenderMode;
use crate::main_window::MainWindow;
use crate::requests::{DockMenuRequest, ResizeHandle};
use crate::surface::RenderData;
use dock_core::{DockCommand, DragHandle, DragPayload, FloatingGroupId, PanelInstance, Rect};
use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Render, SharedString, Styled, Subscription,
    WeakEntity, Window, div, px, rgb,
};

/// A native window which presents one floating group from the main workspace.
///
/// The main window remains the single owner of layout commands, history and
/// persistence. This view observes it rather than keeping a second layout copy.
pub(crate) struct FloatingWindow {
    owner: WeakEntity<MainWindow>,
    group: FloatingGroupId,
    layout_bounds: Rect,
    owner_subscription: Option<Subscription>,
    bounds_subscription: Option<Subscription>,
}

impl FloatingWindow {
    pub(crate) fn new(
        owner: WeakEntity<MainWindow>,
        group: FloatingGroupId,
        initial_bounds: Rect,
    ) -> Self {
        Self {
            owner,
            group,
            layout_bounds: initial_bounds,
            owner_subscription: None,
            bounds_subscription: None,
        }
    }

    pub(crate) fn layout_bounds(&self) -> Rect {
        self.layout_bounds
    }

    fn sync_native_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let native_bounds = window.window_bounds().get_bounds();
        let viewport_size = window.viewport_size();
        let owner = self.owner.clone();
        let group = self.group;

        let Some(owner_entity) = owner.upgrade() else {
            window.remove_window();
            return;
        };

        let updated_bounds = owner_entity.update(cx, |main, main_cx| {
            main.sync_floating_native_bounds(group, native_bounds, viewport_size, main_cx)
        });

        if let Some(bounds) = updated_bounds {
            self.layout_bounds = bounds;
        }
    }
}

impl Render for FloatingWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if self.owner_subscription.is_none()
            && let Some(owner) = self.owner.upgrade()
        {
            self.owner_subscription = Some(cx.observe(&owner, |_this, _owner, cx| cx.notify()));
        }

        if self.bounds_subscription.is_none() {
            self.bounds_subscription =
                Some(cx.observe_window_bounds(window, |this, window, cx| {
                    this.sync_native_bounds(window, cx)
                }));
        }

        let Some(owner) = self.owner.upgrade() else {
            window.remove_window();
            return div().size_full();
        };
        let Some(snapshot) = owner.read(cx).floating_window_snapshot(self.group) else {
            window.remove_window();
            return div().size_full();
        };

        let mut host = FloatingWindowHost {
            owner: self.owner.clone(),
            group_origin: snapshot.bounds.origin,
        };
        let render_data = RenderData {
            layout: &snapshot.layout,
            rects: snapshot.frame.rects(),
            frame: Some(&snapshot.frame),
            theme: &snapshot.theme,
        };
        let elements = crate::layers::floating::render_group(
            &render_data,
            &mut host,
            self.group,
            snapshot.area,
            snapshot.bounds,
            FloatingGroupRenderMode::NativeWindow,
        );

        let mut root = div()
            .relative()
            .w(px(snapshot.bounds.size.width))
            .h(px(snapshot.bounds.size.height))
            .bg(rgb(snapshot.theme.colors.window_background));
        for element in elements {
            root = root.child(element);
        }
        root.on_mouse_move(host.surface_mouse_move())
            .on_mouse_up(MouseButton::Left, host.surface_mouse_up())
            .on_mouse_up_out(MouseButton::Left, host.surface_mouse_up())
    }
}

struct FloatingWindowHost {
    owner: WeakEntity<MainWindow>,
    group_origin: dock_core::Point,
}

impl FloatingWindowHost {
    fn noop(&self) -> NoopHost {
        NoopHost
    }
}

impl DockHost for FloatingWindowHost {
    fn panel_title(&self, instance: &PanelInstance) -> SharedString {
        self.noop().panel_title(instance)
    }

    fn panel_icon(&self, instance: &PanelInstance) -> &'static str {
        self.noop().panel_icon(instance)
    }

    fn render_panel(&mut self, instance: &PanelInstance) -> AnyElement {
        self.noop().render_panel(instance)
    }

    fn command(&self, command: DockCommand) -> CommandClickHandler {
        let owner = self.owner.clone();
        Box::new(move |_, _, app| {
            if let Some(owner) = owner.upgrade() {
                owner.update(app, |main, cx| {
                    main.apply_dock_command(command.clone(), cx);
                    cx.notify();
                });
            }
        })
    }

    fn command_key(&self, command: DockCommand) -> CommandKeyHandler {
        let owner = self.owner.clone();
        Box::new(move |_, _, app| {
            if let Some(owner) = owner.upgrade() {
                owner.update(app, |main, cx| {
                    main.apply_dock_command(command.clone(), cx);
                    cx.notify();
                });
            }
        })
    }

    fn close_floating(&self, group: FloatingGroupId) -> CommandClickHandler {
        let owner = self.owner.clone();
        Box::new(move |_, _, app| {
            if let Some(owner) = owner.upgrade() {
                owner.update(app, |main, cx| {
                    main.close_floating_window(group, cx);
                    cx.notify();
                });
            }
        })
    }

    fn menu(&self, request: DockMenuRequest) -> CommandClickHandler {
        let owner = self.owner.clone();
        let origin = self.group_origin;
        Box::new(move |event: &ClickEvent, _, app| {
            if let Some(owner) = owner.upgrade() {
                let position = gpui::point(
                    event.position().x + px(origin.x),
                    event.position().y + px(origin.y),
                );
                owner.update(app, |main, cx| {
                    main.open_menu(request, position);
                    cx.notify();
                });
            }
        })
    }

    fn drag_start(&self, payload: DragPayload, handle: DragHandle) -> DragStartHandler {
        let owner = self.owner.clone();
        Box::new(move |event: &MouseDownEvent, window, app| {
            let Some(owner) = owner.upgrade() else {
                return;
            };
            let source_bounds = window.window_bounds().get_bounds();
            let source_viewport = window.viewport_size();
            owner.update(app, |main, _cx| {
                main.begin_external_drag(
                    payload,
                    handle,
                    source_bounds,
                    source_viewport,
                    event.position,
                );
            });
        })
    }

    fn resize_start(&self, _handle: ResizeHandle) -> ResizeStartHandler {
        Box::new(|_, _, _| {})
    }

    fn resize_key(&self, _handle: ResizeHandle) -> CommandKeyHandler {
        Box::new(|_: &KeyDownEvent, _, _| {})
    }

    fn surface_mouse_move(&self) -> SurfaceMouseMoveHandler {
        let owner = self.owner.clone();
        Box::new(move |event: &MouseMoveEvent, window, app| {
            let Some(owner) = owner.upgrade() else {
                return;
            };
            let source_bounds = window.window_bounds().get_bounds();
            let source_viewport = window.viewport_size();
            owner.update(app, |main, cx| {
                main.handle_external_surface_mouse_move(event, source_bounds, source_viewport, cx);
            });
        })
    }

    fn surface_mouse_up(&self) -> SurfaceMouseUpHandler {
        let owner = self.owner.clone();
        Box::new(move |event: &MouseUpEvent, window, app| {
            let Some(owner) = owner.upgrade() else {
                return;
            };
            let source_bounds = window.window_bounds().get_bounds();
            let source_viewport = window.viewport_size();
            owner.update(app, |main, cx| {
                main.handle_external_surface_mouse_up(event, source_bounds, source_viewport, cx);
            });
        })
    }
}
