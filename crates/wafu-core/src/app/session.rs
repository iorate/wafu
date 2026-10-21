use std::path::PathBuf;
use std::sync::Arc;

use super::{App, ConfigSource, Effect, Side, SpawnKind};
use crate::command::Command;
use crate::config::{Config, parse_config};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::external::{CommandLine, Expansion};
use crate::history::History;
use crate::listing::{SortKey, SortSpec};
use crate::pane::TargetRule;
use crate::state::{PaneState, StateError, StateFile};
use crate::view::Style;

impl App {
    pub(super) fn start(
        &mut self,
        left: Option<PathBuf>,
        right: Option<PathBuf>,
        config: ConfigSource,
        state: Result<Option<StateFile>, StateError>,
        effects: &mut Vec<Effect>,
    ) {
        self.load_config(config);
        let state = match state {
            Ok(state) => state,
            Err(source) => {
                self.log
                    .push(AppError::StateUnreadable { source }.to_string());
                None
            }
        };
        let (left_state, right_state) = match &state {
            Some(state) => (Some(&state.left), Some(&state.right)),
            None => (None, None),
        };
        self.restore_pane(Side::Left, left_state);
        self.restore_pane(Side::Right, right_state);
        if let Some(state) = &state {
            if let Some(percent) = state.split_percent {
                self.split_percent = percent.clamp(super::SPLIT_MIN, super::SPLIT_MAX);
            }
            if let Some(rows) = state.log_rows {
                self.log
                    .set_viewport_rows(rows.clamp(super::LOG_ROWS_MIN, super::LOG_ROWS_MAX));
            }
        }
        effects.push(Effect::WatchConfig(self.config_path.clone()));
        let saved = |pane: Option<&PaneState>| pane.and_then(|pane| pane.path.clone());
        let left = left
            .or_else(|| saved(left_state))
            .unwrap_or_else(|| self.home.clone());
        let right = right
            .or_else(|| saved(right_state))
            .unwrap_or_else(|| self.home.clone());
        self.resolve(Side::Left, left, None, effects);
        self.resolve(Side::Right, right, None, effects);
    }

    pub(super) fn config_loaded(&mut self, result: Result<String, IoError>) {
        let text = match result {
            Ok(text) => text,
            Err(source) => {
                let path = self.config_path.clone();
                self.log
                    .push(AppError::ConfigUnreadable { path, source }.to_string());
                return;
            }
        };
        if let Some(config) = self.parse(&text) {
            self.apply_config(config);
            self.log.push("config reloaded".to_owned());
        }
    }

    pub(super) fn state_file(&self) -> StateFile {
        let pane_state = |side: Side| {
            let pane = self.pane(side);
            let base = &pane.location().base;
            let sort = pane.sort();
            PaneState {
                path: (!base.as_os_str().is_empty()).then(|| base.clone()),
                sort: Some(sort.key.name().to_owned()),
                descending: Some(sort.descending),
                dirs_first: Some(sort.dirs_first),
                ignore_case: Some(sort.ignore_case),
                show_hidden: Some(pane.show_hidden()),
                history: pane.history().items().to_vec(),
                history_pos: Some(pane.history().pos()),
            }
        };
        StateFile {
            left: pane_state(Side::Left),
            right: pane_state(Side::Right),
            window: None,
            split_percent: Some(self.split_percent),
            log_rows: Some(self.log.viewport_rows()),
        }
    }

    pub(super) fn quit(&mut self, save: bool, effects: &mut Vec<Effect>) {
        if save {
            effects.push(Effect::SaveState(self.state_file()));
        }
        effects.push(Effect::Quit);
    }

    pub(super) fn new_window(&mut self, effects: &mut Vec<Effect>) {
        effects.push(Effect::SpawnNewWindow(self.state_file()));
    }

    pub(super) fn run_external(&mut self, name: &str, effects: &mut Vec<Effect>) {
        let Some(line) = self.config.commands.get(name) else {
            self.log.push(
                AppError::UnknownCommand {
                    command: Command::External(name.to_owned()),
                }
                .to_string(),
            );
            return;
        };
        let expansion = self.expansion();
        Self::spawn(line, &expansion, effects);
    }

