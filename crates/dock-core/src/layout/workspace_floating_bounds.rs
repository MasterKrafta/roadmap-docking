use super::*;

impl Layout {
    #[cfg(feature = "serde")]
    pub(crate) fn clamp_floating_groups_for_workspace_load(
        &mut self,
        client_area: Rect,
        metrics: &DockMetrics,
        original_indices: &HashMap<FloatingGroupId, usize>,
    ) -> FloatingBoundsClampResult {
        let mut groups: Vec<_> = self
            .floating_groups
            .iter()
            .enumerate()
            .map(|(fallback_index, (id, group))| {
                (
                    original_indices.get(&id).copied().unwrap_or(fallback_index),
                    id,
                    group.z_order,
                )
            })
            .collect();
        groups.sort_by_key(|(original_index, _, z_order)| (*z_order, *original_index));

        let mut changes = Vec::new();
        for (index, id, _) in groups {
            let Some(group) = self.floating_groups.get(id).copied() else {
                return Err((index, crate::ComputeRectsError::InvalidLayoutState));
            };
            let Some(root) = self.dock_areas.get(group.area).map(|area| area.root) else {
                return Err((index, crate::ComputeRectsError::InvalidLayoutState));
            };
            let original = group.bounds;
            let clamped = crate::compute_rects::clamp_floating_bounds(
                self,
                root,
                original,
                client_area,
                metrics,
            )
            .map_err(|error| (index, error))?;

            if clamped != original {
                let Some(group) = self.floating_groups.get_mut(id) else {
                    return Err((index, crate::ComputeRectsError::InvalidLayoutState));
                };
                group.bounds = clamped;
                changes.push((index, original, clamped));
            }
        }
        Ok(changes)
    }

}
