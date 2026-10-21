use crate::Size;

/// The effective value produced by a constrained resize command.
///
/// A resize may be clamped to panel, column, or window constraints; the command
/// outcome reports the value that was actually accepted rather than the
/// originally requested value.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CommandValue {
    /// Effective width of a resized column, in logical pixels.
    ColumnWidth(f32),
    /// Effective signed movement of a main-column boundary, in logical pixels.
    ColumnBoundaryDelta(f32),
    /// Effective signed movement of a boundary between adjacent groups.
    GroupBoundaryDelta(f32),
    /// Ratios of groups after a constrained resize or reset.
    GroupRatios(Vec<f32>),
    /// Effective floating-window size after constraints.
    FloatingSize(Size),
}

#[cfg(test)]
mod tests {
    use super::CommandValue;
    use crate::Size;

    #[test]
    fn command_values_keep_their_kind() {
        assert!(matches!(
            CommandValue::ColumnWidth(280.0),
            CommandValue::ColumnWidth(value) if value == 280.0
        ));
        assert_ne!(
            CommandValue::ColumnWidth(280.0),
            CommandValue::ColumnBoundaryDelta(280.0)
        );
        assert_eq!(
            CommandValue::GroupBoundaryDelta(12.0),
            CommandValue::GroupBoundaryDelta(12.0)
        );
        assert_eq!(
            CommandValue::FloatingSize(Size::new(400.0, 300.0)),
            CommandValue::FloatingSize(Size::new(400.0, 300.0))
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_effective_value() {
        let value = CommandValue::GroupRatios(vec![0.3, 0.7]);
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<CommandValue>(&encoded).unwrap(), value);
    }
}
