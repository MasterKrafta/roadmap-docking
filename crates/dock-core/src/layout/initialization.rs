use super::*;

impl Layout {
    /// Creates an empty layout with an explicit profile and a mandatory canvas.
    pub fn new(profile: Profile) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panel_instances: SlotMap::with_key(),
            dock_areas: SlotMap::with_key(),
            main_areas: [None, None],
            floating_groups: SlotMap::with_key(),
            last_dock_locations: HashMap::new(),
            last_dock_location_order: VecDeque::new(),
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
            flyout: None,
            flyout_auto_collapse: true,
            history_identity: Arc::new(()),
            flyout_widths: HashMap::new(),
            flyout_width_order: VecDeque::new(),
            validation_cache: ValidationCache::default(),
        }
    }
}
