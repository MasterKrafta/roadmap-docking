use dock_core::{NodeId, PanelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerClickIntent {
    ActivateTab { group: NodeId, panel: PanelId },
    CollapseGroup { group: NodeId },
    OpenOverflowMenu { group: NodeId },
    None,
}
