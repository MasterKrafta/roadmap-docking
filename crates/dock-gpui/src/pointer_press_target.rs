use dock_core::{NodeId, PanelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerPressTarget {
    TabHeader { group: NodeId, panel: PanelId },
    TabStrip { group: NodeId },
    OverflowButton { group: NodeId },
}
