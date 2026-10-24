use std::collections::HashMap;

use dock_core::{Layout, LayoutRects, NodeId};
use gpui::{Div, ParentElement, ScrollHandle, Styled, div, px, rgb};
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
        }
    }

    pub fn build(self, host: &mut dyn DockHost) -> Div {
        render_layout(
            self.layout,
            self.frame.rects(),
            Some(self.frame),
            self.scroll_handles,
            self.theme,
            host,
            self.overlay,
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
    for element in layers::floating::render(&data, host) {
        root = root.child(element);
    }
    for element in layers::flyout::render(&data, host) {
        root = root.child(element);
    }
    for element in layers::drop_overlay::render(&data, overlay, &overflows, scroll_handles) {
        root = root.child(element);
    }

    root
}
