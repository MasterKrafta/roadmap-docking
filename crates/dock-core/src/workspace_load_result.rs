use crate::{Layout, WorkspaceDiagnostic};

/// A validated workspace layout plus non-fatal migration/recovery diagnostics.
#[derive(Debug)]
pub struct WorkspaceLoadResult {
    pub layout: Layout,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}
