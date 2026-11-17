use dock_core::{NodeId, PanelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerDownIntent {
    TabHeaderCandidate { group: NodeId, panel: PanelId },
    GroupCandidate { group: NodeId },
    FlyoutPanelCandidate { panel: PanelId },
    Resize,
    OverflowButton { group: NodeId },
    DismissFlyout,
    None,
}
