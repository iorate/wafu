use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{
    App, ColorScheme, ConfigContent, ConfigFile, ConfigSource, Effect, Os, Side, SpawnKind,
    SpawnName, Start,
};
use crate::command::Command;
use crate::compare::{CompareCondition, CompareSpec};
use crate::config::{Config, parse_config};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::external::{CommandLine, Expansion, format_line, quote_arg, quote_program};
use crate::history::History;
use crate::listing::{SortKey, SortSpec};
use crate::location::Location;
use crate::pane::Pane;
use crate::popup::{InputKind, InputPopup, Popup};
use crate::state::{CompareState, PaneState, StateFile};
use crate::view::{Style, extension_colors};

impl App {
    pub(super) fn start(&mut self, start: Start, effects: &mut Vec<Effect>) {
        let Start {
            left,
            right,
            home,
            os,
            elevated,
            config_path,
            config,
            state,
            state_read_from,
            inherited,
        } = start;
        self.home = home;
        self.os = os;
        self.elevated = elevated;
        self.config_path = config_path;
        self.panes = [
            Pane::new(Location::new(PathBuf::new())),
            Pane::new(Location::new(PathBuf::new())),
        ];
        self.log
            .push(format!("Started wafu {}", env!("CARGO_PKG_VERSION")));
        self.load_config(config);
        let state = match state {
            Ok(state) => state,
            Err(source) => {
                self.log.push(
                    AppError::StateUnreadable {
                        path: state_read_from,
                        source,
                    }
                    .to_string(),
                );
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
            self.compare = restore_compare(&state.compare);
        }
        effects.push(Effect::WatchConfig(self.config_path.clone()));
        effects.push(Effect::WatchColorScheme);
        // Desktop launchers on macOS and Linux start apps without what the
        // shell's rc files set up.
        // An inherited instance already has the environment of the one that
        // started it; when that one failed to read it, trying again in every
        // new window would only repeat the error. The setting is read here
        // only, since the shell runs once per process.
        if self.os != Os::Windows && !inherited && self.config.programs.use_login_shell_env {
            effects.push(Effect::LoadShellEnv);
            self.held_spawns = Some(Vec::new());
        }
        let saved = |pane: Option<&PaneState>| pane.and_then(|pane| pane.path.clone());
        let left = left
            .or_else(|| saved(left_state))
            .unwrap_or_else(|| self.home.clone());
        let right = right
            .or_else(|| saved(right_state))
            .unwrap_or_else(|| self.home.clone());
        self.resolve(Side::Left, &left, true, None, effects);
        self.resolve(Side::Right, &right, true, None, effects);
    }

    pub(super) fn config_loaded(&mut self, files: Vec<ConfigFile>) {
        if let Some(config) = self.parse(files) {
            self.apply_config(config);
            self.log.push("Reloaded config".to_owned());
        }
    }

    pub(super) fn color_scheme_changed(&mut self, scheme: ColorScheme) {
        self.color_scheme = Some(scheme);
        self.restyle();
    }

    pub(super) fn shell_env_loaded(&mut self, error: Option<IoError>, effects: &mut Vec<Effect>) {
        if let Some(source) = error {
            self.log
                .push(AppError::ShellEnvFailed { source }.to_string());
        }
        effects.extend(self.held_spawns.take().into_iter().flatten());
    }

    pub(super) fn hold_spawns(&mut self, effects: &mut Vec<Effect>) {
        if let Some(held) = &mut self.held_spawns {
            held.extend(effects.extract_if(.., |effect| {
                matches!(effect, Effect::Spawn { .. } | Effect::SpawnNewWindow { .. })
            }));
        }
    }

    pub(super) fn state_file(&self) -> StateFile {
        let pane_state = |side: Side| {
            let pane = self.pane(side);
            let dir = pane.location().dir();
            let sort = pane.sort();
            PaneState {
                // A pane that has shown nothing yet saves no path, even while
                // its first listing is on the way.
                path: (!dir.as_os_str().is_empty()).then(|| dir.to_path_buf()),
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
            compare: CompareState {
                condition: Some(self.compare.condition.name().to_owned()),
                include_dirs: Some(self.compare.include_dirs),
                ignore_case: Some(self.compare.ignore_case),
            },
            window: None,
            split_percent: Some(self.split_percent),
            log_rows: Some(self.log.viewport_rows()),
        }
    }

    // The only place the state is saved: a quit the OS forces (logout,
    // shutdown) saves nothing.
    pub(super) fn quit(&mut self, save: bool, effects: &mut Vec<Effect>) {
        if save {
            effects.push(Effect::SaveState(self.state_file()));
        }
        effects.push(Effect::Quit);
    }

    pub(super) fn new_window(&mut self, as_admin: bool, effects: &mut Vec<Effect>) {
        effects.push(Effect::SpawnNewWindow {
            state: self.state_file(),
            as_admin,
        });
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
        let line = line.clone();
        let name = SpawnName::Command(Command::External(name.to_owned()));
        self.start_line(name, line, effects);
    }

    // The path is quoted to read back as one word: as the program when it can
    // be started, so that arguments go after it, and otherwise as an argument,
    // so that a program goes before it.
    pub(super) fn open_run(&mut self, path: &Path, executable: bool) {
        let path = path.to_string_lossy();
        let (text, caret) = if executable {
            let text = format!("{} ", quote_program(&path, self.os));
            let caret = text.len();
            (text, caret)
        } else {
            (format!(" {}", quote_arg(&path, self.os)), 0)
        };
        let popup = InputPopup::single(InputKind::Run, text, Some((caret, caret)));
        self.popup = Some(Popup::Input(popup));
    }

    pub(super) fn run_line(&mut self, command: Command, text: &str, effects: &mut Vec<Effect>) {
        if text.trim().is_empty() {
            return;
        }
        let line = match CommandLine::parse(text, self.os) {
            Ok(line) => line,
            Err(source) => {
                self.log
                    .push(AppError::InvalidCommandLine { source }.to_string());
                return;
            }
        };
        self.start_line(SpawnName::Command(command), line, effects);
    }

    pub(super) fn edit_config(&mut self, effects: &mut Vec<Effect>) {
        let expansion = Expansion {
            paths: vec![self.config_path.clone()],
            ..self.expansion()
        };
        Self::spawn_editor(&self.config.programs.editor, &expansion, effects);
    }

    pub(super) fn spawned(&mut self, kind: SpawnKind, error: Option<IoError>) {
        let line = match (kind, error) {
            (
                SpawnKind::External {
                    name,
                    program,
                    args,
                },
                None,
            ) => format!("Started {name}: {}", format_line(&program, &args, self.os)),
            (SpawnKind::External { program, .. }, Some(source)) => {
                AppError::SpawnFailed { program, source }.to_string()
            }
            (SpawnKind::NewWindow { .. }, None) => return,
            (SpawnKind::NewWindow { as_admin }, Some(source)) => {
                AppError::NewWindowFailed { as_admin, source }.to_string()
            }
        };
        self.log.push(line);
    }

    pub(super) fn exited(&mut self, name: SpawnName, code: Option<i32>) {
        let error = match code {
            Some(0) => return,
            Some(code) => AppError::ExitedWithStatus { name, code },
            None => AppError::TerminatedBySignal { name },
        };
        self.log.push(error.to_string());
    }

    fn load_config(&mut self, source: ConfigSource) {
        let config = match source {
            ConfigSource::Read(files) => self.parse(files),
            ConfigSource::Created => {
                self.log
                    .push(format!("Created {}", self.config_path.display()));
                None
            }
        };
        self.apply_config(config.unwrap_or_else(|| Config::default_for(self.os)));
    }

    // Logs every warning; a file that cannot be read or has a syntax error
    // is logged and yields nothing, so that the caller keeps what it had.
    fn parse(&mut self, files: Vec<ConfigFile>) -> Option<Config> {
        let mut texts = Vec::with_capacity(files.len());
        let mut paths = Vec::with_capacity(files.len());
        for ConfigFile { path, content } in files {
            let error = match content {
                ConfigContent::Text(text) => {
                    texts.push(text);
                    paths.push(path);
                    continue;
                }
                ConfigContent::Unreadable(source) => AppError::ConfigUnreadable { path, source },
                ConfigContent::Cycle => AppError::ConfigCycle { path },
            };
            self.log.push(error.to_string());
            return None;
        }
        let files = paths
            .iter()
            .map(PathBuf::as_path)
            .zip(texts.iter().map(String::as_str));
        match parse_config(files, self.os) {
            Ok((config, diagnostics)) => {
                for diagnostic in diagnostics {
                    self.log.push(diagnostic.to_string());
                }
                Some(config)
            }
            Err((index, error)) => {
                self.log.push(
                    AppError::ConfigSyntax {
                        path: paths.swap_remove(index),
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
        let extension_colors = extension_colors(&config.colors);
        for pane in &mut self.panes {
            pane.set_size_format(config.filelist.size_format);
            pane.set_size_digits(config.filelist.size_digits);
            pane.set_time_format(config.filelist.time_format);
            pane.set_extension_colors(extension_colors.clone());
            pane.set_max_history(config.filelist.max_history);
        }
        self.log.set_max_lines(config.log.max_lines);
        self.config = config;
        self.restyle();
    }

    fn restyle(&mut self) {
        let style = Style::from_config(&self.config, self.os, self.color_scheme);
        if *self.style != style {
            self.style = Arc::new(style);
        }
    }

    fn restore_pane(&mut self, side: Side, state: Option<&PaneState>) {
        let Some(state) = state else {
            return;
        };
        let pane = &mut self.panes[side.index()];
        let default = SortSpec::default();
        pane.set_sort(SortSpec {
            key: state
                .sort
                .as_deref()
                .and_then(SortKey::from_name)
                .unwrap_or(default.key),
            descending: state.descending.unwrap_or(default.descending),
            dirs_first: state.dirs_first.unwrap_or(default.dirs_first),
            ignore_case: state.ignore_case.unwrap_or(default.ignore_case),
        });
        pane.set_show_hidden(state.show_hidden.unwrap_or(false));
        pane.restore_history(History::restore(
            state.history.clone(),
            state.history_pos.unwrap_or(usize::MAX),
            self.config.filelist.max_history,
        ));
    }

    // The placeholder values for the active pane. Inside an archive the paths
    // of entries do not exist on disk until they are extracted, so there this
    // serves only a line that uses no such path, or a cursor on the archive
    // file itself with nothing marked.
    pub(super) fn expansion(&self) -> Expansion {
        let pane = self.active_pane();
        let full = |location: &Location, entry: &Entry| location.base.join(&entry.name);
        Expansion {
            cursor_path: pane
                .cursor_target()
                .map(|(location, entry)| full(&location, &entry)),
            marked_paths: pane
                .marked_entries()
                .map(|entry| full(pane.location(), entry))
                .collect(),
            paths: pane.targets().map_or_else(Vec::new, |targets| {
                targets
                    .entries
                    .iter()
                    .map(|entry| full(&targets.location, entry))
                    .collect()
            }),
            dir: pane.location().dir().to_path_buf(),
            other_dir: self
                .pane(self.active.other())
                .location()
                .dir()
                .to_path_buf(),
        }
    }

    pub(super) fn spawn_editor(
        line: &CommandLine,
        expansion: &Expansion,
        effects: &mut Vec<Effect>,
    ) {
        Self::spawn(SpawnName::Editor, line, expansion, effects);
    }

    pub(super) fn spawn(
        name: SpawnName,
        line: &CommandLine,
        expansion: &Expansion,
        effects: &mut Vec<Effect>,
    ) {
        effects.push(Effect::Spawn {
            name,
            program: line.program().to_owned(),
            args: line.expand(expansion),
            cwd: expansion.dir.clone(),
        });
    }
}

fn restore_compare(state: &CompareState) -> CompareSpec {
    let default = CompareSpec::default();
    CompareSpec {
        condition: state
            .condition
            .as_deref()
            .and_then(CompareCondition::from_name)
            .unwrap_or(default.condition),
        include_dirs: state.include_dirs.unwrap_or(default.include_dirs),
        ignore_case: state.ignore_case.unwrap_or(default.ignore_case),
    }
}
