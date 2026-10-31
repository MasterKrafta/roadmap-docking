//! Workspace serialization and filesystem persistence for DockController.

use std::path::Path;

use dock_core::{
    InitialLayoutError, WorkspaceDiagnostic, WorkspaceFile, WorkspaceFileError,
    WorkspaceLoadOptions, create_initial_layout,
};
use thiserror::Error;

use crate::controller::DockController;

/// Errors returned while reading or writing a workspace file.
#[derive(Debug, Error)]
pub enum WorkspacePersistenceError {
    #[error("workspace file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Format(#[from] WorkspaceFileError),
}

impl DockController {
    /// Serializes the current Core layout using the versioned workspace format.
    pub fn save_workspace(&self) -> Result<String, WorkspaceFileError> {
        WorkspaceFile::from_layout(self.layout(), &self.metrics())?.to_json_pretty()
    }

    /// Writes the current workspace JSON to a file.
    pub fn save_workspace_to(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<(), WorkspacePersistenceError> {
        std::fs::write(path, self.save_workspace()?)?;
        Ok(())
    }

    /// Loads a validated workspace and starts a fresh history/cache lineage.
    pub fn load_workspace(
        &mut self,
        json: &str,
    ) -> Result<Vec<WorkspaceDiagnostic>, WorkspaceFileError> {
        let descriptors = self.registry().descriptors_for_core();
        let loaded = WorkspaceFile::load_json(
            json,
            &descriptors,
            &self.metrics(),
            WorkspaceLoadOptions::new(Some(self.viewport())),
        )?;
        let diagnostics = loaded.diagnostics;
        self.replace_layout(loaded.layout);
        self.queue_workspace_loaded(diagnostics.clone());
        Ok(diagnostics)
    }

    /// Reads and loads a workspace JSON file.
    pub fn load_workspace_from(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<WorkspaceDiagnostic>, WorkspacePersistenceError> {
        let json = std::fs::read_to_string(path)?;
        Ok(self.load_workspace(&json)?)
    }

    /// Recreates the profile baseline from registered panel descriptors.
    pub fn reset_workspace(&mut self) -> Result<(), InitialLayoutError> {
        let descriptors = self.registry().descriptors_for_core();
        let layout = create_initial_layout(self.layout().profile(), &descriptors, self.metrics())?;
        self.replace_layout(layout);
        Ok(())
    }
}
