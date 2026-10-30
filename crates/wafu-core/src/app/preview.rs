use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use super::{App, Effect, MouseInput, PreviewError, PreviewSource, Side};
use crate::command::Context;
use crate::entry::Entry;
use crate::error::AppError;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::Location;
use crate::pane::RequestId;
use crate::popup::{InputPopup, PasswordPurpose, Popup};
use crate::preview::{Document, DocumentMode, ImageData, is_image_name};
use crate::view::{PreviewContent, PreviewPlacement, PreviewView, SharedImage};

impl App {
    pub(super) fn open_preview(&mut self, entry: &Entry, effects: &mut Vec<Effect>) {
        if is_image_name(&entry.name) {
            let placement = match self.active {
                Side::Left => PreviewPlacement::RightPane,
                Side::Right => PreviewPlacement::Full,
            };
            // The follow-up at the end of `update` requests the image.
            self.preview = Some(Preview::Image(ImagePreview {
                placement,
                shown: None,
                loading: None,
                image: None,
                error: None,
            }));
            return;
        }
        self.preview = Some(Preview::Document(DocumentPreview {
            name: entry.name.to_string_lossy().into_owned(),
            source: self.preview_source(&entry.name),
            loading: None,
            document: None,
            mode: DocumentMode::Text,
            scroll_top: 0,
        }));
        self.request_document(effects);
    }

    pub(super) fn preview_key(&mut self, key: Key, effects: &mut Vec<Effect>) {
        if matches!(key.base, KeyBase::Named(NamedKey::Enter | NamedKey::Escape)) {
            self.close_preview(effects);
            return;
        }
        match &mut self.preview {
            Some(Preview::Document(preview)) => {
                if key.ctrl || key.shift || key.alt {
                    return;
                }
                let rows = self.preview_rows;
                match key.base {
                    KeyBase::Named(NamedKey::Up) => preview.scroll_by(-1),
                    KeyBase::Named(NamedKey::Down) => preview.scroll_by(1),
                    KeyBase::Named(NamedKey::PageUp) => preview.scroll_by(-to_i64(rows)),
                    KeyBase::Named(NamedKey::PageDown) => preview.scroll_by(to_i64(rows)),
                    KeyBase::Named(NamedKey::Home) => preview.scroll_top = 0,
                    KeyBase::Named(NamedKey::End) => preview.scroll_to_end(rows),
                    KeyBase::Named(NamedKey::Tab) => preview.toggle_mode(),
                    _ => {}
                }
            }
            Some(Preview::Image(_)) => {
                if let Some(command) = self.config.keys.lookup(Context::Image, &key) {
                    self.run_command(command, effects);
                }
                self.active_pane_mut().ensure_cursor_visible();
            }
            None => {}
        }
    }

