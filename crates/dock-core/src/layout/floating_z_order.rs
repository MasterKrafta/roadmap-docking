use super::*;

impl Layout {
    pub(super) fn next_floating_z_order(&self) -> Result<i32, CommandError> {
        match self
            .floating_groups
            .values()
            .map(|group| group.z_order)
            .max()
        {
            Some(value) => value.checked_add(1).ok_or(CommandError::InvalidValue {
                field: "z_order",
                reason: "floating z-order overflow",
            }),
            None => Ok(0),
        }
    }

    pub(super) fn apply_raise_floating_command(
        &mut self,
        group: FloatingGroupId,
    ) -> Result<CommandApplication, CommandError> {
        let current = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let max_other = self
            .floating_groups
            .iter()
            .filter(|(other_id, _)| *other_id != group)
            .map(|(_, value)| value.z_order)
            .max();
        if max_other.is_none_or(|max| current.z_order > max) {
            return Ok(CommandApplication::persistent());
        }
        let next_z = self.next_floating_z_order()?;
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.z_order = next_z;
        }
        Ok(CommandApplication::persistent())
    }

    #[cfg(feature = "serde")]
    pub(super) fn repair_duplicate_floating_z_orders(&mut self) -> Vec<String> {
        let mut seen = HashSet::new();
        let has_duplicates = self
            .floating_groups
            .iter()
            .any(|(_, group)| !seen.insert(group.z_order));
        if !has_duplicates {
            return Vec::new();
        }

        let mut ordered: Vec<_> = self
            .floating_groups
            .iter()
            .map(|(group_id, group)| (group_id, group.z_order))
            .collect();
        ordered.sort_by_key(|(_, z_order)| *z_order);

        // A valid workspace cannot practically contain more than i32::MAX
        // floating groups. Keep the old invalid values if it somehow does.
        if ordered.len() > i32::MAX as usize {
            return Vec::new();
        }

        let mut notes = Vec::new();
        for (index, (group_id, old_z_order)) in ordered.into_iter().enumerate() {
            let new_z_order = index as i32;
            if old_z_order == new_z_order {
                continue;
            }
            if let Some(group) = self.floating_groups.get_mut(group_id) {
                group.z_order = new_z_order;
                notes.push(format!(
                    "floating group {group_id:?}: z_order changed from {old_z_order} to {new_z_order} to remove duplicate stacking order"
                ));
            }
        }
        notes
    }

    /// Returns floating windows in paint order: lowest z_order first (back-to-front).
    ///
    /// Render adapters should draw windows in this order. Valid layouts require
    /// unique z_order values, so stacking order is unambiguous.
    pub fn floating_groups_in_paint_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by_key(|(_, group)| group.z_order);
        groups
    }

    /// Returns floating windows in hit-test order: highest z_order first.
    ///
    /// Input adapters should test the first returned window before the rest.
    pub fn floating_groups_in_hit_test_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by_key(|(_, group)| std::cmp::Reverse(group.z_order));
        groups
    }
}
