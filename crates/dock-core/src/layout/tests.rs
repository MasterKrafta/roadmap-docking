use super::Layout;
use crate::{
    Axis, CanvasState, ChromeMode, CommandError, CommandValue, DockArea, DockAreaId, DockCommand,
    DockMetrics, DockSide, DropTarget, DropZone, FloatingGroup, FloatingGroupId, Node, NodeId,
    Panel, PanelConstraints, PanelId, PanelInstance, PanelKey, Point, Profile, Rect, Size, Split,
    Tabs, ValidationError,
};
use slotmap::{Key, SlotMap};

fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
    ["layers", "color", "properties", "history"]
        .into_iter()
        .map(|key| {
            (
                PanelKey::new(key).unwrap(),
                PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false),
            )
        })
        .collect()
}

fn initial_layout(profile: Profile) -> Layout {
    crate::create_initial_layout(profile, &descriptors(), DockMetrics::default()).unwrap()
}

mod commands;
mod floating;
mod model_validation;
mod moves;
mod normalization;
mod resize;
