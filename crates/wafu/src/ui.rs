use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use slint::{Model, ModelRc, SharedString, VecModel};
use wafu_core::{
    App, Effect, Ellipsis, ItemMark, MouseInput, Msg, PaneView, PopupView, RowView, Side, View,
};
use wafu_io::Runtime;

use crate::keys::{key_input, physical_ctrl};
use crate::{AppWindow, PaneData, PopupData, PopupItem, Row};

// One wheel notch scrolls three rows. The pixel amount per notch is what the
// platform reports through Slint and is tuned on real hardware.
const WHEEL_PIXELS_PER_NOTCH: f32 = 60.0;
const WHEEL_ROWS_PER_NOTCH: f32 = 3.0;

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
    rendered: RefCell<Rendered>,
    // Left pane, right pane, log.
    wheel_remainder: RefCell<[f32; 3]>,
}

struct Rendered {
    left_rows: Arc<[RowView]>,
    right_rows: Arc<[RowView]>,
    left_marks: Arc<[bool]>,
    right_marks: Arc<[bool]>,
    log: Arc<[String]>,
    popup: Option<PopupView>,
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
        }
    }
}

impl Ui {
    pub fn install(window: AppWindow) -> Rc<Self> {
        let runtime = Runtime::new(|event| {
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
            rendered: RefCell::new(Rendered::default()),
            wheel_remainder: RefCell::new([0.0; 3]),
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

        Self::connect(&ui);

        UI.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&ui)));
        ui
    }

    pub fn window(&self) -> &AppWindow {
        &self.window
    }

    pub fn dispatch(&self, msg: Msg) {
        let effects = self.app.borrow_mut().update(msg);
        for effect in effects {
            match effect {
                Effect::SetTitle(title) => self.window.set_window_title(title.into()),
                effect => self.runtime.run(effect),
            }
        }
        self.render(&self.app.borrow().view());
    }

    fn connect(ui: &Rc<Self>) {
        let weak = Rc::downgrade(ui);
        ui.window
            .on_key_input(move |text, control, shift, alt, meta| {
                if let Some(ui) = weak.upgrade() {
                    let ctrl = physical_ctrl(control, meta);
                    ui.dispatch(Msg::Key(key_input(&text, ctrl, shift, alt)));
                }
            });

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
        ui.window.on_row_right_clicked(move |side, row| {
            if let Some(ui) = Weak::upgrade(&weak) {
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
        ui.window.on_input_edited(move |text| {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::InputChanged(text.to_string()));
            }
        });

        let weak = Rc::downgrade(ui);
        ui.window.on_input_accepted(move || {
            if let Some(ui) = Weak::upgrade(&weak) {
                ui.dispatch(Msg::Key(key_input("Enter", false, false, false)));
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
        self.window
            .set_ellipsis_middle(view.ellipsis == Ellipsis::Middle);
        self.window.set_split_percent(i32::from(view.split_percent));
        self.window
            .set_log_rows(i32::try_from(view.log_rows).unwrap_or(i32::MAX));
        self.window
            .set_log_scroll_top(i32::try_from(view.log.scroll_top).unwrap_or(i32::MAX));
        // Replacing a model resets the list, which is too slow to do on every
        // key repeat, so the models are only touched when their content changes.
        let mut rendered = self.rendered.borrow_mut();
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
        if rendered.popup != view.popup {
            self.window.set_popup(popup_data(view.popup.as_ref()));
            let items = view
                .popup
                .as_ref()
                .map(|popup| popup.items.iter().map(popup_item).collect::<Vec<_>>())
                .unwrap_or_default();
            self.popup_items.set_vec(items);
            rendered.popup.clone_from(&view.popup);
        }
        // The field keeps its own text while typing; it is only written back
        // when the core refused an edit or opened the popup with a value.
        if let Some(input) = view.popup.as_ref().and_then(|popup| popup.input.as_deref())
            && self.window.get_popup_text() != input
        {
            self.window.set_popup_text(input.into());
        }
    }
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
            has_input: popup.input.is_some(),
            cursor: popup
                .cursor
                .and_then(|cursor| i32::try_from(cursor).ok())
                .unwrap_or(-1),
        },
        None => PopupData {
            visible: false,
            title: SharedString::default(),
            has_input: false,
            cursor: -1,
        },
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
