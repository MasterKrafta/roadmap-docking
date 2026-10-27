use super::*;
use crate::{FloatingGroupRect, GroupRect};

#[path = "drop_zone_floating/edge_zones.rs"]
mod edge_zones;

struct FloatingZoneContext<'a> {
    layout: &'a Layout,
    source: DropSource,
    source_root: NodeId,
    source_panels: &'a [PanelId],
    tree_index: &'a DropZoneTreeIndex,
    rects: &'a LayoutRects,
    metrics: &'a DockMetrics,
    zones: &'a mut Vec<DropZoneRect>,
}

impl Layout {
    pub(in crate::compute_drop_zones) fn append_floating_window_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        FloatingZoneContext {
            layout: self,
            source: context.source,
            source_root: context.source_root,
            source_panels: context.source_panels,
            tree_index: context.tree_index,
            rects: context.rects,
            metrics: context.metrics,
            zones,
        }
        .append_all()
    }
}

impl FloatingZoneContext<'_> {
    fn append_all(&mut self) -> Result<(), ComputeRectsError> {
        // Floating-window drop zones are stacked over their contents. Their outer
        // top/bottom targets address the first/last group in a vertical stack.
        for index in (0..self.rects.floating_groups.len()).rev() {
            let floating_rect = self.rects.floating_groups[index];
            self.append_floating_group(floating_rect)?;
        }
        Ok(())
    }

    fn append_floating_group(
        &mut self,
        floating_rect: FloatingGroupRect,
    ) -> Result<(), ComputeRectsError> {
        let Some(floating) = self.layout.floating_group(floating_rect.group) else {
            return Ok(());
        };
        let Some(area) = self.layout.dock_area(floating.area) else {
            return Ok(());
        };
        if !source_allows(self.layout, self.source_panels, AllowedArea::FLOATING) {
            return Ok(());
        }

        let mut target_groups: Vec<GroupRect> = self
            .rects
            .groups
            .iter()
            .copied()
            .filter(|group| contains_node(self.tree_index, area.root, group.group))
            .collect();
        target_groups.sort_by(|left, right| {
            left.rect
                .origin
                .y
                .partial_cmp(&right.rect.origin.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if target_groups.is_empty() {
            return Ok(());
        }

        self.append_center_zones(&target_groups)?;
        self.append_edge_zones(floating_rect, area.root, &target_groups)?;
        if self.layout.profile() == Profile::Generic {
            self.append_side_zones(&target_groups)?;
        }
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    pub(super) fn append_center_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // Multiple groups can share a vertical position in a horizontal floating
        // composition, so center zones are emitted per group.
        for target_group in target_groups {
            let tabs = match self.layout.node(target_group.group) {
                Some(Node::Tabs(tabs)) if !tabs.collapsed => tabs,
                _ => continue,
            };
            let is_self = matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group);
            let panel_already_in_group = match self.source {
                DropSource::Panel { panel } => {
                    panel_node(self.tree_index, panel)
                        .and_then(|node_id| direct_parent(self.tree_index, node_id))
                        == Some(target_group.group)
                }
                DropSource::Group { .. } => false,
            };
            if is_self || panel_already_in_group {
                continue;
            }

            let target = match self.source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
            };
            push_zone(
                self.zones,
                target,
                DropZone::FloatingCenter,
                group_center_rect(target_group.rect, self.metrics)?,
            )?;
        }
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    pub(super) fn append_side_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // The Generic profile supports horizontal nested splits within a
        // floating area; Photoshop uses its column-edge policy instead.
        for target_group in target_groups {
            if matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Left, DropZone::Left),
                (DropZone::Right, DropZone::Right),
            ] {
                if group_edge_is_noop(
                    self.layout,
                    self.tree_index,
                    self.source,
                    target_group.group,
                    edge,
                ) {
                    continue;
                }
                push_zone(
                    self.zones,
                    DropTarget::GroupEdge {
                        group: target_group.group,
                        zone: edge,
                    },
                    kind,
                    group_edge_rect(target_group.rect, edge, self.metrics.edge_drop_width)?,
                )?;
            }
        }
        Ok(())
    }
}
