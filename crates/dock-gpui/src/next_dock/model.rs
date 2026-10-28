//! Minimal independent tree model used by the next dock prototype.

/// Orientation of a split between two dock nodes.
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

/// The complete tree of dock areas.
#[derive(Clone, Debug)]
pub(super) enum DockNode {
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
    Tabs(TabGroup),
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

        Self {
            root: split(
                SplitAxis::Horizontal,
                0.21,
                left,
                split(
                    SplitAxis::Horizontal,
                    0.75,
                    split(SplitAxis::Vertical, 0.76, canvas, bottom),
                    right,
                ),
            ),
        }
    }
}

fn tabs(panels: &[(&'static str, &'static str)]) -> DockNode {
    DockNode::Tabs(TabGroup {
        panels: panels
            .iter()
            .map(|(title, description)| Panel {
                title,
                description,
            })
            .collect(),
        active: 0,
    })
}

fn split(axis: SplitAxis, ratio: f32, first: DockNode, second: DockNode) -> DockNode {
    DockNode::Split {
        axis,
        ratio: ratio.clamp(0.1, 0.9),
        first: Box::new(first),
        second: Box::new(second),
    }
}
