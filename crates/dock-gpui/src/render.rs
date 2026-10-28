use std::collections::HashMap;

use dock_core::{Layout, LayoutRects, NodeId};
use gpui::{Div, ScrollHandle};
use ui::Theme;

use crate::frame::FrameGeometry;
use crate::host::DockHost;
use crate::requests::DragOverlay;
use crate::surface::render_layout;

/// Renders a caller-provided rectangle snapshot through the common host contract.
pub fn render_with_host(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    host: &mut dyn DockHost,
    drag_overlay: Option<&DragOverlay>,
) -> Div {
    render_layout(
        layout,
        rects,
        None,
        scroll_handles,
        theme,
        host,
        drag_overlay,
    )
}

/// Preferred frame-aware entrypoint: geometry, drop zones and drawing share one snapshot.
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
