use super::helpers::is_normalizable_validation_error;
use super::*;

impl Layout {
    /// Test-only wrapper for the C3 normalization contract. Production mutations
    /// call normalize_in_place on a private candidate inside Layout::apply.
    #[cfg(test)]
    pub(super) fn normalize(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut candidate = self.clone();
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(())
    }

    /// Crate-visible entry point for tests in sibling modules. This does not
    /// expose normalization in production builds or to downstream UI code.
    #[cfg(test)]
    pub(crate) fn normalize_for_test(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        self.normalize(metrics)
    }

    pub(super) fn normalize_in_place(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        let existing_errors = self.validate(metrics).err().unwrap_or_default();
        let blockers: Vec<_> = existing_errors
            .into_iter()
            .filter(|error| !is_normalizable_validation_error(error))
            .collect();
        if !blockers.is_empty() {
            return Err(blockers);
        }

        self.normalize_main_areas()?;

        self.normalize_floating_areas()?;

        self.close_invalid_flyout();

        self.validate(metrics)
    }
}
