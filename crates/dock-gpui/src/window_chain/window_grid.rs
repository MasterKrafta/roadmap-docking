use std::collections::{HashSet, VecDeque};

use super::{WindowGridDirection, WindowGridLink, WindowGridMember};

/// Connected component of independent Floating windows.
///
/// `members` preserves a deterministic traversal/insertion order for existing
/// move and native Z-order operations. `links` carries the actual 2D topology.
#[derive(Clone, Debug, Default)]
pub(super) struct WindowGrid {
    pub(super) members: Vec<WindowGridMember>,
    pub(super) links: Vec<WindowGridLink>,
}

impl WindowGrid {
    pub(super) fn single(group: WindowGridMember) -> Self {
        Self {
            members: vec![group],
            links: Vec::new(),
        }
    }

    pub(super) fn direction_between(
        &self,
        from: WindowGridMember,
        to: WindowGridMember,
    ) -> Option<WindowGridDirection> {
        self.links.iter().find_map(|link| {
            if link.from == from && link.to == to {
                Some(link.direction)
            } else if link.from == to && link.to == from {
                Some(link.direction.opposite())
            } else {
                None
            }
        })
    }

    /// Chooses the member on the side of this component that faces the target.
    ///
    /// When a grid has a single exposed member on that edge, it is used even
    /// when a different member initiated the drag. For a multi-contact edge,
    /// prefer the grabbed member if it is itself on that boundary; otherwise
    /// the placement is ambiguous and must not be committed.
    pub(super) fn boundary_member(
        &self,
        outward_side: WindowGridDirection,
        preferred: WindowGridMember,
    ) -> Option<WindowGridMember> {
        let candidates = self
            .members
            .iter()
            .copied()
            .filter(|member| {
                !self.links.iter().any(|link| {
                    if link.from == *member {
                        link.direction == outward_side
                    } else if link.to == *member {
                        link.direction.opposite() == outward_side
                    } else {
                        false
                    }
                })
            })
            .collect::<Vec<_>>();

        if candidates.contains(&preferred) {
            Some(preferred)
        } else if candidates.len() == 1 {
            candidates.first().copied()
        } else {
            None
        }
    }

    /// Removes a member and returns a possible straight-through replacement link.
    ///
    /// Reconnection is optional because a replacement edge must be checked
    /// against the surviving windows' native geometry before it is committed.
    pub(super) fn remove_member_inner(
        &mut self,
        group: WindowGridMember,
        reconnect: bool,
    ) -> Option<WindowGridLink> {
        if !self.members.contains(&group) {
            return None;
        }

        let incident = self
            .links
            .iter()
            .filter_map(|link| {
                if link.from == group {
                    Some((link.to, link.direction))
                } else if link.to == group {
                    Some((link.from, link.direction.opposite()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let replacement = if let [(first, first_side), (second, second_side)] =
            incident.as_slice()
            && first_side.opposite() == *second_side
        {
            match first_side {
                WindowGridDirection::Left => {
                    WindowGridLink::new(*first, *second, WindowGridDirection::Right)
                }
                WindowGridDirection::Above => {
                    WindowGridLink::new(*first, *second, WindowGridDirection::Below)
                }
                WindowGridDirection::Right => {
                    WindowGridLink::new(*second, *first, WindowGridDirection::Right)
                }
                WindowGridDirection::Below => {
                    WindowGridLink::new(*second, *first, WindowGridDirection::Below)
                }
            }
        } else {
            None
        };

        self.members.retain(|member| *member != group);
        self.links
            .retain(|link| link.from != group && link.to != group);

        if reconnect
            && let Some(replacement) = replacement
            && self.direction_between(replacement.from, replacement.to).is_none()
        {
            self.links.push(replacement);
        }

        replacement
    }

    pub(super) fn remove_member(&mut self, group: WindowGridMember) -> bool {
        if !self.members.contains(&group) {
            return false;
        }
        // Direct detach is fail-safe: reconnect only after the manager validates
        // the resulting native geometry through a prune candidate.
        self.remove_member_inner(group, false);
        true
    }

    /// Splits a graph after removal into its still-connected components.
    pub(super) fn connected_components(&self) -> Vec<Self> {
        let mut visited = HashSet::new();
        let mut components = Vec::new();

        for start in &self.members {
            if !visited.insert(*start) {
                continue;
            }

            let mut component_members = HashSet::from([*start]);
            let mut pending = VecDeque::from([*start]);
            while let Some(current) = pending.pop_front() {
                for link in &self.links {
                    let neighbour = if link.from == current {
                        Some(link.to)
                    } else if link.to == current {
                        Some(link.from)
                    } else {
                        None
                    };

                    if let Some(neighbour) = neighbour
                        && visited.insert(neighbour)
                    {
                        component_members.insert(neighbour);
                        pending.push_back(neighbour);
                    }
                }
            }

            let members = self
                .members
                .iter()
                .copied()
                .filter(|member| component_members.contains(member))
                .collect::<Vec<_>>();
            let links = self
                .links
                .iter()
                .copied()
                .filter(|link| {
                    component_members.contains(&link.from)
                        && component_members.contains(&link.to)
                })
                .collect::<Vec<_>>();
            if !members.is_empty() {
                components.push(Self { members, links });
            }
        }

        components
    }

    /// Each stored grid is a connected, acyclic graph with unique member IDs.
    pub(super) fn is_valid(&self) -> bool {
        if self.members.len() < 2 || self.links.len() != self.members.len() - 1 {
            return false;
        }

        let members = self.members.iter().copied().collect::<HashSet<_>>();
        if members.len() != self.members.len() {
            return false;
        }

        let mut edges = HashSet::new();
        for link in &self.links {
            if link.from == link.to
                || !members.contains(&link.from)
                || !members.contains(&link.to)
            {
                return false;
            }

            if edges.contains(&(link.to, link.from))
                || !edges.insert((link.from, link.to))
            {
                return false;
            }
        }

        self.connected_components().len() == 1
    }
}
