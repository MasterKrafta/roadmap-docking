use crate::{DockAreaId, DockSide, FloatingGroupId, GeometryError, NodeId, PanelId, PanelKey};

/// A diagnostic emitted when a Layout violates a core invariant.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("invalid dock metrics: {reason}")]
    InvalidMetrics { reason: &'static str },

    #[error("invalid canvas state: {reason}")]
    InvalidCanvas { reason: &'static str },

    #[error("panel instance {panel:?} has invalid metadata: {reason:?}")]
    InvalidPanelInstance {
        panel: PanelId,
        reason: GeometryError,
    },

    #[error("node {owner:?} references missing node {missing:?}")]
    MissingNode {
        owner: Option<NodeId>,
        missing: NodeId,
    },

    #[error("cycle detected from node {from:?} to node {to:?}")]
    NodeCycle { from: NodeId, to: NodeId },

    #[error("node {node:?} has {references} incoming ownership references")]
    MultipleParents { node: NodeId, references: usize },

    #[error("node {node:?} is not reachable from a registered dock area")]
    UnreachableNode { node: NodeId },

    #[error("panel leaf node {node:?} references missing panel instance {panel:?}")]
    MissingPanelInstance { node: NodeId, panel: PanelId },

    #[error(
        "panel instance {panel:?} is placed in more than one leaf: {first_node:?} and {second_node:?}"
    )]
    DuplicatePanelPlacement {
        panel: PanelId,
        first_node: NodeId,
        second_node: NodeId,
    },

    #[error("panel instance {panel:?} is not represented by a panel leaf")]
    UnplacedPanelInstance { panel: PanelId },

    #[error("singleton panel key {key:?} is used by instances {first:?} and {second:?}")]
    DuplicateSingletonPanel {
        key: PanelKey,
        first: PanelId,
        second: PanelId,
    },

    #[error("invalid Split node {node:?}: {reason}")]
    InvalidSplit { node: NodeId, reason: &'static str },

    #[error("invalid Tabs node {node:?}: {reason}")]
    InvalidTabs { node: NodeId, reason: &'static str },

    #[error("invalid Column node {node:?}: {reason}")]
    InvalidColumn { node: NodeId, reason: &'static str },

    #[error("invalid DockArea {area:?}: {reason}")]
    InvalidDockArea {
        area: DockAreaId,
        reason: &'static str,
    },

    #[error("dock area {area:?} is not registered by a main side or floating group")]
    UnreferencedDockArea { area: DockAreaId },

    #[error("invalid main dock area {area:?} on side {side:?}: {reason}")]
    InvalidMainArea {
        area: DockAreaId,
        side: DockSide,
        reason: &'static str,
    },

    #[error("invalid root {root:?} for main area {area:?}: {reason}")]
    InvalidMainRoot {
        area: DockAreaId,
        root: NodeId,
        reason: &'static str,
    },

    #[error("invalid floating group {group:?}: {reason}")]
    InvalidFloatingGroup {
        group: FloatingGroupId,
        reason: &'static str,
    },

    #[error("floating dock area {area:?} is referenced by groups {first:?} and {second:?}")]
    FloatingAreaReferencedMoreThanOnce {
        area: DockAreaId,
        first: FloatingGroupId,
        second: FloatingGroupId,
    },

    #[error("floating groups {first:?} and {second:?} share z-order {z_order}")]
    DuplicateFloatingZOrder {
        first: FloatingGroupId,
        second: FloatingGroupId,
        z_order: i32,
    },

    #[error("invalid root {root:?} for floating group {group:?}: {reason}")]
    InvalidFloatingRoot {
        group: FloatingGroupId,
        root: NodeId,
        reason: &'static str,
    },

    #[error("Photoshop profile has {actual} main columns, exceeding maximum {max}")]
    TooManyColumns { actual: usize, max: usize },

    #[error("invalid transient flyout for column {column:?} and group {group:?}: {reason}")]
    InvalidFlyout {
        column: NodeId,
        group: NodeId,
        reason: &'static str,
    },
}
