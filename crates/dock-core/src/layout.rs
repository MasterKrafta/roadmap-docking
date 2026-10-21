use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    Axis, CanvasState, ChromeMode, ColumnMode, DockArea, DockAreaId, DockAreaKind,
    DockMetrics, DockSide, FloatingGroup, FloatingGroupId, FlyoutState, GeometryError, Node,
    NodeId, PanelId, PanelInstance, Profile, ValidationError,
};

const RATIO_TOLERANCE: f32 = 1.0e-4;
const PIXEL_TOLERANCE: f32 = 1.0e-3;

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// persistent edits will be performed through DockCommand once the command
/// layer is introduced. Flyout state is transient and kept separately from the
/// persistent tree.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
    flyout: Option<FlyoutState>,
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
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
            flyout: None,
        }
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

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance> {
        self.panel_instances.get(id)
    }

    pub fn dock_area(&self, id: DockAreaId) -> Option<&DockArea> {
        self.dock_areas.get(id)
    }

    pub fn floating_group(&self, id: FloatingGroupId) -> Option<&FloatingGroup> {
        self.floating_groups.get(id)
    }

    /// Returns the main area for a side, if one exists.
    pub fn main_area(&self, side: DockSide) -> Option<&DockArea> {
        let index = side_index(side);
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

    /// Validates the entire model and returns every detected issue.
    ///
    /// Metrics are supplied explicitly so callers and tests use the same
    /// thresholds as the core algorithms. An empty layout is valid: the canvas
    /// always exists, while empty main dock areas are represented by None.
    pub fn validate(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let metrics_result = metrics.validate();
        if let Err(reason) = metrics_result {
            errors.push(ValidationError::InvalidMetrics { reason });
        }
        let metrics_are_valid = metrics_result.is_ok();

        if let Err(reason) = self.canvas.validate() {
            errors.push(ValidationError::InvalidCanvas {
                reason: geometry_error_reason(reason),
            });
        }
        if metrics_are_valid {
            if let Some(bounds) = self.canvas.last_known_bounds()
                && bounds.size.width + PIXEL_TOLERANCE < metrics.min_canvas_width
            {
                errors.push(ValidationError::InvalidCanvas {
                    reason: "canvas width is below min_canvas_width",
                });
            }
        }

        let mut panel_placements: HashMap<PanelId, NodeId> = HashMap::new();

        for (node_id, node) in self.nodes.iter() {
            match node {
                Node::Split(split) => {
                    if split.children.len() < 2 {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason: "must contain at least two children",
                        });
                    }
                    for reason in ratio_issues(&split.ratios, split.children.len()) {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason,
                        });
                    }
                }
                Node::Tabs(tabs) => {
                    if tabs.children.is_empty() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "must contain at least one child",
                        });
                    }
                    if tabs.active_index >= tabs.children.len() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "active_index is outside children",
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for child in &tabs.children {
                            if matches!(self.nodes.get(*child), Some(Node::Panel(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*child) {
                                errors.push(ValidationError::InvalidTabs {
                                    node: node_id,
                                    reason: "Photoshop Tabs children must be Panel nodes",
                                });
                            }
                        }
                    }
                }
                Node::Panel(panel) => {
                    if !self.panel_instances.contains_key(panel.panel_id) {
                        errors.push(ValidationError::MissingPanelInstance {
                            node: node_id,
                            panel: panel.panel_id,
                        });
                    }
                    if let Some(first_node) = panel_placements.get(&panel.panel_id) {
                        errors.push(ValidationError::DuplicatePanelPlacement {
                            panel: panel.panel_id,
                            first_node: *first_node,
                            second_node: node_id,
                        });
                    } else {
                        panel_placements.insert(panel.panel_id, node_id);
                    }
                }
                Node::Column(column) => {
                    if !column.width.is_finite() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be finite",
                        });
                    } else if column.width <= 0.0 {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be positive",
                        });
                    } else if metrics_are_valid {
                        match column.mode {
                            ColumnMode::Expanded
                                if column.width + PIXEL_TOLERANCE
                                    < metrics.min_column_width =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "expanded width is below min_column_width",
                                });
                            }
                            ColumnMode::IconsOnly
                                if (column.width - metrics.rail_width).abs()
                                    > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "IconsOnly width must equal rail_width",
                                });
                            }
                            _ => {}
                        }
                    }

                    if column.groups.is_empty() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "must contain at least one group before normalization",
                        });
                    }
                    for reason in ratio_issues(&column.group_ratios, column.groups.len()) {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason,
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for group in &column.groups {
                            if matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*group) {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "Photoshop Column groups must be Tabs nodes",
                                });
                            }
                        }
                    }
                }
            }
        }

        let mut singleton_by_key: HashMap<crate::PanelKey, (PanelId, bool)> = HashMap::new();
        for (panel_id, instance) in self.panel_instances.iter() {
            if let Err(reason) = instance.validate() {
                errors.push(ValidationError::InvalidPanelInstance {
                    panel: panel_id,
                    reason,
                });
            }
            if !panel_placements.contains_key(&panel_id) {
                errors.push(ValidationError::UnplacedPanelInstance { panel: panel_id });
            }

            let key = instance.key.clone();
            let singleton = instance.constraints.singleton;
            if let Some((first_id, first_singleton)) = singleton_by_key.get_mut(&key) {
                if singleton || *first_singleton {
                    errors.push(ValidationError::DuplicateSingletonPanel {
                        key,
                        first: *first_id,
                        second: panel_id,
                    });
                }
                *first_singleton |= singleton;
            } else {
                singleton_by_key.insert(key, (panel_id, singleton));
            }
        }

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
                        GeometryError::NegativeDimension => "bounds dimensions must not be negative",
                        GeometryError::NonPositiveDimension => "bounds must have positive dimensions",
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
                    validate_photoshop_floating_root(
                        &self.nodes,
                        group_id,
                        area.root,
                        &mut errors,
                    );
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

        if metrics_are_valid && self.profile == Profile::Photoshop
            && photoshop_main_columns > metrics.max_columns
        {
            errors.push(ValidationError::TooManyColumns {
                actual: photoshop_main_columns,
                max: metrics.max_columns,
            });
        }

        if let Some(flyout) = self.flyout {
            match self.nodes.get(flyout.column) {
                None => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing column node",
                }),
                Some(Node::Column(column)) => {
                    if column.mode != ColumnMode::IconsOnly {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "flyout column must be in IconsOnly mode",
                        });
                    }
                    if !column.groups.contains(&flyout.group) {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "group is not a member of the referenced column",
                        });
                    }
                }
                Some(_) => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "column reference does not point to a Column node",
                }),
            }

            if !self.nodes.contains_key(flyout.group) {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing group node",
                });
            }
        }

        // Count all tree-edge references to find reused children, then mark
        // reachability only from registered area roots. These are deliberately
        // separate passes: cycles and missing links in an orphaned component
        // must still be reported instead of hidden by an early return.
        let mut incoming: HashMap<NodeId, usize> = HashMap::new();
        for root in &roots {
            *incoming.entry(*root).or_default() += 1;
        }
        let mut graph_visited = HashSet::new();
        let mut visiting = HashSet::new();
        for (node_id, _) in self.nodes.iter() {
            inspect_graph(
                &self.nodes,
                node_id,
                None,
                &mut graph_visited,
                &mut visiting,
                &mut incoming,
                &mut errors,
            );
        }

        for (node_id, _) in self.nodes.iter() {
            if let Some(references) = incoming.get(&node_id)
                && *references > 1
            {
                errors.push(ValidationError::MultipleParents {
                    node: node_id,
                    references: *references,
                });
            }
        }

        let mut reachable = HashSet::new();
        for root in &roots {
            mark_reachable(&self.nodes, *root, &mut reachable);
        }
        for (node_id, _) in self.nodes.iter() {
            if !reachable.contains(&node_id) {
                errors.push(ValidationError::UnreachableNode { node: node_id });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Normalizes a candidate layout and commits the result only if it remains valid.
    ///
    /// Empty Tabs/Columns and empty Split containers are removed. Generic Split
    /// nodes with one child collapse into that child, while a non-empty Column
    /// remains a Column even with one group. The operation refuses to guess how
    /// to repair broken links, cycles, shared children, or missing panel instances.
    pub fn normalize(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut candidate = self.clone();
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(())
    }

    fn normalize_in_place(
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

        // Main roots are normalized first. An empty main root removes its
        // DockArea and clears the corresponding side; it never leaves an empty
        // invisible area in the arena.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let root = self
                .dock_areas
                .get(area_id)
                .expect("preflight validation rejects missing main areas")
                .root;

            match self.normalize_node(root) {
                Some(normalized_root) => {
                    self.dock_areas
                        .get_mut(area_id)
                        .expect("main area remains present")
                        .root = normalized_root;
                }
                None => {
                    self.dock_areas.remove(area_id);
                    self.main_areas[index] = None;
                }
            }
        }

        // Floating roots are normalized the same way. Empty floating roots
        // remove both the FloatingGroup and its associated DockArea.
        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let root = self
                .dock_areas
                .get(group.area)
                .expect("preflight validation rejects missing floating areas")
                .root;

            match self.normalize_node(root) {
                Some(normalized_root) => {
                    self.dock_areas
                        .get_mut(group.area)
                        .expect("floating area remains present")
                        .root = normalized_root;
                }
                None => {
                    self.floating_groups.remove(group_id);
                    self.dock_areas.remove(group.area);
                }
            }
        }

        // Flyouts are transient. If normalization removed or changed their
        // target, close the flyout instead of retaining stale runtime handles.
        let keep_flyout = self.flyout.is_some_and(|flyout| {
            matches!(
                self.nodes.get(flyout.column),
                Some(Node::Column(column))
                    if column.mode == ColumnMode::IconsOnly
                        && column.groups.contains(&flyout.group)
                        && self.nodes.contains_key(flyout.group)
            )
        });
        if !keep_flyout {
            self.flyout = None;
        }

        self.validate(metrics)
    }

    /// Returns the normalized representative of a node, or None when the
    /// entire subtree is empty. Preflight validation guarantees that every
    /// referenced node exists and that the graph is acyclic and single-parent.
    fn normalize_node(&mut self, node_id: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(node_id)?.clone();
        match node {
            Node::Panel(_) => Some(node_id),
            Node::Tabs(mut tabs) => {
                let old_children = tabs.children;
                let old_active_index = tabs.active_index;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((old_index, normalized_child));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                tabs.active_index = kept
                    .iter()
                    .position(|(old_index, _)| *old_index == old_active_index)
                    .unwrap_or_else(|| old_active_index.min(kept.len() - 1));
                tabs.children = kept.into_iter().map(|(_, child)| child).collect();
                *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                    Node::Tabs(tabs);
                Some(node_id)
            }
            Node::Split(mut split) => {
                let old_children = split.children;
                let old_ratios = split.ratios;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((normalized_child, old_ratios.get(old_index).copied()));
                    }
                }

                match kept.len() {
                    0 => {
                        self.nodes.remove(node_id);
                        None
                    }
                    1 => {
                        let (only_child, _) = kept[0];
                        self.nodes.remove(node_id);
                        Some(only_child)
                    }
                    _ => {
                        split.children = kept.iter().map(|(child, _)| *child).collect();
                        split.ratios = normalized_ratios(
                            kept.iter().map(|(_, ratio)| *ratio).collect(),
                        );
                        *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                            Node::Split(split);
                        Some(node_id)
                    }
                }
            }
            Node::Column(mut column) => {
                let old_groups = column.groups;
                let old_ratios = column.group_ratios;
                let mut kept = Vec::with_capacity(old_groups.len());

                for (old_index, group) in old_groups.into_iter().enumerate() {
                    if let Some(normalized_group) = self.normalize_node(group) {
                        kept.push((normalized_group, old_ratios.get(old_index).copied()));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                // Deliberately do not collapse a Column that has exactly one group.
                column.groups = kept.iter().map(|(group, _)| *group).collect();
                column.group_ratios =
                    normalized_ratios(kept.iter().map(|(_, ratio)| *ratio).collect());
                *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                    Node::Column(column);
                Some(node_id)
            }
        }
    }

    /// Internal constructor used by pure core factories and loading/migration.
    ///
    /// It builds a complete value rather than exposing mutable arena access to
    /// consumers. Public persistent mutation remains command-driven.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: SlotMap<NodeId, Node>,
        panel_instances: SlotMap<PanelId, PanelInstance>,
        dock_areas: SlotMap<DockAreaId, DockArea>,
        main_areas: [Option<DockAreaId>; 2],
        floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
        canvas: CanvasState,
        profile: Profile,
        version: u64,
        chrome: ChromeMode,
    ) -> Self {
        Self {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            canvas,
            profile,
            version,
            chrome,
            flyout: None,
        }
    }
}

