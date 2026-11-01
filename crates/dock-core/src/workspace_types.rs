//! Workspace data types, diagnostics, load options/results, and error definitions.

use crate::{
    Axis, CanvasState, ChromeMode, ColumnMode, GeometryError, Layout, PanelKey, PanelState,
    Profile, Rect, ValidationError,
};

/// Non-fatal information produced while loading or recovering a workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceDiagnostic {
    /// A supported older schema was migrated to the current version.
    SchemaMigrated { from: u32, to: u32 },
    /// A missing panel descriptor was preserved using placeholder constraints.
    UnknownPanelKeyPreserved { key: String },
    /// Safe, deterministic core normalization repaired these validation issues.
    LayoutNormalized { issues: Vec<String> },
    /// Floating bounds were clamped to the supplied visible client region.
    FloatingBoundsClamped {
        index: usize,
        original: Rect,
        clamped: Rect,
    },
}

/// Errors returned by parsing, migrating, and recovering the versioned workspace JSON boundary.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceFileError {
    #[error("invalid workspace JSON: {0}")]
    Json(String),

    #[error("unsupported workspace format '{found}'")]
    UnsupportedFormat { found: String },

    #[error("workspace version {found} is unsupported; the current schema version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("invalid dock metrics: {0}")]
    InvalidMetrics(&'static str),

    #[error("duplicate panel descriptor key '{key}'")]
    DuplicatePanelDescriptor { key: String },

    #[error("panel descriptor '{key}' has invalid constraints: {reason:?}")]
    InvalidPanelDescriptor { key: String, reason: GeometryError },

    #[error("floating group {index} has invalid bounds: {reason:?}")]
    InvalidFloatingBounds { index: usize, reason: GeometryError },

    #[error(
        "floating group {index} cannot fit in the visible client region: {axis:?} has {available} available, {required} required"
    )]
    FloatingBoundsInsufficientSpace {
        index: usize,
        axis: Axis,
        available: f32,
        required: f32,
    },

    #[error("visible client region has invalid bounds: {0:?}")]
    InvalidVisibleClientRegion(GeometryError),

    #[error("cannot create workspace file from layout: {0}")]
    InvalidRuntimeLayout(String),

    #[error("workspace layout violates core invariants: {0:?}")]
    InvalidLayout(Vec<ValidationError>),
}

/// Persistent floating-area placement without runtime area or group handles.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFloatingGroup {
    pub root: WorkspaceNode,
    pub bounds: Rect,
    pub z_order: i32,
}

/// Persistent structural portion of a workspace file.
///
/// UI handles, panel content state, scroll positions, flyout selection, runtime
/// layout version, and session-only placement hints are intentionally excluded.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLayout {
    pub profile: Profile,
    pub chrome: ChromeMode,
    #[serde(default)]
    pub canvas: CanvasState,
    pub left: Option<WorkspaceNode>,
    pub right: Option<WorkspaceNode>,
    #[serde(default)]
    pub floating_groups: Vec<WorkspaceFloatingGroup>,
}

/// Environment-neutral options for loading persistent workspace geometry.
///
/// The app/platform adapter may supply the current visible client rectangle in
/// logical pixels. Core never enumerates monitors or interprets physical DPI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorkspaceLoadOptions {
    pub visible_client_region: Option<Rect>,
}

impl WorkspaceLoadOptions {
    pub const fn new(visible_client_region: Option<Rect>) -> Self {
        Self {
            visible_client_region,
        }
    }
}

/// A validated workspace layout plus non-fatal migration/recovery diagnostics.
#[derive(Debug)]
pub struct WorkspaceLoadResult {
    pub layout: Layout,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

/// Recursive, handle-free representation of one persistent docking tree node.
///
/// This type is part of the versioned workspace JSON schema. It deliberately
/// contains no SlotMap keys: panel leaves refer to stable panel kinds by
/// PanelKey, while parent-child ownership is represented by nested values.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceNode {
    Split {
        axis: Axis,
        children: Vec<WorkspaceNode>,
        ratios: Vec<f32>,
    },
    Tabs {
        children: Vec<WorkspaceNode>,
        active_index: usize,
        collapsed: bool,
    },
    Panel {
        key: PanelKey,
        #[serde(default)]
        state: PanelState,
    },
    Column {
        width: f32,
        mode: ColumnMode,
        groups: Vec<WorkspaceNode>,
        group_ratios: Vec<f32>,
        #[serde(default)]
        last_expanded_width: Option<f32>,
    },
}
