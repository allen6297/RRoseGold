//! Runtime helpers: arithmetic, bitwise ops, equality, formatting, and `io`.

use std::collections::HashMap;

use crate::host::HostEffect;
use crate::parser::AssignOp;
use crate::{RuntimeError, Span};

use super::value::*;

pub(super) fn runtime_err(message: impl Into<String>, span: Span) -> RuntimeError {
    RuntimeError {
        message: message.into(),
        span,
    }
}

pub(super) fn value_as_float(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Float(n)) => Some(*n),
        Some(Value::Int(n)) => Some(*n as f64),
        _ => None,
    }
}

pub(super) fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Float(n) => Some(*n),
        Value::Int(n) => Some(*n as f64),
        _ => None,
    }
}

pub(super) fn csv_has(csv: &str, code: &str) -> bool {
    if code.is_empty() {
        return false;
    }
    csv.split(',').any(|token| token.trim() == code)
}

pub(super) fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::json!(n),
        Value::Float(n) => serde_json::json!(n),
        Value::Bool(b) => serde_json::json!(b),
        Value::String(s) => serde_json::json!(s),
        _ => serde_json::Value::Null,
    }
}

pub(super) fn map_string(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

pub(super) fn map_f64(map: &HashMap<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(as_f64)
}

pub(super) fn spawn_from_map(map: &HashMap<String, Value>) -> HostEffect {
    HostEffect::Spawn {
        name: map_string(map, "name").unwrap_or_else(|| "Entity".into()),
        kind: map_string(map, "kind").unwrap_or_else(|| "sprite".into()),
        x: map_f64(map, "x").unwrap_or(0.0),
        y: map_f64(map, "y").unwrap_or(0.0),
        width: map_f64(map, "w")
            .or_else(|| map_f64(map, "width"))
            .unwrap_or(32.0),
        height: map_f64(map, "h")
            .or_else(|| map_f64(map, "height"))
            .unwrap_or(32.0),
        color: map_string(map, "color").unwrap_or_else(|| "#61afef".into()),
        script: map_string(map, "script"),
    }
}

pub(super) fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Int(a), Value::Float(b)) => (*a as f64 - b).abs() < f64::EPSILON,
        (Value::Float(a), Value::Int(b)) => (a - *b as f64).abs() < f64::EPSILON,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::None, Value::None) => true,
        (Value::Void, Value::Void) => true,
        (Value::Array(a), Value::Array(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| value_eq(x, y))
        }
        (Value::Map(a), Value::Map(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).map(|bv| value_eq(v, bv)).unwrap_or(false))
        }
        (
            Value::Enum {
                module: ma,
                variant: va,
                value: a,
            },
            Value::Enum {
                module: mb,
                variant: vb,
                value: b,
            },
        ) => {
            ma == mb
                && va == vb
                && match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => value_eq(a, b),
                    _ => false,
                }
        }
        _ => false,
    }
}

pub(super) fn format_value(
    value: &Value,
    format: Option<&str>,
    span: Span,
) -> Result<String, RuntimeError> {
    if let Some(spec) = format {
        if spec.ends_with('f') {
            let prec_str = spec.trim_start_matches('.').trim_end_matches('f');
            if let Ok(prec) = prec_str.parse::<usize>() {
                let n = match value {
                    Value::Int(n) => *n as f64,
                    Value::Float(n) => *n,
                    _ => {
                        return Err(runtime_err(
                            format!(
                                "format {:?} requires numeric value, got {}",
                                spec,
                                value.type_name()
                            ),
                            span,
                        ));
                    }
                };
                return Ok(format!("{:.prec$}", n, prec = prec));
            }
        }
        return Err(runtime_err(
            format!("unsupported format spec {:?}", spec),
            span,
        ));
    }
    Ok(value.to_string())
}

pub(super) fn int_bitwise<F: FnOnce(i64, i64) -> i64>(
    l: Value,
    r: Value,
    op: F,
    span: Span,
) -> Result<Value, RuntimeError> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(op(a, b))),
        (l, r) => Err(runtime_err(
            format!(
                "bitwise operators require Int, got {} and {}",
                l.type_name(),
                r.type_name()
            ),
            span,
        )),
    }
}