fn side_index(side: DockSide) -> usize {
    match side {
        DockSide::Left => 0,
        DockSide::Right => 1,
    }
}

fn ratio_issues(ratios: &[f32], expected_len: usize) -> Vec<&'static str> {
    let mut issues = Vec::new();
    if ratios.len() != expected_len {
        issues.push("ratio count must match child/group count");
    }
    if ratios.iter().any(|ratio| !ratio.is_finite() || *ratio < 0.0) {
        issues.push("ratios must be finite and non-negative");
    }
    let sum: f32 = ratios.iter().copied().sum();
    if !sum.is_finite() || (sum - 1.0).abs() > RATIO_TOLERANCE {
        issues.push("ratios must sum to 1.0 within 1e-4");
    }
    issues
}

fn is_normalizable_validation_error(error: &ValidationError) -> bool {
    match error {
        ValidationError::InvalidSplit { reason, .. } => {
            *reason == "must contain at least two children" || is_ratio_error(reason)
        }
        ValidationError::InvalidTabs { reason, .. } => {
            *reason == "must contain at least one child"
                || *reason == "active_index is outside children"
        }
        ValidationError::InvalidColumn { reason, .. } => {
            *reason == "must contain at least one group before normalization"
                || is_ratio_error(reason)
        }
        // All other errors describe broken references, invalid metadata, or
        // profile constraints that normalization must not guess how to repair.
        _ => false,
    }
}

