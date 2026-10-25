use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Weak};

use slotmap::SlotMap;

use crate::{
    Axis, CanvasState, ChromeMode, Column, ColumnMode, CommandError, CommandOutcome, CommandValue,
    DockArea, DockAreaId, DockAreaKind, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
    FloatingGroup, FloatingGroupId, FlyoutState, GeometryError, LastDockLocation, Node, NodeId,
    Panel, PanelId, PanelInstance, PanelKey, Point, Profile, Rect, Size, Split, Tabs,
    ValidationError, command_application::CommandApplication,
};

const RATIO_TOLERANCE: f32 = 1.0e-4;
const PIXEL_TOLERANCE: f32 = 1.0e-3;
const MAX_REMEMBERED_DOCK_LOCATIONS: usize = 256;

/**
 * Internal validation memoization is not part of the logical Layout state.
 * Keep Debug snapshots stable when read-only operations warm the cache.
 */
#[derive(Clone, Default)]
struct ValidationCache(Cell<Option<(u64, DockMetrics)>>);

impl ValidationCache {
    fn get(&self) -> Option<(u64, DockMetrics)> {
        self.0.get()
    }

    fn set(&self, value: Option<(u64, DockMetrics)>) {
        self.0.set(value);
    }
}

impl fmt::Debug for ValidationCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidationCache")
    }
}


/// Arena parts restored from the persistent workspace DTO.
#[cfg(feature = "serde")]
pub(crate) struct WorkspaceLayoutParts {
    pub(crate) nodes: SlotMap<NodeId, Node>,
    pub(crate) panel_instances: SlotMap<PanelId, PanelInstance>,
    pub(crate) dock_areas: SlotMap<DockAreaId, DockArea>,
    pub(crate) main_areas: [Option<DockAreaId>; 2],
    pub(crate) floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    pub(crate) canvas: CanvasState,
    pub(crate) profile: Profile,
    pub(crate) chrome: ChromeMode,
}

#[cfg(feature = "serde")]
pub(crate) type FloatingBoundsClampResult =
    Result<Vec<(usize, Rect, Rect)>, (usize, crate::ComputeRectsError)>;

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// external edits are performed only through Layout::apply and typed DockCommand
/// values. Clones retain the same history lineage; newly created or loaded layouts do not.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    last_dock_locations: HashMap<PanelKey, LastDockLocation>,
    last_dock_location_order: VecDeque<PanelKey>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
    flyout: Option<FlyoutState>,
    flyout_auto_collapse: bool,
    history_identity: Arc<()>,
    flyout_widths: HashMap<PanelKey, f32>,
    flyout_width_order: VecDeque<PanelKey>,
    validation_cache: ValidationCache,
}

