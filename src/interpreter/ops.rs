//! Runtime helpers: arithmetic, bitwise ops, equality, formatting, and host I/O.

use crate::parser::AssignOp;
use crate::{RuntimeError, Span};

use super::value::*;

pub(super) fn runtime_err(message: impl Into<String>, span: Span) -> RuntimeError {
    RuntimeError {
        message: message.into(),
        span,
        exit_code: None,
        trace: Vec::new(),
        file: String::new(),
    }
}

pub(super) fn exit_err(code: i32, span: Span) -> RuntimeError {
    RuntimeError {
        message: format!("exit {code}"),
        span,
        exit_code: Some(code),
        trace: Vec::new(),
        file: String::new(),
    }
}

pub(super) fn attach_trace(
    mut err: RuntimeError,
    name: &str,
    span: Span,
    file: &str,
) -> RuntimeError {
    if err.exit_code.is_some() {
        return err;
    }
    err.trace.push(crate::TraceFrame {
        name: name.to_string(),
        span,
        file: file.to_string(),
    });
    err
}

/// Label used in traces: `helpers.rg` from a path or in-memory module key.
pub(super) fn file_label(key: &str) -> String {
    let name = std::path::Path::new(key)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(key);
    if name.ends_with(".rg") {
        name.to_string()
    } else {
        format!("{name}.rg")
    }
}

pub(super) fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Float(n) => Some(*n),
        Value::Int(n) => Some(*n as f64),
        _ => None,
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
            if std::sync::Arc::ptr_eq(a, b) {
                true
            } else {
                let a = lock(a).clone();
                let b = lock(b).clone();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| value_eq(x, y))
            }
        }
        (Value::Bytes(a), Value::Bytes(b)) => a == b,
        (Value::Map(a), Value::Map(b)) => {
            if std::sync::Arc::ptr_eq(a, b) {
                true
            } else {
                let a = lock(a).clone();
                let b = lock(b).clone();
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).map(|bv| value_eq(v, bv)).unwrap_or(false))
            }
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

pub(super) fn rng_seed() -> u64 {
    let bits = unix_now_secs().to_bits();
    let mix = bits ^ bits.rotate_left(17) ^ 0x9E37_79B9_7F4A_7C15;
    if mix == 0 { 0xA5A5_A5A5_A5A5_A5A5 } else { mix }
}

pub(super) fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

pub(super) fn rng_f64(state: &mut u64) -> f64 {
    (rng_next(state) >> 11) as f64 / ((1u64 << 53) as f64)
}

