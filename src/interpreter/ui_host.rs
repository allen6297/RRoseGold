//! Thin `__ui` host: window, run loop, paint a widget, apply a style blob.
//! Theme maps and modifiers live in `stdlib/ui.rg`.

use std::collections::HashMap;

use crate::{RuntimeError, Span};

use super::eval::EvalContext;
use super::ops::{option_none, option_some, runtime_err, value_eq};
use super::value::*;

pub(crate) struct UiState {
    theme: HashMap<String, Value>,
    last_alert: Option<String>,
    alerts: Vec<String>,
    windows: Vec<UiWindow>,
    stack: Vec<usize>,
    children: HashMap<usize, Vec<Value>>,
    actions: HashMap<usize, Value>,
    frame_error: Option<String>,
    quit: bool,
    invalidate: bool,
    dirty: bool,
    /// True only while `ui.run()` owns the native event loop.
    running: bool,
    /// Values returned to the script last pump (`pass current`).
    input_out: Vec<Value>,
    /// Values painted last frame (`get next`).
    input_in: Vec<Value>,
    input_cursor: usize,
    paint_cursor: usize,
    /// Stable egui ids for immediate-mode widgets (Arc handle ptrs change every frame).
    egui_salt: usize,
}

struct UiWindow {
    handle: Value,
    body: Value,
}

impl UiState {
    pub(super) fn new() -> Self {
        Self {
            theme: HashMap::new(),
            last_alert: None,
            alerts: Vec::new(),
            windows: Vec::new(),
            stack: Vec::new(),
            children: HashMap::new(),
            actions: HashMap::new(),
            frame_error: None,
            quit: false,
            invalidate: false,
            dirty: false,
            running: false,
            input_out: Vec::new(),
            input_in: Vec::new(),
            input_cursor: 0,
            paint_cursor: 0,
            egui_salt: 0,
        }
    }

    fn push_child(&mut self, handle: Value) {
        if let Some(&parent) = self.stack.last() {
            self.children.entry(parent).or_default().push(handle);
        }
    }

    fn take_input(&mut self, current: Value) -> Value {
        let idx = self.input_cursor;
        self.input_cursor += 1;
        let next = match (self.input_out.get(idx), self.input_in.get(idx)) {
            (Some(out), Some(painted)) if value_eq(out, &current) => painted.clone(),
            _ => current,
        };
        if idx < self.input_out.len() {
            self.input_out[idx] = next.clone();
        } else {
            self.input_out.push(next.clone());
        }
        next
    }

    fn store_painted(&mut self, value: Value, changed: bool) {
        let idx = self.paint_cursor;
        self.paint_cursor += 1;
        if idx < self.input_in.len() {
            self.input_in[idx] = value;
        } else {
            self.input_in.push(value);
        }
        if changed {
            self.dirty = true;
        }
    }

    fn next_egui_salt(&mut self) -> u64 {
        let i = self.egui_salt;
        self.egui_salt += 1;
        i as u64
    }
}

fn handle_id(v: &Value) -> Option<usize> {
    match v {
        Value::Struct { fields, .. } => Some(std::sync::Arc::as_ptr(fields) as usize),
        Value::Map(m) => Some(std::sync::Arc::as_ptr(m) as usize),
        _ => None,
    }
}

fn field(handle: &Value, name: &str) -> Option<Value> {
    match handle {
        Value::Struct { fields, .. } => lock(fields).get(name).cloned(),
        Value::Map(m) => lock(m).get(name).cloned(),
        _ => None,
    }
}

