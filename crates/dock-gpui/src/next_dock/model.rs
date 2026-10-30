//! Layout tree and composition state for the GPUI docking implementation.

/// Orientation of a split between dock nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SplitAxis {
    Horizontal,
    Vertical,
}

/// A panel displayed as a tab inside one leaf of the dock tree.
#[derive(Clone, Copy, Debug)]
pub(super) struct Panel {
    pub(super) title: &'static str,
    pub(super) description: &'static str,
}

/// A tab group is one leaf in the dock tree.
#[derive(Clone, Debug)]
pub(super) struct TabGroup {
    pub(super) panels: Vec<Panel>,
    pub(super) active: usize,
}

/// A node in the dock layout tree.
#[derive(Clone, Debug)]
pub(super) enum DockNode {
    /// A split containing any number of adjacent children.
    ///
    /// Weights define their relative sizes along the split axis.
    Split {
        axis: SplitAxis,
        weights: Vec<f32>,
        children: Vec<DockNode>,
    },
    /// A column with its own header and content.
    Column { content: Box<DockNode> },
    /// A tab group and its active panel.
    Tabs(TabGroup),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum NodePathStep {
    SplitChild(usize),
    ColumnContent,
}

/// Owns one dock layout tree and the operations that mutate it.
#[derive(Clone, Debug)]
pub(super) struct DockLayout {
    pub(super) root: DockNode,
}

impl DockLayout {
    /// Creates a visible starting layout to use while the new behaviors are built.
    pub(super) fn initial() -> Self {
        let left = tabs(&[
            ("Navigator", "Document navigation and outline."),
            ("Assets", "Imported assets and reusable resources."),
        ]);

        let canvas = tabs(&[
            ("Canvas", "Main document workspace."),
            ("Preview", "Preview the current document."),
        ]);

        let bottom = tabs(&[
            ("Timeline", "Animation and sequence tracks."),
            ("Console", "Messages and diagnostics."),
        ]);

        let right = tabs(&[
            ("Layers", "Layer hierarchy and visibility."),
            ("Color", "Color selection and recent colors."),
            ("Properties", "Properties of the selected item."),
        ]);

        let workspace = column(split(
            SplitAxis::Vertical,
            &[0.76, 0.24],
            vec![canvas, bottom],
        ));

        Self {
            root: split(
                SplitAxis::Horizontal,
                &[0.21, 0.59, 0.20],
                vec![column(left), workspace, column(right)],
            ),
        }
    }

    pub(super) fn split_weights_mut(&mut self, path: &[NodePathStep]) -> Option<&mut Vec<f32>> {
        let mut node = &mut self.root;

        for step in path {
            node = match (node, *step) {
                (DockNode::Split { children, .. }, NodePathStep::SplitChild(index)) => {
                    children.get_mut(index)?
                }

                (DockNode::Column { content }, NodePathStep::ColumnContent) => content.as_mut(),

                _ => return None,
            };
        }

        match node {
            DockNode::Split { weights, .. } => Some(weights),
            _ => None,
        }
    }

    pub(super) fn single_panel(panel: Panel) -> Self {
        Self {
            root: column(DockNode::Tabs(TabGroup {
                panels: vec![panel],
                active: 0,
            })),
        }
    }

