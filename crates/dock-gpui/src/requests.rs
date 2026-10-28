use dock_core::{
    Axis, DockSide, DragHandle, DragPreview, DropZoneRect, FloatingGroupId, NodeId, PanelId,
    ResizeEdge,
};

/// Semantic resize handle emitted by the GPUI adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeHandle {
    Splitter {
        owner: NodeId,
        boundary_index: usize,
        axis: Axis,
    },
    OuterColumn {
        column: NodeId,
        side: DockSide,
    },
    FloatingEdge {
        group: FloatingGroupId,
        edge: ResizeEdge,
    },
    /// Resizes the active rail flyout along the edge facing the canvas.
    Flyout {
        panel: PanelId,
    },
}

/// Source-aware, transient overlay produced from Core's drag preview and drop-zone geometry.
#[derive(Clone, Debug)]
pub struct DragOverlay {
    pub preview: DragPreview,
    pub handle: DragHandle,
    pub zones: Vec<DropZoneRect>,
}

/// Requests app-owned menu state without transferring ownership of the layout to the adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockMenuRequest {
    /// Opens the application-level list of registered panels.
    Dockers,
    TabOverflow {
        group: NodeId,
    },
    PanelGroup {
        group: NodeId,
        panel: Option<PanelId>,
    },
}
