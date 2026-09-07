//! Call dispatch: builtins, instance methods, UFCS, and host modules
//! (`io`, `time`, `process`, `json`, `path`, `http`, `regex`, `__math`, `__str`).

use std::collections::HashMap;

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
        if let Some(Value::FnRef { name: fn_name }) = self.env.get(name) {
            if let Some(decl) = self.lookup_fn(&fn_name) {
                return self.invoke_user_fn(decl, args, span);
            }
            return self.call_fn(&fn_name, args, span);
        }
        if let Some(Value::Closure(c)) = self.env.get(name) {
            return self.call_closure(&c, args, span);
        }
        match name {
            "print" => {
                let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                self.append_stdout(&parts.join(" "));
                self.append_stdout("\n");
                Ok(Value::Void)
            }
            "len" => {
                if args.len() != 1 {
                    return Err(runtime_err("len takes 1 argument".to_string(), span));
                }
                match &args[0] {
                    Value::String(s) => Ok(Value::Int(string_char_len(s))),
                    Value::Array(a) => Ok(Value::Int(lock(a).len() as i64)),
                    Value::Map(m) => Ok(Value::Int(lock(m).len() as i64)),
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
            "Array" => Ok(array_value(args)),
            "Mutex" => {
                if !args.is_empty() {
                    return Err(runtime_err("Mutex takes 0 arguments".to_string(), span));
                }
                Ok(Value::Mutex(LangMutex::new()))
            }
            "Channel" => {
                if !args.is_empty() {
                    return Err(runtime_err("Channel takes 0 arguments".to_string(), span));
                }
                Ok(Value::Channel(LangChannel::new()))
            }
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
                Ok(map_value(map))
            }
            _ => {
                if let Some(decl) = self.lookup_fn(name) {
                    self.invoke_user_fn(decl, args, span)
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
            let signal = signal.clone();
            let arity = *arity;
            match name {
                "emit" => {
                    if _args.len() != arity {
                        return Err(runtime_err(
                            format!(
                                "signal '{signal}' expected {arity} args, got {}",
                                _args.len()
                            ),
                            span,
                        ));
                    }
                    return self.emit_signal(&signal, _args, span);
                }
                "connect" => {
                    if _args.len() != 1 {
                        return Err(runtime_err(
                            format!("signal '{signal}' connect takes 1 argument"),
                            span,
                        ));
                    }
                    let listener = _args[0].clone();
                    self.connect_signal(&signal, listener, arity, span)?;
                    return Ok(Value::Void);
                }
                _ => {
                    return Err(runtime_err(
                        format!("signal '{signal}' has no method '{name}'"),
                        span,
                    ));
                }
            }
        }
        if name == "emit" || name == "connect" {
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
                return self.invoke_user_fn(decl, args, span);
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
                "len" => Ok(Value::Int(lock(a).len() as i64)),
                "push" => {
                    if _args.len() != 1 {
                        return Err(runtime_err("Array.push takes 1 argument".to_string(), span));
                    }
                    lock(a).push(_args[0].clone());
                    Ok(Value::Void)
                }
                "pop" => Ok(lock(a).pop().unwrap_or(Value::None)),
                "first" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Array.first takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    Ok(lock(a).first().cloned().unwrap_or(Value::None))
                }
                "last" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Array.last takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    Ok(lock(a).last().cloned().unwrap_or(Value::None))
                }
                "contains" => {
                    if _args.len() != 1 {
                        return Err(runtime_err(
                            "Array.contains takes 1 argument".to_string(),
                            span,
                        ));
                    }
                    let items: Vec<Value> = lock(a).clone();
                    let found = items.iter().any(|v| value_eq(v, &_args[0]));
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
                "len" => Ok(Value::Int(lock(m).len() as i64)),
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
                    Ok(Value::Bool(lock(m).contains_key(&key)))
                }
                "keys" => {
                    let keys: Vec<Value> =
                        lock(m).keys().map(|k| Value::String(k.clone())).collect();
                    Ok(array_value(keys))
                }
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
                    Ok(lock(m).remove(&key).unwrap_or(Value::None))
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
                    lock(m).insert(key, _args[1].clone());
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
                    let present = (*module == "Option" && *variant == "Some")
                        || (*module == "Result" && *variant == "Ok");
                    if present {
                        match value {
                            Some(v) => Ok((**v).clone()),
                            None => Ok(Value::Void),
                        }
                    } else {
                        Ok(_args[0].clone())
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
                        array_value(_args)
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
                    let borrowed = lock(m);
                    borrowed.functions.get(name).cloned()
                };
                if let Some(decl) = decl {
                    let fns = lock(m).functions.clone();
                    self.module_fns.push(fns);
                    let result = self.invoke_user_fn(decl, _args, span);
                    self.module_fns.pop();
                    return result;
                }
                if _args.is_empty() {
                    if let Some(value) = lock(m).values.get(name) {
                        return Ok(value.clone());
                    }
                }
                Err(runtime_err(
                    format!("module has no member '{}'", name),
                    span,
                ))
            }
            Value::Mutex(m) => match name {
                "lock" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Mutex.lock takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    m.lock().map_err(|e| runtime_err(e, span))?;
                    Ok(Value::Void)
                }
                "unlock" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Mutex.unlock takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    m.unlock().map_err(|e| runtime_err(e, span))?;
                    Ok(Value::Void)
                }
                _ => Err(runtime_err(format!("Mutex has no method '{name}'"), span)),
            },
            Value::Channel(ch) => match name {
                "send" => {
                    if _args.len() != 1 {
                        return Err(runtime_err(
                            "Channel.send takes 1 argument".to_string(),
                            span,
                        ));
                    }
                    ch.send(_args[0].clone())
                        .map_err(|e| runtime_err(e, span))?;
                    Ok(Value::Void)
                }
                "recv" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Channel.recv takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    Ok(ch.recv())
                }
                "close" => {
                    if !_args.is_empty() {
                        return Err(runtime_err(
                            "Channel.close takes 0 arguments".to_string(),
                            span,
                        ));
                    }
                    ch.close();
                    Ok(Value::Void)
                }
                "recv_timeout" => {
                    let secs = expect_secs(&_args, span, "Channel.recv_timeout")?;
                    Ok(match ch.recv_timeout(secs) {
                        Some(v) => option_some(v),
                        None => option_none(),
                    })
                }
                _ => Err(runtime_err(format!("Channel has no method '{name}'"), span)),
            },
            Value::Task(handle) => match name {
                "wait" => {
                    let secs = expect_secs(&_args, span, "Task.wait")?;
                    self.task_wait(handle, secs, span)
                }
                _ => Err(runtime_err(format!("Task has no method '{name}'"), span)),
            },
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
                Ok(array_value(parts))
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
                        result_ok(array_value(lines))
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
                        result_ok(array_value(values))
                    }
                    Err(e) => result_err(e),
                })
            }
            ("io", "read_stdin") => {
                if !args.is_empty() {
                    return Err(runtime_err(
                        "io.read_stdin takes 0 arguments".to_string(),
                        span,
                    ));
                }
                Ok(match io_read_stdin() {
                    Ok(content) => result_ok(Value::String(content)),
                    Err(e) => result_err(e),
                })
            }
            ("io", "read_line") => {
                if !args.is_empty() {
                    return Err(runtime_err(
                        "io.read_line takes 0 arguments".to_string(),
                        span,
                    ));
                }
                Ok(match io_read_line() {
                    Ok(Some(line)) => option_some(Value::String(line)),
                    Ok(None) => option_none(),
                    Err(e) => {
                        return Err(runtime_err(e, span));
                    }
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
                Ok(Value::Float(self.shared.started.elapsed_secs()))
            }
            ("process", "argv") => {
                if !args.is_empty() {
                    return Err(runtime_err(
                        "process.argv takes 0 arguments".to_string(),
                        span,
                    ));
                }
                let values: Vec<Value> = self.argv().iter().cloned().map(Value::String).collect();
                Ok(array_value(values))
            }
            ("process", "env") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "process.env takes 1 argument".to_string(),
                        span,
                    ));
                }
                let name = expect_string_arg(&args, 0, "process.env", "name", span)?;
                Ok(match process_env(&name) {
                    Some(value) => option_some(Value::String(value)),
                    None => option_none(),
                })
            }
            ("process", "exit") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "process.exit takes 1 argument".to_string(),
                        span,
                    ));
                }
                let code = match &args[0] {
                    Value::Int(n) => *n as i32,
                    _ => {
                        return Err(runtime_err(
                            format!("process.exit expects Int, got {}", args[0].type_name()),
                            span,
                        ));
                    }
                };
                Err(exit_err(code, span))
            }
            ("process", "run") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "process.run takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let cmd = expect_string_arg(&args, 0, "process.run", "command", span)?;
                Ok(match process_run(&cmd, &args[1]) {
                    Ok(stdout) => result_ok(Value::String(stdout)),
                    Err(e) => result_err(e),
                })
            }
            ("json", "parse") => {
                if args.len() != 1 {
                    return Err(runtime_err("json.parse takes 1 argument".to_string(), span));
                }
                let text = expect_string_arg(&args, 0, "json.parse", "text", span)?;
                Ok(match json_parse(&text) {
                    Ok(v) => result_ok(v),
                    Err(e) => result_err(e),
                })
            }
            ("json", "stringify") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "json.stringify takes 1 argument".to_string(),
                        span,
                    ));
                }
                Ok(match json_stringify(&args[0]) {
                    Ok(s) => result_ok(Value::String(s)),
                    Err(e) => result_err(e),
                })
            }
            ("path", "join") => {
                if args.len() != 2 {
                    return Err(runtime_err("path.join takes 2 arguments".to_string(), span));
                }
                let a = expect_string_arg(&args, 0, "path.join", "a", span)?;
                let b = expect_string_arg(&args, 1, "path.join", "b", span)?;
                Ok(Value::String(path_join(&a, &b)))
            }
            ("path", "dirname") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "path.dirname takes 1 argument".to_string(),
                        span,
                    ));
                }
                let p = expect_string_arg(&args, 0, "path.dirname", "path", span)?;
                Ok(Value::String(path_dirname(&p)))
            }
            ("path", "ext") => {
                if args.len() != 1 {
                    return Err(runtime_err("path.ext takes 1 argument".to_string(), span));
                }
                let p = expect_string_arg(&args, 0, "path.ext", "path", span)?;
                Ok(Value::String(path_ext(&p)))
            }
            ("http", "get") => {
                if args.len() != 1 {
                    return Err(runtime_err("http.get takes 1 argument".to_string(), span));
                }
                let url = expect_string_arg(&args, 0, "http.get", "url", span)?;
                Ok(match http_get(&url) {
                    Ok(body) => result_ok(Value::String(body)),
                    Err(e) => result_err(e),
                })
            }
            ("http", "post") => {
                if args.len() != 2 {
                    return Err(runtime_err("http.post takes 2 arguments".to_string(), span));
                }
                let url = expect_string_arg(&args, 0, "http.post", "url", span)?;
                let body = expect_string_arg(&args, 1, "http.post", "body", span)?;
                Ok(match http_post(&url, &body) {
                    Ok(resp) => result_ok(Value::String(resp)),
                    Err(e) => result_err(e),
                })
            }
            ("regex", "is_match") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "regex.is_match takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let pattern = expect_string_arg(&args, 0, "regex.is_match", "pattern", span)?;
                let text = expect_string_arg(&args, 1, "regex.is_match", "text", span)?;
                Ok(match regex_is_match(&pattern, &text) {
                    Ok(matched) => result_ok(Value::Bool(matched)),
                    Err(e) => result_err(e),
                })
            }
            ("regex", "find") => {
                if args.len() != 2 {
                    return Err(runtime_err(
                        "regex.find takes 2 arguments".to_string(),
                        span,
                    ));
                }
                let pattern = expect_string_arg(&args, 0, "regex.find", "pattern", span)?;
                let text = expect_string_arg(&args, 1, "regex.find", "text", span)?;
                Ok(match regex_find(&pattern, &text) {
                    Ok(Some(s)) => result_ok(option_some(Value::String(s))),
                    Ok(None) => result_ok(option_none()),
                    Err(e) => result_err(e),
                })
            }
            ("Array", "first") => {
                if args.len() != 1 {
                    return Err(runtime_err(
                        "Array.first takes 1 argument".to_string(),
                        span,
                    ));
                }
                match &args[0] {
                    Value::Array(a) => Ok(lock(a).first().cloned().unwrap_or(Value::None)),
                    _ => Err(runtime_err("Array.first expects Array".to_string(), span)),
                }
            }
            ("Array", "last") => {
                if args.len() != 1 {
                    return Err(runtime_err("Array.last takes 1 argument".to_string(), span));
                }
                match &args[0] {
                    Value::Array(a) => Ok(lock(a).last().cloned().unwrap_or(Value::None)),
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
                    Value::Array(a) => {
                        let items: Vec<Value> = lock(a).clone();
                        Ok(Value::Bool(items.iter().any(|v| value_eq(v, &args[1]))))
                    }
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
            _ => Err(runtime_err(
                format!("unknown stdlib function {}.{}", module, name),
                span,
            )),
        }
    }
}

fn expect_secs(args: &[Value], span: Span, who: &str) -> Result<f64, RuntimeError> {
    if args.len() != 1 {
        return Err(runtime_err(format!("{who} takes 1 argument"), span));
    }
    as_f64(&args[0]).ok_or_else(|| {
        runtime_err(
            format!("{who} expects Float seconds, got {}", args[0].type_name()),
            span,
        )
    })
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
