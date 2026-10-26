use super::helpers::*;
use super::*;

const MAX_LAYOUT_TREE_DEPTH: usize = 256;

impl Layout {
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
        true
    }
}
