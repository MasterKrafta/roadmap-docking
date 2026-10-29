//! Minimal independent tree model used by the next dock prototype.

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

/// A node in the independent dock layout.
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

/// Owns only the new prototype tree; it has no dependency on the previous dock model.
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

    pub(super) fn remove_panel(
        &mut self,
        path: &[NodePathStep],
        panel_index: usize,
    ) -> Option<Panel> {
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

        Some(panel)
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
