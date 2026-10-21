use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use slint::{
    CloseRequestResponse, ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc,
    Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};
use wafu_core::{
    App, Color, Colors, Effect, Ellipsis, InputField, IoError, IoEvent, ItemMark, MenuAnchor,
    MouseInput, Msg, PaneView, PopupView, PreviewContent, PreviewPlacement, PreviewView,
    ResolvedFont, RowKind, RowView, SharedImage, Side, StateFile, Style, Theme as ConfigTheme,
    View, WindowState,
};
use wafu_io::{Paths, Runtime};

use crate::keys::{Modifiers, key_input};
use crate::{
    AppWindow, PaneData, PopupData, PopupField, PopupItem, PreviewData, PreviewLine, Row, Theme,
    ThemeColors,
};

/// How many input fields the popup can show.
const MAX_POPUP_FIELDS: usize = 2;

// One wheel notch scrolls three rows. The pixel amount per notch is what the
// platform reports through Slint and is tuned on real hardware.
const WHEEL_PIXELS_PER_NOTCH: f32 = 60.0;
const WHEEL_ROWS_PER_NOTCH: f32 = 3.0;

// A menu opened from the keyboard sits a little in from the row's left edge.
const MENU_INSET: f32 = 16.0;

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

/// Ties the core, the I/O runtime, and the window together on the main thread.
pub struct Ui {
    app: RefCell<App>,
    window: AppWindow,
    runtime: Runtime,
    left_rows: Rc<VecModel<Row>>,
    right_rows: Rc<VecModel<Row>>,
    left_marks: Rc<VecModel<bool>>,
    right_marks: Rc<VecModel<bool>>,
    log_lines: Rc<VecModel<SharedString>>,
    popup_items: Rc<VecModel<PopupItem>>,
    popup_fields: Rc<VecModel<PopupField>>,
    preview_lines: Rc<VecModel<PreviewLine>>,
    rendered: RefCell<Rendered>,
    // Left pane, right pane, log, preview.
    wheel_remainder: RefCell<[f32; 4]>,
    // Where the last right click landed, in logical window coordinates.
    last_pointer: Cell<(f32, f32)>,
}

struct Rendered {
    left_rows: Arc<[RowView]>,
    right_rows: Arc<[RowView]>,
    left_marks: Arc<[bool]>,
    right_marks: Arc<[bool]>,
    log: Arc<[String]>,
    popup: Option<PopupView>,
    preview: Option<PreviewView>,
    // The image last handed to Slint, so that it is converted only once.
    image: Option<SharedImage>,
    style: Option<Arc<Style>>,
}

impl Default for Rendered {
    fn default() -> Self {
        Self {
            left_rows: Arc::from([]),
            right_rows: Arc::from([]),
            left_marks: Arc::from([]),
            right_marks: Arc::from([]),
            log: Arc::from([]),
            popup: None,
            preview: None,
            image: None,
            style: None,
        }
    }
}

