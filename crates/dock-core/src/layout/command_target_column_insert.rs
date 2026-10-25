use super::*;
use super::helpers::{equal_ratios, side_index};

impl Layout {
    pub(super) fn insert_existing_column_at(
        &mut self,
        side: DockSide,
        index: usize,
        new_column: NodeId,
        split_ratio: Option<(f32, usize)>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if !matches!(self.nodes.get(new_column), Some(Node::Column(_))) {
            return Err(CommandError::InvalidTarget {
                reason: "inserted main column is not a Column node",
            });
        }
        let split_ratio = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue {
                    field: "ratio",
                    reason: "split ratio must be between zero and one",
                });
            }
            Some((ratio, target_index))
        } else {
            None
        };

        let Some(area_id) = self.main_areas[side_index(side)] else {
            if index != 0 {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "first column on an empty dock must use index zero",
                });
            }
            let area_id = self.dock_areas.insert(DockArea::main(side, new_column));
            self.main_areas[side_index(side)] = Some(area_id);
            return Ok(());
        };
        let area_root = self
            .dock_areas
            .get(area_id)
            .ok_or(CommandError::InvalidTarget {
                reason: "main dock side references a missing area",
            })?
            .root;

        match self.nodes.get(area_root).cloned() {
            Some(Node::Column(_)) => {
                if index > 1 {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index != 0 {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target does not identify the existing column",
                        });
                    }
                    self.resize_inserted_column_for_split_ratio(
                        area_root,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                let children = if index == 0 {
                    vec![new_column, area_root]
                } else {
                    vec![area_root, new_column]
                };
                let split = self.nodes.insert(Node::Split(crate::Split {
                    axis: Axis::X,
                    ratios: equal_ratios(2),
                    children,
                }));
                if let Some(area) = self.dock_areas.get_mut(area_id) {
                    area.root = split;
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(Node::Split(mut split)) if split.axis == Axis::X => {
                if index > split.children.len() {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split contains a non-Column child",
                    });
                }
                let old_len = split.children.len();
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index >= old_len {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target or column insertion index is outside the dock",
                        });
                    }
                    let target_column = split.children[target_index];
                    self.resize_inserted_column_for_split_ratio(
                        target_column,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                split.children.insert(index, new_column);
                // Main-column Split(X) ratios mirror visible column widths. They
                // are recalculated after the tree is updated below.
                split.ratios = equal_ratios(old_len + 1);
                if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
                    *current = split;
                } else {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split changed unexpectedly",
                    });
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(_) => Err(CommandError::InvalidTarget {
                reason: "main dock root must be a Column or Split(X) of Columns",
            }),
            None => Err(CommandError::InvalidTarget {
                reason: "main dock root is missing",
            }),
        }
    }
}
