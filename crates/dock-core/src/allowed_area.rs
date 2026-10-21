/// Bit-mask of destination areas a panel is allowed to occupy.
///
/// Combine the constants with the bitwise OR operator to permit multiple areas.
/// A fresh panel permits every supported area by default.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AllowedArea(u8);

impl AllowedArea {
    pub const MAIN_DOCK: Self = Self(0b001);
    pub const FLOATING: Self = Self(0b010);
    pub const ICON_RAIL: Self = Self(0b100);
    pub const ANY: Self = Self(0b111);

    const VALID_BITS: u8 = Self::ANY.0;

    /// Returns true when this policy permits the requested single destination area.
    pub const fn contains(self, area: Self) -> bool {
        area.0 != 0 && (self.0 & area.0) == area.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0 && self.0 & !Self::VALID_BITS == 0
    }
}

impl Default for AllowedArea {
    fn default() -> Self {
        Self::ANY
    }
}

impl std::ops::BitOr for AllowedArea {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::AllowedArea;

    #[test]
    fn combines_individual_areas_and_defaults_to_all() {
        let dock_or_float = AllowedArea::MAIN_DOCK | AllowedArea::FLOATING;
        assert!(dock_or_float.contains(AllowedArea::MAIN_DOCK));
        assert!(dock_or_float.contains(AllowedArea::FLOATING));
        assert!(!dock_or_float.contains(AllowedArea::ICON_RAIL));
        assert_eq!(AllowedArea::default(), AllowedArea::ANY);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn policies_round_trip_through_serde() {
        let policy = AllowedArea::MAIN_DOCK | AllowedArea::ICON_RAIL;
        let encoded = serde_json::to_string(&policy).unwrap();
        assert_eq!(serde_json::from_str::<AllowedArea>(&encoded).unwrap(), policy);
    }
}