impl Ui {
    pub fn install(window: AppWindow, paths: Paths) -> Rc<Self> {
        let runtime = Runtime::new(paths, |event| {
            let _ = slint::invoke_from_event_loop(move || {
                UI.with(|ui| {
                    if let Some(ui) = ui.borrow().as_ref() {
                        ui.dispatch(Msg::Io(event));
                    }
                });
            });
        });
        let ui = Rc::new(Self {
            app: RefCell::new(App::new()),
            window,
            runtime,
            left_rows: Rc::new(VecModel::default()),
            right_rows: Rc::new(VecModel::default()),
            left_marks: Rc::new(VecModel::default()),
            right_marks: Rc::new(VecModel::default()),
            log_lines: Rc::new(VecModel::default()),
            popup_items: Rc::new(VecModel::default()),
            popup_fields: Rc::new(VecModel::default()),
            preview_lines: Rc::new(VecModel::default()),
            rendered: RefCell::new(Rendered::default()),
            wheel_remainder: RefCell::new([0.0; 4]),
            last_pointer: Cell::new((0.0, 0.0)),
        });
        ui.window
            .set_left_rows(ModelRc::from(Rc::clone(&ui.left_rows)));
        ui.window
            .set_right_rows(ModelRc::from(Rc::clone(&ui.right_rows)));
        ui.window
            .set_left_marks(ModelRc::from(Rc::clone(&ui.left_marks)));
        ui.window
            .set_right_marks(ModelRc::from(Rc::clone(&ui.right_marks)));
        ui.window
            .set_log_lines(ModelRc::from(Rc::clone(&ui.log_lines)));
        ui.window
            .set_popup_items(ModelRc::from(Rc::clone(&ui.popup_items)));
        ui.window
            .set_popup_fields(ModelRc::from(Rc::clone(&ui.popup_fields)));
        ui.window
            .set_preview_lines(ModelRc::from(Rc::clone(&ui.preview_lines)));
        ui.window.set_menu_bar_visible(cfg!(target_os = "macos"));

        Self::connect(&ui);

        UI.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&ui)));
        ui
    }

    pub fn window(&self) -> &AppWindow {
        &self.window
    }

    pub fn dispatch(self: &Rc<Self>, msg: Msg) {
        let effects = self.app.borrow_mut().update(msg);
        for effect in effects {
            match effect {
                Effect::SetTitle(title) => self.window.set_window_title(title.into()),
                Effect::ShellMenu { paths, anchor } => self.shell_menu(paths, anchor),
                // Only the window knows where it is, so its place is added here.
                Effect::SaveState(state) => {
                    self.runtime.run(Effect::SaveState(self.with_window(state)));
                }
                Effect::SpawnNewWindow(state) => {
                    self.runtime
                        .run(Effect::SpawnNewWindow(self.with_window(state)));
                }
                Effect::Quit => {
                    let _ = slint::quit_event_loop();
                }
                effect => self.runtime.run(effect),
            }
        }
        self.render(&self.app.borrow().view());
    }

    /// Puts the window where it was, in logical pixels. The position is left
    /// alone on Wayland, which has no notion of it.
    pub fn restore_window(&self, state: Option<&WindowState>) {
        let Some(state) = state else {
            return;
        };
        let window = self.window.window();
        // Sizes are small enough to be exact as floats.
        #[allow(clippy::cast_precision_loss)]
        window.set_size(LogicalSize::new(state.width as f32, state.height as f32));
        if let (Some(x), Some(y)) = (state.x, state.y)
            && !wafu_io::is_wayland()
        {
            // Likewise for positions.
            #[allow(clippy::cast_precision_loss)]
            window.set_position(LogicalPosition::new(x as f32, y as f32));
        }
    }

    // The menu runs a modal loop of its own, so it is opened from the event
    // loop rather than from inside `dispatch`, where the I/O events arriving
    // meanwhile would re-enter the application ahead of the remaining effects.
    fn shell_menu(self: &Rc<Self>, paths: Vec<PathBuf>, anchor: MenuAnchor) {
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                let (x, y) = ui.menu_position(anchor);
                let error = show_shell_menu(&ui.window, &paths, x, y);
                ui.dispatch(Msg::Io(IoEvent::ShellMenuClosed { error }));
            }
        });
    }

    // Logical window coordinates of the menu's top-left corner.
    fn menu_position(&self, anchor: MenuAnchor) -> (f32, f32) {
        match anchor {
            MenuAnchor::Pointer => self.last_pointer.get(),
            MenuAnchor::Row { side, row } => {
                let (list_x, list_y, scroll_top) = match side {
                    Side::Left => (
                        self.window.get_left_list_x(),
                        self.window.get_left_list_y(),
                        self.window.get_left_scroll_top(),
                    ),
                    Side::Right => (
                        self.window.get_right_list_x(),
                        self.window.get_right_list_y(),
                        self.window.get_right_scroll_top(),
                    ),
                };
                let row = i32::try_from(row).unwrap_or(i32::MAX);
                let visible_row = (row - scroll_top).max(0);
                let row_height = self.window.global::<Theme>().get_row_height();
                // Rows on screen are few enough to be exact as floats.
                #[allow(clippy::cast_precision_loss)]
                let y = list_y + (visible_row + 1) as f32 * row_height;
                (list_x + MENU_INSET, y)
            }
        }
    }

    fn with_window(&self, mut state: StateFile) -> StateFile {
        let window = self.window.window();
        let scale = window.scale_factor();
        let size = window.size().to_logical(scale);
        let position = window.position().to_logical(scale);
        let (x, y) = if wafu_io::is_wayland() {
            (None, None)
        } else {
            (Some(round(position.x)), Some(round(position.y)))
        };
        state.window = Some(WindowState {
            x,
            y,
            width: round(size.width).unsigned_abs(),
            height: round(size.height).unsigned_abs(),
        });
        state
    }

    fn connect(ui: &Rc<Self>) {
        Self::connect_window(ui);

        let weak = Rc::downgrade(ui);
        ui.window.on_viewport_rows_changed(move |side, rows| {
            if let Some(ui) = Weak::upgrade(&weak) {
                let rows = usize::try_from(rows).unwrap_or(0);
                ui.dispatch(Msg::ViewportRows {
                    side: side_of(side),
                    rows,
                });
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_row_clicked(move |side, row| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::Mouse(MouseInput::Click {
                    side: side_of(side),
                    row: row_of(row),
                }));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_row_double_clicked(move |side, row| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::Mouse(MouseInput::DoubleClick {
                    side: side_of(side),
                    row: row_of(row),
                }));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_row_right_clicked(move |side, row, x, y| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.last_pointer.set((x, y));
                ui.dispatch(Msg::Mouse(MouseInput::RightClick {
                    side: side_of(side),
                    row: row_of(row),
                }));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_wheel(move |side, delta| {
            if let Some(ui) = Weak::upgrade(&weak) {
                let side = side_of(side);
                if let Some(rows) = ui.wheel_rows(usize::from(side == Side::Right), delta) {
                    ui.dispatch(Msg::Mouse(MouseInput::Wheel { side, rows }));
                }
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_log_wheel(move |delta| {
            if let Some(ui) = Weak::upgrade(&weak)
                && let Some(rows) = ui.wheel_rows(2, delta)
            {
                ui.dispatch(Msg::Mouse(MouseInput::LogWheel { rows }));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_preview_rows_changed(move |rows| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::PreviewRows {
                    rows: usize::try_from(rows).unwrap_or(1),
                });
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_preview_wheel(move |delta| {
            if let Some(ui) = Weak::upgrade(&weak)
                && let Some(rows) = ui.wheel_rows(3, delta)
            {
                ui.dispatch(Msg::Mouse(MouseInput::PreviewWheel { rows }));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_input_edited(move |field, text| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::InputChanged {
                    field: usize::try_from(field).unwrap_or(0),
                    text: text.to_string(),
                });
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_input_accepted(move || {
            let none = Modifiers {
                control: false,
                shift: false,
                alt: false,
                meta: false,
            };
            if let Some(ui) = Weak::upgrade(&weak)
                && let Some(input) = key_input("Enter", none)
            {
                ui.dispatch(Msg::Key(input));
            }
        });
    }

    fn connect_window(ui: &Rc<Self>) {
        let weak = Rc::downgrade(ui);
        ui.window.window().on_close_requested(move || {
            if let Some(ui) = weak.upgrade() {
                ui.dispatch(Msg::CloseRequested);
            }
            // Quitting is the core's decision, carried out by `Effect::Quit`.
            CloseRequestResponse::KeepWindowShown
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_quit_requested(move || {
            if let Some(ui) = weak.upgrade() {
                ui.dispatch(Msg::CloseRequested);
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window
            .on_key_input(move |text, control, shift, alt, meta| {
                let modifiers = Modifiers {
                    control,
                    shift,
                    alt,
                    meta,
                };
                if let Some(ui) = weak.upgrade()
                    && let Some(input) = key_input(&text, modifiers)
                {
                    ui.dispatch(Msg::Key(input));
                }
            });
    }

    // Wheel deltas arrive in pixels and in fractions of a notch; whole rows are
    // returned and the rest is carried over to the next event of the same area.
    fn wheel_rows(&self, index: usize, delta: f32) -> Option<i32> {
        let mut remainder = self.wheel_remainder.borrow_mut();
        let total = remainder[index] - delta / WHEEL_PIXELS_PER_NOTCH * WHEEL_ROWS_PER_NOTCH;
        let rows = total.trunc();
        remainder[index] = total - rows;
        // The truncated value is a small integer, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        let rows = rows as i32;
        (rows != 0).then_some(rows)
    }

    fn render(&self, view: &View) {
        self.window.set_left(pane_data(&view.left));
        self.window.set_right(pane_data(&view.right));
        self.window
            .set_left_scroll_top(i32::try_from(view.left.scroll_top).unwrap_or(i32::MAX));
        self.window
            .set_right_scroll_top(i32::try_from(view.right.scroll_top).unwrap_or(i32::MAX));
        self.window.set_split_percent(i32::from(view.split_percent));
        self.window
            .set_log_rows(i32::try_from(view.log_rows).unwrap_or(i32::MAX));
        self.window
            .set_log_scroll_top(i32::try_from(view.log.scroll_top).unwrap_or(i32::MAX));
        self.window.set_hide_log(view.hide_log);
        // Replacing a model resets the list, which is too slow to do on every
        // key repeat, so the models are only touched when their content changes.
        let mut rendered = self.rendered.borrow_mut();
        if rendered
            .style
            .as_ref()
            .is_none_or(|style| !Arc::ptr_eq(style, &view.style))
        {
            self.apply_style(&view.style);
            rendered.style = Some(Arc::clone(&view.style));
        }
        if !Arc::ptr_eq(&rendered.left_rows, &view.left.rows) {
            self.left_rows
                .set_vec(view.left.rows.iter().map(row).collect::<Vec<_>>());
            rendered.left_rows = Arc::clone(&view.left.rows);
        }
        if !Arc::ptr_eq(&rendered.right_rows, &view.right.rows) {
            self.right_rows
                .set_vec(view.right.rows.iter().map(row).collect::<Vec<_>>());
            rendered.right_rows = Arc::clone(&view.right.rows);
        }
        if !Arc::ptr_eq(&rendered.left_marks, &view.left.marks) {
            update_marks(&self.left_marks, &rendered.left_marks, &view.left.marks);
            rendered.left_marks = Arc::clone(&view.left.marks);
        }
        if !Arc::ptr_eq(&rendered.right_marks, &view.right.marks) {
            update_marks(&self.right_marks, &rendered.right_marks, &view.right.marks);
            rendered.right_marks = Arc::clone(&view.right.marks);
        }
        if !Arc::ptr_eq(&rendered.log, &view.log.lines) {
            self.log_lines.set_vec(
                view.log
                    .lines
                    .iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            );
            rendered.log = Arc::clone(&view.log.lines);
        }
        // A field keeps its own text while typing; it is only written back
        // when the core refused an edit or opened the popup with a value. It is
        // written before the popup appears so that the field opens with it.
        let fields = view
            .popup
            .as_ref()
            .map(|popup| popup.fields.as_slice())
            .unwrap_or_default();
        for (index, field) in fields.iter().enumerate().take(MAX_POPUP_FIELDS) {
            let current = if index == 0 {
                self.window.get_popup_text()
            } else {
                self.window.get_popup_text_2()
            };
            if current != field.text {
                let text = SharedString::from(field.text.as_str());
                if index == 0 {
                    self.window.set_popup_text(text);
                } else {
                    self.window.set_popup_text_2(text);
                }
            }
        }
        if rendered.preview != view.preview {
            self.render_preview(&mut rendered, view.preview.as_ref());
            rendered.preview.clone_from(&view.preview);
        }
        if rendered.popup != view.popup {
            self.popup_fields.set_vec(
                fields
                    .iter()
                    .take(MAX_POPUP_FIELDS)
                    .map(popup_field)
                    .collect::<Vec<_>>(),
            );
            self.window.set_popup(popup_data(view.popup.as_ref()));
            let items = view
                .popup
                .as_ref()
                .map(|popup| popup.items.iter().map(popup_item).collect::<Vec<_>>())
                .unwrap_or_default();
            self.popup_items.set_vec(items);
            rendered.popup.clone_from(&view.popup);
        }
    }
}

impl Ui {
    fn render_preview(&self, rendered: &mut Rendered, preview: Option<&PreviewView>) {
        let (lines, digits, image, data) = match preview {
            None => (Vec::new(), 0, None, PreviewData::default()),
            Some(preview) => {
                let (kind, lines, digits, image, message) = match &preview.content {
                    PreviewContent::Lines { lines, digits } => {
                        (0, lines.as_slice(), *digits, None, "")
                    }
                    PreviewContent::Image(image) => (1, [].as_slice(), 0, image.clone(), ""),
                    PreviewContent::Message(message) => {
                        (2, [].as_slice(), 0, None, message.as_str())
                    }
                };
                let data = PreviewData {
                    visible: true,
                    full: preview.placement == PreviewPlacement::Full,
                    kind,
                    message: message.into(),
                    name: preview.name.as_str().into(),
                    detail: preview.detail.as_str().into(),
                };
                (
                    lines.iter().map(preview_line).collect(),
                    digits,
                    image,
                    data,
                )
            }
        };
        if rendered.image != image {
            self.window.set_preview_image(match &image {
                Some(image) => slint_image(&image.0),
                None => slint::Image::default(),
            });
            rendered.image = image;
        }
        self.window.set_preview_has_image(rendered.image.is_some());
        self.window
            .set_preview_number_sample("0".repeat(digits).into());
        self.preview_lines.set_vec(lines);
        self.window.set_preview(data);
    }

    fn apply_style(&self, style: &Style) {
        self.window
            .set_ellipsis_middle(style.ellipsis == Ellipsis::Middle);
        self.window.set_theme(match style.theme {
            ConfigTheme::Auto => 0,
            ConfigTheme::Light => 1,
            ConfigTheme::Dark => 2,
        });
        let theme = self.window.global::<Theme>();
        theme.set_light(theme_colors(&style.colors.light));
        theme.set_dark(theme_colors(&style.colors.dark));
        let (family, size) = font(&style.fonts.ui);
        theme.set_ui_font_family(family);
        theme.set_ui_font_size(size);
        let (family, size) = font(&style.fonts.filelist);
        theme.set_filelist_font_family(family);
        theme.set_filelist_font_size(size);
        let (family, size) = font(&style.fonts.log);
        theme.set_log_font_family(family);
        theme.set_log_font_size(size);
        let (family, size) = font(&style.fonts.preview);
        theme.set_preview_font_family(family);
        theme.set_preview_font_size(size);
    }
}

// Shows the shell menu at a logical window position and blocks until it closes.
#[cfg(windows)]
fn show_shell_menu(window: &AppWindow, paths: &[PathBuf], x: f32, y: f32) -> Option<IoError> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let window = window.window();
    let handle = window.window_handle();
    let hwnd = match handle.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
        _ => return Some(IoError::new("the window has no handle")),
    };
    let scale = window.scale_factor();
    wafu_io::shell_menu(hwnd, paths, round(x * scale), round(y * scale))
        .err()
        .map(|error| IoError::new(error.to_string()))
}

#[cfg(not(windows))]
fn show_shell_menu(_window: &AppWindow, _paths: &[PathBuf], _x: f32, _y: f32) -> Option<IoError> {
    None
}

fn preview_line(line: &wafu_core::PreviewLine) -> PreviewLine {
    PreviewLine {
        number: line.number.as_str().into(),
        text: line.text.as_str().into(),
    }
}

fn slint_image(image: &wafu_core::ImageData) -> slint::Image {
    let buffer =
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&image.rgba, image.width, image.height);
    slint::Image::from_rgba8(buffer)
}

fn theme_colors(colors: &Colors) -> ThemeColors {
    ThemeColors {
        fg: color(colors.fg),
        bg: color(colors.bg),
        mark: color(colors.mark),
        cursor: color(colors.cursor),
        dir: color(colors.dir),
        link: color(colors.link),
    }
}

fn color(color: Color) -> slint::Color {
    slint::Color::from_rgb_u8(color.r, color.g, color.b)
}

fn font(font: &ResolvedFont) -> (SharedString, f32) {
    (font.family.as_str().into(), f32::from(font.size))
}

// Window coordinates are whole pixels in practice; the rounding only drops a
// fractional scale remainder.
fn round(value: f32) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value.round() as i32;
    rounded
}

// Marks change one row at a time, so only the rows that differ are notified.
fn update_marks(model: &VecModel<bool>, before: &[bool], after: &[bool]) {
    if model.row_count() != after.len() {
        model.set_vec(after.to_vec());
        return;
    }
    for (index, (&was, &is)) in before.iter().zip(after).enumerate() {
        if was != is {
            model.set_row_data(index, is);
        }
    }
}

fn side_of(side: i32) -> Side {
    if side == 0 { Side::Left } else { Side::Right }
}

fn row_of(row: i32) -> usize {
    usize::try_from(row).unwrap_or(usize::MAX)
}

fn pane_data(pane: &PaneView) -> PaneData {
    PaneData {
        path: pane.path.as_str().into(),
        info: pane.info.as_str().into(),
        active: pane.active,
        cursor: pane
            .cursor
            .and_then(|cursor| i32::try_from(cursor).ok())
            .unwrap_or(-1),
    }
}

fn row(row: &RowView) -> Row {
    Row {
        kind: match row.kind {
            RowKind::Parent => 0,
            RowKind::Dir => 1,
            RowKind::File => 2,
            RowKind::Link => 3,
        },
        name: row.name.as_str().into(),
        name_tail: row.name_tail.as_str().into(),
        extension: row.extension.as_str().into(),
        marker: row.marker.as_str().into(),
        size: row.size.as_str().into(),
        modified: row.modified.as_str().into(),
    }
}

fn popup_data(popup: Option<&PopupView>) -> PopupData {
    match popup {
        Some(popup) => PopupData {
            visible: true,
            title: popup.title.as_str().into(),
            message: popup.message.as_deref().unwrap_or_default().into(),
            has_input: !popup.fields.is_empty(),
            cursor: popup
                .cursor
                .and_then(|cursor| i32::try_from(cursor).ok())
                .unwrap_or(-1),
            progress: popup.progress,
        },
        None => PopupData {
            visible: false,
            title: SharedString::default(),
            message: SharedString::default(),
            has_input: false,
            cursor: -1,
            progress: false,
        },
    }
}

fn popup_field(field: &InputField) -> PopupField {
    let (start, end) = field.select.unwrap_or((0, 0));
    PopupField {
        label: field.label.into(),
        secret: field.secret,
        select_start: i32::try_from(start).unwrap_or(0),
        select_end: i32::try_from(end).unwrap_or(0),
    }
}

fn popup_item(item: &wafu_core::PopupItem) -> PopupItem {
    let (mark, on) = match item.mark {
        ItemMark::None => (0, false),
        ItemMark::Radio(on) => (1, on),
        ItemMark::Check(on) => (2, on),
    };
    PopupItem {
        label: item.label.as_str().into(),
        detail: item.detail.as_str().into(),
        mark,
        on,
    }
}
