use crate::{Axis, ColumnMode, PanelKey, PanelState};

/// Recursive, handle-free representation of one persistent docking tree node.
///
/// This type is part of the versioned workspace JSON schema. It deliberately
/// contains no SlotMap keys: panel leaves refer to stable panel kinds by
/// PanelKey, while parent-child ownership is represented by nested values.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceNode {
    Split {
        axis: Axis,
        children: Vec<WorkspaceNode>,
        ratios: Vec<f32>,
    },
    Tabs {
        children: Vec<WorkspaceNode>,
        active_index: usize,
        collapsed: bool,
    },
    Panel {
        key: PanelKey,
        #[serde(default)]
        state: PanelState,
    },
    Column {
        width: f32,
        mode: ColumnMode,
        groups: Vec<WorkspaceNode>,
        group_ratios: Vec<f32>,
        #[serde(default)]
        last_expanded_width: Option<f32>,
    },
}
