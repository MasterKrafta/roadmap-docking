use slotmap::SlotMap;

use crate::{
    CanvasState, ChromeMode, DockArea, DockAreaId, DockSide, FloatingGroup, FloatingGroupId,
    FlyoutState, Node, NodeId, PanelId, PanelInstance, Profile,
};

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// persistent edits will be performed through DockCommand once the command
/// layer is introduced. Flyout state is transient and kept separately from the
/// persistent tree.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
    flyout: Option<FlyoutState>,
}

impl Layout {
    /// Creates an empty layout with an explicit profile and a mandatory canvas.
    pub fn new(profile: Profile) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panel_instances: SlotMap::with_key(),
            dock_areas: SlotMap::with_key(),
            main_areas: [None, None],
            floating_groups: SlotMap::with_key(),
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
            flyout: None,
        }
    }

    pub const fn profile(&self) -> Profile {
        self.profile
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    pub const fn chrome(&self) -> ChromeMode {
        self.chrome
    }

    pub const fn canvas(&self) -> &CanvasState {
        &self.canvas
    }

    pub const fn flyout(&self) -> Option<FlyoutState> {
        self.flyout
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance> {
        self.panel_instances.get(id)
    }

    pub fn dock_area(&self, id: DockAreaId) -> Option<&DockArea> {
        self.dock_areas.get(id)
    }

    pub fn floating_group(&self, id: FloatingGroupId) -> Option<&FloatingGroup> {
        self.floating_groups.get(id)
    }

    /// Returns the main area for a side, if one exists.
    pub fn main_area(&self, side: DockSide) -> Option<&DockArea> {
        let index = match side {
            DockSide::Left => 0,
            DockSide::Right => 1,
        };
        self.main_areas[index].and_then(|id| self.dock_areas.get(id))
    }

    /// Read-only arena views for validation, rendering adapters, and inspection.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }

    pub fn panel_instances(&self) -> impl Iterator<Item = (PanelId, &PanelInstance)> {
        self.panel_instances.iter()
    }

    pub fn dock_areas(&self) -> impl Iterator<Item = (DockAreaId, &DockArea)> {
        self.dock_areas.iter()
    }

    pub fn floating_groups(&self) -> impl Iterator<Item = (FloatingGroupId, &FloatingGroup)> {
        self.floating_groups.iter()
    }

    /// Internal constructor used by pure core factories and loading/migration.
    ///
    /// It builds a complete value rather than exposing mutable arena access to
    /// consumers. Public persistent mutation remains command-driven.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: SlotMap<NodeId, Node>,
        panel_instances: SlotMap<PanelId, PanelInstance>,
        dock_areas: SlotMap<DockAreaId, DockArea>,
        main_areas: [Option<DockAreaId>; 2],
        floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
        canvas: CanvasState,
        profile: Profile,
        version: u64,
        chrome: ChromeMode,
    ) -> Self {
        Self {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            canvas,
            profile,
            version,
            chrome,
            flyout: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{CanvasState, ChromeMode, DockArea, DockAreaId, DockSide, FloatingGroup, FloatingGroupId, Node, NodeId, PanelId, PanelInstance, Profile};
    use slotmap::SlotMap;

    #[test]
    fn new_layout_owns_empty_arenas_and_an_independent_canvas() {
        let layout = Layout::new(Profile::Generic);

        assert_eq!(layout.profile(), Profile::Generic);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.chrome(), ChromeMode::Visible);
        assert_eq!(layout.flyout(), None);
        assert_eq!(layout.canvas(), &CanvasState::default());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.panel_instances().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert_eq!(layout.floating_groups().count(), 0);
        assert_eq!(layout.main_area(DockSide::Left), None);
        assert_eq!(layout.main_area(DockSide::Right), None);
    }

    #[test]
    fn clone_preserves_runtime_handles_and_read_only_views() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let _root = nodes.insert(Node::Split(crate::Split {
            axis: crate::Axis::Y,
            children: Vec::new(),
            ratios: Vec::new(),
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let root = nodes.iter().next().unwrap().0;
        let area = areas.insert(DockArea::main(DockSide::Left, root));
        let floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [Some(area), None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            7,
            ChromeMode::Hidden,
        );
        let cloned = layout.clone();

        assert_eq!(cloned.main_area(DockSide::Left).unwrap().root, root);
        assert_eq!(cloned.version(), 7);
        assert_eq!(cloned.chrome(), ChromeMode::Hidden);
    }
}