fn field_str(handle: &Value, name: &str) -> String {
    match field(handle, name) {
        Some(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn style_map(handle: &Value) -> HashMap<String, Value> {
    match field(handle, "style") {
        Some(Value::Map(m)) => lock(&m).clone(),
        _ => HashMap::new(),
    }
}

fn set_field(handle: &Value, name: &str, val: Value) {
    match handle {
        Value::Struct { fields, .. } => {
            lock(fields).insert(name.to_string(), val);
        }
        Value::Map(m) => {
            lock(m).insert(name.to_string(), val);
        }
        _ => {}
    }
}

fn set_style_entry(handle: &Value, key: &str, val: Value) {
    match field(handle, "style") {
        Some(Value::Map(m)) => {
            lock(&m).insert(key.to_string(), val);
        }
        _ => {}
    }
}

fn style_bool(handle: &Value, key: &str) -> bool {
    matches!(style_map(handle).get(key), Some(Value::Bool(true)))
}

fn style_float(handle: &Value, key: &str) -> f64 {
    match style_map(handle).get(key) {
        Some(Value::Float(n)) => *n,
        Some(Value::Int(n)) => *n as f64,
        _ => 0.0,
    }
}

fn slider_range(min: f64, max: f64) -> (f64, f64) {
    if min <= max { (min, max) } else { (max, min) }
}

fn style_strings(handle: &Value, key: &str) -> Vec<String> {
    match style_map(handle).get(key) {
        Some(Value::Array(a)) => lock(a)
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_file_path(save: bool) -> Value {
    #[cfg(test)]
    {
        let _ = save;
        option_none()
    }
    #[cfg(not(test))]
    {
        let dialog = rfd::FileDialog::new();
        let picked = if save {
            dialog.save_file()
        } else {
            dialog.pick_file()
        };
        match picked {
            Some(p) => option_some(Value::String(p.to_string_lossy().into_owned())),
            None => option_none(),
        }
    }
}

impl EvalContext {
    pub(super) fn ui_host(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (name, args);
            return Err(runtime_err("ui is not supported on this target", span));
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.ui_host_native(name, args, span)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ui_host_native(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match name {
            "theme" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.theme takes 1 argument", span));
                }
                let map = match &args[0] {
                    Value::Map(m) => lock(m).clone(),
                    _ => {
                        return Err(runtime_err(
                            format!("__ui.theme expects Map, got {}", args[0].type_name()),
                            span,
                        ));
                    }
                };
                self.data_mut(|d| d.ui.theme = map);
                Ok(Value::Void)
            }
            "theme_get" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.theme_get takes 0 arguments", span));
                }
                Ok(self.data(|d| map_value(d.ui.theme.clone())))
            }
            "alert" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.alert takes 1 argument", span));
                }
                let msg = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                self.data_mut(|d| {
                    d.ui.last_alert = Some(msg.clone());
                    d.ui.alerts.push(msg);
                });
                Ok(Value::Void)
            }
            "last_alert" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.last_alert takes 0 arguments", span));
                }
                Ok(self.data(|d| match &d.ui.last_alert {
                    Some(s) => option_some(Value::String(s.clone())),
                    None => option_none(),
                }))
            }
            "window" => {
                if args.len() != 2 {
                    return Err(runtime_err("__ui.window takes 2 arguments", span));
                }
                self.data_mut(|d| {
                    d.ui.windows.push(UiWindow {
                        handle: args[0].clone(),
                        body: args[1].clone(),
                    });
                });
                Ok(Value::Void)
            }
            "begin" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.begin takes 1 argument", span));
                }
                let Some(id) = handle_id(&args[0]) else {
                    return Err(runtime_err("__ui.begin expects a widget handle", span));
                };
                self.data_mut(|d| {
                    d.ui.children.insert(id, Vec::new());
                    d.ui.stack.push(id);
                });
                Ok(Value::Void)
            }
            "end" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.end takes 0 arguments", span));
                }
                self.data_mut(|d| {
                    d.ui.stack.pop();
                });
                Ok(Value::Void)
            }
            "widget" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.widget takes 1 argument", span));
                }
                self.data_mut(|d| d.ui.push_child(args[0].clone()));
                Ok(Value::Void)
            }
            "field" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.field takes 1 argument", span));
                }
                if handle_id(&args[0]).is_none() {
                    return Err(runtime_err("__ui.field expects a widget handle", span));
                }
                let current = field_str(&args[0], "label");
                let next = self.data_mut(|d| {
                    d.ui.push_child(args[0].clone());
                    d.ui.take_input(Value::String(current))
                });
                set_field(&args[0], "label", next.clone());
                Ok(next)
            }
            "checkbox" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.checkbox takes 1 argument", span));
                }
                if handle_id(&args[0]).is_none() {
                    return Err(runtime_err("__ui.checkbox expects a widget handle", span));
                }
                let current = style_bool(&args[0], "on");
                let next = self.data_mut(|d| {
                    d.ui.push_child(args[0].clone());
                    d.ui.take_input(Value::Bool(current))
                });
                set_style_entry(&args[0], "on", next.clone());
                Ok(next)
            }
            "slider" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.slider takes 1 argument", span));
                }
                if handle_id(&args[0]).is_none() {
                    return Err(runtime_err("__ui.slider expects a widget handle", span));
                }
                let (lo, hi) =
                    slider_range(style_float(&args[0], "min"), style_float(&args[0], "max"));
                let current = style_float(&args[0], "value").clamp(lo, hi);
                let next = self.data_mut(|d| {
                    d.ui.push_child(args[0].clone());
                    d.ui.take_input(Value::Float(current))
                });
                set_style_entry(&args[0], "value", next.clone());
                Ok(next)
            }
            "select" => {
                if args.len() != 1 {
                    return Err(runtime_err("__ui.select takes 1 argument", span));
                }
                if handle_id(&args[0]).is_none() {
                    return Err(runtime_err("__ui.select expects a widget handle", span));
                }
                let current = field_str(&args[0], "label");
                let next = self.data_mut(|d| {
                    d.ui.push_child(args[0].clone());
                    d.ui.take_input(Value::String(current))
                });
                set_field(&args[0], "label", next.clone());
                Ok(next)
            }
            "open" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.open takes 0 arguments", span));
                }
                if !self.data(|d| d.ui.running) {
                    return Ok(option_none());
                }
                Ok(native_file_path(false))
            }
            "save" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.save takes 0 arguments", span));
                }
                if !self.data(|d| d.ui.running) {
                    return Ok(option_none());
                }
                Ok(native_file_path(true))
            }
            "quit" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.quit takes 0 arguments", span));
                }
                self.data_mut(|d| d.ui.quit = true);
                Ok(Value::Void)
            }
            "invalidate" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.invalidate takes 0 arguments", span));
                }
                self.data_mut(|d| d.ui.invalidate = true);
                Ok(Value::Void)
            }
            "bind" => {
                if args.len() != 2 {
                    return Err(runtime_err("__ui.bind takes 2 arguments", span));
                }
                let Some(id) = handle_id(&args[0]) else {
                    return Err(runtime_err("__ui.bind expects a widget handle", span));
                };
                self.data_mut(|d| {
                    d.ui.actions.insert(id, args[1].clone());
                });
                Ok(Value::Void)
            }
            "pump" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.pump takes 0 arguments", span));
                }
                self.ui_pump(span)?;
                Ok(Value::Void)
            }
            "run" => {
                if !args.is_empty() {
                    return Err(runtime_err("__ui.run takes 0 arguments", span));
                }
                self.ui_run(span)
            }
            _ => Err(runtime_err(
                format!("unknown stdlib function __ui.{name}"),
                span,
            )),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ui_pump(&mut self, span: Span) -> Result<(), RuntimeError> {
        // Rebuild the widget tree each frame (immediate mode). Theme / alerts / registered
        // windows stay; children and click bindings are per-frame.
        self.data_mut(|d| {
            d.ui.children.clear();
            d.ui.actions.clear();
            d.ui.stack.clear();
            d.ui.input_cursor = 0;
        });
        let windows = self.data(|d| {
            d.ui.windows
                .iter()
                .map(|w| (w.handle.clone(), w.body.clone()))
                .collect::<Vec<_>>()
        });
        for (handle, body) in windows {
            if let Some(id) = handle_id(&handle) {
                self.data_mut(|d| {
                    d.ui.children.insert(id, Vec::new());
                    d.ui.stack.push(id);
                });
            }
            let result = self.call_value(body, vec![], span);
            self.data_mut(|d| {
                d.ui.stack.pop();
            });
            result?;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ui_run(&mut self, span: Span) -> Result<Value, RuntimeError> {
        let n = self.data(|d| d.ui.windows.len());
        if n == 0 {
            return Ok(Value::Int(0));
        }
        native::run(self, span)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::any::Any;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::ptr::NonNull;

    use eframe::egui::{
        self, Button, CentralPanel, Color32, ComboBox, Context, Frame, Modal, ProgressBar,
        RichText, ScrollArea, Slider, TextEdit, ViewportCommand, Window,
    };

    use crate::{RuntimeError, Span};

    use super::{EvalContext, Value, field_str, handle_id, runtime_err, style_map, style_strings};

    thread_local! {
        static UI_EVAL: Cell<Option<NonNull<EvalContext>>> = const { Cell::new(None) };
    }

    fn native_options(title: &str) -> eframe::NativeOptions {
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([520.0, 360.0])
                .with_min_inner_size([320.0, 200.0]),
            centered: true,
            ..Default::default()
        }
    }

    fn panic_text(payload: Box<dyn Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        }
    }

    pub(super) fn run(ctx: &mut EvalContext, span: Span) -> Result<Value, RuntimeError> {
        let title = ctx.data(|d| {
            d.ui.windows
                .first()
                .map(|w| {
                    let label = field_str(&w.handle, "label");
                    if label.is_empty() {
                        "RoseGold".to_string()
                    } else {
                        label
                    }
                })
                .unwrap_or_else(|| "RoseGold".to_string())
        });

        ctx.data_mut(|d| d.ui.running = true);
        let ptr = NonNull::from(&mut *ctx);
        UI_EVAL.with(|c| c.set(Some(ptr)));
        let result = catch_unwind(AssertUnwindSafe(|| {
            eframe::run_native(
                &title,
                native_options(&title),
                Box::new(|_cc| Ok(Box::new(App { span }))),
            )
        }));
        UI_EVAL.with(|c| c.set(None));
        ctx.data_mut(|d| d.ui.running = false);

        match result {
            Ok(Ok(())) => {
                if let Some(msg) = ctx.data(|d| d.ui.frame_error.clone()) {
                    Err(runtime_err(msg, span))
                } else {
                    Ok(Value::Int(0))
                }
            }
            Ok(Err(e)) => Err(runtime_err(
                format!("ui.run failed: {e} (needs a working GPU/OpenGL display; glow backend)"),
                span,
            )),
            Err(payload) => Err(runtime_err(
                format!(
                    "ui.run panicked: {} (needs a working GPU/OpenGL display; glow backend)",
                    panic_text(payload)
                ),
                span,
            )),
        }
    }

    struct App {
        span: Span,
    }

    impl eframe::App for App {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let Some(eval) = UI_EVAL.with(|c| c.get()) else {
                return;
            };
            // Safety: `run` keeps EvalContext on this thread for `run_native`.
            let eval = unsafe { &mut *eval.as_ptr() };
            if let Err(e) = frame(eval, ui, self.span) {
                eval.data_mut(|d| d.ui.frame_error = Some(e.message.clone()));
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn frame(eval: &mut EvalContext, ui: &mut egui::Ui, span: Span) -> Result<(), RuntimeError> {
        let theme = eval.data(|d| d.ui.theme.clone());
        apply_theme(ui.ctx(), &theme);
        eval.ui_pump(span)?;
        eval.data_mut(|d| {
            d.ui.paint_cursor = 0;
            d.ui.egui_salt = 0;
        });

        let windows = eval.data(|d| {
            d.ui.windows
                .iter()
                .map(|w| w.handle.clone())
                .collect::<Vec<_>>()
        });

        if windows.len() == 1 {
            CentralPanel::default().show(ui, |ui| {
                paint_node(eval, ui, &windows[0]);
            });
        } else {
            for handle in &windows {
                let title = field_str(handle, "label");
                Window::new(title).show(ui.ctx(), |ui| {
                    paint_node(eval, ui, handle);
                });
            }
        }

        show_alerts(eval, ui.ctx());

        let (quit, need_repaint) = eval.data_mut(|d| {
            let quit = d.ui.quit;
            let need = d.ui.invalidate || d.ui.dirty;
            d.ui.invalidate = false;
            d.ui.dirty = false;
            (quit, need)
        });
        if quit {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        } else if need_repaint {
            ui.ctx().request_repaint();
        }
        Ok(())
    }

    fn show_alerts(eval: &mut EvalContext, ctx: &Context) {
        let Some(msg) = eval.data(|d| d.ui.alerts.first().cloned()) else {
            return;
        };
        let dismissed = Modal::new(egui::Id::new("rosegold_alert")).show(ctx, |ui| {
            ui.heading("Alert");
            ui.label(&msg);
            ui.add_space(8.0);
            ui.button("OK").clicked()
        });
        if dismissed.inner || dismissed.backdrop_response.clicked() {
            eval.data_mut(|d| {
                if !d.ui.alerts.is_empty() {
                    d.ui.alerts.remove(0);
                }
                d.ui.dirty = true;
            });
        }
    }

    fn apply_theme(ctx: &Context, theme: &HashMap<String, Value>) {
        if theme.is_empty() {
            return;
        }
        let mut visuals = ctx.global_style().visuals.clone();
        if let Some(c) = map_color(theme, "bg") {
            visuals.panel_fill = c;
            visuals.window_fill = c;
            visuals.extreme_bg_color = c;
        }
        if let Some(c) = map_color(theme, "text") {
            visuals.override_text_color = Some(c);
        }
        if let Some(c) = map_color(theme, "accent") {
            visuals.selection.bg_fill = c;
            visuals.widgets.hovered.bg_fill = c;
            visuals.widgets.active.bg_fill = c;
        }
        ctx.set_visuals(visuals);
        if let Some(sz) = map_number(theme, "font_size") {
            ctx.global_style_mut(|style| {
                for font in style.text_styles.values_mut() {
                    font.size = sz;
                }
            });
        }
    }

    fn paint_node(eval: &mut EvalContext, ui: &mut egui::Ui, handle: &Value) {
        let kind = field_str(handle, "kind");
        let style = style_map(handle);
        let pad = map_number(&style, "padding").unwrap_or(0.0);
        let disabled = matches!(style.get("disabled"), Some(Value::Bool(true)));
        let id = handle_id(handle);

        let mut paint = |ui: &mut egui::Ui| {
            ui.add_enabled_ui(!disabled, |ui| match kind.as_str() {
                "column" | "window" => {
                    ui.vertical(|ui| {
                        let kids = id
                            .and_then(|id| eval.data(|d| d.ui.children.get(&id).cloned()))
                            .unwrap_or_default();
                        for child in kids {
                            paint_node(eval, ui, &child);
                        }
                    });
                }
                "row" => {
                    ui.horizontal(|ui| {
                        let kids = id
                            .and_then(|id| eval.data(|d| d.ui.children.get(&id).cloned()))
                            .unwrap_or_default();
                        for child in kids {
                            paint_node(eval, ui, &child);
                        }
                    });
                }
                "scroll" => {
                    let kids = id
                        .and_then(|id| eval.data(|d| d.ui.children.get(&id).cloned()))
                        .unwrap_or_default();
                    let width = map_number(&style, "width").unwrap_or_else(|| ui.available_width());
                    let height = map_number(&style, "height")
                        .unwrap_or_else(|| ui.available_height().max(64.0));
                    let salt = eval.data_mut(|d| d.ui.next_egui_salt());
                    ui.allocate_ui(egui::vec2(width, height), |ui| {
                        Frame::canvas(ui.style()).show(ui, |ui| {
                            ScrollArea::vertical()
                                .id_salt(("rg_scroll", salt))
                                .auto_shrink([false, false])
                                .max_height(ui.available_height())
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        for child in &kids {
                                            paint_node(eval, ui, child);
                                        }
                                    });
                                });
                        });
                    });
                }
                "button" => {
                    let label = styled_text(field_str(handle, "label"), &style);
                    let mut btn = Button::new(label);
                    if let Some(c) = map_color(&style, "bg").or_else(|| map_color(&style, "color"))
                    {
                        btn = btn.fill(c);
                    }
                    let w = map_number(&style, "width");
                    let h = map_number(&style, "height");
                    if w.is_some() || h.is_some() {
                        btn = btn.min_size(egui::vec2(w.unwrap_or(0.0), h.unwrap_or(0.0)));
                    }
                    if ui.add(btn).clicked() {
                        if let Some(id) = id {
                            if let Some(action) = eval.data(|d| d.ui.actions.get(&id).cloned()) {
                                if let Err(e) = eval.call_value(action, vec![], Span::default()) {
                                    eval.data_mut(|d| d.ui.frame_error = Some(e.message));
                                }
                            }
                        }
                        eval.data_mut(|d| d.ui.dirty = true);
                    }
                }
                "field" => {
                    let mut text = field_str(handle, "label");
                    let idx = eval.data(|d| d.ui.paint_cursor);
                    let mut edit =
                        TextEdit::singleline(&mut text).id(egui::Id::new(("rg_field", idx)));
                    if let Some(w) = map_number(&style, "width") {
                        edit = edit.desired_width(w);
                    }
                    let changed = ui.add(edit).changed();
                    eval.data_mut(|d| d.ui.store_painted(Value::String(text), changed));
                }
                "checkbox" => {
                    let mut on = matches!(style.get("on"), Some(Value::Bool(true)));
                    let label = field_str(handle, "label");
                    let changed = ui.checkbox(&mut on, label).changed();
                    eval.data_mut(|d| d.ui.store_painted(Value::Bool(on), changed));
                }
                "slider" => {
                    let mut value = map_number(&style, "value").unwrap_or(0.0);
                    let min = map_number(&style, "min").unwrap_or(0.0);
                    let max = map_number(&style, "max").unwrap_or(1.0);
                    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
                    let changed = ui.add(Slider::new(&mut value, lo..=hi)).changed();
                    eval.data_mut(|d| d.ui.store_painted(Value::Float(value as f64), changed));
                }
                "select" => {
                    let mut selected = field_str(handle, "label");
                    let options = style_strings(handle, "options");
                    let idx = eval.data(|d| d.ui.paint_cursor);
                    let mut combo =
                        ComboBox::from_id_salt(("rg_select", idx)).selected_text(selected.clone());
                    if let Some(w) = map_number(&style, "width") {
                        combo = combo.width(w);
                    }
                    let before = selected.clone();
                    combo.show_ui(ui, |ui| {
                        for opt in &options {
                            ui.selectable_value(&mut selected, opt.clone(), opt.as_str());
                        }
                    });
                    let changed = selected != before;
                    eval.data_mut(|d| d.ui.store_painted(Value::String(selected), changed));
                }
                "progress" => {
                    let t = map_number(&style, "value").unwrap_or(0.0).clamp(0.0, 1.0);
                    let mut bar = ProgressBar::new(t).show_percentage();
                    if let Some(c) = map_color(&style, "color").or_else(|| map_color(&style, "bg"))
                    {
                        bar = bar.fill(c);
                    }
                    if let Some(w) = map_number(&style, "width") {
                        bar = bar.desired_width(w);
                    }
                    if let Some(h) = map_number(&style, "height") {
                        bar = bar.desired_height(h);
                    }
                    ui.add(bar);
                }
                "separator" => {
                    ui.separator();
                }
                "spacer" => {
                    ui.add_space(map_number(&style, "height").unwrap_or(8.0));
                }
                _ => {
                    let label = styled_text(field_str(handle, "label"), &style);
                    ui.label(label);
                }
            });
        };

        if pad > 0.0 {
            Frame::NONE.inner_margin(pad).show(ui, |ui| paint(ui));
        } else {
            paint(ui);
        }
    }

    fn styled_text(label: String, style: &HashMap<String, Value>) -> RichText {
        let mut text = RichText::new(label);
        if let Some(c) = map_color(style, "color") {
            text = text.color(c);
        }
        if let Some(sz) = map_number(style, "font_size") {
            text = text.size(sz);
        }
        text
    }

    fn map_color(map: &HashMap<String, Value>, key: &str) -> Option<Color32> {
        match map.get(key) {
            Some(Value::String(s)) => parse_hex(s),
            _ => None,
        }
    }

    fn map_number(map: &HashMap<String, Value>, key: &str) -> Option<f32> {
        match map.get(key) {
            Some(Value::Int(n)) => Some(*n as f32),
            Some(Value::Float(n)) => Some(*n as f32),
            _ => None,
        }
    }

    fn parse_hex(s: &str) -> Option<Color32> {
        let s = s.strip_prefix('#')?;
        let (r, g, b) = if s.len() == 6 {
            (
                u8::from_str_radix(&s[0..2], 16).ok()?,
                u8::from_str_radix(&s[2..4], 16).ok()?,
                u8::from_str_radix(&s[4..6], 16).ok()?,
            )
        } else if s.len() == 3 {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            (r * 17, g * 17, b * 17)
        } else {
            return None;
        };
        Some(Color32::from_rgb(r, g, b))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn eframe_app_constructs_without_event_loop() {
            let _opts = native_options("RoseGold");
            let _app = App {
                span: Span::default(),
            };
        }

        #[test]
        fn parse_hex_accepts_css_colors() {
            assert_eq!(
                parse_hex("#c45c26"),
                Some(Color32::from_rgb(0xc4, 0x5c, 0x26))
            );
            assert_eq!(
                parse_hex("#1b1b1b"),
                Some(Color32::from_rgb(0x1b, 0x1b, 0x1b))
            );
            assert_eq!(parse_hex("#f2e"), Some(Color32::from_rgb(0xff, 0x22, 0xee)));
            assert_eq!(parse_hex("c45c26"), None);
        }

        #[test]
        fn native_file_path_skips_dialog_in_tests() {
            fn is_none(v: &Value) -> bool {
                matches!(
                    v,
                    Value::Enum {
                        variant,
                        ..
                    } if variant == "None"
                )
            }
            assert!(is_none(&super::super::native_file_path(false)));
            assert!(is_none(&super::super::native_file_path(true)));
        }
    }
}
