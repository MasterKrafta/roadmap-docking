use std::collections::{HashMap, HashSet};

use dock_core::{FloatingGroupId, PanelId};
use platform::NativeWindowId;

#[derive(Clone)]
pub(super) struct DragHostHandoff {
    pub(super) primary_target_group: FloatingGroupId,
    pub(super) required_panels_by_group: HashMap<FloatingGroupId, Vec<PanelId>>,
    pub(super) rendered_panels: HashSet<PanelId>,
    pub(super) foreground_at_release: Option<NativeWindowId>,
    pub(super) source_placeholder_rendered: bool,
    pub(super) source_released: bool,
}
