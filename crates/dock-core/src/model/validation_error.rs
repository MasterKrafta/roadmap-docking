use std::fmt;

use crate::ids::{FloatingGroupId, NodeId, PanelId};

/// A diagnostic describing one violated layout invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidMetrics(&'static str),
    NodeReferenceMissing(NodeId),
    NodeHasMultipleParents(NodeId),
    NodeCycle(NodeId),
    UnreachableNode(NodeId),
    SplitTooFewChildren(NodeId),
    SplitRatioCountMismatch(NodeId),
    SplitRatioInvalid(NodeId),
    SplitRatiosDoNotSumToOne(NodeId),
    PhotoshopSplitMustBeVertical(NodeId),
    EmptyTabs(NodeId),
    ActiveTabOutOfBounds(NodeId),
    UnknownPanelReference {
        node: NodeId,
        panel: PanelId,
    },
    UnknownFloatingPanelReference {
        group: FloatingGroupId,
        panel: PanelId,
    },
    PanelPlacedMoreThanOnce(PanelId),
    PanelNotPlaced(PanelId),
    EmptyColumn {
        area_index: usize,
        column_index: usize,
    },
    ColumnRatioCountMismatch {
        area_index: usize,
        column_index: usize,
    },
    ColumnRatioInvalid {
        area_index: usize,
        column_index: usize,
    },
    ColumnRatiosDoNotSumToOne {
        area_index: usize,
        column_index: usize,
    },
    InvalidColumnWidth {
        area_index: usize,
        column_index: usize,
    },
    ColumnBelowMinimum {
        area_index: usize,
        column_index: usize,
    },
    TooManyColumns {
        actual: usize,
        maximum: usize,
    },
    PhotoshopColumnGroupMustBeTabs {
        area_index: usize,
        column_index: usize,
        group: NodeId,
    },
    EmptyFloatingGroup(FloatingGroupId),
    FloatingActiveTabOutOfBounds(FloatingGroupId),
    InvalidFloatingBounds(FloatingGroupId),
    InvalidPanelConstraints(PanelId),
    SingletonPanelDuplicated(String),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetrics(message) => write!(f, "invalid dock metrics: {message}"),
            Self::NodeReferenceMissing(node) => {
                write!(f, "node reference does not exist: {node:?}")
            }
            Self::NodeHasMultipleParents(node) => {
                write!(f, "node has more than one parent: {node:?}")
            }
            Self::NodeCycle(node) => write!(f, "node cycle detected at {node:?}"),
            Self::UnreachableNode(node) => {
                write!(f, "node is unreachable from every dock column: {node:?}")
            }
            Self::SplitTooFewChildren(node) => {
                write!(f, "split has fewer than two children: {node:?}")
            }
            Self::SplitRatioCountMismatch(node) => {
                write!(f, "split child and ratio counts differ: {node:?}")
            }
            Self::SplitRatioInvalid(node) => {
                write!(f, "split ratios must be finite and non-negative: {node:?}")
            }
            Self::SplitRatiosDoNotSumToOne(node) => {
                write!(f, "split ratios do not sum to 1.0: {node:?}")
            }
            Self::PhotoshopSplitMustBeVertical(node) => write!(
                f,
                "Photoshop profile only permits vertical splits: {node:?}"
            ),
            Self::EmptyTabs(node) => write!(f, "tab group is empty: {node:?}"),
            Self::ActiveTabOutOfBounds(node) => {
                write!(f, "active tab index is out of bounds: {node:?}")
            }
            Self::UnknownPanelReference { node, panel } => {
                write!(f, "node {node:?} references missing panel {panel:?}")
            }
            Self::UnknownFloatingPanelReference { group, panel } => write!(
                f,
                "floating group {group:?} references missing panel {panel:?}"
            ),
            Self::PanelPlacedMoreThanOnce(panel) => {
                write!(f, "panel is placed more than once: {panel:?}")
            }
            Self::PanelNotPlaced(panel) => {
                write!(f, "panel is not placed in dock or floating UI: {panel:?}")
            }
            Self::EmptyColumn {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} has no groups"
            ),
            Self::ColumnRatioCountMismatch {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} has mismatched group ratios"
            ),
            Self::ColumnRatioInvalid {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} has invalid group ratios"
            ),
            Self::ColumnRatiosDoNotSumToOne {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} group ratios do not sum to 1.0"
            ),
            Self::InvalidColumnWidth {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} has an invalid width"
            ),
            Self::ColumnBelowMinimum {
                area_index,
                column_index,
            } => write!(
                f,
                "dock area {area_index} column {column_index} is narrower than the profile minimum"
            ),
            Self::TooManyColumns { actual, maximum } => write!(
                f,
                "layout has {actual} columns; profile maximum is {maximum}"
            ),
            Self::PhotoshopColumnGroupMustBeTabs {
                area_index,
                column_index,
                group,
            } => write!(
                f,
                "Photoshop column {area_index}:{column_index} group {group:?} must be a Tabs node"
            ),
            Self::EmptyFloatingGroup(group) => write!(f, "floating group is empty: {group:?}"),
            Self::FloatingActiveTabOutOfBounds(group) => {
                write!(f, "floating group active tab is out of bounds: {group:?}")
            }
            Self::InvalidFloatingBounds(group) => write!(
                f,
                "floating group has a non-finite or non-positive rectangle: {group:?}"
            ),
            Self::InvalidPanelConstraints(panel) => {
                write!(f, "panel has invalid size constraints: {panel:?}")
            }
            Self::SingletonPanelDuplicated(key) => {
                write!(f, "singleton panel key is duplicated: {key}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}
