use crate::{
    Axis, DropTarget, DropZone, FloatingGroupId, NodeId, PanelId, ResizeEdge,
};

/// Semantic result of hit-testing a point against a LayoutRects snapshot.
///
/// The result contains model handles and interaction meaning only. Pointer
/// events, views, windows, and platform-specific objects belong to the adapter layer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HitResult {
    /// The point belongs to the central application Canvas.
    Canvas,
    /// The point does not belong to any known interactive geometry.
    Miss,
    Column {
        column: NodeId,
    },
    Group {
        group: NodeId,
    },
    TabStrip {
        group: NodeId,
    },
    TabHeader {
        group: NodeId,
        panel: PanelId,
        index: usize,
    },
    /// Button that exposes tabs omitted from the visible tab headers.
    TabOverflowButton {
        group: NodeId,
    },
    Panel {
        panel: PanelId,
    },
    /// The active panel surface inside a transient rail flyout.
    FlyoutPanel {
        column: NodeId,
        group: NodeId,
        panel: PanelId,
    },
    /// The flyout's resize edge facing the Canvas.
    FlyoutResizeEdge {
        column: NodeId,
        panel: PanelId,
        edge: ResizeEdge,
    },
    Splitter {
        owner: NodeId,
        boundary_index: usize,
        axis: Axis,
    },
    /// The title band of a floating window, used for moving the window.
    FloatingTitle {
        group: FloatingGroupId,
    },
    /// The body/background of a floating window without a more specific hit.
    FloatingGroup {
        group: FloatingGroupId,
    },
    FloatingResizeEdge {
        group: FloatingGroupId,
        edge: ResizeEdge,
    },
    DropZone {
        target: DropTarget,
        zone: DropZone,
    },
}

#[cfg(test)]
mod tests {
    use slotmap::Key;

    use super::HitResult;
    use crate::{Axis, DropTarget, DropZone, NodeId, ResizeEdge};
    #[cfg(feature = "serde")]
    use crate::PanelId;

    #[test]
    fn hit_results_are_pure_semantic_values() {
        let hit = HitResult::Splitter {
            owner: NodeId::null(),
            boundary_index: 1,
            axis: Axis::Y,
        };
        assert!(matches!(
            hit,
            HitResult::Splitter {
                boundary_index: 1,
                axis: Axis::Y,
                ..
            }
        ));

        let hit = HitResult::DropZone {
            target: DropTarget::Tabs { group: NodeId::null(), index: 2 },
            zone: DropZone::Bottom,
        };
        assert!(matches!(hit, HitResult::DropZone { zone: DropZone::Bottom, .. }));

        let hit = HitResult::TabOverflowButton {
            group: NodeId::null(),
        };
        assert!(matches!(hit, HitResult::TabOverflowButton { group } if group == NodeId::null()));

        let hit = HitResult::FloatingResizeEdge {
            group: crate::FloatingGroupId::null(),
            edge: ResizeEdge::BottomRight,
        };
        assert!(matches!(
            hit,
            HitResult::FloatingResizeEdge { edge: ResizeEdge::BottomRight, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn hit_result_round_trips_through_serde() {
        let hit = HitResult::TabHeader {
            group: NodeId::null(),
            panel: PanelId::null(),
            index: 3,
        };
        let encoded = serde_json::to_string(&hit).unwrap();
        assert_eq!(serde_json::from_str::<HitResult>(&encoded).unwrap(), hit);

        let overflow = HitResult::TabOverflowButton {
            group: NodeId::null(),
        };
        let encoded = serde_json::to_string(&overflow).unwrap();
        assert_eq!(serde_json::from_str::<HitResult>(&encoded).unwrap(), overflow);
    }
}
