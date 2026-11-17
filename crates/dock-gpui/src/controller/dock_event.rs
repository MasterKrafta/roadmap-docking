use dock_core::{CommandError, CommandValue, HitResult, PanelId, PanelKey};

/// Events emitted by the controller entity after successful model operations.
#[derive(Clone, Debug)]
pub enum DockEvent {
    LayoutChanged {
        version: u64,
    },
    /// Core geometry and cached hit-test snapshots use a new DockMetrics revision.
    MetricsChanged {
        revision: u64,
    },
    PanelOpened {
        panel: PanelId,
        key: PanelKey,
    },
    PanelClosed {
        panel: PanelId,
        key: PanelKey,
    },
    PanelActivated {
        group: dock_core::NodeId,
        panel: PanelId,
    },
    /// Effective values returned by Core after one resize operation.
    ResizeApplied {
        hit: Option<HitResult>,
        actual_values: Vec<CommandValue>,
        version: u64,
    },
    CommandFailed(CommandError),
}
