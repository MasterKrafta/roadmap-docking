#![forbid(unsafe_code)]
pub mod command;
pub mod drag_session;
pub mod geometry;
pub mod ids;
pub mod model;

pub use drag_session::{
    DRAG_START_THRESHOLD, DragCancelReason, DragDropRequest, DragSession, DragSessionState,
};
pub use command::{
    ColumnRef, CommandError, CommandOutcome, DockCommand, DropTarget, DropZone, FloatSource,
    SplitSide,
};
pub use geometry::{
    compute_rects, compute_drop_zones, hit_test, resolve_drop, snap_logical_to_physical_pixel,
    snap_rect_to_physical_pixels, ColumnHeaderRect, ColumnRect, DockAreaRect, DockMetrics,
    DropPayload, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, FloatingTitleRect,
    GroupRect, HitResult,
    LayoutRects, PanelRect, Point, Rect, ResizeEdge, Size, SplitterKind, SplitterRect,
    TabHeaderRect, TabStripRect,
};
pub use ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
pub use model::{
    Axis, CanvasState, Column, ColumnMode, DockArea, DockSide, FloatingGroup, Layout, Node,
    PanelConstraints, PanelInstance, PanelState, Profile, Split, Tabs, ValidationError,
};
