use super::helpers::*;
use super::*;

impl Layout {
    pub(super) fn validate_dock_areas(
        &self,
        metrics: &DockMetrics,
        metrics_are_valid: bool,
        errors: &mut Vec<ValidationError>,
    ) -> Vec<NodeId> {
        // Validate area records and discover the only legal entry points into
        // the tree: registered main sides and floating groups with a floating area.
        let mut registered_areas = HashSet::new();
        let mut roots = Vec::new();
        let mut photoshop_main_columns = 0usize;

        for (area_id, area) in self.dock_areas.iter() {
            if let Err(reason) = area.validate_metadata() {
                errors.push(ValidationError::InvalidDockArea {
                    area: area_id,
                    reason,
                });
            }
        }

        for (side, index) in [(DockSide::Left, 0usize), (DockSide::Right, 1usize)] {
            let Some(area_id) = self.main_areas[index] else {
                continue;
            };
            if !registered_areas.insert(area_id) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "the same DockAreaId is assigned to more than one main side",
                });
            }

            let Some(area) = self.dock_areas.get(area_id) else {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };

            if area.kind != DockAreaKind::Main {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "main side must reference a Main DockArea",
                });
            }
            if area.side != Some(side) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "DockArea side does not match its main-area slot",
                });
            }

            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop
                    && area.kind == DockAreaKind::Main
                    && area.side == Some(side)
                {
                    photoshop_main_columns +=
                        validate_photoshop_main_root(&self.nodes, area_id, area.root, &mut errors);
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        let mut groups_by_area: HashMap<DockAreaId, Vec<FloatingGroupId>> = HashMap::new();
        let mut groups_by_z: HashMap<i32, FloatingGroupId> = HashMap::new();

        for (group_id, group) in self.floating_groups.iter() {
            if let Err(reason) = group.validate() {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: match reason {
                        GeometryError::NonFinite => "bounds contain a non-finite value",
                        GeometryError::NegativeDimension => {
                            "bounds dimensions must not be negative"
                        }
                        GeometryError::NonPositiveDimension => {
                            "bounds must have positive dimensions"
                        }
                        GeometryError::ArithmeticOverflow => "bounds edges overflow",
                        GeometryError::InconsistentConstraints => "invalid bounds",
                    },
                });
            }

            if let Some(first_id) = groups_by_z.insert(group.z_order, group_id) {
                errors.push(ValidationError::DuplicateFloatingZOrder {
                    first: first_id,
                    second: group_id,
                    z_order: group.z_order,
                });
            }

            let Some(area) = self.dock_areas.get(group.area) else {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };
            if area.kind != DockAreaKind::Floating {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "must reference a Floating DockArea",
                });
                continue;
            }

            registered_areas.insert(group.area);
            groups_by_area.entry(group.area).or_default().push(group_id);
            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop {
                    validate_photoshop_floating_root(&self.nodes, group_id, area.root, &mut errors);
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        for (area_id, group_ids) in &groups_by_area {
            if group_ids.len() > 1 {
                errors.push(ValidationError::FloatingAreaReferencedMoreThanOnce {
                    area: *area_id,
                    first: group_ids[0],
                    second: group_ids[1],
                });
            }
        }

        for (area_id, _) in self.dock_areas.iter() {
            if !registered_areas.contains(&area_id) {
                errors.push(ValidationError::UnreferencedDockArea { area: area_id });
            }
        }

        if metrics_are_valid
            && self.profile == Profile::Photoshop
            && photoshop_main_columns > metrics.max_columns
        {
            errors.push(ValidationError::TooManyColumns {
                actual: photoshop_main_columns,
                max: metrics.max_columns,
            });
        }
        roots
    }
}