fn is_ratio_error(reason: &str) -> bool {
    matches!(
        reason,
        "ratio count must match child/group count"
            | "ratios must be finite and non-negative"
            | "ratios must sum to 1.0 within 1e-4"
    )
}

fn normalized_ratios(ratios: Vec<Option<f32>>) -> Vec<f32> {
    if ratios.is_empty() {
        return Vec::new();
    }

    let values: Option<Vec<f32>> = ratios
        .into_iter()
        .collect::<Option<Vec<f32>>>()
        .filter(|values| values.iter().all(|value| value.is_finite() && *value >= 0.0));

    let Some(values) = values else {
        return equal_ratios(ratios.len());
    };
    let sum: f32 = values.iter().copied().sum();
    if !sum.is_finite() || sum <= f32::EPSILON {
        return equal_ratios(values.len());
    }

    // Leave already-normalized ratios alone to make repeated normalization
    // exactly idempotent rather than introducing floating-point drift.
    if (sum - 1.0).abs() <= RATIO_TOLERANCE {
        return values;
    }

    let normalized: Vec<f32> = values.iter().map(|value| *value / sum).collect();
    if normalized.iter().all(|value| value.is_finite()) {
        normalized
    } else {
        equal_ratios(values.len())
    }
}

fn equal_ratios(len: usize) -> Vec<f32> {
    if len == 0 {
        return Vec::new();
    }
    vec![1.0 / len as f32; len]
}