    pub(super) fn edit_config(&mut self, effects: &mut Vec<Effect>) {
        let expansion = Expansion {
            paths: vec![self.config_path.clone()],
            ..self.expansion()
        };
        Self::spawn(&self.config.general.editor, &expansion, effects);
    }

    pub(super) fn spawned(&mut self, kind: SpawnKind, error: Option<IoError>) {
        let line = match (kind, error) {
            (SpawnKind::External { program }, None) => format!("started {program}"),
            (SpawnKind::External { program }, Some(source)) => {
                AppError::SpawnFailed { program, source }.to_string()
            }
            (SpawnKind::NewWindow, None) => return,
            (SpawnKind::NewWindow, Some(source)) => {
                AppError::NewWindowFailed { source }.to_string()
            }
        };
        self.log.push(line);
    }

    fn load_config(&mut self, source: ConfigSource) {
        let config = match source {
            ConfigSource::Text(text) => self.parse(&text),
            ConfigSource::Created => {
                self.log
                    .push(format!("created {}", self.config_path.display()));
                None
            }
            ConfigSource::Unreadable(source) => {
                let path = self.config_path.clone();
                self.log
                    .push(AppError::ConfigUnreadable { path, source }.to_string());
                None
            }
        };
        self.apply_config(config.unwrap_or_else(|| Config::default_for(self.os)));
    }

    // Logs every warning; a syntax error is logged and yields nothing, so that
    // the caller keeps what it had.
    fn parse(&mut self, text: &str) -> Option<Config> {
        match parse_config(text, self.os) {
            Ok((config, diagnostics)) => {
                for diagnostic in diagnostics {
                    self.log.push(diagnostic.to_string());
                }
                Some(config)
            }
            Err(error) => {
                self.log.push(
                    AppError::ConfigSyntax {
                        path: self.config_path.clone(),
                        line: error.line,
                        message: error.message,
                    }
                    .to_string(),
                );
                None
            }
        }
    }

    fn apply_config(&mut self, config: Config) {
        self.style = Arc::new(Style::from_config(&config));
        self.config = config;
    }

    // The saved state wins over `[initial]`, which only describes a pane
    // that was never saved.
    fn restore_pane(&mut self, side: Side, state: Option<&PaneState>) {
        let initial = self.config.initial;
        let pane = &mut self.panes[side.index()];
        let Some(state) = state else {
            pane.set_sort(initial.sort);
            pane.set_show_hidden(initial.show_hidden);
            return;
        };
        pane.set_sort(SortSpec {
            key: state
                .sort
                .as_deref()
                .and_then(SortKey::from_name)
                .unwrap_or(initial.sort.key),
            descending: state.descending.unwrap_or(initial.sort.descending),
            dirs_first: state.dirs_first.unwrap_or(initial.sort.dirs_first),
            ignore_case: state.ignore_case.unwrap_or(initial.sort.ignore_case),
        });
        pane.set_show_hidden(state.show_hidden.unwrap_or(initial.show_hidden));
        pane.restore_history(History::restore(
            state.history.clone(),
            state.history_pos.unwrap_or(0),
        ));
    }

    pub(super) fn expansion(&self) -> Expansion {
        let pane = self.active_pane();
        let base = &pane.location().base;
        let full = |entry: &Entry| base.join(&entry.name);
        Expansion {
            cursor_path: pane.cursor_entry().map(full),
            marked_paths: pane.marked_entries().map(full).collect(),
            paths: pane
                .targets(TargetRule::Multiple)
                .into_iter()
                .map(full)
                .collect(),
            dir: base.clone(),
            other_dir: self.pane(self.active.other()).location().base.clone(),
        }
    }

    pub(super) fn spawn(line: &CommandLine, expansion: &Expansion, effects: &mut Vec<Effect>) {
        effects.push(Effect::Spawn {
            program: line.program().to_owned(),
            args: line.expand(expansion),
            cwd: expansion.dir.clone(),
        });
    }
}