    pub(super) fn single_group(group: TabGroup) -> Self {
        Self {
            root: column(DockNode::Tabs(group)),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        matches!(&self.root, DockNode::Tabs(group) if group.panels.is_empty())
    }

    pub(super) fn take_group(&mut self, path: &[NodePathStep]) -> Option<TabGroup> {
        let group = {
            let mut node = &mut self.root;

            for step in path {
                node = match (node, *step) {
                    (DockNode::Split { children, .. }, NodePathStep::SplitChild(index)) => {
                        children.get_mut(index)?
                    }
                    (DockNode::Column { content }, NodePathStep::ColumnContent) => content.as_mut(),
                    _ => return None,
                };
            }

            let DockNode::Tabs(group) = node else {
                return None;
            };

            std::mem::replace(
                group,
                TabGroup {
                    panels: Vec::new(),
                    active: 0,
                },
            )
        };

        self.prune_empty_nodes();
        Some(group)
    }

    pub(super) fn remove_panel(
        &mut self,
        path: &[NodePathStep],
        panel_index: usize,
    ) -> Option<Panel> {
        let (panel, group_is_empty) = {
            let mut node = &mut self.root;

            for step in path {
                node = match (node, *step) {
                    (DockNode::Split { children, .. }, NodePathStep::SplitChild(index)) => {
                        children.get_mut(index)?
                    }
                    (DockNode::Column { content }, NodePathStep::ColumnContent) => content.as_mut(),
                    _ => return None,
                };
            }

            let DockNode::Tabs(group) = node else {
                return None;
            };

            let panel = group.panels.get(panel_index).copied()?;
            group.panels.remove(panel_index);

            if group.panels.is_empty() {
                group.active = 0;
            } else if panel_index < group.active {
                group.active -= 1;
            } else if group.active >= group.panels.len() {
                group.active = group.panels.len() - 1;
            }

            (panel, group.panels.is_empty())
        };

        if group_is_empty {
            self.prune_empty_nodes();
        }

        Some(panel)
    }

    fn prune_empty_nodes(&mut self) {
        let root = std::mem::replace(&mut self.root, empty_tabs_node());
        self.root = prune_empty_node(root).unwrap_or_else(empty_tabs_node);
    }

    pub(super) fn reorder_panel(
        &mut self,
        path: &[NodePathStep],
        from_index: usize,
        to_index: usize,
    ) -> bool {
        if from_index == to_index {
            return true;
        }

        let mut node = &mut self.root;

        for step in path {
            node = match (node, *step) {
                (DockNode::Split { children, .. }, NodePathStep::SplitChild(index)) => {
                    let Some(child) = children.get_mut(index) else {
                        return false;
                    };
                    child
                }
                (DockNode::Column { content }, NodePathStep::ColumnContent) => content.as_mut(),
                _ => return false,
            };
        }

        let DockNode::Tabs(group) = node else {
            return false;
        };

        if from_index >= group.panels.len() || to_index >= group.panels.len() {
            return false;
        }

        let active_index = group.active;
        let panel = group.panels.remove(from_index);
        group.panels.insert(to_index, panel);

        group.active = if active_index == from_index {
            to_index
        } else if from_index < active_index && to_index >= active_index {
            active_index - 1
        } else if from_index > active_index && to_index <= active_index {
            active_index + 1
        } else {
            active_index
        };

        true
    }
}

fn empty_tabs_node() -> DockNode {
    DockNode::Tabs(TabGroup {
        panels: Vec::new(),
        active: 0,
    })
}

fn prune_empty_node(node: DockNode) -> Option<DockNode> {
    match node {
        DockNode::Tabs(group) => (!group.panels.is_empty()).then_some(DockNode::Tabs(group)),
        DockNode::Column { content } => {
            prune_empty_node(*content).map(|content| DockNode::Column {
                content: Box::new(content),
            })
        }
        DockNode::Split {
            axis,
            weights,
            children,
        } => {
            let weights = if weights.len() == children.len() {
                weights
            } else {
                vec![1.0; children.len()]
            };

            let mut pruned_children = Vec::with_capacity(children.len());
            let mut pruned_weights = Vec::with_capacity(children.len());

            for (child, weight) in children.into_iter().zip(weights) {
                if let Some(child) = prune_empty_node(child) {
                    pruned_children.push(child);
                    pruned_weights.push(weight.max(0.0));
                }
            }

            match pruned_children.len() {
                0 => None,
                1 => pruned_children.pop(),
                _ => {
                    let total_weight: f32 = pruned_weights.iter().sum();

                    if total_weight > f32::EPSILON {
                        for weight in &mut pruned_weights {
                            *weight /= total_weight;
                        }
                    } else {
                        let equal_weight = 1.0 / pruned_weights.len() as f32;
                        pruned_weights.fill(equal_weight);
                    }

                    Some(DockNode::Split {
                        axis,
                        weights: pruned_weights,
                        children: pruned_children,
                    })
                }
            }
        }
    }
}

fn tabs(panels: &[(&'static str, &'static str)]) -> DockNode {
    DockNode::Tabs(TabGroup {
        panels: panels
            .iter()
            .map(|(title, description)| Panel { title, description })
            .collect(),
        active: 0,
    })
}

fn column(content: DockNode) -> DockNode {
    DockNode::Column {
        content: Box::new(content),
    }
}

fn split(axis: SplitAxis, weights: &[f32], children: Vec<DockNode>) -> DockNode {
    let mut normalized_weights: Vec<f32> = if weights.len() == children.len() {
        weights.iter().map(|weight| weight.max(0.0)).collect()
    } else {
        vec![1.0; children.len()]
    };

    let total: f32 = normalized_weights.iter().sum();

    if total > f32::EPSILON {
        for weight in &mut normalized_weights {
            *weight /= total;
        }
    } else if !normalized_weights.is_empty() {
        let equal_weight = 1.0 / normalized_weights.len() as f32;
        normalized_weights.fill(equal_weight);
    }

    DockNode::Split {
        axis,
        weights: normalized_weights,
        children,
    }
}

/// One composition slot in the workspace container.
/// An empty slot is reserved for another composition to be docked later.
#[derive(Clone, Debug)]
pub(super) struct CompositionSlot {
    pub(super) layout: Option<DockLayout>,
    pub(super) weight: f32,
}

/// Contains side-by-side compositions while keeping each tree independent.
#[derive(Clone, Debug)]
pub(super) struct CompositionContainer {
    pub(super) slots: Vec<CompositionSlot>,
}

impl CompositionContainer {
    /// Creates a workspace with one full-width composition.
    pub(super) fn initial() -> Self {
        Self {
            slots: vec![CompositionSlot {
                layout: Some(DockLayout::initial()),
                weight: 1.0,
            }],
        }
    }