fn node_children(node: &Node) -> Vec<NodeId> {
    match node {
        Node::Split(split) => split.children.clone(),
        Node::Tabs(tabs) => tabs.children.clone(),
        Node::Column(column) => column.groups.clone(),
        Node::Panel(_) => Vec::new(),
    }
}

fn inspect_graph(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    parent: Option<NodeId>,
    visited: &mut HashSet<NodeId>,
    visiting: &mut HashSet<NodeId>,
    incoming: &mut HashMap<NodeId, usize>,
    errors: &mut Vec<ValidationError>,
) {
    if !nodes.contains_key(node_id) {
        if let Some(owner) = parent {
            errors.push(ValidationError::MissingNode {
                owner: Some(owner),
                missing: node_id,
            });
        }
        return;
    }
    if visiting.contains(&node_id) {
        errors.push(ValidationError::NodeCycle {
            from: parent.unwrap_or(node_id),
            to: node_id,
        });
        return;
    }
    if visited.contains(&node_id) {
        return;
    }

    visiting.insert(node_id);
    if let Some(node) = nodes.get(node_id) {
        for child in node_children(node) {
            if nodes.contains_key(child) {
                *incoming.entry(child).or_default() += 1;
                inspect_graph(
                    nodes,
                    child,
                    Some(node_id),
                    visited,
                    visiting,
                    incoming,
                    errors,
                );
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: Some(node_id),
                    missing: child,
                });
            }
        }
    }
    visiting.remove(&node_id);
    visited.insert(node_id);
}

