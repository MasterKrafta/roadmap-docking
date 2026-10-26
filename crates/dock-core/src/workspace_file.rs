#[path = "workspace_file/export.rs"]
mod export;
#[path = "workspace_file/restore.rs"]
mod restore;
#[path = "workspace_file/schema.rs"]
mod schema;

use self::schema::migrate_v1_to_v2;
use crate::{
    DockMetrics, Layout, PanelConstraints, PanelKey, WorkspaceDiagnostic, WorkspaceFileError,
    WorkspaceLayout, WorkspaceLoadOptions, WorkspaceLoadResult,
};

/// Marker identifying this file as a Roadmap Docking workspace.
pub const WORKSPACE_FORMAT: &str = "roadmap-docking-workspace";

/// Current public workspace JSON schema version.
pub const WORKSPACE_VERSION: u32 = 2;

/// Maximum accepted JSON workspace file size (16 MiB).
const MAX_WORKSPACE_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of tree nodes restored from one workspace (including floating trees).
const MAX_WORKSPACE_NODES: usize = 100_000;
/// Maximum nesting depth accepted by workspace import/export, counting the root as level 1.
const MAX_WORKSPACE_TREE_DEPTH: usize = 256;

/// Versioned, JSON-encoded persistent workspace boundary.
///
/// Persist and restore this type, not Layout or individual arena-backed model
/// fragments. The nested DTO tree has no SlotMap handles, and loading always
/// creates fresh runtime arenas. JSON is the sole public workspace file format.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFile {
    pub format: String,
    pub version: u32,
    pub layout: WorkspaceLayout,
}

impl WorkspaceFile {
    /// Wraps persistent layout data in the current versioned file envelope.
    pub fn new(layout: WorkspaceLayout) -> Self {
        Self {
            format: WORKSPACE_FORMAT.to_owned(),
            version: WORKSPACE_VERSION,
            layout,
        }
    }

    /// Exports a validated Layout using the active metric profile.
    ///
    /// The structural export runs first so malformed or excessively deep runtime
    /// trees are rejected before recursive model validation.
    pub fn from_layout(
        layout: &Layout,
        metrics: &DockMetrics,
    ) -> Result<Self, WorkspaceFileError> {
        let file = Self::export_layout_unchecked(layout)?;
        layout
            .validate(metrics)
            .map_err(WorkspaceFileError::InvalidLayout)?;
        Ok(file)
    }

    /// Export helper for test fixtures that deliberately need an invalid Layout.
    #[cfg(test)]
    pub(crate) fn from_layout_unchecked_for_test(
        layout: &Layout,
    ) -> Result<Self, WorkspaceFileError> {
        Self::export_layout_unchecked(layout)
    }

    /// Parses a workspace file and migrates supported older versions.
    ///
    /// This convenience method returns the upgraded file. Use
    /// `from_json_with_diagnostics` or `load_json` when migration warnings
    /// must be presented to the user or logged.
    pub fn from_json(json: &str) -> Result<Self, WorkspaceFileError> {
        Self::from_json_with_diagnostics(json).map(|(file, _diagnostics)| file)
    }

    /// Parses the JSON envelope and returns any schema migration diagnostics.
    pub fn from_json_with_diagnostics(
        json: &str,
    ) -> Result<(Self, Vec<WorkspaceDiagnostic>), WorkspaceFileError> {
        if json.len() > MAX_WORKSPACE_JSON_BYTES {
            return Err(WorkspaceFileError::Json(format!(
                "workspace JSON exceeds the maximum size of {MAX_WORKSPACE_JSON_BYTES} bytes"
            )));
        }
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))?;

        let format = value
            .get("format")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| WorkspaceFileError::Json("missing string field 'format'".to_owned()))?;
        if format != WORKSPACE_FORMAT {
            return Err(WorkspaceFileError::UnsupportedFormat {
                found: format.to_owned(),
            });
        }

        let version_value = value
            .get("version")
            .ok_or_else(|| WorkspaceFileError::Json("missing field 'version'".to_owned()))?;
        let version_u64 = version_value.as_u64().ok_or_else(|| {
            WorkspaceFileError::Json("field 'version' must be a non-negative integer".to_owned())
        })?;
        let version = u32::try_from(version_u64).map_err(|_| {
            WorkspaceFileError::Json(
                "workspace version is outside the supported integer range".to_owned(),
            )
        })?;
        if !matches!(version, 1 | WORKSPACE_VERSION) {
            return Err(WorkspaceFileError::UnsupportedVersion {
                found: version,
                supported: WORKSPACE_VERSION,
            });
        }

        // Version 1 and 2 share the same structural DTO. The explicit v1
        // migration establishes the compatibility seam for future shape changes.
        let mut file: Self = serde_json::from_value(value)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))?;
        let mut diagnostics = Vec::new();
        if file.version == 1 {
            file = migrate_v1_to_v2(file);
            diagnostics.push(WorkspaceDiagnostic::SchemaMigrated {
                from: 1,
                to: WORKSPACE_VERSION,
            });
        }
        file.validate_header()?;
        Ok((file, diagnostics))
    }

    /// Encodes this workspace as the canonical pretty-printed JSON file.
    pub fn to_json_pretty(&self) -> Result<String, WorkspaceFileError> {
        self.validate_header()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))
    }

    /// Restores a validated layout, preserving unknown panel kinds as placeholders.
    ///
    /// Without load options, floating bounds are left unchanged because Core
    /// cannot infer the active monitor or the current client region.
    pub fn into_layout(
        self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        self.into_layout_with_options(descriptors, metrics, WorkspaceLoadOptions::default())
    }

    /// Restores a layout, optionally clamping floating bounds to a client region.
    pub fn into_layout_with_options(
        self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
        options: WorkspaceLoadOptions,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        self.into_layout_with_diagnostics(descriptors, metrics, options, Vec::new())
    }

    /// Full load pipeline: parse, migrate, rebuild fresh arenas, normalize safely,
    /// and return diagnostics in a single report.
    pub fn load_json(
        json: &str,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
        options: WorkspaceLoadOptions,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        let (file, diagnostics) = Self::from_json_with_diagnostics(json)?;
        file.into_layout_with_diagnostics(descriptors, metrics, options, diagnostics)
    }
}

