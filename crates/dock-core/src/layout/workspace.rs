//! Workspace loading, export, and floating-bound adjustment.

use super::*;
use crate::{WorkspaceFile, WorkspaceFileError};

impl Layout {
#[cfg(feature = "serde")]
    pub(crate) fn from_workspace_parts(parts: WorkspaceLayoutParts) -> Self {
        let WorkspaceLayoutParts {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            canvas,
            profile,
            chrome,
        } = parts;

        Self {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            last_dock_locations: HashMap::new(),
            last_dock_location_order: VecDeque::new(),
            canvas,
            profile,
            version: 0,
            chrome,
            flyout: None,
            flyout_auto_collapse: true,
            history_identity: Arc::new(()),
            flyout_widths: HashMap::new(),
            flyout_width_order: VecDeque::new(),
            validation_cache: ValidationCache::default(),
        }
    }

    #[cfg(feature = "serde")]
    pub(crate) fn normalize_for_workspace_load(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<Option<Vec<String>>, Vec<ValidationError>> {
        let original_issues = self.validate(metrics).err().unwrap_or_default();
        let mut candidate = self.clone();
        let mut repair_notes = Vec::new();

        // A canvas measurement saved by an older release may be valid geometry
        // but too narrow for newer metrics. Drop the stale measurement so the host
        // can supply the current viewport instead of inventing a wider canvas.
        if let Some(bounds) = candidate.canvas.last_known_bounds()
            && bounds.validate_non_empty().is_ok()
            && bounds.size.width + PIXEL_TOLERANCE < metrics.min_canvas_width
        {
            repair_notes.push(format!(
                "canvas last_known_bounds cleared because stored width {} is below current min_canvas_width {}",
                bounds.size.width, metrics.min_canvas_width
            ));
            candidate.canvas = CanvasState::default();
        }

        repair_notes.extend(candidate.repair_duplicate_singleton_panels());
        repair_notes.extend(candidate.repair_excess_photoshop_columns(metrics.max_columns));
        repair_notes.extend(candidate.repair_duplicate_floating_z_orders());

        candidate.repair_workspace_column_widths(metrics, &mut repair_notes)?;

        let mut diagnostic_issues: Vec<String> =
            original_issues.iter().map(ToString::to_string).collect();
        diagnostic_issues.extend(repair_notes);

        if original_issues.is_empty() && diagnostic_issues.is_empty() {
            return Ok(None);
        }
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(Some(diagnostic_issues))
    }

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

/// Creates a validated workspace DTO suitable for the public JSON boundary.
    pub fn to_workspace_file(
        &self,
        metrics: &DockMetrics,
    ) -> Result<WorkspaceFile, WorkspaceFileError> {
        WorkspaceFile::from_layout(self, metrics)
    }
}