    pub(super) fn layout_mut(&mut self, index: usize) -> Option<&mut DockLayout> {
        self.slots.get_mut(index)?.layout.as_mut()
    }

    pub(super) fn insert_floating_layout(&mut self, layout: DockLayout) -> usize {
        if let Some(index) = self.slots.iter().position(|slot| slot.layout.is_none()) {
            self.slots[index].layout = Some(layout);
            return index;
        }

        let index = self.slots.len();

        self.slots.push(CompositionSlot {
            layout: Some(layout),
            weight: 1.0,
        });

        index
    }

    pub(super) fn remove_empty_layout(&mut self, index: usize) -> bool {
        let is_empty = self
            .slots
            .get(index)
            .and_then(|slot| slot.layout.as_ref())
            .is_some_and(DockLayout::is_empty);

        if !is_empty {
            return false;
        }

        self.slots.remove(index);
        true
    }

    pub(super) fn reserve_empty_neighbor(
        &mut self,
        composition_index: usize,
        empty_before: bool,
    ) -> bool {
        if composition_index >= self.slots.len() || self.slots[composition_index].layout.is_none() {
            return false;
        }

        match self.slots.len() {
            1 => {
                if composition_index != 0 {
                    return false;
                }

                let Some(mut composition) = self.slots.pop() else {
                    return false;
                };

                composition.weight = 0.5;

                let empty_slot = CompositionSlot {
                    layout: None,
                    weight: 0.5,
                };

                if empty_before {
                    self.slots.push(empty_slot);
                    self.slots.push(composition);
                } else {
                    self.slots.push(composition);
                    self.slots.push(empty_slot);
                }

                true
            }
            2 => {
                let other_index = 1 - composition_index;

                if self.slots[other_index].layout.is_some() {
                    return false;
                }

                let mut composition = self.slots.remove(composition_index);
                let mut empty_slot = self.slots.remove(0);

                composition.weight = 0.5;
                empty_slot.weight = 0.5;

                if empty_before {
                    self.slots.push(empty_slot);
                    self.slots.push(composition);
                } else {
                    self.slots.push(composition);
                    self.slots.push(empty_slot);
                }

                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taking_group_removes_empty_column_and_redistributes_weights() {
        let mut layout = DockLayout {
            root: split(
                SplitAxis::Horizontal,
                &[0.2, 0.3, 0.5],
                vec![
                    column(tabs(&[("A", "test")])),
                    column(tabs(&[("B", "test")])),
                    column(tabs(&[("C", "test")])),
                ],
            ),
        };

        let group = layout
            .take_group(&[NodePathStep::SplitChild(0), NodePathStep::ColumnContent])
            .expect("source group should be returned");

        assert_eq!(group.panels[0].title, "A");

        let DockNode::Split {
            weights, children, ..
        } = &layout.root
        else {
            panic!("the remaining two columns should stay split");
        };

        assert_eq!(children.len(), 2);
        assert!((weights[0] - 0.375).abs() < 0.001);
        assert!((weights[1] - 0.625).abs() < 0.001);

        let DockNode::Column { content } = &children[0] else {
            panic!("first remaining node should still be a column");
        };
        let DockNode::Tabs(group) = content.as_ref() else {
            panic!("first remaining column should keep its tab group");
        };
        assert_eq!(group.panels[0].title, "B");
    }

    #[test]
    fn removing_last_group_removes_the_empty_composition_slot() {
        let mut container = CompositionContainer {
            slots: vec![CompositionSlot {
                layout: Some(DockLayout {
                    root: column(tabs(&[("A", "test")])),
                }),
                weight: 1.0,
            }],
        };

        let group = container
            .layout_mut(0)
            .and_then(|layout| layout.take_group(&[NodePathStep::ColumnContent]))
            .expect("source group should be returned");

        assert!(
            container
                .layout_mut(0)
                .is_some_and(|layout| layout.is_empty())
        );
        assert!(container.remove_empty_layout(0));
        assert!(container.slots.is_empty());

        let floating_index = container.insert_floating_layout(DockLayout::single_group(group));
        assert_eq!(floating_index, 0);
        assert!(container.layout_mut(floating_index).is_some());
    }

    #[test]
    fn removing_last_panel_collapses_single_child_split() {
        let mut layout = DockLayout {
            root: column(split(
                SplitAxis::Vertical,
                &[0.7, 0.3],
                vec![tabs(&[("A", "test")]), tabs(&[("B", "test")])],
            )),
        };

        let removed = layout
            .remove_panel(
                &[NodePathStep::ColumnContent, NodePathStep::SplitChild(0)],
                0,
            )
            .expect("source panel should be returned");

        assert_eq!(removed.title, "A");

        let DockNode::Column { content } = &layout.root else {
            panic!("outer column should remain");
        };
        let DockNode::Tabs(group) = content.as_ref() else {
            panic!("single-child split should collapse to its remaining tab group");
        };

        assert_eq!(group.panels.len(), 1);
        assert_eq!(group.panels[0].title, "B");
    }
}