pub(super) fn int_shift(l: Value, r: Value, left: bool, span: Span) -> Result<Value, RuntimeError> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => {
            if b < 0 || b >= 64 {
                return Err(runtime_err(
                    format!("shift count {b} is out of range"),
                    span,
                ));
            }
            let n = if left {
                a.wrapping_shl(b as u32)
            } else {
                a.wrapping_shr(b as u32)
            };
            Ok(Value::Int(n))
        }
        (l, r) => Err(runtime_err(
            format!(
                "shift requires Int, got {} and {}",
                l.type_name(),
                r.type_name()
            ),
            span,
        )),
    }
}

pub(super) fn string_char_len(s: &str) -> i64 {
    s.chars().count() as i64
}

pub(super) fn int_div(a: i64, b: i64, span: Span) -> Result<Value, RuntimeError> {
    if b == 0 {
        Err(runtime_err("division by zero".to_string(), span))
    } else {
        Ok(Value::Int(a / b))
    }
}

pub(super) fn is_numeric_zero(v: &Value) -> bool {
    match v {
        Value::Int(0) => true,
        Value::Float(n) if *n == 0.0 => true,
        _ => false,
    }
}

pub(super) fn checked_mod(l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
    if is_numeric_zero(&r) {
        return Err(runtime_err("division by zero".to_string(), span));
    }
    numeric_binop(l, r, |a, b| a % b, span)
}

pub(super) fn numeric_binop<F: FnOnce(f64, f64) -> f64>(
    l: Value,
    r: Value,
    op: F,
    span: Span,
) -> Result<Value, RuntimeError> {
    let a = to_float(&l, span)?;
    let b = to_float(&r, span)?;
    let result = op(a, b);
    if result.fract() == 0.0 && result.is_finite() {
        Ok(Value::Int(result as i64))
    } else {
        Ok(Value::Float(result))
    }
}

pub(super) fn to_float(v: &Value, span: Span) -> Result<f64, RuntimeError> {
    match v {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(n) => Ok(*n),
        _ => Err(runtime_err(
            format!("expected numeric, got {}", v.type_name()),
            span,
        )),
    }
}

pub(super) fn compare_op<F: FnOnce(f64, f64) -> bool>(
    l: Value,
    r: Value,
    op: F,
    span: Span,
) -> Result<Value, RuntimeError> {
    let a = to_float(&l, span)?;
    let b = to_float(&r, span)?;
    Ok(Value::Bool(op(a, b)))
}

pub(super) fn apply_assign_op(
    old: Value,
    new: &Value,
    op: &AssignOp,
    span: Span,
) -> Result<Value, RuntimeError> {
    match op {
        AssignOp::Assign => Ok(new.clone()),
        AssignOp::Add => numeric_binop(old, new.clone(), |a, b| a + b, span),
        AssignOp::Sub => numeric_binop(old, new.clone(), |a, b| a - b, span),
        AssignOp::Mul => numeric_binop(old, new.clone(), |a, b| a * b, span),
        AssignOp::Div => numeric_binop(old, new.clone(), |a, b| a / b, span),
        AssignOp::Mod => checked_mod(old, new.clone(), span),
        AssignOp::BitAnd => int_bitwise(old, new.clone(), |a, b| a & b, span),
        AssignOp::BitOr => int_bitwise(old, new.clone(), |a, b| a | b, span),
        AssignOp::BitXor => int_bitwise(old, new.clone(), |a, b| a ^ b, span),
    }
}

/// Start time for `time.elapsed` (this VM, not frame `dt`).
#[derive(Clone, Copy)]
pub(super) struct Clock {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    start_ms: f64,
}

impl Clock {
    pub(super) fn capture() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
            #[cfg(target_arch = "wasm32")]
            start_ms: js_sys::Date::now(),
        }
    }

    pub(super) fn elapsed_secs(self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_secs_f64()
        }
        #[cfg(target_arch = "wasm32")]
        {
            ((js_sys::Date::now() - self.start_ms) / 1000.0).max(0.0)
        }
    }
}

pub(super) fn unix_now_secs() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() / 1000.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

