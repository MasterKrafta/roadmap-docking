use std::collections::HashMap;

use dock_core::{
    CommandError, CommandOutcome, DockCommand, InitialLayoutError, PanelId, PanelKey,
    WorkspaceDiagnostic, WorkspaceFile, WorkspaceFileError, WorkspaceLoadOptions,
    create_initial_layout,
};
use gpui::{AnyView, App, AppContext, Context, Window};

use super::{DockController, DockEvent, PanelCommandError};

impl DockController {
    /// Applies one persistent or transient command through the shared history facade.
    pub fn dispatch(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();
        let activated = match &command {
            DockCommand::ActivateTab { group, panel } => Some((*group, *panel)),
            _ => None,
        };

        match self.history.apply(&mut self.layout, command, &self.metrics) {
            Ok(outcome) => {
                self.finish_success(before_panels, previous_version, activated.into_iter());
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

    /// Applies several commands atomically as one history step.
    pub fn dispatch_sequence(
        &mut self,
        commands: Vec<DockCommand>,
    ) -> Result<Vec<CommandOutcome>, CommandError> {
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
                Ok(outcomes)
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    pub fn undo(&mut self) -> Result<bool, CommandError> {
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

    /// Serializes the current core layout using the versioned workspace format.
    pub fn save_workspace(&self) -> Result<String, WorkspaceFileError> {
        WorkspaceFile::from_layout(&self.layout, &self.metrics)?.to_json_pretty()
    }

    /// Loads a validated workspace and starts a fresh history/cache lineage.
    pub fn load_workspace(
        &mut self,
        json: &str,
    ) -> Result<Vec<WorkspaceDiagnostic>, WorkspaceFileError> {
        let descriptors = self.registry.descriptors_for_core();
        let loaded = WorkspaceFile::load_json(
            json,
            &descriptors,
            &self.metrics,
            WorkspaceLoadOptions::new(Some(self.viewport)),
        )?;
        let diagnostics = loaded.diagnostics;
        self.replace_layout(loaded.layout);
        self.pending_events.push_back(DockEvent::WorkspaceLoaded {
            diagnostics: diagnostics.clone(),
        });
        Ok(diagnostics)
    }

    /// Recreates the profile's baseline layout from the registered panel descriptors.
    ///
    /// For the Photoshop profile, dock-core requires the four baseline panel keys:
    /// layers, color, properties, and history. The regular application startup can
    /// still use an empty Layout; reset explicitly restores the baseline composition.
    pub fn reset_workspace(&mut self) -> Result<(), InitialLayoutError> {
        let descriptors = self.registry.descriptors_for_core();
        let layout =
            create_initial_layout(self.layout.profile(), &descriptors, self.metrics)?;
        self.replace_layout(layout);
        Ok(())
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
            self.interaction = None;
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

    fn finish_failure(&mut self, error: &CommandError) {
        self.last_error = Some(error.clone());
        self.pending_events
            .push_back(DockEvent::CommandFailed(error.clone()));
        tracing::warn!(error = %error, "dock controller command failed");
    }

    fn replace_layout(&mut self, layout: dock_core::Layout) {
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
        self.rects_key = None;
        self.last_error = None;
        self.pending_events.push_back(DockEvent::LayoutChanged {
            version: self.layout.version(),
        });
    }
}
