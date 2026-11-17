use dock_core::PanelId;
use platform::NativeError;

use crate::DragHostId;

#[derive(Debug, thiserror::Error)]
pub enum DragHostError {
    #[error("a DragHost is already active")]
    AlreadyActive,
    #[error("the main native window has not been attached")]
    MainWindowUnavailable,
    #[error("the DragHost size must be positive")]
    InvalidLogicalSize,
    #[error("the pointer grab offset is invalid")]
    InvalidGrabOffset,
    #[error("the native monitor scale factor is invalid")]
    InvalidScaleFactor,
    #[error("the pointer position cannot be converted to GPUI screen coordinates")]
    InvalidScreenPosition,
    #[error("no active DragHost matches {0:?}")]
    NotActive(DragHostId),
    #[error("Panel {0:?} has no cached view or matching active content drag session")]
    PanelTransferUnavailable(PanelId),
    #[error("the dragged composition has no matching active content drag session")]
    ContentTransferUnavailable,
    #[error("DragHost runtime identifiers are exhausted")]
    IdExhausted,
    #[error("GPUI could not create a DragHost window: {0}")]
    OpenWindow(String),
    #[error("GPUI window operation failed: {0}")]
    WindowUpdate(String),
    #[error(transparent)]
    Native(#[from] NativeError),
}
