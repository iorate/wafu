use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use slint::{ModelRc, SharedString, VecModel};
use wafu_core::{App, Effect, Msg, PaneView, RowView, Side, View};
use wafu_io::Runtime;

use crate::keys::{key_input, physical_ctrl};
use crate::{AppWindow, PaneData, Row};

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
    log_lines: Rc<VecModel<SharedString>>,
    rendered: RefCell<Rendered>,
}

struct Rendered {
    left_rows: Arc<[RowView]>,
    right_rows: Arc<[RowView]>,
    log: Vec<String>,
}

impl Default for Rendered {
    fn default() -> Self {
        Self {
            left_rows: Arc::from([]),
            right_rows: Arc::from([]),
            log: Vec::new(),
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
            log_lines: Rc::new(VecModel::default()),
            rendered: RefCell::new(Rendered::default()),
        });
        ui.window
            .set_left_rows(ModelRc::from(Rc::clone(&ui.left_rows)));
        ui.window
            .set_right_rows(ModelRc::from(Rc::clone(&ui.right_rows)));
        ui.window
            .set_log_lines(ModelRc::from(Rc::clone(&ui.log_lines)));

        let weak = Rc::downgrade(&ui);
        ui.window
            .on_key_input(move |text, control, shift, alt, meta| {
                if let Some(ui) = weak.upgrade() {
                    let ctrl = physical_ctrl(control, meta);
                    ui.dispatch(Msg::Key(key_input(&text, ctrl, shift, alt)));
                }
            });

        let weak = Rc::downgrade(&ui);
        ui.window.on_viewport_rows_changed(move |side, rows| {
            if let Some(ui) = Weak::upgrade(&weak) {
                let side = if side == 0 { Side::Left } else { Side::Right };
                let rows = usize::try_from(rows).unwrap_or(0);
                ui.dispatch(Msg::ViewportRows { side, rows });
            }
        });

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

    fn render(&self, view: &View) {
        self.window.set_left(pane_data(&view.left));
        self.window.set_right(pane_data(&view.right));
        self.window
            .set_left_scroll_top(i32::try_from(view.left.scroll_top).unwrap_or(i32::MAX));
        self.window
            .set_right_scroll_top(i32::try_from(view.right.scroll_top).unwrap_or(i32::MAX));
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
        if rendered.log != view.log {
            self.log_lines
                .set_vec(view.log.iter().map(SharedString::from).collect::<Vec<_>>());
            rendered.log.clone_from(&view.log);
        }
    }
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
        extension: row.extension.as_str().into(),
        size: row.size.as_str().into(),
        modified: row.modified.as_str().into(),
    }
}
