/// Edge or corner of a floating window that can initiate a resize gesture.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResizeEdge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[cfg(test)]
mod tests {
    use super::ResizeEdge;

    #[test]
    fn all_edges_and_corners_are_distinct() {
        let edges = [
            ResizeEdge::Left,
            ResizeEdge::Right,
            ResizeEdge::Top,
            ResizeEdge::Bottom,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
            ResizeEdge::BottomLeft,
            ResizeEdge::BottomRight,
        ];
        for (index, edge) in edges.iter().enumerate() {
            assert!(!edges[index + 1..].contains(edge));
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn edge_round_trips_through_serde() {
        for edge in [
            ResizeEdge::Left,
            ResizeEdge::Right,
            ResizeEdge::Top,
            ResizeEdge::Bottom,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
            ResizeEdge::BottomLeft,
            ResizeEdge::BottomRight,
        ] {
            let encoded = serde_json::to_string(&edge).unwrap();
            assert_eq!(serde_json::from_str::<ResizeEdge>(&encoded).unwrap(), edge);
        }
    }
}
