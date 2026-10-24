use std::collections::HashMap;

use dock_core::{DockCommand, DragHandle, DragPayload, Layout, LayoutRects, NodeId, PanelInstance};
use gpui::{AnyElement, Div, ScrollHandle};
use ui::Theme;

use crate::frame::FrameGeometry;
use crate::host::{CommandClickHandler, DockHost, DragStartHandler, ResizeStartHandler};
use crate::requests::{DockMenuRequest, DragOverlay, ResizeHandle};
use crate::surface::render_layout;

/// Render with the built-in panel registry and caller-owned handlers.
#[allow(clippy::too_many_arguments)]
pub fn render_from_registry<H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    render(
        layout,
        rects,
        scroll_handles,
        theme,
        crate::panels::render_panel,
        crate::panels::panel_title,
        crate::panels::panel_icon,
        make_command_handler,
        make_menu_handler,
        make_drag_start_handler,
        make_resize_start_handler,
        drag_overlay,
    )
}

/// Compatibility adapter for existing callback-based callers.
#[allow(clippy::too_many_arguments)]
pub fn render<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    render_panel: F,
    panel_title: T,
    panel_icon: I,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    I: Fn(&PanelInstance) -> &'static str,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    let mut host = CallbackHost {
        render_panel_callback: render_panel,
        panel_title_callback: panel_title,
        panel_icon_callback: panel_icon,
        command_callback: make_command_handler,
        menu_callback: make_menu_handler,
        drag_start_callback: make_drag_start_handler,
        resize_start_callback: make_resize_start_handler,
    };
    render_layout(layout, rects, None, scroll_handles, theme, &mut host, drag_overlay)
}

/// Trait-based entrypoint for a rectangle snapshot.
pub fn render_with_host(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    host: &mut dyn DockHost,
    drag_overlay: Option<&DragOverlay>,
) -> Div {
    render_layout(layout, rects, None, scroll_handles, theme, host, drag_overlay)
}

/// Frame-aware compatibility adapter for existing callback-based callers.
#[allow(clippy::too_many_arguments)]
pub fn render_frame_from_registry<H, M, D, R>(
    layout: &Layout,
    frame: &FrameGeometry,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    let mut host = CallbackHost {
        render_panel_callback: crate::panels::render_panel,
        panel_title_callback: crate::panels::panel_title,
        panel_icon_callback: crate::panels::panel_icon,
        command_callback: make_command_handler,
        menu_callback: make_menu_handler,
        drag_start_callback: make_drag_start_handler,
        resize_start_callback: make_resize_start_handler,
    };
    render_layout(
        layout,
        frame.rects(),
        Some(frame),
        scroll_handles,
        theme,
        &mut host,
        drag_overlay,
    )
}

/// Preferred frame-aware entrypoint using a single `DockHost` object.
pub fn render_frame_with_host(
    layout: &Layout,
    frame: &FrameGeometry,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    host: &mut dyn DockHost,
    drag_overlay: Option<&DragOverlay>,
) -> Div {
    render_layout(
        layout,
        frame.rects(),
        Some(frame),
        scroll_handles,
        theme,
        host,
        drag_overlay,
    )
}

struct CallbackHost<F, T, I, H, M, D, R> {
    render_panel_callback: F,
    panel_title_callback: T,
    panel_icon_callback: I,
    command_callback: H,
    menu_callback: M,
    drag_start_callback: D,
    resize_start_callback: R,
}

impl<F, T, I, H, M, D, R> DockHost for CallbackHost<F, T, I, H, M, D, R>
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    I: Fn(&PanelInstance) -> &'static str,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    fn panel_title(&self, instance: &PanelInstance) -> gpui::SharedString {
        (self.panel_title_callback)(instance).into()
    }

    fn panel_icon(&self, instance: &PanelInstance) -> &'static str {
        (self.panel_icon_callback)(instance)
    }

    fn render_panel(&mut self, instance: &PanelInstance) -> AnyElement {
        (self.render_panel_callback)(instance)
    }

    fn command(&self, command: DockCommand) -> CommandClickHandler {
        (self.command_callback)(command)
    }

    fn menu(&self, request: DockMenuRequest) -> CommandClickHandler {
        (self.menu_callback)(request)
    }

    fn drag_start(&self, payload: DragPayload, handle: DragHandle) -> DragStartHandler {
        (self.drag_start_callback)(payload, handle)
    }

    fn resize_start(&self, handle: ResizeHandle) -> ResizeStartHandler {
        (self.resize_start_callback)(handle)
    }
}
