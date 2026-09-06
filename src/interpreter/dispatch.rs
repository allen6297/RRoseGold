//! Call dispatch: builtins, instance methods, UFCS, and host modules
//! (`strata`, `input`, `io`, `time`, `ui`, `__math`, `__str`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::host::HostEffect;
use crate::{RuntimeError, Span};

use super::ops::*;
use super::value::*;

impl super::eval::EvalContext {
    pub(super) fn call_builtin_or_fn(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if let Some(Value::NativeFn {
            module,
            name: fn_name,
        }) = self.env.get(name)
        {
            return self.call_qualified(&module, &fn_name, args, span);
        }
        match name {
            "print" => {
                let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                self.stdout.push_str(&parts.join(" "));
                self.stdout.push('\n');
                Ok(Value::Void)
            }
            "len" => {
                if args.len() != 1 {
                    return Err(runtime_err("len takes 1 argument".to_string(), span));
                }
                match &args[0] {
                    Value::String(s) => Ok(Value::Int(string_char_len(s))),
                    Value::Array(a) => Ok(Value::Int(a.borrow().len() as i64)),
                    Value::Map(m) => Ok(Value::Int(m.borrow().len() as i64)),
                    _ => Err(runtime_err(
                        format!(
                            "len expects String, Array, or Map, got {}",
                            args[0].type_name()
                        ),
                        span,
                    )),
                }
            }
            "assert" => {
                if args.len() != 1 {
                    return Err(runtime_err("assert takes 1 argument".to_string(), span));
                }
                if !args[0].truthy() {
                    return Err(runtime_err("assertion failed".to_string(), span));
                }
                Ok(Value::Void)
            }
            "Array" => Ok(Value::Array(Rc::new(RefCell::new(args)))),
            "Map" => {
                if args.len() % 2 != 0 {
                    return Err(runtime_err(
                        "Map literal requires even number of key/value arguments".to_string(),
                        span,
                    ));
                }
                let mut map = HashMap::new();
                for chunk in args.chunks(2) {
                    let key = match &chunk[0] {
                        Value::String(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => {
                            return Err(runtime_err(
                                format!("Map key must be scalar, got {}", chunk[0].type_name()),
                                span,
                            ));
                        }
                    };
                    map.insert(key, chunk[1].clone());
                }
                Ok(Value::Map(Rc::new(RefCell::new(map))))
            }
            _ => {
                if self.lookup_fn(name).is_some() {
                    self.call_fn(name, args, span)
                } else if let Some(object) = self.env.get("self") {
                    let type_name = match &object {
                        Value::Struct { name: n, .. } => Some(n.clone()),
                        Value::Enum { module, .. } => Some(module.clone()),
                        _ => None,
                    };
                    if let Some(ty) = type_name {
                        if let Some(v) = self.call_type_method(&ty, &object, name, args, span)? {
                            return Ok(v);
                        }
                    }
                    Err(runtime_err(format!("undefined function '{}'", name), span))
                } else {
                    Err(runtime_err(format!("undefined function '{}'", name), span))
                }
            }
        }
    }

    pub(super) fn call_member(
        &mut self,
        object: &Value,
        name: &str,
        _args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if let Value::Signal {
            name: signal,
            arity,
        } = object
        {
            if name != "emit" {
                return Err(runtime_err(
                    format!("signal '{signal}' has no method '{name}'"),
                    span,
                ));
            }
            if _args.len() != *arity {
                return Err(runtime_err(
                    format!(
                        "signal '{signal}' expected {arity} args, got {}",
                        _args.len()
                    ),
                    span,
                ));
            }
            let args = _args.iter().map(value_to_json).collect();
            self.effects.push(HostEffect::Emit {
                signal: signal.clone(),
                args,
            });
            return Ok(Value::Void);
        }
        if name == "emit" {
            return Err(runtime_err("unknown signal".to_string(), span));
        }
        // User-defined methods on structs/enums (impl blocks)
        let type_key = match object {
            Value::Struct { name: n, .. } => Some(n.clone()),
            Value::Enum { module, .. } => Some(module.clone()),
            _ => None,
        };
        if let Some(type_name) = type_key {
            if let Some(v) = self.call_type_method(&type_name, object, name, _args.clone(), span)? {
                return Ok(v);
            }
        }

        self.dispatch_value_method(object, name, _args, span)
    }

    fn ufcs_or_err(
        &mut self,
        object: &Value,
        name: &str,
        args: Vec<Value>,
        span: Span,
        err: String,
    ) -> Result<Value, RuntimeError> {
        if let Some(decl) = self.lookup_fn(name) {
            if decl.is_ufcs {
                let mut args = args;
                args.insert(0, object.clone());
                return self.call_fn_decl(&decl, args, span);
            }
        }
        Err(runtime_err(err, span))
    }

    fn dispatch_value_method(
        &mut self,
        object: &Value,
        name: &str,
        _args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match object {
            Value::String(s) => match name {
                "len" => Ok(Value::Int(string_char_len(s))),
                "push" | "pop" => self.ufcs_or_err(
                    object,
                    name,
                    _args,
                    span,
                    format!("String has no method '{name}'"),
                ),
                _ => self.ufcs_or_err(
                    object,
                    name,
                    _args,
                    span,
                    format!("String has no method '{name}'"),
                ),
            },
            Value::Array(a) => match name {
                "len" => Ok(Value::Int(a.borrow().len() as i64)),
                "push" => {
                    if _args.len() != 1 {
                        return Err(runtime_err("Array.push takes 1 argument".to_string(), span));
                    }
                    a.borrow_mut().push(_args[0].clone());
                    Ok(Value::Void)
                }
                "pop" => Ok(a.borrow_mut().pop().unwrap_or(Value::None)),
                "first" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Array.first takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    Ok(a.borrow().first().cloned().unwrap_or(Value::None))
                }
                "last" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Array.last takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    Ok(a.borrow().last().cloned().unwrap_or(Value::None))
                }
                "contains" => {
                    if _args.len() != 1 {
                        return Err(runtime_err(
                            "Array.contains takes 1 argument".to_string(),
                            span,
                        ));
                    }
                    let found = a.borrow().iter().any(|v| value_eq(v, &_args[0]));
                    Ok(Value::Bool(found))
                }
                _ => self.ufcs_or_err(
                    object,
                    name,
                    _args,
                    span,
                    format!("Array has no method '{name}'"),
                ),
            },
            Value::Map(m) => match name {
                "len" => Ok(Value::Int(m.borrow().len() as i64)),
                "has" => {
                    if _args.len() != 1 {
                        return Err(runtime_err("Map.has takes 1 argument".to_string(), span));
                    }
                    let key = match &_args[0] {
                        Value::String(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => {
                            return Err(runtime_err(
                                format!("Map key must be scalar, got {}", _args[0].type_name()),
                                span,
                            ));
                        }
                    };
                    Ok(Value::Bool(m.borrow().contains_key(&key)))
                }
                "keys" => Ok(Value::Array(Rc::new(RefCell::new(
                    m.borrow()
                        .keys()
                        .map(|k| Value::String(k.clone()))
                        .collect::<Vec<_>>(),
                )))),
                "remove" => {
                    if _args.len() != 1 {
                        return Err(runtime_err("Map.remove takes 1 argument".to_string(), span));
                    }
                    let key = match &_args[0] {
                        Value::String(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => {
                            return Err(runtime_err(
                                format!("Map key must be scalar, got {}", _args[0].type_name()),
                                span,
                            ));
                        }
                    };
                    Ok(m.borrow_mut().remove(&key).unwrap_or(Value::None))
                }
                "insert" => {
                    if _args.len() != 2 {
                        return Err(runtime_err(
                            "Map.insert takes 2 arguments".to_string(),
                            span,
                        ));
                    }
                    let key = match &_args[0] {
                        Value::String(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => {
                            return Err(runtime_err(
                                format!("Map key must be scalar, got {}", _args[0].type_name()),
                                span,
                            ));
                        }
                    };
                    m.borrow_mut().insert(key, _args[1].clone());
                    Ok(Value::Void)
                }
                _ => self.ufcs_or_err(
                    object,
                    name,
                    _args,
                    span,
                    format!("Map has no method '{name}'"),
                ),
            },
            Value::Enum {
                module,
                variant,
                value,
            } => match name {
                "is_some" => Ok(Value::Bool(*module == "Option" && *variant == "Some")),
                "is_none" => Ok(Value::Bool(*module == "Option" && *variant == "None")),
                "is_ok" => Ok(Value::Bool(*module == "Result" && *variant == "Ok")),
                "is_err" => Ok(Value::Bool(*module == "Result" && *variant == "Err")),
                "unwrap" => match value {
                    Some(v) => Ok((**v).clone()),
                    None => Err(runtime_err(
                        format!("called unwrap on {}.{}", module, variant),
                        span,
                    )),
                },
                "unwrap_or" => {
                    if _args.len() != 1 {
                        return Err(runtime_err("unwrap_or takes 1 argument".to_string(), span));
                    }
                    match value {
                        Some(v) => Ok((**v).clone()),
                        None => Ok(_args[0].clone()),
                    }
                }
                _ => self.ufcs_or_err(
                    object,
                    name,
                    _args,
                    span,
                    format!("Enum {module} has no method '{name}'"),
                ),
            },
            Value::EnumType(e) => {
                if let Some(v) = e.variants.get(name) {
                    if v.arity == 0 {
                        if !_args.is_empty() {
                            return Err(runtime_err(
                                format!("{}.{} does not take arguments", e.name, name),
                                span,
                            ));
                        }
                        return Ok(Value::Enum {
                            module: e.name.clone(),
                            variant: name.to_string(),
                            value: None,
                        });
                    }
                    let value = if _args.len() == 1 {
                        _args[0].clone()
                    } else {
                        Value::Array(Rc::new(RefCell::new(_args)))
                    };
                    return Ok(Value::Enum {
                        module: e.name.clone(),
                        variant: name.to_string(),
                        value: Some(Box::new(value)),
                    });
                }
                Err(runtime_err(
                    format!("enum {} has no variant '{}'", e.name, name),
                    span,
                ))
            }
            Value::Module(m) => {
                let decl = {
                    let m = m.borrow();
                    m.functions.get(name).cloned()
                };
                if let Some(decl) = decl {
                    let fns = m.borrow().functions.clone();
                    self.module_fns.push(fns);
                    let result = self.call_fn_decl(&decl, _args, span);
                    self.module_fns.pop();
                    return result;
                }
                if _args.is_empty() {
                    if let Some(value) = m.borrow().values.get(name) {
                        return Ok(value.clone());
                    }
                }
                Err(runtime_err(
                    format!("module has no member '{}'", name),
                    span,
                ))
            }
            Value::Struct {
                name: struct_name, ..
            } => self.ufcs_or_err(
                object,
                name,
                _args,
                span,
                format!("struct {struct_name} has no method '{name}'"),
            ),
            _ => self.ufcs_or_err(
                object,
                name,
                _args,
                span,
                format!("type {} has no method '{name}'", object.type_name()),
            ),
        }
    }

    pub fn call_qualified(
        &mut self,
        module: &str,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match (module, name) {
            ("__str", "contains") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "str.contains takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(runtime_err(
                            "str.contains first arg must be String".to_string(),
                            span,
                        ));
                    }
                };
                let sub = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(runtime_err(
                            "str.contains second arg must be String".to_string(),
                            span,
                        ));
                    }
                };
                Ok(Value::Bool(s.contains(&sub)))
            }
            ("__str", "starts_with") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "str.starts_with takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let sub = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::Bool(s.starts_with(&sub)))
            }
            ("__str", "ends_with") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "str.ends_with takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let sub = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::Bool(s.ends_with(&sub)))
            }
            ("__str", "length") => {
                if args.len() != 1 {
                    return Err(runtime_err("str.length takes 1 argument".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::Int(string_char_len(&s)))
            }
            ("__str", "is_empty") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "str.is_empty takes 1 argument".to_string(),
                        span,
                    ));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::Bool(s.is_empty()))
            }
            ("__str", "repeat") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "str.repeat takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let n = match &args[1] {
                    Value::Int(n) => *n,
                    _ => return Err(runtime_err("expected Int".to_string(), span)),
                };
                Ok(Value::String(s.repeat(n.max(0) as usize)))
            }
            ("__str", "upper") => {
                if args.len() != 1 {
                    return Err(runtime_err("str.upper takes 1 argument".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::String(s.to_uppercase()))
            }
            ("__str", "lower") => {
                if args.len() != 1 {
                    return Err(runtime_err("str.lower takes 1 argument".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::String(s.to_lowercase()))
            }
            ("__str", "trim") => {
                if args.len() != 1 {
                    return Err(runtime_err("str.trim takes 1 argument".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                Ok(Value::String(s.trim().to_string()))
            }
            ("__str", "split") => {
                if args.len() != 2 {
                    return Err(runtime_err("str.split takes 2 arguments".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let sep = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let parts: Vec<Value> = if sep.is_empty() {
                    vec![Value::String(s)]
                } else {
                    s.split(&sep)
                        .map(|p| Value::String(p.to_string()))
                        .collect()
                };
                Ok(Value::Array(Rc::new(RefCell::new(parts))))
            }
            ("__str", "slice") => {
                if args.len() != 3 {
                    return Err(runtime_err("str.slice takes 3 arguments".to_string(), span));
                }
                let s = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(runtime_err("expected String".to_string(), span)),
                };
                let start = match &args[1] {
                    Value::Int(n) => *n,
                    _ => return Err(runtime_err("str.slice start must be Int".to_string(), span)),
                };
                let end = match &args[2] {
                    Value::Int(n) => *n,
                    _ => return Err(runtime_err("str.slice end must be Int".to_string(), span)),
                };
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let start = start.clamp(0, len) as usize;
                let end = end.clamp(0, len) as usize;
                let end = end.max(start);
                Ok(Value::String(chars[start..end].iter().collect()))
            }
            ("io", "read_text") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "io.read_text takes 1 argument".to_string(),
                        span,
                    ));
                }
                let path = expect_string_arg(&args, 0, "io.read_text", "path", span)?;
                Ok(match io_read_text(&path) {
                    Ok(content) => result_ok(Value::String(content)),
                    Err(e) => result_err(e),
                })
            }
            ("io", "read_lines") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "io.read_lines takes 1 argument".to_string(),
                        span,
                    ));
                }
                let path = expect_string_arg(&args, 0, "io.read_lines", "path", span)?;
                Ok(match io_read_text(&path) {
                    Ok(content) => {
                        let lines: Vec<Value> = content
                            .lines()
                            .map(|line| Value::String(line.to_string()))
                            .collect();
                        result_ok(Value::Array(Rc::new(RefCell::new(lines))))
                    }
                    Err(e) => result_err(e),
                })
            }
            ("io", "write_text") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "io.write_text takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let path = expect_string_arg(&args, 0, "io.write_text", "path", span)?;
                let content = expect_string_arg(&args, 1, "io.write_text", "content", span)?;
                Ok(io_unit_result(io_write_text(&path, &content)))
            }
            ("io", "append_text") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "io.append_text takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let path = expect_string_arg(&args, 0, "io.append_text", "path", span)?;
                let content = expect_string_arg(&args, 1, "io.append_text", "content", span)?;
                Ok(io_unit_result(io_append_text(&path, &content)))
            }
            ("io", "remove") => {
                if args.len() != 1 {
                    return Err(runtime_err("io.remove takes 1 argument".to_string(), span));
                }
                let path = expect_string_arg(&args, 0, "io.remove", "path", span)?;
                Ok(io_unit_result(io_remove(&path)))
            }
            ("io", "exists") => {
                if args.len() != 1 {
                    return Err(runtime_err("io.exists takes 1 argument".to_string(), span));
                }
                let path = expect_string_arg(&args, 0, "io.exists", "path", span)?;
                Ok(Value::Bool(io_exists(&path)))
            }
            ("io", "is_dir") => {
                if args.len() != 1 {
                    return Err(runtime_err("io.is_dir takes 1 argument".to_string(), span));
                }
                let path = expect_string_arg(&args, 0, "io.is_dir", "path", span)?;
                Ok(Value::Bool(io_is_dir(&path)))
            }
            ("io", "mkdir") => {
                if args.len() != 1 {
                    return Err(runtime_err("io.mkdir takes 1 argument".to_string(), span));
                }
                let path = expect_string_arg(&args, 0, "io.mkdir", "path", span)?;
                Ok(io_unit_result(io_mkdir(&path)))
            }
            ("io", "list_dir") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "io.list_dir takes 1 argument".to_string(),
                        span,
                    ));
                }
                let path = expect_string_arg(&args, 0, "io.list_dir", "path", span)?;
                Ok(match io_list_dir(&path) {
                    Ok(names) => {
                        let values: Vec<Value> = names.into_iter().map(Value::String).collect();
                        result_ok(Value::Array(Rc::new(RefCell::new(values))))
                    }
                    Err(e) => result_err(e),
                })
            }
            ("time", "now") => {
                if !args.is_empty() {
                    return Err(runtime_err("time.now takes 0 arguments".to_string(), span));
                }
                Ok(Value::Float(unix_now_secs()))
            }
            ("time", "elapsed") => {
                if !args.is_empty() {
                    return Err(runtime_err(
                        "time.elapsed takes 0 arguments".to_string(),
                        span,
                    ));
                }
                Ok(Value::Float(self.started.elapsed_secs()))
            }
            ("ui", "text") => {
                if args.len() != 3 {
                    return Err(runtime_err(
                        "ui.text takes 3 arguments".to_string(),
                        span,
                    ));
                }
                let x = as_f64(&args[0]).ok_or_else(|| {
                    runtime_err("ui.text expects numbers for x, y".to_string(), span)
                })?;
                let y = as_f64(&args[1]).ok_or_else(|| {
                    runtime_err("ui.text expects numbers for x, y".to_string(), span)
                })?;
                let text = match &args[2] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                self.effects.push(HostEffect::UiText { x, y, text });
                Ok(Value::Void)
            }
            ("Array", "first") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "Array.first takes 1 argument".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::Array(a) => Ok(a.borrow().first().cloned().unwrap_or(Value::None)),
                    _ => Err(runtime_err("Array.first expects Array".to_string(), span)),
                }
            }
            ("Array", "last") => {
                if args.len() != 1 {
                    return Err(runtime_err("Array.last takes 1 argument".to_string(), span));
                }
                match &args[0] {
                    Value::Array(a) => Ok(a.borrow().last().cloned().unwrap_or(Value::None)),
                    _ => Err(runtime_err("Array.last expects Array".to_string(), span)),
                }
            }
            ("Array", "contains") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "Array.contains takes 2 arguments".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::Array(a) => Ok(Value::Bool(
                        a.borrow().iter().any(|v| value_eq(v, &args[1])),
                    )),
                    _ => Err(runtime_err(
                        "Array.contains expects Array".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "pow") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "__math.pow takes 2 arguments".to_string(),
                        span,
                    ));
                }
                match (&args[0], &args[1]) {
                    (Value::Int(base), Value::Int(exp)) => {
                        if *exp < 0 {
                            Ok(Value::Int(0))
                        } else {
                            Ok(Value::Int(base.pow(*exp as u32)))
                        }
                    }
                    (Value::Float(base), Value::Int(exp)) => {
                        if *exp < 0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(base.powi(*exp as i32)))
                        }
                    }
                    (Value::Float(base), Value::Float(exp)) => Ok(Value::Float(base.powf(*exp))),
                    (Value::Int(base), Value::Float(exp)) => {
                        Ok(Value::Float((*base as f64).powf(*exp)))
                    }
                    _ => Err(runtime_err(
                        "__math.pow expects numeric arguments".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "to_int") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "__math.to_int takes 1 argument".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::Float(n) => Ok(Value::Int(*n as i64)),
                    Value::Int(n) => Ok(Value::Int(*n)),
                    _ => Err(runtime_err(
                        "__math.to_int expects Float or Int".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "to_float") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "__math.to_float takes 1 argument".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Float(n) => Ok(Value::Float(*n)),
                    _ => Err(runtime_err(
                        "__math.to_float expects Float or Int".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "sqrt") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "__math.sqrt takes 1 argument".to_string(),
                        span,
                    ));
                }
                match as_f64(&args[0]) {
                    Some(n) => Ok(Value::Float(n.sqrt())),
                    None => Err(runtime_err(
                        "__math.sqrt expects Int or Float".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "sin") => {
                if args.len() != 1 {
                    return Err(runtime_err("__math.sin takes 1 argument".to_string(), span));
                }
                match as_f64(&args[0]) {
                    Some(n) => Ok(Value::Float(n.sin())),
                    None => Err(runtime_err(
                        "__math.sin expects Int or Float".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "cos") => {
                if args.len() != 1 {
                    return Err(runtime_err("__math.cos takes 1 argument".to_string(), span));
                }
                match as_f64(&args[0]) {
                    Some(n) => Ok(Value::Float(n.cos())),
                    None => Err(runtime_err(
                        "__math.cos expects Int or Float".to_string(),
                        span,
                    )),
                }
            }
            ("__math", "atan2") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "__math.atan2 takes 2 arguments".to_string(),
                        span,
                    ));
                }
                match (as_f64(&args[0]), as_f64(&args[1])) {
                    (Some(y), Some(x)) => Ok(Value::Float(y.atan2(x))),
                    _ => Err(runtime_err(
                        "__math.atan2 expects Int or Float".to_string(),
                        span,
                    )),
                }
            }
            ("input", "pressed") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "input.pressed takes 1 argument".to_string(),
                        span,
                    ));
                }
                let code = match &args[0] {
                    Value::String(s) => s.as_str(),
                    _ => {
                        return Err(runtime_err(
                            "input.pressed expects a String key code".to_string(),
                            span,
                        ));
                    }
                };
                Ok(Value::Bool(csv_has(&self.pressed, code)))
            }
            ("input", "held") => {
                if args.len() != 1 {
                    return Err(runtime_err("input.held takes 1 argument".to_string(), span));
                }
                let code = match &args[0] {
                    Value::String(s) => s.as_str(),
                    _ => {
                        return Err(runtime_err(
                            "input.held expects a String key code".to_string(),
                            span,
                        ));
                    }
                };
                Ok(Value::Bool(csv_has(&self.keys, code)))
            }
            ("strata", "move") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "strata.move takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let dx = as_f64(&args[0])
                    .ok_or_else(|| runtime_err("strata.move expects numbers".to_string(), span))?;
                let dy = as_f64(&args[1])
                    .ok_or_else(|| runtime_err("strata.move expects numbers".to_string(), span))?;
                self.effects.push(HostEffect::Move { dx, dy });
                Ok(Value::Void)
            }
            ("strata", "rot") => {
                if args.len() != 1 {
                    return Err(runtime_err("strata.rot takes 1 argument".to_string(), span));
                }
                let degrees = as_f64(&args[0])
                    .ok_or_else(|| runtime_err("strata.rot expects a number".to_string(), span))?;
                self.effects.push(HostEffect::Rot { degrees });
                Ok(Value::Void)
            }
            ("strata", "set") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "strata.set takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let x = as_f64(&args[0])
                    .ok_or_else(|| runtime_err("strata.set expects numbers".to_string(), span))?;
                let y = as_f64(&args[1])
                    .ok_or_else(|| runtime_err("strata.set expects numbers".to_string(), span))?;
                self.effects.push(HostEffect::Set {
                    x: Some(x),
                    y: Some(y),
                    rot: None,
                });
                Ok(Value::Void)
            }
            ("strata", "play_sound") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "strata.play_sound takes 1 argument".to_string(),
                        span,
                    ));
                }
                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(runtime_err(
                            "strata.play_sound expects a String".to_string(),
                            span,
                        ));
                    }
                };
                self.effects
                    .push(HostEffect::PlaySound { name: Some(name) });
                Ok(Value::Void)
            }
            ("strata", "destroy") => {
                if args.len() > 1 {
                    return Err(runtime_err(
                        "strata.destroy takes 0 or 1 arguments".to_string(),
                        span,
                    ));
                }
                let name = match args.first() {
                    None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(_) => {
                        return Err(runtime_err(
                            "strata.destroy expects a String".to_string(),
                            span,
                        ));
                    }
                };
                self.effects.push(HostEffect::Destroy { name });
                Ok(Value::Void)
            }
            ("strata", "find") => {
                if args.len() > 1 {
                    return Err(runtime_err(
                        "strata.find takes 0 or 1 arguments".to_string(),
                        span,
                    ));
                }
                match args.first() {
                    None => Ok(self.find_nearest()),
                    Some(Value::String(name)) => Ok(self.find_by_name(name)),
                    Some(_) => Err(runtime_err(
                        "strata.find expects a String name".to_string(),
                        span,
                    )),
                }
            }
            ("strata", "after") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "strata.after takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let delay = as_f64(&args[0]).ok_or_else(|| {
                    runtime_err("strata.after expects a number delay".to_string(), span)
                })?;
                let method = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(runtime_err(
                            "strata.after expects a String method name".to_string(),
                            span,
                        ));
                    }
                };
                self.effects.push(HostEffect::After { delay, method });
                Ok(Value::Void)
            }
            ("strata", "spawn") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "strata.spawn takes 1 argument".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::String(name) => {
                        self.effects.push(HostEffect::SpawnPrefab {
                            prefab: name.clone(),
                            x: None,
                            y: None,
                        });
                        Ok(Value::Void)
                    }
                    Value::Map(m) => {
                        let map = m.borrow();
                        if let Some(prefab) = map_string(&map, "prefab") {
                            self.effects.push(HostEffect::SpawnPrefab {
                                prefab,
                                x: map_f64(&map, "x"),
                                y: map_f64(&map, "y"),
                            });
                            Ok(Value::Void)
                        } else {
                            self.effects.push(spawn_from_map(&map));
                            Ok(Value::Void)
                        }
                    }
                    _ => Err(runtime_err(
                        "strata.spawn expects a prefab name or a Map".to_string(),
                        span,
                    )),
                }
            }
            _ => Err(runtime_err(
                format!("unknown stdlib function {}.{}", module, name),
                span,
            )),
        }
    }
}

fn expect_string_arg(
    args: &[Value],
    index: usize,
    fn_name: &str,
    what: &str,
    span: Span,
) -> Result<String, RuntimeError> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(runtime_err(
            format!("{fn_name} expects String {what}, got {}", v.type_name()),
            span,
        )),
        None => Err(runtime_err(
            format!("{fn_name} missing {what} argument"),
            span,
        )),
    }
}

fn io_unit_result(r: Result<(), String>) -> Value {
    match r {
        Ok(()) => result_ok(Value::Void),
        Err(e) => result_err(e),
    }
}