    pub(super) fn preview_mouse(&mut self, input: MouseInput, effects: &mut Vec<Effect>) {
        if input == MouseInput::PreviewDoubleClick {
            self.close_preview(effects);
            return;
        }
        match &mut self.preview {
            Some(Preview::Document(preview)) => {
                if let MouseInput::PreviewWheel { rows } = input {
                    preview.scroll_by(i64::from(rows));
                }
            }
            Some(Preview::Image(preview)) if preview.placement == PreviewPlacement::RightPane => {
                let pane = &mut self.panes[Side::Left.index()];
                match input {
                    MouseInput::Click {
                        side: Side::Left,
                        row,
                    } => pane.set_cursor(row),
                    MouseInput::Wheel {
                        side: Side::Left,
                        rows,
                    } => pane.scroll_by(rows),
                    MouseInput::ScrollTo {
                        side: Side::Left,
                        top,
                    } => pane.scroll_to(top),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub(super) fn sync_image_preview(&mut self, effects: &mut Vec<Effect>) {
        let Some(Preview::Image(preview)) = &self.preview else {
            return;
        };
        let target = self
            .active_pane()
            .cursor_entry()
            .filter(|entry| {
                !entry.kind.is_dir_like() && !entry.kind.is_special() && is_image_name(&entry.name)
            })
            .map(|entry| entry.name.clone());
        if target == preview.shown {
            return;
        }
        let source = target.as_deref().map(|name| self.preview_source(name));
        let request = source.is_some().then(|| self.next_request_id());
        let Some(Preview::Image(preview)) = &mut self.preview else {
            return;
        };
        if let Some(request) = preview.loading.take() {
            effects.push(Effect::CancelRead { request });
        }
        preview.shown = target;
        preview.error = None;
        match (source, request) {
            (Some(source), Some(request)) => {
                effects.push(Effect::LoadImage { source, request });
                preview.loading = Some(request);
            }
            _ => preview.image = None,
        }
    }

    pub(super) fn preview_loaded(
        &mut self,
        request: RequestId,
        result: Result<Document, PreviewError>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(Preview::Document(preview)) = &mut self.preview else {
            return;
        };
        if preview.loading != Some(request) {
            return;
        }
        preview.loading = None;
        match result {
            Ok(document) => {
                preview.mode = if document.is_text {
                    DocumentMode::Text
                } else {
                    DocumentMode::Binary
                };
                preview.scroll_top = 0;
                preview.document = Some(document);
            }
            Err(PreviewError::PasswordRequired { retry }) => {
                let archive = preview.source.path.clone();
                self.ask_preview_password(archive, retry);
            }
            Err(PreviewError::Failed(source)) => {
                let path = preview.source.shown();
                self.log
                    .push(AppError::PreviewFailed { path, source }.to_string());
                self.close_preview(effects);
            }
        }
    }

    pub(super) fn image_loaded(
        &mut self,
        request: RequestId,
        result: Result<ImageData, PreviewError>,
    ) {
        let archive = self.active_pane().location().base.clone();
        let shown = match &self.preview {
            Some(Preview::Image(preview)) if preview.loading == Some(request) => preview
                .shown
                .as_deref()
                .map(|name| self.active_pane().location().child_display(name))
                .unwrap_or_default(),
            _ => return,
        };
        let Some(Preview::Image(preview)) = &mut self.preview else {
            return;
        };
        preview.loading = None;
        match result {
            Ok(image) => {
                preview.image = Some(Arc::new(image));
                preview.error = None;
            }
            Err(PreviewError::PasswordRequired { retry }) => {
                if self.popup.is_none() {
                    self.ask_preview_password(archive, retry);
                }
            }
            Err(PreviewError::Failed(source)) => {
                let line = AppError::PreviewFailed {
                    path: PathBuf::from(shown),
                    source,
                }
                .to_string();
                preview.image = None;
                preview.error = Some(line.clone());
                self.log.push(line);
            }
        }
    }

    pub(super) fn preview_password(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        let password = popup.text(0).to_owned();
        if password.is_empty() {
            self.popup = Some(Popup::Input(popup));
            return;
        }
        self.active_pane_mut()
            .set_archive_password(Some(password.clone()));
        match &mut self.preview {
            Some(Preview::Document(preview)) => {
                preview.source.password = Some(password);
                self.request_document(effects);
            }
            // Forgetting what is shown makes the follow-up ask again, now with the password.
            Some(Preview::Image(preview)) => preview.shown = None,
            None => {}
        }
    }

    // Esc on the password question: a document cannot be shown without it,
    // an image preview just shows nothing until the cursor moves.
    pub(super) fn preview_password_refused(&mut self, effects: &mut Vec<Effect>) {
        match &mut self.preview {
            Some(Preview::Document(_)) => self.close_preview(effects),
            Some(Preview::Image(preview)) => preview.image = None,
            None => {}
        }
    }

    pub(super) fn preview_loading_name(&self) -> Option<String> {
        match &self.preview {
            Some(Preview::Document(preview)) if preview.loading.is_some() => {
                Some(preview.name.clone())
            }
            _ => None,
        }
    }

    pub(super) fn preview_view(&self) -> Option<PreviewView> {
        Some(match self.preview.as_ref()? {
            Preview::Document(preview) => {
                let (lines, digits, detail) = match &preview.document {
                    Some(document) => (
                        document.lines(preview.mode, preview.scroll_top, self.preview_rows),
                        document.digits(preview.mode),
                        match preview.mode {
                            DocumentMode::Text => document.encoding.to_lowercase(),
                            DocumentMode::Binary => String::new(),
                        },
                    ),
                    None => (Vec::new(), 0, String::new()),
                };
                PreviewView {
                    placement: PreviewPlacement::Full,
                    content: PreviewContent::Lines { lines, digits },
                    detail,
                }
            }
            Preview::Image(preview) => {
                let pane = self.active_pane();
                let marked = pane
                    .cursor_entry()
                    .is_some_and(|entry| pane.is_marked(entry));
                let detail = preview
                    .image
                    .as_ref()
                    .map(|image| image.format)
                    .into_iter()
                    .chain(marked.then_some("marked"))
                    .collect::<Vec<_>>()
                    .join(", ");
                PreviewView {
                    placement: preview.placement,
                    content: match &preview.error {
                        Some(error) => PreviewContent::Message(error.clone()),
                        None => PreviewContent::Image(preview.image.clone().map(SharedImage)),
                    },
                    detail,
                }
            }
        })
    }

    pub(super) fn hide_log(&self) -> bool {
        self.config.general.hide_log_in_preview
            && self
                .preview_view()
                .is_some_and(|preview| preview.placement == PreviewPlacement::Full)
    }

    fn close_preview(&mut self, effects: &mut Vec<Effect>) {
        let loading = match self.preview.take() {
            Some(Preview::Document(preview)) => preview.loading,
            Some(Preview::Image(preview)) => preview.loading,
            None => None,
        };
        if let Some(request) = loading {
            effects.push(Effect::CancelRead { request });
        }
    }

    fn request_document(&mut self, effects: &mut Vec<Effect>) {
        let request = self.next_request_id();
        let Some(Preview::Document(preview)) = &mut self.preview else {
            return;
        };
        if let Some(request) = preview.loading.take() {
            effects.push(Effect::CancelRead { request });
        }
        effects.push(Effect::LoadPreview {
            source: preview.source.clone(),
            request,
        });
        preview.loading = Some(request);
    }

    fn ask_preview_password(&mut self, archive: PathBuf, retry: bool) {
        if retry {
            self.active_pane_mut().set_archive_password(None);
        }
        self.popup = Some(Popup::Input(InputPopup::password(
            archive,
            retry,
            PasswordPurpose::Preview,
        )));
    }

    fn preview_source(&self, name: &OsStr) -> PreviewSource {
        let pane = self.active_pane();
        let location = pane.location();
        match &location.archive {
            None => PreviewSource {
                path: location.base.join(name),
                entry: None,
                password: None,
            },
            Some(archive) => PreviewSource {
                path: location.base.clone(),
                entry: Some((archive.clone(), name.to_string_lossy().into_owned())),
                password: pane.archive_password().map(ToOwned::to_owned),
            },
        }
    }
}

impl PreviewSource {
    /// The path as the user sees it, with the entry's place inside the archive.
    #[must_use]
    pub fn shown(&self) -> PathBuf {
        match &self.entry {
            None => self.path.clone(),
            Some((archive, name)) => {
                let location = Location {
                    base: self.path.clone(),
                    archive: Some(archive.clone()),
                };
                PathBuf::from(location.child_display(OsStr::new(name)))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Preview {
    Document(DocumentPreview),
    Image(ImagePreview),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentPreview {
    name: String,
    // Kept so that the file can be read again with a password.
    source: PreviewSource,
    loading: Option<RequestId>,
    document: Option<Document>,
    mode: DocumentMode,
    scroll_top: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagePreview {
    placement: PreviewPlacement,
    // The entry being shown or requested; `None` when the cursor is not on an image.
    shown: Option<OsString>,
    loading: Option<RequestId>,
    image: Option<Arc<ImageData>>,
    error: Option<String>,
}

impl DocumentPreview {
    fn line_count(&self) -> usize {
        self.document
            .as_ref()
            .map_or(0, |document| document.line_count(self.mode))
    }

    // The last line may come up to the top, so that a wrapped tail is never out of reach.
    fn scroll_by(&mut self, delta: i64) {
        let max = to_i64(self.line_count().saturating_sub(1));
        let top = to_i64(self.scroll_top).saturating_add(delta).clamp(0, max);
        self.scroll_top = usize::try_from(top).unwrap_or(0);
    }

    fn scroll_to_end(&mut self, rows: usize) {
        self.scroll_top = self.line_count().saturating_sub(rows);
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DocumentMode::Text => DocumentMode::Binary,
            DocumentMode::Binary => DocumentMode::Text,
        };
        self.scroll_top = 0;
    }
}

fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
