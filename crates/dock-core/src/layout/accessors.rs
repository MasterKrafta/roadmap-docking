use super::*;

impl Layout {
    pub(crate) fn flyout_width(&self, key: &PanelKey) -> Option<f32> {
        self.flyout_widths.get(key).copied()
    }

    /// Validates once per persistent revision and metric set for render/drag hot paths.
    /// Public `validate` always performs the full check for callers asking for diagnostics.
    pub(crate) fn validate_cached(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        if self
            .validation_cache
            .get()
            .is_some_and(|(version, cached_metrics)| version == self.version && cached_metrics == *metrics)
        {
            return Ok(());
        }
        self.validate(metrics)?;
        self.validation_cache.set(Some((self.version, *metrics)));
        Ok(())
    }

    /// Records host-measured canvas bounds without creating a persistent command revision.
    /// Narrow windows are valid measurements and must not block layout commands.
    pub fn update_canvas_bounds(&mut self, bounds: Option<Rect>) -> Result<bool, GeometryError> {
        if let Some(bounds) = bounds {
            bounds.validate_non_empty()?;
        }
        if self.canvas.last_known_bounds() == bounds {
            return Ok(false);
        }
        self.canvas = CanvasState::new(bounds);
        Ok(true)
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

    /// Whether passive dismissal and clicking the selected rail icon close a flyout.
    pub const fn flyout_auto_collapse_enabled(&self) -> bool {
        self.flyout_auto_collapse
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance> {
        self.panel_instances.get(id)
    }

    /// Returns the last known main-dock placement for a panel key in this
    /// runtime session. This preference is intentionally not workspace state.
    pub fn last_dock_location(&self, key: &PanelKey) -> Option<LastDockLocation> {
        self.last_dock_locations.get(key).copied()
    }

    pub fn dock_area(&self, id: DockAreaId) -> Option<&DockArea> {
        self.dock_areas.get(id)
    }

    pub fn floating_group(&self, id: FloatingGroupId) -> Option<&FloatingGroup> {
        self.floating_groups.get(id)
    }

    /// Returns the main area for a side, if one exists.
    pub fn main_area(&self, side: DockSide) -> Option<&DockArea> {
        let index = helpers::side_index(side);
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
}
