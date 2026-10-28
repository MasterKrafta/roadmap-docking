use std::collections::{HashMap, HashSet};

use dock_core::{FloatingGroupId, Layout, LayoutRects, NodeId};
use gpui::{
    Div, InteractiveElement, MouseButton, ParentElement, ScrollHandle, Styled, div, px, rgb,
};
use ui::Theme;

use crate::components::RectBox;
use crate::frame::FrameGeometry;
use crate::host::DockHost;
use crate::layers::{self, main_areas};
use crate::requests::DragOverlay;

/// Stateless presentation data shared by all frame layers.
pub(crate) struct RenderData<'a> {
    pub layout: &'a Layout,
    pub rects: &'a LayoutRects,
    pub frame: Option<&'a FrameGeometry>,
    pub theme: &'a Theme,
}

/// Complete dock frame assembled in the canonical layer order.
///
/// Components own their display data after `build`; this surface only borrows the
/// current immutable layout snapshot, frame geometry, scroll handles and theme.
pub struct DockSurface<'a> {
    layout: &'a Layout,
    frame: &'a FrameGeometry,
    scroll_handles: &'a HashMap<NodeId, ScrollHandle>,
    theme: &'a Theme,
    overlay: Option<&'a DragOverlay>,
    excluded_floating_groups: Option<&'a HashSet<FloatingGroupId>>,
}

impl<'a> DockSurface<'a> {
    pub fn new(
        layout: &'a Layout,
        frame: &'a FrameGeometry,
        scroll_handles: &'a HashMap<NodeId, ScrollHandle>,
        theme: &'a Theme,
        overlay: Option<&'a DragOverlay>,
    ) -> Self {
        Self {
            layout,
            frame,
            scroll_handles,
            theme,
            overlay,
            excluded_floating_groups: None,
        }
    }

    /// Omits floating groups that are rendered in independent native windows.
    pub(crate) fn exclude_floating_groups(
        mut self,
        groups: &'a HashSet<FloatingGroupId>,
    ) -> Self {
        self.excluded_floating_groups = Some(groups);
        self
    }

    pub fn build(self, host: &mut dyn DockHost) -> Div {
        render_layout_excluding(
            self.layout,
            self.frame.rects(),
            Some(self.frame),
            self.scroll_handles,
            self.theme,
            host,
            self.overlay,
            self.excluded_floating_groups,
        )
    }
}

/// Shared composition for the frame-aware surface and compatibility entrypoints.
pub(crate) fn render_layout(
    layout: &Layout,
    rects: &LayoutRects,
    frame: Option<&FrameGeometry>,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    host: &mut dyn DockHost,
    overlay: Option<&DragOverlay>,
) -> Div {
    render_layout_excluding(
        layout,
        rects,
        frame,
        scroll_handles,
        theme,
        host,
        overlay,
        None,
    )
}

/// Shared composition for regular and native-window-backed floating groups.
pub(crate) fn render_layout_excluding(
    layout: &Layout,
    rects: &LayoutRects,
    frame: Option<&FrameGeometry>,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    host: &mut dyn DockHost,
    overlay: Option<&DragOverlay>,
    excluded_floating_groups: Option<&HashSet<FloatingGroupId>>,
) -> Div {
    let data = RenderData {
        layout,
        rects,
        frame,
        theme,
    };
    let mut root = div()
        .relative()
        .w(px(rects.viewport.map_or(1.0, |value| value.size.width)))
        .h(px(rects.viewport.map_or(1.0, |value| value.size.height)))
        .bg(rgb(theme.colors.window_background));

    if let Some(canvas) = rects.canvas {
        root = root.child(RectBox::new(canvas, theme.colors.canvas_background));
    }

    // Layer order is significant: main areas, outer edges, floating windows,
    // flyout, then transient drop feedback.
    let (main_elements, overflows) = main_areas::render(&data, host, scroll_handles);
    for element in main_elements {
        root = root.child(element);
    }
    for element in layers::outer_edges::render(&data, host) {
        root = root.child(element);
    }
    for element in layers::floating::render(&data, host, excluded_floating_groups) {
        root = root.child(element);
    }
    for element in layers::flyout::render(&data, host) {
        root = root.child(element);
    }
    for element in layers::drop_overlay::render(&data, overlay, &overflows, scroll_handles) {
        root = root.child(element);
    }

    // Gesture tracking belongs to the surface root so resize handles do not each
    // implement their own move/up state machine.
    root.on_mouse_move(host.surface_mouse_move())
        .on_mouse_up(MouseButton::Left, host.surface_mouse_up())
        .on_mouse_up_out(MouseButton::Left, host.surface_mouse_up())
}
