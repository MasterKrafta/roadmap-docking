use dock_core::{DockCommand, DragHandle, DragPayload, FloatingGroupId, PanelInstance};
use gpui::{
    AnyElement, ClickEvent, Context, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    SharedString,
};

use crate::host::{
    CommandClickHandler, CommandKeyHandler, DockHost, DragStartHandler, NoopHost,
    ResizeStartHandler, SurfaceMouseMoveHandler, SurfaceMouseUpHandler,
};
use crate::main_window::MainWindow;
use crate::requests::{DockMenuRequest, ResizeHandle};

/// Routes command clicks and pointer gesture starts through the owning window's controller.
///
/// Menu requests update the root window's transient menu state.
pub(crate) struct WindowHost<'a, 'cx> {
    cx: &'a Context<'cx, MainWindow>,
}

impl<'a, 'cx> WindowHost<'a, 'cx> {
    pub(crate) fn new(cx: &'a Context<'cx, MainWindow>) -> Self {
        Self { cx }
    }

    fn noop(&self) -> NoopHost {
        NoopHost
    }
}

impl DockHost for WindowHost<'_, '_> {
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
        Box::new(self.cx.listener::<ClickEvent>(
            move |this, _event, _window, cx| {
                this.apply_dock_command(command.clone(), cx);
                cx.notify();
            },
        ))
    }

    fn command_key(&self, command: DockCommand) -> CommandKeyHandler {
        Box::new(self.cx.listener::<KeyDownEvent>(
            move |this, _event, _window, cx| {
                this.apply_dock_command(command.clone(), cx);
                cx.notify();
            },
        ))
    }

    fn close_floating(&self, group: FloatingGroupId) -> CommandClickHandler {
        Box::new(self.cx.listener::<ClickEvent>(
            move |this, _event, _window, cx| {
                this.close_floating_window(group, cx);
                cx.notify();
            },
        ))
    }

    fn menu(&self, request: DockMenuRequest) -> CommandClickHandler {
        Box::new(self.cx.listener::<ClickEvent>(
            move |this, event, _window, cx| {
                this.open_menu(request, event.position());
                cx.notify();
            },
        ))
    }

    fn drag_start(&self, payload: DragPayload, handle: DragHandle) -> DragStartHandler {
        Box::new(self.cx.listener::<MouseDownEvent>(
            move |this, event, _window, _cx| {
                this.begin_drag(payload, handle, event.position);
            },
        ))
    }

    fn resize_start(&self, handle: ResizeHandle) -> ResizeStartHandler {
        Box::new(self.cx.listener::<MouseDownEvent>(
            move |this, event, window, cx| {
                this.begin_resize(handle, event.position, window, cx);
            },
        ))
    }

    fn resize_key(&self, handle: ResizeHandle) -> CommandKeyHandler {
        Box::new(self.cx.listener::<KeyDownEvent>(
            move |this, event, window, cx| {
                this.resize_splitter_by_keyboard(handle, event, window, cx);
            },
        ))
    }

    fn surface_mouse_move(&self) -> SurfaceMouseMoveHandler {
        Box::new(self.cx.listener::<MouseMoveEvent>(
            move |this, event, window, cx| {
                this.handle_surface_mouse_move(event, window, cx);
            },
        ))
    }

    fn surface_mouse_up(&self) -> SurfaceMouseUpHandler {
        Box::new(self.cx.listener::<MouseUpEvent>(
            move |this, event, window, cx| {
                this.handle_surface_mouse_up(event, window, cx);
            },
        ))
    }
}
