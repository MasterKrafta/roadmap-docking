use crate::{CanvasState, ChromeMode, Profile, WorkspaceFloatingGroup, WorkspaceNode};

/// Persistent structural portion of a workspace file.
///
/// UI handles, panel content state, scroll positions, flyout selection, runtime
/// layout version, and session-only placement hints are intentionally excluded.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLayout {
    pub profile: Profile,
    pub chrome: ChromeMode,
    #[serde(default)]
    pub canvas: CanvasState,
    pub left: Option<WorkspaceNode>,
    pub right: Option<WorkspaceNode>,
    #[serde(default)]
    pub floating_groups: Vec<WorkspaceFloatingGroup>,
}