#[cfg(target_arch = "wasm32")]
fn normalize_io_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        ".".to_string()
    } else {
        p.to_string()
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_fs {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    use super::normalize_io_path;

    enum Entry {
        File(String),
        Dir,
    }

    thread_local! {
      static FILES: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    }

    fn child_name(full: &str, dir: &str) -> Option<String> {
        let rest = if dir == "." {
            full
        } else {
            full.strip_prefix(&format!("{dir}/"))?
        };
        if rest.is_empty() {
            return None;
        }
        Some(rest.split('/').next()?.to_string())
    }

    fn has_children(map: &HashMap<String, Entry>, dir: &str) -> bool {
        map.keys().any(|k| child_name(k, dir).is_some())
    }

    fn is_dir_entry(map: &HashMap<String, Entry>, path: &str) -> bool {
        matches!(map.get(path), Some(Entry::Dir)) || has_children(map, path)
    }

    pub fn read(path: &str) -> Result<String, String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| match fs.borrow().get(&path) {
            Some(Entry::File(content)) => Ok(content.clone()),
            Some(Entry::Dir) => Err(format!("is a directory: {path}")),
            None => Err(format!("file not found: {path}")),
        })
    }

    pub fn write(path: &str, content: &str) -> Result<(), String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let mut map = fs.borrow_mut();
            if matches!(map.get(&path), Some(Entry::Dir)) {
                return Err(format!("is a directory: {path}"));
            }
            map.insert(path, Entry::File(content.to_string()));
            Ok(())
        })
    }

    pub fn append(path: &str, content: &str) -> Result<(), String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let mut map = fs.borrow_mut();
            match map.get_mut(&path) {
                Some(Entry::Dir) => Err(format!("is a directory: {path}")),
                Some(Entry::File(existing)) => {
                    existing.push_str(content);
                    Ok(())
                }
                None => {
                    map.insert(path, Entry::File(content.to_string()));
                    Ok(())
                }
            }
        })
    }

    pub fn remove(path: &str) -> Result<(), String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let mut map = fs.borrow_mut();
            match map.remove(&path) {
                Some(Entry::File(_)) => Ok(()),
                Some(Entry::Dir) => {
                    map.insert(path.clone(), Entry::Dir);
                    Err(format!("is a directory: {path}"))
                }
                None => Err(format!("file not found: {path}")),
            }
        })
    }

    pub fn exists(path: &str) -> bool {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let map = fs.borrow();
            map.contains_key(&path) || is_dir_entry(&map, &path)
        })
    }

    pub fn is_dir(path: &str) -> bool {
        let path = normalize_io_path(path);
        FILES.with(|fs| is_dir_entry(&fs.borrow(), &path))
    }

    pub fn mkdir(path: &str) -> Result<(), String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let mut map = fs.borrow_mut();
            if matches!(map.get(&path), Some(Entry::File(_))) {
                return Err(format!("file exists: {path}"));
            }
            map.insert(path, Entry::Dir);
            Ok(())
        })
    }

    pub fn list_dir(path: &str) -> Result<Vec<String>, String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let map = fs.borrow();
            if matches!(map.get(&path), Some(Entry::File(_))) {
                return Err(format!("not a directory: {path}"));
            }
            if !is_dir_entry(&map, &path) {
                return Err(format!("file not found: {path}"));
            }
            let mut names: HashSet<String> = HashSet::new();
            for key in map.keys() {
                if let Some(name) = child_name(key, &path) {
                    names.insert(name);
                }
            }
            let mut names: Vec<String> = names.into_iter().collect();
            names.sort();
            Ok(names)
        })
    }
}

pub(super) fn io_read_text(path: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::read(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}

pub(super) fn io_write_text(path: &str, content: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::write(path, content)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::write(path, content).map_err(|e| e.to_string())
    }
}

pub(super) fn io_append_text(path: &str, content: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::append(path, content)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(content.as_bytes()))
            .map_err(|e| e.to_string())
    }
}

pub(super) fn io_remove(path: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::remove(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

pub(super) fn io_exists(path: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::exists(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::path::Path::new(path).exists()
    }
}

pub(super) fn io_is_dir(path: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::is_dir(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::path::Path::new(path).is_dir()
    }
}

pub(super) fn io_mkdir(path: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::mkdir(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())
    }
}

pub(super) fn io_list_dir(path: &str) -> Result<Vec<String>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::list_dir(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut names = Vec::new();
        let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        for entry in rd {
            let entry = entry.map_err(|e| e.to_string())?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }
}

pub(super) fn result_ok(value: Value) -> Value {
    Value::Enum {
        module: "Result".to_string(),
        variant: "Ok".to_string(),
        value: Some(Box::new(value)),
    }
}

pub(super) fn result_err(message: String) -> Value {
    Value::Enum {
        module: "Result".to_string(),
        variant: "Err".to_string(),
        value: Some(Box::new(Value::String(message))),
    }
}
