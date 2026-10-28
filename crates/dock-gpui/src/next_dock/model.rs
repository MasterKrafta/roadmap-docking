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
