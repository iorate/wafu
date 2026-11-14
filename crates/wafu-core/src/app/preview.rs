use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use encoding_rs::Encoding;

use super::navigation::{is_link_in_archive, level_shown};
use super::{App, Effect, MouseInput, PreviewSource, ReadError, Side};
use crate::command::{Command, Context};
use crate::config::ImagePlacement;
use crate::entry::Entry;
use crate::error::AppError;
use crate::location::{Location, Passwords};
use crate::pane::RequestId;
use crate::popup::{InputKind, InputPopup, ListPopup, PasswordPurpose, Popup};
use crate::preview::{Document, DocumentMode, ImageData, is_image_name};
use crate::text::show_line_breaks;
use crate::view::{PreviewContent, PreviewPlacement, PreviewView, SharedImage};

impl App {
    pub(super) fn open_preview(&mut self, entry: &Entry, effects: &mut Vec<Effect>) {
        if is_image_name(&entry.name) {
            let image = &self.config.preview.image;
            let placement = match self.active {
                Side::Left if image.from_left == ImagePlacement::OtherPane => {
                    PreviewPlacement::RightPane
                }
                Side::Right if image.from_right == ImagePlacement::OtherPane => {
                    PreviewPlacement::LeftPane
                }
                _ => PreviewPlacement::Full,
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
        let location = self.active_pane().location().clone();
        self.open_document(&location, &entry.name, effects);
    }

    pub(super) fn open_document(
        &mut self,
        location: &Location,
        name: &OsStr,
        effects: &mut Vec<Effect>,
    ) {
        self.preview = Some(Preview::Document(DocumentPreview {
            name: name.to_string_lossy().into_owned(),
            source: self.preview_source(location, name),
            loading: None,
            document: None,
            mode: DocumentMode::Text,
            scroll_top: 0,
        }));
        self.request_document(effects);
    }

    pub(super) fn context(&self) -> Context {
        match &self.preview {
            None => Context::FileList,
            Some(Preview::Image(_)) => Context::Image,
            Some(Preview::Document(_)) => Context::Text,
        }
    }

    pub(super) fn run_preview_command(&mut self, command: &Command, effects: &mut Vec<Effect>) {
        match command {
            Command::ClosePreview => {
                self.close_preview(effects);
                return;
            }
            Command::PreviewEncoding => {
                self.open_encodings();
                return;
            }
            Command::PreviewEdit => {
                self.edit_previewed(effects);
                return;
            }
            _ => {}
        }
        // A page is that many unwrapped lines; how many screen rows a line
        // takes is known only to the UI.
        let rows = self.preview_rows;
        let Some(Preview::Document(preview)) = &mut self.preview else {
            return;
        };
        match command {
            Command::PreviewScrollUp => preview.scroll_by(-1),
            Command::PreviewScrollDown => preview.scroll_by(1),
            Command::PreviewPageUp => preview.scroll_by(-to_i64(rows)),
            Command::PreviewPageDown => preview.scroll_by(to_i64(rows)),
            Command::PreviewTop => preview.scroll_top = 0,
            Command::PreviewBottom => preview.scroll_to_end(rows),
            Command::PreviewToggleMode => preview.toggle_mode(),
            _ => {}
        }
    }

    pub(super) fn preview_mouse(&mut self, input: MouseInput, effects: &mut Vec<Effect>) {
        if input == MouseInput::PreviewDoubleClick {
            self.close_preview(effects);
            return;
        }
        // The UI offers a divider only where one is on screen; whatever it
        // sends is applied. The log stays under every preview.
        if matches!(
            input,
            MouseInput::SplitTo { .. }
                | MouseInput::LogRowsTo { .. }
                | MouseInput::LogWheel { .. }
                | MouseInput::LogScrollTo { .. }
        ) {
            self.mouse(input, effects);
            return;
        }
        match &mut self.preview {
            Some(Preview::Document(preview)) => {
                if let MouseInput::PreviewWheel { rows } = input {
                    preview.scroll_by(i64::from(rows));
                }
            }
            Some(Preview::Image(preview)) => {
                let shown = match preview.placement {
                    PreviewPlacement::Full => return,
                    PreviewPlacement::LeftPane => Side::Right,
                    PreviewPlacement::RightPane => Side::Left,
                };
                let pane = &mut self.panes[shown.index()];
                match input {
                    MouseInput::Click { side, row } if side == shown => pane.set_cursor(row),
                    MouseInput::Wheel { side, rows } if side == shown => pane.scroll_by(rows),
                    MouseInput::ScrollTo { side, top } if side == shown => pane.scroll_to(top),
                    MouseInput::DoubleClick { side, row } if side == shown => {
                        pane.set_cursor(row);
                        self.close_preview(effects);
                    }
                    _ => {}
                }
            }
            None => {}
        }
    }

    pub(super) fn sync_image_preview(&mut self, effects: &mut Vec<Effect>) {
        let Some(Preview::Image(preview)) = &self.preview else {
            return;
        };
        let pane = self.active_pane();
        let target = pane
            .cursor_entry()
            .filter(|entry| {
                !entry.kind.is_dir_like()
                    && !entry.kind.is_special()
                    && !entry.kind.is_broken_link()
                    && !is_link_in_archive(pane.location(), entry.kind)
                    && is_image_name(&entry.name)
            })
            .map(|entry| entry.name.clone());
        if target == preview.shown {
            return;
        }
        let source = target
            .as_deref()
            .map(|name| self.preview_source(pane.location(), name));
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

    // Holding an archive open keeps a removable drive from being ejected
    // and, on Windows, keeps a deleted archive's name taken, so the io is
    // told to let it go as soon as nothing previews its entries.
    pub(super) fn release_preview(&self, before: Option<&Location>, effects: &mut Vec<Effect>) {
        if before.is_some() && self.previewed_archive().as_ref() != before {
            effects.push(Effect::ReleasePreview);
        }
    }

    // Two archives in one file are told apart by the root of each.
    pub(super) fn previewed_archive(&self) -> Option<Location> {
        self.preview.as_ref()?;
        self.active_pane().location().innermost_root()
    }

    pub(super) fn preview_loaded(
        &mut self,
        request: RequestId,
        result: Result<Document, ReadError>,
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
                preview.mode = if document.is_text || preview.source.encoding.is_some() {
                    DocumentMode::Text
                } else {
                    DocumentMode::Binary
                };
                preview.scroll_top = 0;
                preview.document = Some(document);
            }
            Err(ReadError::PasswordRequired { level, retry }) => {
                let location = preview.source.location();
                self.ask_preview_password(&location, level, retry);
            }
            Err(ReadError::Failed(source)) => {
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
        result: Result<ImageData, ReadError>,
    ) {
        let location = self.active_pane().location().clone();
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
            Err(ReadError::PasswordRequired { level, retry }) => {
                if self.popup.is_none() {
                    self.ask_preview_password(&location, level, retry);
                }
            }
            Err(ReadError::Failed(source)) => {
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
        let InputKind::ArchivePassword { level, .. } = popup.kind else {
            return;
        };
        self.active_pane_mut()
            .set_archive_password(level, Some(password.clone()));
        match &mut self.preview {
            Some(Preview::Document(preview)) => {
                preview.source.passwords.set(level, Some(password));
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

    pub(super) fn set_preview_encoding(
        &mut self,
        encoding: Option<&'static Encoding>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(Preview::Document(preview)) = &mut self.preview else {
            return;
        };
        // The file is read and decoded again on the read thread even though
        // its bytes are at hand: decoding up to 16 MiB here would hold up the UI.
        preview.source.encoding = encoding;
        self.request_document(effects);
    }

    pub(super) fn preview_loading(&self) -> Option<(String, RequestId)> {
        match &self.preview {
            Some(Preview::Document(preview)) => preview
                .loading
                .map(|request| (preview.name.clone(), request)),
            Some(Preview::Image(preview)) => preview
                .loading
                .zip(preview.shown.as_ref())
                .map(|(request, name)| (name.to_string_lossy().into_owned(), request)),
            None => None,
        }
    }

    pub(super) fn preview_view(&self) -> Option<PreviewView> {
        Some(match self.preview.as_ref()? {
            Preview::Document(preview) => {
                let (lines, digits) = match &preview.document {
                    Some(document) => (
                        document.lines(preview.mode, preview.scroll_top, self.preview_rows),
                        document.digits(preview.mode),
                    ),
                    None => (Vec::new(), 0),
                };
                PreviewView {
                    placement: PreviewPlacement::Full,
                    content: PreviewContent::Lines {
                        lines,
                        digits,
                        hex_dump: preview.mode == DocumentMode::Binary,
                    },
                    marked: false,
                }
            }
            Preview::Image(preview) => {
                let pane = self.active_pane();
                let marked = pane
                    .cursor_entry()
                    .is_some_and(|entry| pane.is_marked(entry));
                PreviewView {
                    placement: preview.placement,
                    content: match &preview.error {
                        Some(error) => PreviewContent::Message(show_line_breaks(error.clone())),
                        None => PreviewContent::Image(preview.image.clone().map(SharedImage)),
                    },
                    marked,
                }
            }
        })
    }

    pub(super) fn hide_log(&self) -> bool {
        let config = &self.config.preview;
        match &self.preview {
            Some(Preview::Document(_)) => config.text.hide_log,
            Some(Preview::Image(preview)) => {
                preview.placement == PreviewPlacement::Full && config.image.hide_log
            }
            None => false,
        }
    }

    fn open_encodings(&mut self) {
        // Not while a read is in flight: if it fails the preview closes, and
        // the list would be left open over the file list.
        let Some(Preview::Document(DocumentPreview {
            loading: None,
            document: Some(document),
            ..
        })) = &self.preview
        else {
            return;
        };
        self.popup = Some(Popup::List(ListPopup::encodings(document.encoding)));
    }

    // The file to open is the one being shown, which the cursor may have
    // left, and never a marked one: the marks are out of sight behind the
    // preview. The preview closes first, so that the file is read again when
    // it is opened after the edit.
    fn edit_previewed(&mut self, effects: &mut Vec<Effect>) {
        let Some(Preview::Document(preview)) = &self.preview else {
            return;
        };
        let source = preview.source.clone();
        self.close_preview(effects);
        self.edit_source(source, effects);
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

    // `location` is where the entry read is: the pane's location, or the one
    // above it for the `..` row, so its levels are the pane's.
    fn ask_preview_password(&mut self, location: &Location, level: usize, retry: bool) {
        if retry {
            self.active_pane_mut().set_archive_password(level, None);
        }
        self.popup = Some(Popup::Input(InputPopup::password(
            level_shown(location, level),
            level,
            retry,
            PasswordPurpose::Preview,
        )));
    }

    fn preview_source(&self, location: &Location, name: &OsStr) -> PreviewSource {
        match &location.archive {
            None => PreviewSource {
                path: location.base.join(name),
                entry: None,
                passwords: Passwords::default(),
                encoding: None,
            },
            Some(archive) => PreviewSource {
                path: location.base.clone(),
                entry: Some((archive.clone(), name.to_string_lossy().into_owned())),
                passwords: self.passwords_for(location),
                encoding: None,
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
            Some((_, name)) => PathBuf::from(self.location().child_display(OsStr::new(name))),
        }
    }

    // The place of the entry read, or the file's own place outside archives.
    fn location(&self) -> Location {
        Location {
            base: self.path.clone(),
            archive: self.entry.as_ref().map(|(archive, _)| archive.clone()),
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
