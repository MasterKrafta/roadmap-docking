use slotmap::SlotMap;

use crate::ids::{FloatingGroupId, NodeId, PanelId};

use super::{CanvasState, DockArea, FloatingGroup, Node, PanelInstance, Profile};

/// Complete dock layout model, independent of GPUI, windows, and panel content.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panels: SlotMap<PanelId, PanelInstance>,
    dock_areas: Vec<DockArea>,
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
}

impl Layout {
    pub fn new(profile: Profile, canvas: CanvasState) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panels: SlotMap::with_key(),
            dock_areas: Vec::new(),
            floating_groups: SlotMap::with_key(),
            canvas,
            profile,
            version: 0,
        }
    }

    pub fn nodes(&self) -> &SlotMap<NodeId, Node> { &self.nodes }
    pub fn panels(&self) -> &SlotMap<PanelId, PanelInstance> { &self.panels }
    pub fn dock_areas(&self) -> &[DockArea] { &self.dock_areas }
    pub fn floating_groups(&self) -> &SlotMap<FloatingGroupId, FloatingGroup> { &self.floating_groups }
    pub const fn canvas(&self) -> &CanvasState { &self.canvas }
    pub const fn profile(&self) -> Profile { self.profile }
    pub const fn version(&self) -> u64 { self.version }

    // Crate-internal construction/mutation hooks. Public callers can only read the model;
    // user-visible mutation will be routed through Layout::apply(DockCommand) in stage 3.
    pub(crate) fn insert_node(&mut self, node: Node) -> NodeId { self.nodes.insert(node) }
    pub(crate) fn insert_panel(&mut self, panel: PanelInstance) -> PanelId { self.panels.insert(panel) }
    pub(crate) fn push_dock_area(&mut self, area: DockArea) { self.dock_areas.push(area); }
    pub(crate) fn insert_floating_group(&mut self, group: FloatingGroup) -> FloatingGroupId {
        self.floating_groups.insert(group)
    }
    pub(crate) fn set_canvas(&mut self, canvas: CanvasState) { self.canvas = canvas; }
    pub(crate) fn panel_mut(&mut self, id: PanelId) -> Option<&mut PanelInstance> { self.panels.get_mut(id) }

    pub(crate) fn advance_version(&mut self) -> Result<u64, &'static str> {
        let next = self.version.checked_add(1).ok_or("layout version exhausted")?;
        self.version = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{CanvasState, Point, Profile, Rect, Size};

    fn empty_layout() -> Layout {
        Layout::new(Profile::Photoshop, CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 800.0)),
        })
    }

    #[test]
    fn starts_with_empty_arenas_and_version_zero() {
        let layout = empty_layout();
        assert!(layout.nodes().is_empty());
        assert!(layout.panels().is_empty());
        assert!(layout.dock_areas().is_empty());
        assert!(layout.floating_groups().is_empty());
        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.version(), 0);
    }

    #[test]
    fn version_advances_only_when_a_change_is_recorded() {
        let mut layout = empty_layout();
        assert_eq!(layout.advance_version(), Ok(1));
        assert_eq!(layout.advance_version(), Ok(2));
        assert_eq!(layout.version(), 2);
    }

    #[test]
    fn version_overflow_is_reported_without_wrapping() {
        let mut layout = empty_layout();
        layout.version = u64::MAX;
        assert!(layout.advance_version().is_err());
        assert_eq!(layout.version(), u64::MAX);
    }
}
