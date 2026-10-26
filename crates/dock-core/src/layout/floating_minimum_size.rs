use std::collections::HashMap;

use super::*;

impl Layout {
    /// Computes a floating window's minimum size, including reserved title rows.
    ///
    /// A collapsed root Tabs group is rendered as a strip, but its floating
    /// window must still fit the minimum panel content size. Composite roots
    /// also reserve a title row above collapsed first branches, matching the
    /// geometry emitted by compute_rects.
    pub(crate) fn minimum_floating_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        self.minimum_floating_size_with_cache(node_id, metrics, &mut HashMap::new())
    }

    /// Computes floating minimum size while reusing per-node minimums in one
    /// immutable geometry operation. The caller owns the cache, so it cannot
    /// survive a mutation or accidentally mix layouts or metrics.
    pub(crate) fn minimum_floating_size_with_cache(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
        cache: &mut HashMap<NodeId, Size>,
    ) -> Result<Size, CommandError> {
        let minimum = match self.nodes.get(node_id) {
            Some(Node::Tabs(tabs)) if tabs.collapsed && !tabs.children.is_empty() => {
                let mut minimum = Size::ZERO;
                for child in &tabs.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    minimum.width = minimum.width.max(child_size.width);
                    minimum.height = minimum.height.max(child_size.height);
                }
                minimum
            }
            Some(Node::Column(column))
                if column.groups.len() > 1
                    && self.floating_branch_starts_collapsed(node_id) =>
            {
                let base = self.minimum_node_size_with_cache(node_id, metrics, cache)?;
                Size::new(base.width, base.height + metrics.header_height)
            }
            Some(Node::Split(split)) if split.axis == Axis::Y && self.floating_branch_starts_collapsed(node_id) => {
                let base = self.minimum_node_size_with_cache(node_id, metrics, cache)?;
                Size::new(base.width, base.height + metrics.header_height)
            }
            Some(Node::Split(split)) if split.axis == Axis::X => {
                let mut width = 0.0_f32;
                let mut height = 0.0_f32;
                for child in &split.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    width += child_size.width;
                    let title_reserve = if self.floating_branch_starts_collapsed(*child) {
                        metrics.header_height
                    } else {
                        0.0
                    };
                    height = height.max(child_size.height + title_reserve);
                }
                width += metrics.splitter_thickness
                    * split.children.len().saturating_sub(1) as f32;
                Size::new(width, height)
            }
            Some(_) => self.minimum_node_size_with_cache(node_id, metrics, cache)?,
            None => return Err(CommandError::MissingNode { node: node_id }),
        };

        if !minimum.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed floating minimum size overflowed",
            });
        }
        if minimum.width < 0.0 || minimum.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed floating minimum size has invalid dimensions",
            });
        }
        Ok(minimum)
    }

    /// Whether the first visible vertical branch starts with a collapsed group.
    ///
    /// Horizontal floating compositions reserve a window-title row above such
    /// branches. A vertical split follows its first child to identify that row.
    pub(crate) fn floating_branch_starts_collapsed(&self, node_id: NodeId) -> bool {
        match self.nodes.get(node_id) {
            Some(Node::Tabs(tabs)) => tabs.collapsed,
            Some(Node::Column(column)) => column.groups.first().is_some_and(|group| {
                matches!(self.nodes.get(*group), Some(Node::Tabs(tabs)) if tabs.collapsed)
            }),
            Some(Node::Split(split)) if split.axis == Axis::Y => split
                .children
                .first()
                .is_some_and(|child| self.floating_branch_starts_collapsed(*child)),
            _ => false,
        }
    }
}