pub(super) fn time_sleep(secs: f64) {
    let d = duration_secs(secs);
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::sleep(d);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let start = js_sys::Date::now();
        let ms = d.as_secs_f64() * 1000.0;
        while js_sys::Date::now() - start < ms {}
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
        File(Vec<u8>),
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
        let bytes = read_bytes(path)?;
        String::from_utf8(bytes).map_err(|_| format!("not utf-8: {path}"))
    }

    pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| match fs.borrow().get(&path) {
            Some(Entry::File(content)) => Ok(content.clone()),
            Some(Entry::Dir) => Err(format!("is a directory: {path}")),
            None => Err(format!("file not found: {path}")),
        })
    }

    pub fn write(path: &str, content: &str) -> Result<(), String> {
        write_bytes(path, content.as_bytes())
    }

    pub fn write_bytes(path: &str, content: &[u8]) -> Result<(), String> {
        let path = normalize_io_path(path);
        FILES.with(|fs| {
            let mut map = fs.borrow_mut();
            if matches!(map.get(&path), Some(Entry::Dir)) {
                return Err(format!("is a directory: {path}"));
            }
            map.insert(path, Entry::File(content.to_vec()));
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
                    existing.extend_from_slice(content.as_bytes());
                    Ok(())
                }
                None => {
                    map.insert(path, Entry::File(content.as_bytes().to_vec()));
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

pub(super) fn io_read_bytes(path: &str) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::read_bytes(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(path).map_err(|e| e.to_string())
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

pub(super) fn io_write_bytes(path: &str, content: &[u8]) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_fs::write_bytes(path, content)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::write(path, content).map_err(|e| e.to_string())
    }
}

pub(super) fn bytes_from_value(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Bytes(b) => Ok(b.to_vec()),
        Value::Array(a) => {
            let items = lock(a).clone();
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Int(n) if (0..=255).contains(n) => out.push(*n as u8),
                    Value::Int(n) => {
                        return Err(format!("byte {i} out of range 0..255: {n}"));
                    }
                    other => {
                        return Err(format!(
                            "write_bytes array items must be Int, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
            Ok(out)
        }
        _ => Err(format!(
            "write_bytes expects Bytes or Array of Int, got {}",
            v.type_name()
        )),
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

pub(super) fn option_some(value: Value) -> Value {
    Value::Enum {
        module: "Option".to_string(),
        variant: "Some".to_string(),
        value: Some(Box::new(value)),
    }
}

pub(super) fn option_none() -> Value {
    Value::Enum {
        module: "Option".to_string(),
        variant: "None".to_string(),
        value: None,
    }
}

pub(super) fn json_parse(text: &str) -> Result<Value, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid json: {e}"))?;
    Ok(json_to_value(&v))
}

pub(super) fn json_stringify(value: &Value) -> Result<String, String> {
    let v = json_from_value(value)?;
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                if u <= i64::MAX as u64 {
                    Value::Int(u as i64)
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => array_value(arr.iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (k, val) in obj {
                map.insert(k.clone(), json_to_value(val));
            }
            map_value(map)
        }
    }
}

fn json_from_value(v: &Value) -> Result<serde_json::Value, String> {
    match v {
        Value::Int(n) => Ok(serde_json::json!(*n)),
        Value::Float(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "json cannot encode NaN or Infinity".to_string()),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::None | Value::Void => Ok(serde_json::Value::Null),
        Value::Array(a) => {
            let mut arr = Vec::new();
            let items: Vec<Value> = lock(a).clone();
            for item in items {
                arr.push(json_from_value(&item)?);
            }
            Ok(serde_json::Value::Array(arr))
        }
        Value::Map(m) => {
            let mut obj = serde_json::Map::new();
            let entries: Vec<(String, Value)> = lock(m)
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (k, val) in entries {
                obj.insert(k, json_from_value(&val)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        Value::Struct { fields, .. } => {
            let mut obj = serde_json::Map::new();
            let entries: Vec<(String, Value)> = lock(fields)
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (k, val) in entries {
                obj.insert(k, json_from_value(&val)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        Value::Enum {
            module,
            variant,
            value,
        } => {
            let option = module == "Option" || module == "option";
            let result = module == "Result" || module == "result";
            if option && variant == "None" {
                return Ok(serde_json::Value::Null);
            }
            if option && variant == "Some" {
                return match value {
                    Some(inner) => json_from_value(inner),
                    None => Ok(serde_json::Value::Null),
                };
            }
            if result && variant == "Ok" {
                return match value {
                    Some(inner) => json_from_value(inner),
                    None => Ok(serde_json::Value::Null),
                };
            }
            if result && variant == "Err" {
                return Err("json cannot encode Result.Err".to_string());
            }
            Err(format!("json cannot encode enum {module}.{variant}"))
        }
        other => Err(format!("json cannot encode {}", other.type_name())),
    }
}

pub(super) fn process_env(name: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(name).ok()
    }
}

pub(super) fn process_run(cmd: &str, args: &Value) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (cmd, args);
        Err("process.run is not supported on this target".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let arg_strs = match args {
            Value::Array(a) => {
                let items = lock(a);
                let mut out = Vec::with_capacity(items.len());
                for v in items.iter() {
                    match v {
                        Value::String(s) => out.push(s.clone()),
                        other => {
                            return Err(format!(
                                "process.run args must be String, got {}",
                                other.type_name()
                            ));
                        }
                    }
                }
                out
            }
            other => {
                return Err(format!(
                    "process.run expects Array args, got {}",
                    other.type_name()
                ));
            }
        };
        let output = std::process::Command::new(cmd)
            .args(&arg_strs)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            if stderr.trim().is_empty() {
                Err(format!("exit {code}"))
            } else {
                Err(format!("exit {code}: {}", stderr.trim()))
            }
        }
    }
}

pub(super) fn io_read_stdin() -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        Err("io.read_stdin is not supported on this target".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    }
}

/// One line from stdin without the trailing newline. `None` on EOF.
pub(super) fn io_read_line() -> Result<Option<String>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        Err("io.read_line is not supported on this target".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::BufRead;
        let mut line = String::new();
        let n = std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Some(line))
    }
}

pub(super) fn path_join(a: &str, b: &str) -> String {
    std::path::Path::new(a)
        .join(b)
        .to_string_lossy()
        .into_owned()
}

pub(super) fn path_dirname(p: &str) -> String {
    match std::path::Path::new(p).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().into_owned(),
        Some(_) => ".".to_string(),
        None => {
            if p.is_empty() {
                ".".to_string()
            } else {
                p.to_string()
            }
        }
    }
}

pub(super) fn path_ext(p: &str) -> String {
    std::path::Path::new(p)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string()
}

pub(super) fn http_get(url: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = url;
        Err("http is not supported on this target".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        http_call("GET", url, None)
    }
}

pub(super) fn http_post(url: &str, body: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (url, body);
        Err("http is not supported on this target".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        http_call("POST", url, Some(body))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn http_call(method: &str, url: &str, body: Option<&str>) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .redirects(5)
        .build();
    let result = match (method, body) {
        ("POST", Some(b)) => agent.post(url).send_string(b),
        _ => agent.get(url).call(),
    };
    match result {
        Ok(resp) => resp.into_string().map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

pub(super) fn regex_is_match(pattern: &str, text: &str) -> Result<bool, String> {
    let re = regex::Regex::new(pattern).map_err(|e| e.to_string())?;
    Ok(re.is_match(text))
}

pub(super) fn regex_find(pattern: &str, text: &str) -> Result<Option<String>, String> {
    let re = regex::Regex::new(pattern).map_err(|e| e.to_string())?;
    Ok(re.find(text).map(|m| m.as_str().to_string()))
}
