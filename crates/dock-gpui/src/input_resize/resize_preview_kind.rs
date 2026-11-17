/// Visual-only geometry kind for the active resize preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizePreviewKind {
    SplitterLine,
    BoundsOutline,
}
