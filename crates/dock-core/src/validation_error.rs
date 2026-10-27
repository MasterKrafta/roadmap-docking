use crate::{DockAreaId, DockSide, FloatingGroupId, GeometryError, NodeId, PanelId, PanelKey};

/// Stable, typed reasons for invalid Split, Tabs, and Column node fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationReason {
    /// A Split must contain at least two child nodes.
    SplitMustContainAtLeastTwoChildren,
    /// The number of ratios differs from the number of children or groups.
    RatioCountMustMatchChildGroupCount,
    /// Each ratio must be finite and non-negative.
    RatiosMustBeFiniteAndNonNegative,
    /// Ratios must sum to one within the supported tolerance.
    RatiosMustSumToOne,
    /// A Tabs node must contain at least one child.
    TabsMustContainAtLeastOneChild,
    /// The active tab index points outside the child list.
    ActiveTabIndexOutsideChildren,
    /// Photoshop Tabs nodes may contain only Panel nodes.
    PhotoshopTabsChildrenMustBePanelNodes,
    /// A Column width must be finite.
    ColumnWidthMustBeFinite,
    /// A Column width must be positive.
    ColumnWidthMustBePositive,
    /// An expanded Column must not be narrower than min_column_width.
    ExpandedWidthBelowMinimumColumnWidth,
    /// IconsAndLabels Columns must use min_column_width.
    IconsAndLabelsWidthMustEqualMinimumColumnWidth,
    /// IconsOnly Columns must use rail_width.
    IconsOnlyWidthMustEqualRailWidth,
    /// The remembered expanded width must be finite and positive.
    LastExpandedWidthMustBeFiniteAndPositive,
    /// A Column must contain a group before normalization.
    ColumnMustContainAtLeastOneGroupBeforeNormalization,
    /// Photoshop Column groups may contain only Tabs nodes.
    PhotoshopColumnGroupsMustBeTabsNodes,
    /// The minimum width required by expanded contents could not be computed.
    CannotComputeMinimumWidthRequiredByExpandedContents,
    /// The maximum width allowed by expanded contents could not be computed.
    CannotComputeMaximumWidthAllowedByExpandedContents,
    /// Expanded contents require more width than the panel maximum allows.
    MinimumWidthRequiredByExpandedContentsExceedsPanelMaximumWidth,
    /// The expanded Column is narrower than its expanded contents require.
    ExpandedWidthBelowMinimumWidthRequiredByContents,
}

impl ValidationReason {
    /// Returns the existing user-facing description for this validation reason.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SplitMustContainAtLeastTwoChildren => "must contain at least two children",
            Self::RatioCountMustMatchChildGroupCount => "ratio count must match child/group count",
            Self::RatiosMustBeFiniteAndNonNegative => "ratios must be finite and non-negative",
            Self::RatiosMustSumToOne => "ratios must sum to 1.0 within 1e-4",
            Self::TabsMustContainAtLeastOneChild => "must contain at least one child",
            Self::ActiveTabIndexOutsideChildren => "active_index is outside children",
            Self::PhotoshopTabsChildrenMustBePanelNodes => "Photoshop Tabs children must be Panel nodes",
            Self::ColumnWidthMustBeFinite => "width must be finite",
            Self::ColumnWidthMustBePositive => "width must be positive",
            Self::ExpandedWidthBelowMinimumColumnWidth => "expanded width is below min_column_width",
            Self::IconsAndLabelsWidthMustEqualMinimumColumnWidth => "IconsAndLabels width must equal min_column_width",
            Self::IconsOnlyWidthMustEqualRailWidth => "IconsOnly width must equal rail_width",
            Self::LastExpandedWidthMustBeFiniteAndPositive => "last_expanded_width must be finite and positive",
            Self::ColumnMustContainAtLeastOneGroupBeforeNormalization => "must contain at least one group before normalization",
            Self::PhotoshopColumnGroupsMustBeTabsNodes => "Photoshop Column groups must be Tabs nodes",
            Self::CannotComputeMinimumWidthRequiredByExpandedContents => "cannot compute minimum width required by expanded contents",
            Self::CannotComputeMaximumWidthAllowedByExpandedContents => "cannot compute maximum width allowed by expanded contents",
            Self::MinimumWidthRequiredByExpandedContentsExceedsPanelMaximumWidth => "minimum width required by expanded contents exceeds panel maximum width",
            Self::ExpandedWidthBelowMinimumWidthRequiredByContents => "expanded width is below minimum width required by its contents",
        }
    }
}

impl std::fmt::Display for ValidationReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }

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

    #[error("layout tree exceeds the maximum depth of {max_depth} nodes")]
    TreeTooDeep { max_depth: usize },

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
    InvalidSplit { node: NodeId, reason: ValidationReason },

    #[error("invalid Tabs node {node:?}: {reason}")]
    InvalidTabs { node: NodeId, reason: ValidationReason },

    #[error("invalid Column node {node:?}: {reason}")]
    InvalidColumn { node: NodeId, reason: ValidationReason },

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