impl Layout {
    /// Creates an empty layout with an explicit profile and a mandatory canvas.
    pub fn new(profile: Profile) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panel_instances: SlotMap::with_key(),
            dock_areas: SlotMap::with_key(),
            main_areas: [None, None],
            floating_groups: SlotMap::with_key(),
            last_dock_locations: HashMap::new(),
            last_dock_location_order: VecDeque::new(),
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
            flyout: None,
            flyout_auto_collapse: true,
            history_identity: Arc::new(()),
            flyout_widths: HashMap::new(),
            flyout_width_order: VecDeque::new(),
            validation_cache: ValidationCache::default(),
        }
    }

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

        // Column presentation mode uniquely determines rail width. Correcting
        // a stale width is deterministic; an invalid expanded width is repaired
        // only when it is finite and positive, since otherwise intent is unclear.
        for (node_id, node) in candidate.nodes.iter_mut() {
            let Node::Column(column) = node else {
                continue;
            };
            match column.mode {
                ColumnMode::IconsOnly if column.width != metrics.rail_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsOnly width reset from {} to {}",
                        column.width, metrics.rail_width
                    ));
                    column.width = metrics.rail_width;
                }
                ColumnMode::IconsAndLabels if column.width != metrics.min_column_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsAndLabels width reset from {} to {}",
                        column.width, metrics.min_column_width
                    ));
                    column.width = metrics.min_column_width;
                }
                ColumnMode::Expanded if column.width.is_finite() && column.width > 0.0 => {
                    if column.width < metrics.min_column_width {
                        repair_notes.push(format!(
                            "column {node_id:?}: expanded width raised from {} to minimum {}",
                            column.width, metrics.min_column_width
                        ));
                        column.width = metrics.min_column_width;
                    }
                    if column
                        .last_expanded_width
                        .is_none_or(|width| !width.is_finite() || width <= 0.0)
                    {
                        repair_notes.push(format!(
                            "column {node_id:?}: missing or invalid last_expanded_width restored from current width {}",
                            column.width
                        ));
                        column.last_expanded_width = Some(column.width);
                    }
                }
                _ => {}
            }
        }

        // Repair finite Expanded widths against the content's complete min/max
        // contract before the normalized candidate is validated or exposed.
        let column_ids = candidate
            .nodes
            .iter()
            .filter_map(|(node_id, node)| matches!(node, Node::Column(_)).then_some(node_id))
            .collect::<Vec<_>>();
        for node_id in column_ids {
            let Some(Node::Column(column)) = candidate.nodes.get(node_id).cloned() else {
                continue;
            };
            if column.mode != ColumnMode::Expanded
                || !column.width.is_finite()
                || column.width <= 0.0
            {
                continue;
            }

            let minimum = candidate
                .minimum_expanded_column_width(node_id, metrics)
                .map_err(|_| {
                    vec![ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "cannot compute minimum width required by expanded contents",
                    }]
                })?;
            let maximum = candidate
                .maximum_expanded_column_width(node_id)
                .map_err(|_| {
                    vec![ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "cannot compute maximum width allowed by expanded contents",
                    }]
                })?;
            if maximum.is_some_and(|maximum| maximum < minimum) {
                return Err(vec![ValidationError::InvalidColumn {
                    node: node_id,
                    reason: "minimum width required by expanded contents exceeds panel maximum width",
                }]);
            }
            let upper = maximum.unwrap_or(f32::MAX);
            let repaired_width = column.width.max(minimum).min(upper);
            if repaired_width != column.width {
                let note = if repaired_width > column.width {
                    format!(
                        "column {node_id:?}: width raised to content minimum from {} to {}",
                        column.width, repaired_width
                    )
                } else {
                    format!(
                        "column {node_id:?}: width reduced to panel maximum from {} to {}",
                        column.width, repaired_width
                    )
                };
                repair_notes.push(note);
            }

            let repaired_last_expanded_width = match column.last_expanded_width {
                Some(width) if width.is_finite() && width > 0.0 => {
                    let repaired = width.max(minimum).min(upper);
                    if repaired != width {
                        repair_notes.push(format!(
                            "column {node_id:?}: last_expanded_width clamped from {width} to {repaired}"
                        ));
                    }
                    Some(repaired)
                }
                _ => {
                    repair_notes.push(format!(
                        "column {node_id:?}: missing or invalid last_expanded_width restored from current width {repaired_width}"
                    ));
                    Some(repaired_width)
                }
            };

            if let Some(Node::Column(value)) = candidate.nodes.get_mut(node_id) {
                value.width = repaired_width;
                value.last_expanded_width = repaired_last_expanded_width;
            }
        }

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
    fn repair_duplicate_singleton_panels(&mut self) -> Vec<String> {
        let mut first_by_key = HashMap::<PanelKey, (PanelId, bool)>::new();
        let mut notes = Vec::new();

        // Traverse the same roots in the same order used while restoring the
        // workspace DTO: left, right, then floating groups in stored order.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                continue;
            };
            if !self.repair_duplicate_singletons_in_node(
                area.root,
                &mut first_by_key,
                &mut notes,
            ) {
                self.main_areas[index] = None;
                self.dock_areas.remove(area_id);
            }
        }

        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let Some(area) = self.dock_areas.get(group.area).copied() else {
                continue;
            };
            if !self.repair_duplicate_singletons_in_node(
                area.root,
                &mut first_by_key,
                &mut notes,
            ) {
                self.floating_groups.remove(group_id);
                self.dock_areas.remove(group.area);
            }
        }

        notes
    }

    #[cfg(feature = "serde")]
    fn repair_duplicate_singletons_in_node(
        &mut self,
        node_id: NodeId,
        first_by_key: &mut HashMap<PanelKey, (PanelId, bool)>,
        notes: &mut Vec<String>,
    ) -> bool {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return true;
        };

        match node {
            Node::Panel(panel) => {
                let Some(instance) = self.panel_instances.get(panel.panel_id).cloned() else {
                    return true;
                };
                let key = instance.key.clone();
                let singleton = instance.constraints.singleton;

                if let Some((first_id, first_singleton)) = first_by_key.get_mut(&key) {
                    if singleton || *first_singleton {
                        let first_id = *first_id;
                        notes.push(format!(
                            "singleton panel {key:?}: duplicate instance {:?} removed; first instance {first_id:?} kept",
                            panel.panel_id
                        ));
                        self.panel_instances.remove(panel.panel_id);
                        self.nodes.remove(node_id);
                        return false;
                    }
                    *first_singleton |= singleton;
                } else {
                    first_by_key.insert(key, (panel.panel_id, singleton));
                }
                true
            }
            Node::Split(mut split) => {
                let original_children = split.children.clone();
                let original_ratios = split.ratios.clone();
                let mut kept = Vec::with_capacity(original_children.len());

                for (index, child) in original_children.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(child, first_by_key, notes) {
                        kept.push((index, child));
                    }
                }

                if kept.len() != original_children.len() {
                    split.children = kept.iter().map(|(_, child)| *child).collect();
                    split.ratios = helpers::normalized_ratios(
                        kept.iter()
                            .map(|(index, _)| original_ratios.get(*index).copied())
                            .collect(),
                    );
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Split(split);
                    }
                }
                true
            }
            Node::Tabs(mut tabs) => {
                let original_children = tabs.children.clone();
                let original_active_index = tabs.active_index;
                let mut kept = Vec::with_capacity(original_children.len());

                for (index, child) in original_children.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(child, first_by_key, notes) {
                        kept.push((index, child));
                    }
                }

                if kept.len() != original_children.len() {
                    tabs.active_index = if kept.is_empty() {
                        0
                    } else {
                        kept.iter()
                            .position(|(index, _)| *index == original_active_index)
                            .unwrap_or_else(|| original_active_index.min(kept.len() - 1))
                    };
                    tabs.children = kept.iter().map(|(_, child)| *child).collect();
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Tabs(tabs);
                    }
                }
                true
            }
            Node::Column(mut column) => {
                let original_groups = column.groups.clone();
                let original_ratios = column.group_ratios.clone();
                let mut kept = Vec::with_capacity(original_groups.len());

                for (index, group) in original_groups.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(group, first_by_key, notes) {
                        kept.push((index, group));
                    }
                }

                if kept.len() != original_groups.len() {
                    column.groups = kept.iter().map(|(_, group)| *group).collect();
                    column.group_ratios = helpers::normalized_ratios(
                        kept.iter()
                            .map(|(index, _)| original_ratios.get(*index).copied())
                            .collect(),
                    );
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Column(column);
                    }
                }
                true
            }
        }
    }

    #[cfg(feature = "serde")]
    fn repair_excess_photoshop_columns(&mut self, max_columns: usize) -> Vec<String> {
        if self.profile != Profile::Photoshop || max_columns == 0 {
            return Vec::new();
        }

        let mut columns_by_side = [Vec::<NodeId>::new(), Vec::<NodeId>::new()];
        for side_index in 0..2 {
            let Some(area_id) = self.main_areas[side_index] else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                return Vec::new();
            };

            let columns = match self.nodes.get(area.root) {
                Some(Node::Column(_)) => vec![area.root],
                Some(Node::Split(split))
                    if split.axis == Axis::X
                        && split
                            .children
                            .iter()
                            .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
                {
                    split.children.clone()
                }
                _ => return Vec::new(),
            };

            // Completely empty columns will be removed by normal normalization.
            // Do not spend the available column budget on data that has no panel.
            columns_by_side[side_index] = columns
                .into_iter()
                .filter(|column| self.subtree_contains_panel(*column))
                .collect();
        }

        let total_columns: usize = columns_by_side.iter().map(Vec::len).sum();
        if total_columns <= max_columns {
            return Vec::new();
        }

        let active_sides = columns_by_side
            .iter()
            .filter(|columns| !columns.is_empty())
            .count();
        let mut retained = HashSet::<NodeId>::new();

        // Preserve a column on each populated side whenever the new limit allows
        // it, then use any remaining slots in stable left-to-right order.
        if max_columns >= active_sides {
            for columns in &columns_by_side {
                if let Some(first) = columns.first() {
                    retained.insert(*first);
                }
            }
            let mut remaining = max_columns - active_sides;
            for columns in &columns_by_side {
                for column in columns.iter().skip(1) {
                    if remaining == 0 {
                        break;
                    }
                    retained.insert(*column);
                    remaining -= 1;
                }
                if remaining == 0 {
                    break;
                }
            }
        } else {
            for columns in &columns_by_side {
                for column in columns {
                    if retained.len() >= max_columns {
                        break;
                    }
                    retained.insert(*column);
                }
                if retained.len() >= max_columns {
                    break;
                }
            }
        }

        let ordered_columns: Vec<_> = columns_by_side
            .iter()
            .flat_map(|columns| columns.iter().copied())
            .collect();
        let ordered_retained: Vec<_> = ordered_columns
            .iter()
            .copied()
            .filter(|column| retained.contains(column))
            .collect();
        let Some(fallback_target) = ordered_retained.last().copied() else {
            return Vec::new();
        };

        let mut merges: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
        for columns in &columns_by_side {
            let side_retained: Vec<_> = columns
                .iter()
                .copied()
                .filter(|column| retained.contains(column))
                .collect();
            let target = side_retained.last().copied().unwrap_or(fallback_target);

            for source in columns
                .iter()
                .copied()
                .filter(|column| !retained.contains(column))
            {
                if let Some((_, sources)) =
                    merges.iter_mut().find(|(existing_target, _)| *existing_target == target)
                {
                    sources.push(source);
                } else {
                    merges.push((target, vec![source]));
                }
            }
        }

        let mut notes = Vec::new();
        let mut removed_columns = HashSet::<NodeId>::new();
        for (target, sources) in &merges {
            // Each source column contributes one equal-height block. Its own
            // group ratios are retained within that block, so no panel is lost.
            let members: Vec<_> = std::iter::once(*target)
                .chain(sources.iter().copied())
                .collect();
            let block_scale = 1.0 / members.len() as f32;
            let mut merged_groups = Vec::new();
            let mut merged_ratios = Vec::new();

            for column_id in &members {
                let Some(Node::Column(column)) = self.nodes.get(*column_id).cloned() else {
                    continue;
                };
                let ratios = helpers::normalized_ratios(
                    column
                        .groups
                        .iter()
                        .enumerate()
                        .map(|(index, _)| column.group_ratios.get(index).copied())
                        .collect(),
                );
                for (group, ratio) in column.groups.into_iter().zip(ratios) {
                    merged_groups.push(group);
                    merged_ratios.push(Some(ratio * block_scale));
                }
            }

            if let Some(Node::Column(column)) = self.nodes.get_mut(*target) {
                column.groups = merged_groups;
                column.group_ratios = helpers::normalized_ratios(merged_ratios);
            }

            for source in sources {
                removed_columns.insert(*source);
                notes.push(format!(
                    "Photoshop main layout: column {source:?} merged into column {target:?} because max_columns decreased to {max_columns}"
                ));
            }
        }

        if removed_columns.is_empty() {
            return notes;
        }

        // Remove the old column wrappers from their main roots after their groups
        // have been adopted by the retained columns.
        for side_index in 0..2 {
            let Some(area_id) = self.main_areas[side_index] else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                continue;
            };

            match self.nodes.get(area.root).cloned() {
                Some(Node::Column(_)) if removed_columns.contains(&area.root) => {
                    self.nodes.remove(area.root);
                    self.dock_areas.remove(area_id);
                    self.main_areas[side_index] = None;
                }
                Some(Node::Split(mut split)) if split.axis == Axis::X => {
                    let original_children = split.children.clone();
                    let original_ratios = split.ratios.clone();
                    let kept: Vec<_> = original_children
                        .iter()
                        .copied()
                        .enumerate()
                        .filter(|(_, child)| !removed_columns.contains(child))
                        .collect();

                    if kept.len() == original_children.len() {
                        continue;
                    }

                    match kept.len() {
                        0 => {
                            self.nodes.remove(area.root);
                            self.dock_areas.remove(area_id);
                            self.main_areas[side_index] = None;
                        }
                        1 => {
                            self.nodes.remove(area.root);
                            if let Some(area) = self.dock_areas.get_mut(area_id) {
                                area.root = kept[0].1;
                            }
                        }
                        _ => {
                            split.children = kept.iter().map(|(_, child)| *child).collect();
                            split.ratios = helpers::normalized_ratios(
                                kept.iter()
                                    .map(|(index, _)| original_ratios.get(*index).copied())
                                    .collect(),
                            );
                            if let Some(node) = self.nodes.get_mut(area.root) {
                                *node = Node::Split(split);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        for source in removed_columns {
            self.nodes.remove(source);
        }

        notes
    }

    #[cfg(feature = "serde")]
    fn subtree_contains_panel(&self, root: NodeId) -> bool {
        fn contains_panel(
            layout: &Layout,
            node_id: NodeId,
            visited: &mut HashSet<NodeId>,
        ) -> bool {
            if !visited.insert(node_id) {
                return false;
            }
            let Some(node) = layout.nodes.get(node_id) else {
                return false;
            };
            matches!(node, Node::Panel(_))
                || node
                    .children()
                    .iter()
                    .copied()
                    .any(|child| contains_panel(layout, child, visited))
        }

        contains_panel(self, root, &mut HashSet::new())
    }

    #[cfg(feature = "serde")]
    fn repair_duplicate_floating_z_orders(&mut self) -> Vec<String> {
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

    /// Captures a session-only history checkpoint without transient flyout UI.
    pub(crate) fn history_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.flyout = None;
        snapshot
    }

    /// Restores a previously validated history checkpoint as a new model revision.
    ///
    /// The model revision is monotonic even though the persistent state moves
    /// backward/forward. Transient flyout state is not restored, and the current
    /// flyout auto-collapse preference remains session-local.
    pub(crate) fn restore_history_snapshot(
        &mut self,
        snapshot: &Self,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let next_version = self
            .version
            .checked_add(1)
            .ok_or(CommandError::InvalidValue {
                field: "version",
                reason: "layout version overflow",
            })?;

        // Rebuild into a clone of the *current* arenas instead of replacing them
        // with the historical arenas. Keys for entities that still exist in
        // both states stay stable. Removed entities receive fresh generations,
        // so undo cannot make stale SlotMap handles valid again (the ABA problem).
        let mut candidate = self.clone();
        candidate.flyout = None;
        candidate.flyout_auto_collapse = self.flyout_auto_collapse;

        let wanted_panel_ids: HashSet<_> = snapshot.panel_instances.keys().collect();
        for id in candidate.panel_instances.keys().collect::<Vec<_>>() {
            if !wanted_panel_ids.contains(&id) {
                candidate.panel_instances.remove(id);
            }
        }
        let mut panel_map = HashMap::<PanelId, PanelId>::new();
        for (old_id, instance) in snapshot.panel_instances.iter() {
            let new_id = if let Some(current) = candidate.panel_instances.get_mut(old_id) {
                let runtime_state = current.state;
                *current = instance.clone();
                current.state = runtime_state;
                old_id
            } else {
                candidate.panel_instances.insert(instance.clone())
            };
            panel_map.insert(old_id, new_id);
        }

        let wanted_node_ids: HashSet<_> = snapshot.nodes.keys().collect();
        for id in candidate.nodes.keys().collect::<Vec<_>>() {
            if !wanted_node_ids.contains(&id) {
                candidate.nodes.remove(id);
            }
        }

        fn rebuild_node(
            snapshot: &Layout,
            target_id: NodeId,
            target_nodes: &mut SlotMap<NodeId, Node>,
            panel_map: &HashMap<PanelId, PanelId>,
            node_map: &mut HashMap<NodeId, NodeId>,
            visiting: &mut HashSet<NodeId>,
        ) -> Result<NodeId, CommandError> {
            if let Some(mapped) = node_map.get(&target_id) {
                return Ok(*mapped);
            }
            if !visiting.insert(target_id) {
                return Err(CommandError::InvalidLayout {
                    errors: vec![ValidationError::NodeCycle {
                        from: target_id,
                        to: target_id,
                    }],
                });
            }

            let source = snapshot
                .nodes
                .get(target_id)
                .cloned()
                .ok_or(CommandError::MissingNode { node: target_id })?;
            let rebuilt = match source {
                Node::Split(split) => {
                    let children = split
                        .children
                        .iter()
                        .map(|child| {
                            rebuild_node(
                                snapshot,
                                *child,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Split(Split {
                        axis: split.axis,
                        children,
                        ratios: split.ratios,
                    })
                }
                Node::Tabs(tabs) => {
                    let children = tabs
                        .children
                        .iter()
                        .map(|child| {
                            rebuild_node(
                                snapshot,
                                *child,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Tabs(Tabs {
                        children,
                        active_index: tabs.active_index,
                        collapsed: tabs.collapsed,
                    })
                }
                Node::Panel(panel) => {
                    let panel_id = panel_map.get(&panel.panel_id).copied().ok_or(
                        CommandError::MissingPanel {
                            panel: panel.panel_id,
                        },
                    )?;
                    Node::Panel(Panel::new(panel_id))
                }
                Node::Column(column) => {
                    let groups = column
                        .groups
                        .iter()
                        .map(|group| {
                            rebuild_node(
                                snapshot,
                                *group,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Column(Column {
                        width: column.width,
                        mode: column.mode,
                        groups,
                        group_ratios: column.group_ratios,
                        last_expanded_width: column.last_expanded_width,
                    })
                }
            };
            visiting.remove(&target_id);

            let mapped = if let Some(current) = target_nodes.get_mut(target_id) {
                *current = rebuilt;
                target_id
            } else {
                target_nodes.insert(rebuilt)
            };
            node_map.insert(target_id, mapped);
            Ok(mapped)
        }

        let mut node_map = HashMap::<NodeId, NodeId>::new();
        let mut visiting = HashSet::<NodeId>::new();
        for old_id in snapshot.nodes.keys() {
            rebuild_node(
                snapshot,
                old_id,
                &mut candidate.nodes,
                &panel_map,
                &mut node_map,
                &mut visiting,
            )?;
        }

        let wanted_area_ids: HashSet<_> = snapshot.dock_areas.keys().collect();
        for id in candidate.dock_areas.keys().collect::<Vec<_>>() {
            if !wanted_area_ids.contains(&id) {
                candidate.dock_areas.remove(id);
            }
        }
        let mut area_map = HashMap::<DockAreaId, DockAreaId>::new();
        for (old_id, area) in snapshot.dock_areas.iter() {
            let root = node_map
                .get(&area.root)
                .copied()
                .ok_or(CommandError::MissingNode { node: area.root })?;
            let restored_area = DockArea { root, ..*area };
            let new_id = if let Some(current) = candidate.dock_areas.get_mut(old_id) {
                *current = restored_area;
                old_id
            } else {
                candidate.dock_areas.insert(restored_area)
            };
            area_map.insert(old_id, new_id);
        }

        candidate.main_areas = [
            match snapshot.main_areas[0] {
                Some(old_id) => {
                    Some(*area_map.get(&old_id).ok_or(CommandError::InvalidTarget {
                        reason: "history checkpoint has a missing left dock area",
                    })?)
                }
                None => None,
            },
            match snapshot.main_areas[1] {
                Some(old_id) => {
                    Some(*area_map.get(&old_id).ok_or(CommandError::InvalidTarget {
                        reason: "history checkpoint has a missing right dock area",
                    })?)
                }
                None => None,
            },
        ];

        let wanted_floating_ids: HashSet<_> = snapshot.floating_groups.keys().collect();
        for id in candidate.floating_groups.keys().collect::<Vec<_>>() {
            if !wanted_floating_ids.contains(&id) {
                candidate.floating_groups.remove(id);
            }
        }
        for (old_id, group) in snapshot.floating_groups.iter() {
            let area = area_map
                .get(&group.area)
                .copied()
                .ok_or(CommandError::InvalidTarget {
                    reason: "history checkpoint has a missing floating area",
                })?;
            let restored_group = FloatingGroup {
                area,
                bounds: group.bounds,
                z_order: group.z_order,
            };
            if let Some(current) = candidate.floating_groups.get_mut(old_id) {
                *current = restored_group;
            } else {
                candidate.floating_groups.insert(restored_group);
            }
        }

        candidate.last_dock_locations = snapshot.last_dock_locations.clone();
        candidate.last_dock_location_order = snapshot.last_dock_location_order.clone();
        candidate.canvas = snapshot.canvas;
        candidate.profile = snapshot.profile;
        candidate.chrome = snapshot.chrome;
        candidate.version = next_version;
        candidate
            .validate(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        *self = candidate;
        Ok(())
    }

    /// Opaque identity shared by Layout clones, but not separately loaded layouts.
    pub(crate) fn history_identity(&self) -> Weak<()> {
        Arc::downgrade(&self.history_identity)
    }

    pub(crate) fn flyout_width(&self, key: &PanelKey) -> Option<f32> {
        self.flyout_widths.get(key).copied()
    }

    /// Validates once per persistent revision and metric set for render/drag hot paths.
    /// Public `validate` always performs the full check for callers asking for diagnostics.
    pub(crate) fn validate_cached(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        if self
            .validation_cache
            .get()
            .is_some_and(|(version, cached_metrics)| version == self.version && cached_metrics == *metrics)
        {
            return Ok(());
        }
        self.validate(metrics)?;
        self.validation_cache.set(Some((self.version, *metrics)));
        Ok(())
    }

    /// Records host-measured canvas bounds without creating a persistent command revision.
    /// Narrow windows are valid measurements and must not block layout commands.
    pub fn update_canvas_bounds(&mut self, bounds: Option<Rect>) -> Result<bool, GeometryError> {
        if let Some(bounds) = bounds {
            bounds.validate_non_empty()?;
        }
        if self.canvas.last_known_bounds() == bounds {
            return Ok(false);
        }
        self.canvas = CanvasState::new(bounds);
        Ok(true)
    }

    pub const fn profile(&self) -> Profile {
        self.profile
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    pub const fn chrome(&self) -> ChromeMode {
        self.chrome
    }

    pub const fn canvas(&self) -> &CanvasState {
        &self.canvas
    }

    pub const fn flyout(&self) -> Option<FlyoutState> {
        self.flyout
    }

    /// Whether passive dismissal and clicking the selected rail icon close a flyout.
    pub const fn flyout_auto_collapse_enabled(&self) -> bool {
        self.flyout_auto_collapse
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance> {
        self.panel_instances.get(id)
    }

    /// Returns the last known main-dock placement for a panel key in this
    /// runtime session. This preference is intentionally not workspace state.
    pub fn last_dock_location(&self, key: &PanelKey) -> Option<LastDockLocation> {
        self.last_dock_locations.get(key).copied()
    }

    pub fn dock_area(&self, id: DockAreaId) -> Option<&DockArea> {
        self.dock_areas.get(id)
    }

    pub fn floating_group(&self, id: FloatingGroupId) -> Option<&FloatingGroup> {
        self.floating_groups.get(id)
    }

    /// Returns the main area for a side, if one exists.
    pub fn main_area(&self, side: DockSide) -> Option<&DockArea> {
        let index = helpers::side_index(side);
        self.main_areas[index].and_then(|id| self.dock_areas.get(id))
    }

    /// Read-only arena views for validation, rendering adapters, and inspection.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }

    pub fn panel_instances(&self) -> impl Iterator<Item = (PanelId, &PanelInstance)> {
        self.panel_instances.iter()
    }

    pub fn dock_areas(&self) -> impl Iterator<Item = (DockAreaId, &DockArea)> {
        self.dock_areas.iter()
    }

    pub fn floating_groups(&self) -> impl Iterator<Item = (FloatingGroupId, &FloatingGroup)> {
        self.floating_groups.iter()
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

mod column_display_order;
mod command_target_policy;
mod command_floating;
mod command_floating_dispatch;
mod command_group_resize_dispatch;
mod command_group_resize_reset;
mod command_column_resize_dispatch;
mod command_column_resize_reset;
mod command_column_resize_boundaries;
mod command_column_resize_single;
mod command_column_resize_helpers;
mod command_layout_state_dispatch;
mod command_flyout_dispatch;
mod command_panel_dispatch;
mod command_panel_open_dispatch;
mod command_panel_open_target;
mod command_panel_close_dispatch;
mod command_panel_tab_commands;
mod command_panel_open_new;
mod command_panel_reopen_singleton;
mod command_panel_transfer_dispatch;
mod command_panel_move_dispatch;
mod command_panel_transfer_inverse;
mod command_panel_target_transfer;
mod command_panel_tab_reorder;
mod command_flyout_panel_transfer_dispatch;
mod command_group_dispatch;
mod command_group_transfer_dispatch;
mod command_changes;
mod command_group_insertion;
mod command_target_insertion;
mod commands;
mod drag_drop;
mod helpers;
mod normalization;
mod sizing;
mod tree;
mod validation;

#[cfg(test)]
mod tests;
