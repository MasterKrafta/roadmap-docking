use super::*;

impl Layout {
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
        let max_z = self
            .floating_groups
            .iter()
            .map(|(_, value)| value.z_order)
            .max()
            .unwrap_or(-1);
        let next_z = max_z.checked_add(1).ok_or(CommandError::InvalidValue {
            field: "z_order",
            reason: "floating z-order overflow",
        })?;
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.z_order = next_z;
        }
        Ok(CommandApplication::persistent())
    }
}
