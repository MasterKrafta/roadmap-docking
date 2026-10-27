use super::helpers::*;
use super::*;

pub(super) fn inspect_graph(
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
        for child in node.children() {
            let child = *child;
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

pub(super) fn mark_reachable(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    reachable: &mut HashSet<NodeId>,
) {
    if !nodes.contains_key(node_id) || !reachable.insert(node_id) {
        return;
    }
    if let Some(node) = nodes.get(node_id) {
        for child in node.children() {
            mark_reachable(nodes, *child, reachable);
        }
    }
}

pub(super) fn validate_photoshop_main_root(
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

pub(super) fn validate_photoshop_floating_root(
    nodes: &SlotMap<NodeId, Node>,
    group: FloatingGroupId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    match nodes.get(root) {
        Some(Node::Tabs(_)) => {}
        Some(Node::Column(column)) => {
            let invalid_child = column.groups.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Tabs(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "floating Column children must all be Tabs nodes",
                });
            }
        }
        Some(Node::Split(split)) if split.axis == Axis::X => {
            let invalid_child = split.children.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Column(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "Split(X) floating composition children must all be Column nodes",
                });
            }
        }
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
            reason: "floating composition Split must use axis X or floating stack Split must use axis Y",
        }),
        Some(_) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating root must be Tabs, Column, Split(X) of Columns, or Split(Y) of Tabs",
        }),
        None => {}
    }
}

const MAX_LAYOUT_TREE_DEPTH: usize = 256;

impl Layout {
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
        let panel_placements =
            self.validate_node_records(metrics, metrics_are_valid, &mut errors);
        self.validate_panel_instances(&panel_placements, &mut errors);

        let roots = self.validate_dock_areas(metrics, metrics_are_valid, &mut errors);

        self.validate_flyout(&mut errors);

        if !self.validate_tree_graph(&roots, &mut errors) {
            return Err(errors);
        }

