#![forbid(unsafe_code)]
pub mod command;
pub mod geometry;
pub mod ids;
pub mod model;

pub use command::{
    ColumnRef, CommandError, CommandOutcome, DockCommand, DropTarget, DropZone, FloatSource,
    SplitSide,
};
pub use geometry::{DockMetrics, Point, Rect, Size};
pub use ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
pub use model::{
    Axis, CanvasState, Column, ColumnMode, DockArea, DockSide, FloatingGroup, Layout, Node,
    PanelConstraints, PanelInstance, PanelState, Profile, Split, Tabs, ValidationError,
};
