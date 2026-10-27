use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use dock_core::{DockMetrics, Layout, WorkspaceFile, WorkspaceFileError, WorkspaceLoadOptions};
use ui::Theme;

use crate::{dock_metrics_from_theme, panels};

#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkspaceError {
    #[error(transparent)]
    Core(#[from] WorkspaceFileError),
    #[error("workspace path '{path}' has no file name")]
    MissingFileName { path: PathBuf },
    #[error("could not {operation} '{path}': {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Loads a workspace if a path is configured and a valid file is present.
///
/// Missing files mean a first launch and quietly select the baseline layout.
/// Read, parse, migration, and validation failures are logged and also select
/// the baseline layout; they never prevent the application window from opening.
pub(crate) fn load_layout(path: Option<&Path>, theme: &Theme) -> Option<Layout> {
    let path = path?;

    let json = match fs::read_to_string(path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not read workspace; using initial layout"
            );
            return None;
        }
    };

    let metrics: DockMetrics = dock_metrics_from_theme(theme);
    let descriptors = panels::descriptors(theme);
    match WorkspaceFile::load_json(
        &json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    ) {
        Ok(report) => {
            if !report.diagnostics.is_empty() {
                tracing::warn!(
                    path = %path.display(),
                    diagnostics = ?report.diagnostics,
                    "workspace loaded with recovery diagnostics"
                );
            }
            Some(report.layout)
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "workspace is invalid; using initial layout"
            );
            None
        }
    }
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Serializes the current persistent workspace state using Core's validated schema.
pub(crate) fn serialize_layout(
    layout: &Layout,
    metrics: &DockMetrics,
) -> Result<String, WorkspaceError> {
    WorkspaceFile::from_layout(layout, metrics)
        .and_then(|file| file.to_json_pretty())
        .map_err(WorkspaceError::from)
}

/// Saves a workspace by writing and syncing a sibling temporary file, then renaming it.
pub(crate) fn save_layout(
    path: Option<&Path>,
    layout: &Layout,
    metrics: &DockMetrics,
) -> Result<bool, WorkspaceError> {
    let Some(path) = path else {
        return Ok(false);
    };
    let json = serialize_layout(layout, metrics)?;
    atomic_replace(path, json.as_bytes())?;
    Ok(true)
}

fn atomic_replace(path: &Path, contents: &[u8]) -> Result<(), WorkspaceError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| WorkspaceError::MissingFileName {
            path: path.to_path_buf(),
        })?;

    fs::create_dir_all(parent).map_err(|source| WorkspaceError::Io {
        operation: "create workspace directory",
        path: parent.to_path_buf(),
        source,
    })?;

    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary_path: PathBuf = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        counter
    ));

    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)?;
        Ok(())
    })();

    if let Err(source) = result {
        let _ = fs::remove_file(&temporary_path);
        return Err(WorkspaceError::Io {
            operation: "atomically save workspace",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use dock_core::{create_initial_layout, Profile, WorkspaceFile};
    use ui::Theme;

    use crate::{dock_metrics_from_theme, panels};

    use super::load_layout;

    fn temporary_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "roadmap-docking-{label}-{}-{nonce}.json",
            std::process::id()
        ))
    }

    #[test]
    fn absent_workspace_selects_initial_layout_without_error() {
        let theme = Theme::default();
        let path = temporary_path("missing");
        assert!(load_layout(Some(&path), &theme).is_none());
    }

    #[test]
    fn malformed_workspace_is_rejected_without_panicking() {
        let theme = Theme::default();
        let path = temporary_path("invalid");
        fs::write(&path, "{not valid json")
            .expect("test should be able to write a malformed workspace");
        assert!(load_layout(Some(&path), &theme).is_none());
        fs::remove_file(path).expect("test workspace should be removable");
    }

    #[test]
    fn valid_workspace_is_loaded_and_validated() {
        let theme = Theme::default();
        let descriptors = panels::descriptors(&theme);
        let metrics = dock_metrics_from_theme(&theme);
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .expect("registered panels should build a valid initial layout");
        let json = WorkspaceFile::from_layout(&layout, &metrics)
            .and_then(|file| file.to_json_pretty())
            .expect("initial layout should serialize as a workspace");
        let path = temporary_path("valid");
        fs::write(&path, json).expect("test should be able to write a workspace");

        let loaded = load_layout(Some(&path), &theme).expect("valid workspace should load");
        assert_eq!(loaded.validate(&metrics), Ok(()));
        assert_eq!(loaded.panel_instances().count(), layout.panel_instances().count());
        fs::remove_file(path).expect("test workspace should be removable");
    }

    #[test]
    fn save_layout_atomically_replaces_existing_workspace_file() {
        use dock_core::{ChromeMode, DockCommand};

        let theme = Theme::default();
        let descriptors = panels::descriptors(&theme);
        let metrics = dock_metrics_from_theme(&theme);
        let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .expect("registered panels should build a valid initial layout");

        let root = temporary_path("atomic-save-root").with_extension("dir");
        let path = root.join("nested").join("workspace.json");
        assert!(super::save_layout(Some(&path), &layout, &metrics).unwrap());

        layout
            .apply(
                DockCommand::SetChrome {
                    mode: ChromeMode::DockHidden,
                },
                &metrics,
            )
            .expect("test should be able to change persisted workspace state");
        assert!(super::save_layout(Some(&path), &layout, &metrics).unwrap());

        let loaded = load_layout(Some(&path), &theme)
            .expect("the replaced workspace should remain loadable");
        assert_eq!(loaded.chrome(), ChromeMode::DockHidden);
        assert_eq!(loaded.validate(&metrics), Ok(()));

        fs::remove_dir_all(root).expect("test workspace directory should be removable");
    }

    #[test]
    fn save_layout_without_a_path_is_a_no_op() {
        let theme = Theme::default();
        let descriptors = panels::descriptors(&theme);
        let metrics = dock_metrics_from_theme(&theme);
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .expect("registered panels should build a valid initial layout");
        assert!(!super::save_layout(None, &layout, &metrics).unwrap());
    }
}
