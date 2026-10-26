//! Export helpers for the persistent workspace format.

use super::Layout;
use crate::{DockMetrics, WorkspaceFile, WorkspaceFileError};

impl Layout {
    /// Creates a validated workspace DTO suitable for the public JSON boundary.
    pub fn to_workspace_file(
        &self,
        metrics: &DockMetrics,
    ) -> Result<WorkspaceFile, WorkspaceFileError> {
        WorkspaceFile::from_layout(self, metrics)
    }
}
