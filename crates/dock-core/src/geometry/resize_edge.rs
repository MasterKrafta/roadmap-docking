/// Edge or corner of an in-window floating group's rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResizeEdge {
    Left,
    Top,
    Right,
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
    fn floating_resize_edges_cover_edges_and_corners() {
        let edges = [
            ResizeEdge::Left,
            ResizeEdge::Top,
            ResizeEdge::Right,
            ResizeEdge::Bottom,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
            ResizeEdge::BottomLeft,
            ResizeEdge::BottomRight,
        ];
        for (index, edge) in edges.iter().enumerate() {
            assert!(!edges[..index].contains(edge));
        }
    }
}
