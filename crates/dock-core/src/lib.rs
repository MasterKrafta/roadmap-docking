#![forbid(unsafe_code)]
pub mod command;
pub mod geometry;
pub mod ids;
pub mod model;

pub use command::{
    ColumnRef, CommandError, CommandOutcome, DockCommand, DropTarget, DropZone, FloatSource,
    SplitSide,
};
pub use geometry::{
    compute_rects, hit_test, compute_drop_zones, resolve_drop, snap_logical_to_physical_pixel, snap_rect_to_physical_pixels, DropPayload, ColumnHeaderRect, ColumnRect, DockAreaRect, DockMetrics, DropZoneRect,
    FloatingGroupRect, FloatingResizeEdgeRect, FloatingTitleRect, GroupRect, HitResult,
    LayoutRects, PanelRect, Point, Rect, ResizeEdge, Size, SplitterKind, SplitterRect,
    TabHeaderRect, TabStripRect,
};
pub use ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
pub use model::{
    Axis, CanvasState, Column, ColumnMode, DockArea, DockSide, FloatingGroup, Layout, Node,
    PanelConstraints, PanelInstance, PanelState, Profile, Split, Tabs, ValidationError,
};
