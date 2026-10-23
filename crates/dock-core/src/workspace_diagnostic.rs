use crate::Rect;

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
