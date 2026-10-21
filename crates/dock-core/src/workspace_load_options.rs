use crate::Rect;

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
        Self { visible_client_region }
    }
}
