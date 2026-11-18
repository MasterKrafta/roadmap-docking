use std::collections::HashMap;

use dock_core::PanelId;

/// Runtime views for panel instances that are currently present in Layout.
///
/// Unlike PanelRegistry, this cache is keyed by the runtime PanelId rather than
/// the stable panel kind. Its value is kept while an instance is moved, floated,
/// merged, split, or made inactive; it is pruned when that PanelId closes and
/// cleared when the entire workspace is replaced.
pub(crate) struct PanelViewCache<T> {
    views: HashMap<PanelId, T>,
}

impl<T> Default for PanelViewCache<T> {
    fn default() -> Self {
        Self {
            views: HashMap::new(),
        }
    }
}

impl<T> PanelViewCache<T> {
    pub(crate) fn get(&self, panel: PanelId) -> Option<&T> {
        self.views.get(&panel)
    }

    /// Returns the existing view for panel, creating it only on the first request.
    ///
    /// Unlike HashMap::insert, this API cannot silently replace an Entity/AnyView
    /// that is already associated with the runtime PanelId.
    pub(crate) fn get_or_insert_with(
        &mut self,
        panel: PanelId,
        create: impl FnOnce() -> T,
    ) -> &T {
        self.views.entry(panel).or_insert_with(create)
    }

    pub(crate) fn retain(&mut self, mut is_open: impl FnMut(PanelId) -> bool) {
        self.views.retain(|panel, _| is_open(*panel));
    }

    pub(crate) fn clear(&mut self) {
        self.views.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.views.len()
    }
}