        self.validate_expanded_column_widths(metrics, metrics_are_valid, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub(super) fn validate_node_records(
        &self,
        metrics: &DockMetrics,
        metrics_are_valid: bool,
        errors: &mut Vec<ValidationError>,
    ) -> HashMap<PanelId, NodeId> {
        let mut panel_placements: HashMap<PanelId, NodeId> = HashMap::new();

        for (node_id, node) in self.nodes.iter() {
            match node {
                Node::Split(split) => {
                    if split.children.len() < 2 {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason: ValidationReason::SplitMustContainAtLeastTwoChildren,
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
                            reason: ValidationReason::TabsMustContainAtLeastOneChild,
                        });
                    }
                    if tabs.active_index >= tabs.children.len() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: ValidationReason::ActiveTabIndexOutsideChildren,
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
                                    reason: ValidationReason::PhotoshopTabsChildrenMustBePanelNodes,
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
                            reason: ValidationReason::ColumnWidthMustBeFinite,
                        });
                    } else if column.width <= 0.0 {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: ValidationReason::ColumnWidthMustBePositive,
                        });
                    } else if metrics_are_valid {
                        match column.mode {
                            ColumnMode::Expanded
                                if column.width + PIXEL_TOLERANCE < metrics.min_column_width =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: ValidationReason::ExpandedWidthBelowMinimumColumnWidth,
                                });
                            }
                            ColumnMode::IconsAndLabels
                                if (column.width - metrics.min_column_width).abs()
                                    > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: ValidationReason::IconsAndLabelsWidthMustEqualMinimumColumnWidth,
                                });
                            }
                            ColumnMode::IconsOnly
                                if (column.width - metrics.rail_width).abs() > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: ValidationReason::IconsOnlyWidthMustEqualRailWidth,
                                });
                            }
                            _ => {}
                        }
                    }

                    if let Some(width) = column.last_expanded_width
                        && (!width.is_finite() || width <= 0.0)
                    {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: ValidationReason::LastExpandedWidthMustBeFiniteAndPositive,
                        });
                    }

                    if column.groups.is_empty() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: ValidationReason::ColumnMustContainAtLeastOneGroupBeforeNormalization,
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
                                    reason: ValidationReason::PhotoshopColumnGroupsMustBeTabsNodes,
                                });
                            }
                        }
                    }
                }
            }
        }
        panel_placements
    }

    pub(super) fn validate_panel_instances(
        &self,
        panel_placements: &HashMap<PanelId, NodeId>,
        errors: &mut Vec<ValidationError>,
    ) {
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
    }

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
                        validate_photoshop_main_root(&self.nodes, area_id, area.root, errors);
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
                    validate_photoshop_floating_root(&self.nodes, group_id, area.root, errors);
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

    /// Checks tree depth and graph invariants before recursive size validation.
    ///
    /// Returns false only when the depth limit is exceeded, which requires the
    /// caller to stop further validation immediately.
    pub(super) fn validate_tree_graph(
        &self,
        roots: &[NodeId],
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        // Bound recursive validation/normalization work for programmatically
        // built trees as well as serde-loaded DTOs. Longest-depth tracking also
        // lets us stop a cyclic graph before a recursive walk can exhaust stack.
        let mut colors = HashMap::<NodeId, u8>::new();
        let mut pending = Vec::<(NodeId, usize, bool)>::new();
        for start in roots.iter().copied().chain(self.nodes.keys()) {
            if colors.contains_key(&start) {
                continue;
            }
            pending.push((start, 1, false));
            while let Some((current, depth, exiting)) = pending.pop() {
                if exiting {
                    colors.insert(current, 2);
                    continue;
                }
                if depth > MAX_LAYOUT_TREE_DEPTH {
                    errors.push(ValidationError::TreeTooDeep {
                        max_depth: MAX_LAYOUT_TREE_DEPTH,
                    });
                    return false;
                }
                match colors.get(&current).copied() {
                    Some(1) | Some(2) => continue,
                    _ => {}
                }
                colors.insert(current, 1);
                pending.push((current, depth, true));
                if let Some(node) = self.nodes.get(current) {
                    pending.extend(
                        node.children()
                            .iter()
                            .rev()
                            .copied()
                            .map(|child| (child, depth + 1, false)),
                    );
                }
            }
        }

        // Count all tree-edge references to find reused children, then mark
        // reachability only from registered area roots. These are deliberately
        // separate passes: cycles and missing links in an orphaned component
        // must still be reported instead of hidden by an early return.
        let mut incoming: HashMap<NodeId, usize> = HashMap::new();
        for root in roots {
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
                errors,
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
        for root in roots {
            mark_reachable(&self.nodes, *root, &mut reachable);
        }
        for (node_id, _) in self.nodes.iter() {
            if !reachable.contains(&node_id) {
                errors.push(ValidationError::UnreachableNode { node: node_id });
            }
        }
        true
    }

    pub(super) fn validate_flyout(&self, errors: &mut Vec<ValidationError>) {
        if let Some(flyout) = self.flyout {
            match self.nodes.get(flyout.column) {
                None => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing column node",
                }),
                Some(Node::Column(column)) => {
                    if !column.mode.is_rail() {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "flyout column must use a rail mode",
                        });
                    }
                    if !column
                        .groups
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, flyout.group))
                    {
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

            let panel_belongs_to_group = match self.nodes.get(flyout.group) {
                Some(Node::Tabs(tabs)) => tabs.children.iter().any(|child| {
                    matches!(
                        self.nodes.get(*child),
                        Some(Node::Panel(panel)) if panel.panel_id == flyout.panel
                    )
                }),
                _ => false,
            };
            if !panel_belongs_to_group {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "panel is not a child of the flyout group",
                });
            }
        }
    }

    pub(super) fn validate_expanded_column_widths(
        &self,
        metrics: &DockMetrics,
        metrics_are_valid: bool,
        errors: &mut Vec<ValidationError>,
    ) {
        // Only recurse into width constraints after graph validation succeeds;
        // minimum-size traversal assumes an acyclic, fully linked node tree.
        if errors.is_empty() && metrics_are_valid {
            for (node_id, node) in self.nodes.iter() {
                let Node::Column(column) = node else {
                    continue;
                };
                if column.mode != ColumnMode::Expanded {
                    continue;
                }

                let minimum = match self.minimum_expanded_column_width(node_id, metrics) {
                    Ok(minimum) => minimum,
                    Err(_) => {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: ValidationReason::CannotComputeMinimumWidthRequiredByExpandedContents,
                        });
                        continue;
                    }
                };
                let maximum = match self.maximum_expanded_column_width(node_id) {
                    Ok(maximum) => maximum,
                    Err(_) => {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: ValidationReason::CannotComputeMaximumWidthAllowedByExpandedContents,
                        });
                        continue;
                    }
                };
                if maximum.is_some_and(|maximum| maximum < minimum) {
                    errors.push(ValidationError::InvalidColumn {
                        node: node_id,
                        reason: ValidationReason::MinimumWidthRequiredByExpandedContentsExceedsPanelMaximumWidth,
                    });
                } else if column.width < minimum {
                    errors.push(ValidationError::InvalidColumn {
                        node: node_id,
                        reason: ValidationReason::ExpandedWidthBelowMinimumWidthRequiredByContents,
                    });
                }
            }
        }
    }
}

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

    pub(super) fn normalize_main_areas(&mut self) -> Result<(), Vec<ValidationError>> {
        // Main roots are normalized first. An empty main root removes its
        // DockArea and clears the corresponding side; it never leaves an empty
        // invisible area in the arena.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let side = if index == 0 { DockSide::Left } else { DockSide::Right };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                return Err(vec![ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "references a missing DockAreaId",
                }]);
            };

            match self.normalize_node(area.root) {
                Some(normalized_root) => {
                    let Some(area) = self.dock_areas.get_mut(area_id) else {
                        return Err(vec![ValidationError::InvalidMainArea {
                            area: area_id,
                            side,
                            reason: "references a missing DockAreaId",
                        }]);
                    };
                    area.root = normalized_root;
                }
                None => {
                    self.dock_areas.remove(area_id);
                    self.main_areas[index] = None;
                }
            }
        }
        Ok(())
    }

    pub(super) fn normalize_floating_areas(&mut self) -> Result<(), Vec<ValidationError>> {
        // Floating roots are normalized the same way. Empty floating roots
        // remove both the FloatingGroup and its associated DockArea.
        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let Some(area) = self.dock_areas.get(group.area).copied() else {
                return Err(vec![ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "references a missing DockAreaId",
                }]);
            };

            match self.normalize_node(area.root) {
                Some(normalized_root) => {
                    let Some(area) = self.dock_areas.get_mut(group.area) else {
                        return Err(vec![ValidationError::InvalidFloatingGroup {
                            group: group_id,
                            reason: "references a missing DockAreaId",
                        }]);
                    };
                    area.root = normalized_root;
                }
                None => {
                    self.floating_groups.remove(group_id);
                    self.dock_areas.remove(group.area);
                }
            }
        }
        Ok(())
    }

    pub(super) fn close_invalid_flyout(&mut self) {
        // Flyouts are transient. If normalization removed or changed their
        // target, close the flyout instead of retaining stale runtime handles.
        let keep_flyout = self.flyout.is_some_and(|flyout| {
            let belongs_to_main_dock = [DockSide::Left, DockSide::Right]
                .into_iter()
                .filter_map(|side| self.main_area(side).map(|area| area.root))
                .any(|root| self.is_descendant_or_same(root, flyout.column));
            belongs_to_main_dock
                && matches!(
                    self.nodes.get(flyout.column),
                    Some(Node::Column(column))
                        if column.mode.is_rail()
                            && column.groups.iter().any(|root| self.is_descendant_or_same(*root, flyout.group))
                            && matches!(
                                self.nodes.get(flyout.group),
                                Some(Node::Tabs(tabs)) if tabs.children.iter().any(|child| matches!(
                                    self.nodes.get(*child),
                                    Some(Node::Panel(panel)) if panel.panel_id == flyout.panel
                                ))
                            )
                )
        });
        if !keep_flyout {
            self.flyout = None;
        }
    }

    /// Returns the normalized representative of a node, or None when the
    /// entire subtree is empty. Preflight validation guarantees that every
    /// referenced node exists and that the graph is acyclic and single-parent.
    pub(super) fn normalize_node(&mut self, node_id: NodeId) -> Option<NodeId> {
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
                let node = self.nodes.get_mut(node_id)?;
                *node = Node::Tabs(tabs);
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
                        split.ratios =
                            normalized_ratios(kept.iter().map(|(_, ratio)| *ratio).collect());
                        let node = self.nodes.get_mut(node_id)?;
                        *node = Node::Split(split);
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
                let node = self.nodes.get_mut(node_id)?;
                *node = Node::Column(column);
                Some(node_id)
            }
        }
    }

}
