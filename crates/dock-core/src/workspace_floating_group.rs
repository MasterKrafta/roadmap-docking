use crate::{Rect, WorkspaceNode};

/// Persistent floating-area placement without runtime area or group handles.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFloatingGroup {
    pub root: WorkspaceNode,
    pub bounds: Rect,
    pub z_order: i32,
}
