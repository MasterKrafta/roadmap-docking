use crate::ids::{FloatingGroupId, NodeId, PanelId};
use crate::model::DockSide;

use super::{ResizeEdge, SplitterKind};

/// Semantic result of hit-testing a point in logical layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HitResult {
    /// The body/content area of a panel.
    Panel(PanelId),
    /// One tab header in a tab group.
    TabHeader {
        group: NodeId,
        index: usize,
    },
    /// Header for a column on the requested dock side.
    ColumnHeader {
        side: DockSide,
        index: usize,
    },
    /// A splitter. The index follows the ordering used by the geometry result.
    Splitter {
        kind: SplitterKind,
        index: usize,
    },
    /// Title bar of an in-window floating group.
    FloatingTitle(FloatingGroupId),
    /// One edge or corner used to resize an in-window floating group.
    FloatingResizeEdge {
        floating: FloatingGroupId,
        edge: ResizeEdge,
    },
    /// The protected document/canvas region.
    Canvas,
    /// No interactive region contains the tested point.
    Empty,
}

#[cfg(test)]
mod tests {
    use super::HitResult;
    use crate::{DockSide, FloatingGroupId, NodeId, PanelId, ResizeEdge, SplitterKind};

    #[test]
    fn hit_results_retain_the_identity_and_kind_of_each_target() {
        let panel = PanelId::null();
        let group = NodeId::null();
        let floating = FloatingGroupId::null();

        assert_eq!(HitResult::Panel(panel), HitResult::Panel(panel));
        assert_eq!(
            HitResult::TabHeader { group, index: 2 },
            HitResult::TabHeader { group, index: 2 },
        );
        assert_eq!(
            HitResult::ColumnHeader { side: DockSide::Left, index: 1 },
            HitResult::ColumnHeader { side: DockSide::Left, index: 1 },
        );
        assert_eq!(
            HitResult::Splitter { kind: SplitterKind::Column, index: 0 },
            HitResult::Splitter { kind: SplitterKind::Column, index: 0 },
        );
        assert_eq!(
            HitResult::FloatingTitle(floating),
            HitResult::FloatingTitle(floating),
        );
        assert_eq!(
            HitResult::FloatingResizeEdge { floating, edge: ResizeEdge::BottomRight },
            HitResult::FloatingResizeEdge { floating, edge: ResizeEdge::BottomRight },
        );
        assert_ne!(HitResult::Canvas, HitResult::Empty);
    }
}
