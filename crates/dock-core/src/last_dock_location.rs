use crate::DockSide;

/// Runtime/session preference for where a panel last lived in a main dock.
///
/// This stores ordinal coordinates rather than NodeId handles, because closing
/// The type intentionally does not implement Serde and is omitted from
/// persistent workspace files because it is a session-only hint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LastDockLocation {
    side: DockSide,
    column_index: usize,
    group_index: usize,
}

impl LastDockLocation {
    pub const fn new(side: DockSide, column_index: usize, group_index: usize) -> Self {
        Self {
            side,
            column_index,
            group_index,
        }
    }

    pub const fn side(self) -> DockSide {
        self.side
    }

    pub const fn column_index(self) -> usize {
        self.column_index
    }

    pub const fn group_index(self) -> usize {
        self.group_index
    }
}

#[cfg(test)]
mod tests {
    use super::LastDockLocation;
    use crate::DockSide;

    #[test]
    fn location_is_an_ordinal_runtime_preference() {
        let location = LastDockLocation::new(DockSide::Right, 1, 2);
        assert_eq!(location.side(), DockSide::Right);
        assert_eq!(location.column_index(), 1);
        assert_eq!(location.group_index(), 2);
    }
}
