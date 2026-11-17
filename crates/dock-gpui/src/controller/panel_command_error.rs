use dock_core::{CommandError, PanelKey};
use thiserror::Error;

/// Errors returned by panel-oriented controller helpers.
#[derive(Debug, Error)]
pub enum PanelCommandError {
    #[error("panel '{0}' is not registered")]
    UnknownPanel(PanelKey),

    #[error(transparent)]
    Command(#[from] CommandError),

    #[error("the main window bounds are not available for positioning a floating panel")]
    MainWindowBoundsUnavailable,
}