fn mark_reachable(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    reachable: &mut HashSet<NodeId>,
) {
    if !nodes.contains_key(node_id) || !reachable.insert(node_id) {
        return;
    }
    if let Some(node) = nodes.get(node_id) {
        for child in node_children(node) {
            mark_reachable(nodes, child, reachable);
        }
    }
}

fn validate_photoshop_main_root(
    nodes: &SlotMap<NodeId, Node>,
    area: DockAreaId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) -> usize {
    match nodes.get(root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) if split.axis == Axis::X => {
            let mut count = 0;
            let mut invalid_child = false;
            for child in &split.children {
                match nodes.get(*child) {
                    Some(Node::Column(_)) => count += 1,
                    Some(_) => invalid_child = true,
                    None => {}
                }
            }
            if invalid_child {
                errors.push(ValidationError::InvalidMainRoot {
                    area,
                    root,
                    reason: "Split(X) main root children must all be Column nodes",
                });
            }
            count
        }
        Some(Node::Split(_)) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main Split must use axis X",
            });
            0
        }
        Some(_) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main root must be a Column or Split(X) of Columns",
            });
            0
        }
        None => 0,
    }
}

fn validate_photoshop_floating_root(
    nodes: &SlotMap<NodeId, Node>,
    group: FloatingGroupId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    match nodes.get(root) {
        Some(Node::Tabs(_)) => {}
        Some(Node::Split(split)) if split.axis == Axis::Y => {
            let invalid_child = split.children.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Tabs(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "Split(Y) floating root children must all be Tabs nodes",
                });
            }
        }
        Some(Node::Split(_)) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating stack Split must use axis Y",
        }),
        Some(_) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating root must be Tabs or Split(Y) of Tabs",
        }),
        None => {}
    }
}

