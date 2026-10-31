use std::collections::HashMap;

use dock_core::{CommandError, CommandOutcome, DockCommand, PanelId, PanelKey};
use gpui::{AnyView, App, AppContext, Context, Window};

use super::{DockController, DockEvent, PanelCommandError};

impl DockController {
    /// Applies one persistent or transient command through the shared history facade.
    pub fn dispatch(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        self.dispatch_with_pointer_policy(command, false)
    }

    /// Applies a resize command owned by the active pointer-resize pipeline.
    ///
    /// Unlike ordinary external dispatch, this command is allowed to advance the
    /// active ResizeInteraction's expected Layout revision.
    fn dispatch_resize(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        self.dispatch_with_pointer_policy(command, true)
    }

    fn dispatch_with_pointer_policy(
        &mut self,
        command: DockCommand,
        from_resize_pipeline: bool,
    ) -> Result<CommandOutcome, CommandError> {
        let is_resize = is_resize_command(&command);
        let resize_owns_command =
            from_resize_pipeline && self.interaction.is_some() && is_resize;
        if self.has_active_pointer_interaction()
            && !resize_owns_command
            && let Err(error) = self.cancel_pointer_interaction()
        {
            tracing::warn!(%error, "could not cancel stale dock interaction before command");
        }
        let resize_owns_command =
            from_resize_pipeline && self.interaction.is_some() && is_resize;
        let resize_hit = if is_resize {
            self.interaction.map(|interaction| interaction.hit)
        } else {
            None
        };
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();
        let activated = match &command {
            DockCommand::ActivateTab { group, panel } => Some((*group, *panel)),
            _ => None,
        };

        match self.history.apply(&mut self.layout, command, &self.metrics) {
            Ok(outcome) => {
                self.finish_success(before_panels, previous_version, activated.into_iter());
                if resize_owns_command {
                    self.record_resize_layout_version();
                }
                if is_resize {
                    self.publish_resize_outcomes(resize_hit, std::slice::from_ref(&outcome));
                }
                Ok(outcome)
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    /// Semantic entry point for flyout state and other Core-declared transient
    /// commands. It deliberately uses the same UndoRedoHistory::apply path:
    /// Core decides whether a command creates a persistent revision.
    pub fn dispatch_transient(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        if !matches!(
            &command,
            DockCommand::OpenFlyout { .. }
                | DockCommand::CloseFlyout
                | DockCommand::DismissFlyout
                | DockCommand::ResizeFlyout { .. }
                | DockCommand::SetFlyoutAutoCollapse { .. }
        ) {
            let error = CommandError::InvalidValue {
                field: "dispatch_transient",
                reason: "the command is not a Core-declared transient command",
            };
            self.finish_failure(&error);
            return Err(error);
        }
        self.dispatch(command)
    }

    /// Dispatches a Core-declared transient command and notifies the owning GPUI entity.
    pub fn dispatch_transient_in(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<CommandOutcome, CommandError> {
        let result = self.dispatch_transient(command);
        self.flush_events(cx);
        cx.notify();
        result
    }

    /// Dispatches a flyout-resize command from the active pointer-resize handler.
    pub(crate) fn dispatch_resize_transient_in(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<CommandOutcome, CommandError> {
        if !matches!(command, DockCommand::ResizeFlyout { .. }) {
            return self.dispatch_transient_in(command, cx);
        }
        let result = self.dispatch_resize(command);
        self.flush_events(cx);
        cx.notify();
        result
    }

    /// Dispatches a persistent resize command from the active pointer-resize handler.
    pub(crate) fn dispatch_resize_in(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<CommandOutcome, CommandError> {
        let result = self.dispatch_resize(command);
        self.flush_events(cx);
        cx.notify();
        result
    }

    /// Applies several commands atomically as one history step.
    pub fn dispatch_sequence(
        &mut self,
        commands: Vec<DockCommand>,
    ) -> Result<Vec<CommandOutcome>, CommandError> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }
        let is_resize_sequence = commands.iter().all(is_resize_command);
        let resize_hit = if is_resize_sequence {
            self.interaction.map(|interaction| interaction.hit)
        } else {
            None
        };
        let owns_resize_gesture = is_resize_sequence && resize_hit.is_some();
        if self.has_active_pointer_interaction()
            && !owns_resize_gesture
            && let Err(error) = self.cancel_pointer_interaction()
        {
            tracing::warn!(%error, "could not cancel dock interaction before command sequence");
        }
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();
        let activated = commands
            .iter()
            .filter_map(|command| match command {
                DockCommand::ActivateTab { group, panel } => Some((*group, *panel)),
                _ => None,
            })
            .collect::<Vec<_>>();

        match self
            .history
            .apply_sequence(&mut self.layout, commands, &self.metrics)
        {
            Ok(outcomes) => {
                self.finish_success(before_panels, previous_version, activated.into_iter());
                if owns_resize_gesture {
                    self.record_resize_layout_version();
                }
                if is_resize_sequence {
                    self.publish_resize_outcomes(resize_hit, &outcomes);
                }
                Ok(outcomes)
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    /// Dispatches a sequence and publishes its events to GPUI subscribers.
    pub fn dispatch_sequence_in(
        &mut self,
        commands: Vec<DockCommand>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<CommandOutcome>, CommandError> {
        let result = self.dispatch_sequence(commands);
        self.flush_events(cx);
        cx.notify();
        result
    }

    pub fn undo(&mut self) -> Result<bool, CommandError> {
        if self.has_active_pointer_interaction()
            && let Err(error) = self.cancel_pointer_interaction()
        {
            tracing::warn!(%error, "could not cancel dock interaction before undo");
        }
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();
        match self.history.undo(&mut self.layout, &self.metrics) {
            Ok(changed) => {
                if changed {
                    self.finish_success(before_panels, previous_version, std::iter::empty());
                }
                Ok(changed)
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    pub fn redo(&mut self) -> Result<bool, CommandError> {
        if self.has_active_pointer_interaction()
            && let Err(error) = self.cancel_pointer_interaction()
        {
            tracing::warn!(%error, "could not cancel dock interaction before redo");
        }
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();
        match self.history.redo(&mut self.layout, &self.metrics) {
            Ok(changed) => {
                if changed {
                    self.finish_success(before_panels, previous_version, std::iter::empty());
                }
                Ok(changed)
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    pub fn open_panel(
        &mut self,
        key: &PanelKey,
    ) -> Result<CommandOutcome, PanelCommandError> {
        let constraints = self
            .registry
            .get(key)
            .ok_or_else(|| PanelCommandError::UnknownPanel(key.clone()))?
            .constraints();
        self.dispatch(DockCommand::OpenPanel {
            key: key.clone(),
            constraints,
            target: None,
        })
        .map_err(PanelCommandError::Command)
    }

    pub fn close_panel(&mut self, panel: PanelId) -> Result<CommandOutcome, CommandError> {
        self.dispatch(DockCommand::ClosePanel { panel })
    }

    /// Closes one existing instance of key; otherwise opens a new instance.
    /// Use open_panel directly when a non-singleton panel should be duplicated.
    pub fn toggle_panel(
        &mut self,
        key: &PanelKey,
    ) -> Result<CommandOutcome, PanelCommandError> {
        let existing = self
            .layout
            .panel_instances()
            .find_map(|(id, instance)| (instance.key == *key).then_some(id));
        match existing {
            Some(panel) => self
                .close_panel(panel)
                .map_err(PanelCommandError::Command),
            None => self.open_panel(key),
        }
    }

    /// Returns a cloned cached view, creating it lazily on first access.
    ///
    /// An unknown panel key is represented by a cached MissingPanelPlaceholder.
    /// None is reserved for a stale PanelId that is no longer in the Layout.
    pub fn panel_view(
        &mut self,
        panel: PanelId,
        window: &mut Window,
        app: &mut App,
    ) -> Option<AnyView> {
        if let Some(view) = self.panel_views.get(&panel) {
            return Some(view.clone());
        }
        let key = self.layout.panel_instance(panel)?.key.clone();
        let view: AnyView = match self.registry.get(&key) {
            Some(descriptor) => descriptor.create_view(window, app),
            None => {
                let missing_key = key.clone();
                app.new(|_| crate::MissingPanelPlaceholder::new(missing_key))
                    .into()
            }
        };
        self.panel_views.insert(panel, view.clone());
        Some(view)
    }

    /// Emits events queued by plain controller methods. Call this from the
    /// controller entity's GPUI update context.
    pub fn flush_events(&mut self, cx: &mut Context<Self>) {
        for event in std::mem::take(&mut self.pending_events) {
            cx.emit(event);
        }
    }

    /// Convenience wrapper for a UI dispatch that also publishes events and
    /// notifies the controller entity after the command is processed.
    pub fn dispatch_in(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<CommandOutcome, CommandError> {
        let result = self.dispatch(command);
        self.flush_events(cx);
        cx.notify();
        result
    }

    /// Publishes a Core drop-resolution failure without applying a command.
    ///
    /// Resolution is pure; this records the error through the same controller
    /// event/last-error path used by rejected command dispatches.
    pub(crate) fn report_drop_resolution_error_in(
        &mut self,
        error: CommandError,
        cx: &mut Context<Self>,
    ) {
        self.finish_failure(&error);
        self.flush_events(cx);
        cx.notify();
    }

    fn panel_keys(&self) -> HashMap<PanelId, PanelKey> {
        self.layout
            .panel_instances()
            .map(|(id, instance)| (id, instance.key.clone()))
            .collect()
    }

    fn finish_success(
        &mut self,
        before_panels: HashMap<PanelId, PanelKey>,
        previous_version: u64,
        activated: impl Iterator<Item = (dock_core::NodeId, PanelId)>,
    ) {
        let after_panels = self.panel_keys();

        for (panel, key) in &before_panels {
            if !after_panels.contains_key(panel) {
                self.pending_events.push_back(DockEvent::PanelClosed {
                    panel: *panel,
                    key: key.clone(),
                });
            }
        }
        for (panel, key) in &after_panels {
            if !before_panels.contains_key(panel) {
                self.pending_events.push_back(DockEvent::PanelOpened {
                    panel: *panel,
                    key: key.clone(),
                });
            }
        }
        if self.layout.version() != previous_version {
            self.rects_key = None;
            self.drag.clear_resolver();
            self.pending_events.push_back(DockEvent::LayoutChanged {
                version: self.layout.version(),
            });
            for (group, panel) in activated {
                self.pending_events
                    .push_back(DockEvent::PanelActivated { group, panel });
            }
        }
        self.panel_views
            .retain(|panel, _| after_panels.contains_key(panel));
        self.last_error = None;
    }

    fn publish_resize_outcomes(
        &mut self,
        hit: Option<dock_core::HitResult>,
        outcomes: &[CommandOutcome],
    ) {
        let actual_values = outcomes
            .iter()
            .filter_map(|outcome| outcome.actual_value.clone())
            .collect::<Vec<_>>();
        if actual_values.is_empty() {
            return;
        }
        let version = outcomes
            .last()
            .map(|outcome| outcome.new_version)
            .unwrap_or_else(|| self.layout.version());
        tracing::debug!(
            ?hit,
            ?actual_values,
            version,
            "dock resize applied with Core-constrained values"
        );
        self.pending_events.push_back(DockEvent::ResizeApplied {
            hit,
            actual_values,
            version,
        });
    }

    fn finish_failure(&mut self, error: &CommandError) {
        self.last_error = Some(error.clone());
        self.pending_events
            .push_back(DockEvent::CommandFailed(error.clone()));
        tracing::warn!(error = %error, "dock controller command failed");
    }

    pub(crate) fn replace_layout(&mut self, layout: dock_core::Layout) {
        let before_panels = self.panel_keys();
        self.layout = layout;
        let after_panels = self.panel_keys();

        for (panel, key) in &before_panels {
            self.pending_events.push_back(DockEvent::PanelClosed {
                panel: *panel,
                key: key.clone(),
            });
        }
        for (panel, key) in &after_panels {
            self.pending_events.push_back(DockEvent::PanelOpened {
                panel: *panel,
                key: key.clone(),
            });
        }

        self.history.clear();
        self.panel_views.clear();
        self.drag.cancel_gesture();
        self.interaction = None;
        self.resize_frame.latest = None;
        self.rects_key = None;
        self.last_error = None;
        self.pending_events.push_back(DockEvent::LayoutChanged {
            version: self.layout.version(),
        });
    }
}

fn is_resize_command(command: &DockCommand) -> bool {
    matches!(
        command,
        DockCommand::ResizeGroups { .. }
            | DockCommand::ResizeColumn { .. }
            | DockCommand::ResizeColumnBoundary { .. }
            | DockCommand::ResizeSplitColumnBoundary { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::ResizeFlyout { .. }
            | DockCommand::ResetColumnRatios { .. }
            | DockCommand::ResetMainColumnRatios { .. }
    )
}