fn geometry_error_reason(error: GeometryError) -> &'static str {
    match error {
        GeometryError::NonFinite => "bounds contain a non-finite value",
        GeometryError::NegativeDimension => "bounds dimensions must not be negative",
        GeometryError::NonPositiveDimension => "bounds must have positive dimensions",
        GeometryError::ArithmeticOverflow => "bounds arithmetic overflowed",
        GeometryError::InconsistentConstraints => "geometry constraints are inconsistent",
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{
        Axis, CanvasState, ChromeMode, DockArea, DockAreaId, DockMetrics, DockSide, FloatingGroup,
        FloatingGroupId, Node, NodeId, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey,
        Point, Profile, Rect, Size, Split, Tabs, ValidationError,
    };
    use slotmap::{Key, SlotMap};

    fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
        ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| {
                (
                    PanelKey::new(key).unwrap(),
                    PanelConstraints::new(
                        Size::new(120.0, 80.0),
                        Size::new(280.0, 220.0),
                        None,
                        false,
                    ),
                )
            })
            .collect()
    }

    fn initial_layout(profile: Profile) -> Layout {
        crate::create_initial_layout(profile, &descriptors(), DockMetrics::default()).unwrap()
    }

    #[test]
    fn new_layout_owns_empty_arenas_and_an_independent_canvas() {
        let layout = Layout::new(Profile::Generic);

        assert_eq!(layout.profile(), Profile::Generic);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.chrome(), ChromeMode::Visible);
        assert_eq!(layout.flyout(), None);
        assert_eq!(layout.canvas(), &CanvasState::default());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.panel_instances().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert_eq!(layout.floating_groups().count(), 0);
        assert_eq!(layout.main_area(DockSide::Left), None);
        assert_eq!(layout.main_area(DockSide::Right), None);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn clone_preserves_runtime_handles_and_read_only_views() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Split(Split {
            axis: Axis::Y,
            children: Vec::new(),
            ratios: Vec::new(),
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::main(DockSide::Left, root));
        let floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [Some(area), None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            7,
            ChromeMode::Hidden,
        );
        let cloned = layout.clone();

        assert_eq!(cloned.main_area(DockSide::Left).unwrap().root, root);
        assert_eq!(cloned.version(), 7);
        assert_eq!(cloned.chrome(), ChromeMode::Hidden);
    }

    #[test]
    fn initial_factory_layout_passes_validation() {
        let layout = initial_layout(Profile::Photoshop);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn validation_collects_ratio_and_tabs_errors() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            panic!("factory should create a column");
        };
        column.group_ratios = vec![0.2, 0.2];

        let group_id = column.groups[0];
        let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
            panic!("factory should create Tabs");
        };
        tabs.active_index = tabs.children.len();

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidColumn { reason: "ratios must sum to 1.0 within 1e-4", .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidTabs { reason: "active_index is outside children", .. }
        )));
    }

    #[test]
    fn validation_detects_missing_node_and_cycle_even_in_broken_trees() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            panic!("factory should create a column");
        };
        column.groups.push(NodeId::null());
        column.group_ratios.push(0.0);

        let group_id = column.groups[0];
        let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
            panic!("factory should create Tabs");
        };
        tabs.children.push(group_id);

        let orphan_panel = layout
            .nodes
            .insert(Node::Panel(Panel::new(PanelId::null())));

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MissingNode { missing, .. } if *missing == NodeId::null()
        )));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle { .. })));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MultipleParents { node, .. } if *node == group_id
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::UnreachableNode { node } if *node == orphan_panel
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MissingPanelInstance { node, panel }
                if *node == orphan_panel && *panel == PanelId::null()
        )));
    }

    #[test]
    fn validation_detects_duplicate_panel_placement_and_singleton_keys() {
        let mut layout = initial_layout(Profile::Generic);
        let ids: Vec<_> = layout.nodes.iter().filter_map(|(id, node)| match node {
            Node::Panel(panel) => Some((id, panel.panel_id)),
            _ => None,
        }).collect();
        let (first_node, first_panel) = ids[0];
        let (second_node, second_panel) = ids[1];
        *layout.nodes.get_mut(second_node).unwrap() = Node::Panel(Panel::new(first_panel));

        let duplicate_instance = layout.panel_instances.get_mut(second_panel).unwrap();
        duplicate_instance.key = PanelKey::new("layers").unwrap();
        duplicate_instance.constraints.singleton = true;
        layout.panel_instances.get_mut(first_panel).unwrap().constraints.singleton = true;

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicatePanelPlacement { panel, .. } if *panel == first_panel
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicateSingletonPanel { .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::UnplacedPanelInstance { panel } if *panel == second_panel
        )));
        assert_ne!(first_node, second_node);
    }

    #[test]
    fn photoshop_profile_rejects_vertical_main_split() {
        let mut layout = initial_layout(Profile::Photoshop);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let children = match layout.nodes.get(root).unwrap() {
            Node::Column(column) => column.groups.clone(),
            _ => unreachable!(),
        };
        *layout.nodes.get_mut(root).unwrap() = Node::Split(Split {
            axis: Axis::Y,
            children: children.clone(),
            ratios: vec![0.5, 0.5],
        });

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidMainRoot { reason: "main Split must use axis X", .. }
        )));
    }

    #[test]
    fn floating_bounds_and_duplicate_z_order_are_validated() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area_one = areas.insert(DockArea::floating(root));
        let area_two = areas.insert(DockArea::floating(root));
        let mut floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let bounds = Rect::new(Point::ZERO, Size::new(0.0, 300.0));
        floating.insert(FloatingGroup::new(area_one, bounds, 1));
        floating.insert(FloatingGroup::new(area_two, Rect::new(Point::ZERO, Size::new(400.0, 300.0)), 1));

        let layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [None, None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            0,
            ChromeMode::Visible,
        );
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidFloatingGroup { reason: "bounds must have positive dimensions", .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicateFloatingZOrder { z_order: 1, .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidTabs { .. }
        )));
    }

    #[test]
    fn validation_rejects_invalid_metrics_and_canvas_width() {
        let mut layout = Layout::new(Profile::Generic);
        layout.canvas = CanvasState::new(Some(Rect::new(Point::ZERO, Size::new(200.0, 600.0))));
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidCanvas { reason: "canvas width is below min_canvas_width" }
        )));

        let mut metrics = DockMetrics::default();
        metrics.max_columns = 0;
        let errors = layout.validate(&metrics).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidMetrics { .. }
        )));
    }
}
