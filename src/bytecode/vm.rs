//! Stack interpreter for a [`Program`].

use crate::interpreter::ops::{
    attach_trace, await_task, box_local, checked_mod, compare_op, enum_payload, format_value,
    index_get, index_get_assign, index_set, int_bitwise, int_div, int_shift, is_variant,
    iter_items, local_get, local_set, member_get, member_set, named_payload_field, new_struct,
    numeric_binop, range_value, runtime_err, task_from_rx, try_result, value_eq,
};
use crate::{EvalContext, RuntimeError, Span, Value};

use super::chunk::{Op, Program};

pub fn interpret(program: &Program, args: Vec<Value>) -> Result<Value, RuntimeError> {
    let mut ctx = EvalContext::new();
    interpret_in(&mut ctx, program, args)
}

pub fn interpret_in(
    ctx: &mut EvalContext,
    program: &Program,
    args: Vec<Value>,
) -> Result<Value, RuntimeError> {
    if let Some(init) = program.init {
        run_fn(ctx, program, init, Vec::new())?;
    }
    let result = run_fn(ctx, program, program.main, args);
    ctx.join_live_tasks();
    result
}

fn run_fn(
    ctx: &mut EvalContext,
    program: &Program,
    idx: usize,
    args: Vec<Value>,
) -> Result<Value, RuntimeError> {
    let func = program
        .functions
        .get(idx)
        .ok_or_else(|| runtime_err("bad function", Span::default()))?;
    if args.len() != func.arity {
        return Err(runtime_err(
            format!(
                "{} expected {} args, got {}",
                func.name,
                func.arity,
                args.len()
            ),
            Span::default(),
        ));
    }
    let file = func.file.clone();
    let prev_file = ctx.set_vm_file(if file.is_empty() {
        ctx.current_file().to_string()
    } else {
        file.clone()
    });
    let mut slots = vec![Value::None; func.local_count.max(func.arity)];
    for (i, arg) in args.into_iter().enumerate() {
        slots[i] = arg;
    }
    let mut stack: Vec<Value> = Vec::new();
    let chunk = &func.chunk;
    let mut ip = 0;
    let mut result = (|| -> Result<Value, RuntimeError> {
        while ip < chunk.ops.len() {
            let span = chunk.spans[ip];
            match &chunk.ops[ip] {
                Op::Constant(i) => {
                    stack.push(
                        chunk
                            .constants
                            .get(*i as usize)
                            .cloned()
                            .ok_or_else(|| runtime_err("bad constant", span))?,
                    );
                }
                Op::Nil => stack.push(Value::None),
                Op::Void => stack.push(Value::Void),
                Op::Pop => {
                    pop(&mut stack, span)?;
                }
                Op::GetLocal(slot) => {
                    let v = slots
                        .get(*slot as usize)
                        .ok_or_else(|| runtime_err("bad local", span))?;
                    stack.push(local_get(v));
                }
                Op::SetLocal(slot) => {
                    let v = stack
                        .last()
                        .cloned()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    let slot = *slot as usize;
                    if slot >= slots.len() {
                        return Err(runtime_err("bad local", span));
                    }
                    local_set(&mut slots[slot], v);
                }
                Op::GetGlobal(i) => {
                    let i = *i as usize;
                    let g = program.globals.lock().unwrap_or_else(|p| p.into_inner());
                    stack.push(g.get(i).cloned().unwrap_or(Value::None));
                }
                Op::SetGlobal(i) => {
                    let v = stack
                        .last()
                        .cloned()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    let i = *i as usize;
                    let mut g = program.globals.lock().unwrap_or_else(|p| p.into_inner());
                    if i >= g.len() {
                        return Err(runtime_err("bad global", span));
                    }
                    g[i] = v;
                }
                Op::Add => {
                    let r = pop(&mut stack, span)?;
                    let l = pop(&mut stack, span)?;
                    stack.push(add(l, r, span)?);
                }
                Op::Sub => bin(&mut stack, span, |l, r| {
                    numeric_binop(l, r, |a, b| a - b, span)
                })?,
                Op::Mul => bin(&mut stack, span, |l, r| {
                    numeric_binop(l, r, |a, b| a * b, span)
                })?,
                Op::Div => bin(&mut stack, span, |l, r| {
                    numeric_binop(l, r, |a, b| a / b, span)
                })?,
                Op::IDiv => {
                    let r = pop(&mut stack, span)?;
                    let l = pop(&mut stack, span)?;
                    stack.push(match (&l, &r) {
                        (Value::Int(a), Value::Int(b)) => int_div(*a, *b, span)?,
                        _ => {
                            return Err(runtime_err(
                                format!(
                                    "integer division requires Int, got {} and {}",
                                    l.type_name(),
                                    r.type_name()
                                ),
                                span,
                            ));
                        }
                    });
                }
                Op::Mod => bin(&mut stack, span, |l, r| checked_mod(l, r, span))?,
                Op::Eq => {
                    let r = pop(&mut stack, span)?;
                    let l = pop(&mut stack, span)?;
                    stack.push(Value::Bool(value_eq(&l, &r)));
                }
                Op::Neq => {
                    let r = pop(&mut stack, span)?;
                    let l = pop(&mut stack, span)?;
                    stack.push(Value::Bool(!value_eq(&l, &r)));
                }
                Op::Lt => bin(&mut stack, span, |l, r| {
                    compare_op(l, r, |a, b| a < b, span)
                })?,
                Op::Lte => bin(&mut stack, span, |l, r| {
                    compare_op(l, r, |a, b| a <= b, span)
                })?,
                Op::Gt => bin(&mut stack, span, |l, r| {
                    compare_op(l, r, |a, b| a > b, span)
                })?,
                Op::Gte => bin(&mut stack, span, |l, r| {
                    compare_op(l, r, |a, b| a >= b, span)
                })?,
                Op::BitAnd => bin(&mut stack, span, |l, r| {
                    int_bitwise(l, r, |a, b| a & b, span)
                })?,
                Op::BitOr => bin(&mut stack, span, |l, r| {
                    int_bitwise(l, r, |a, b| a | b, span)
                })?,
                Op::BitXor => bin(&mut stack, span, |l, r| {
                    int_bitwise(l, r, |a, b| a ^ b, span)
                })?,
                Op::Shl => bin(&mut stack, span, |l, r| int_shift(l, r, true, span))?,
                Op::Shr => bin(&mut stack, span, |l, r| int_shift(l, r, false, span))?,
                Op::Neg => {
                    let v = pop(&mut stack, span)?;
                    stack.push(match v {
                        Value::Int(n) => Value::Int(-n),
                        Value::Float(n) => Value::Float(-n),
                        _ => {
                            return Err(runtime_err(
                                format!("cannot negate {}", v.type_name()),
                                span,
                            ));
                        }
                    });
                }
                Op::Not => {
                    let v = pop(&mut stack, span)?;
                    stack.push(Value::Bool(!v.truthy()));
                }
                Op::BitNot => {
                    let v = pop(&mut stack, span)?;
                    stack.push(match v {
                        Value::Int(n) => Value::Int(!n),
                        _ => {
                            return Err(runtime_err(
                                format!("bitwise not requires Int, got {}", v.type_name()),
                                span,
                            ));
                        }
                    });
                }
                Op::Truthy => {
                    let v = pop(&mut stack, span)?;
                    stack.push(Value::Bool(v.truthy()));
                }
                Op::Jump(t) => {
                    ip = *t as usize;
                    continue;
                }
                Op::JumpIfFalse(t) => {
                    let v = stack
                        .last()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    if !v.truthy() {
                        ip = *t as usize;
                        continue;
                    }
                }
                Op::JumpIfTrue(t) => {
                    let v = stack
                        .last()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    if v.truthy() {
                        ip = *t as usize;
                        continue;
                    }
                }
                Op::Return => return pop(&mut stack, span),
                Op::Call(idx, argc) => {
                    let idx = *idx as usize;
                    let argc = *argc as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(pop(&mut stack, span)?);
                    }
                    args.reverse();
                    let v = traced_call(ctx, program, idx, args, span)?;
                    stack.push(v);
                }
                Op::Host(h, argc) => {
                    let h = *h as usize;
                    let argc = *argc as usize;
                    let (module, name) = program
                        .hosts
                        .get(h)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad host", span))?;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(pop(&mut stack, span)?);
                    }
                    args.reverse();
                    let v = if module.is_empty() {
                        ctx.call_builtin_or_fn(&name, args, span)?
                    } else {
                        ctx.call_qualified(&module, &name, args, span)?
                    };
                    stack.push(v);
                }
                Op::Invoke(m, argc) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let argc = *argc as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(pop(&mut stack, span)?);
                    }
                    args.reverse();
                    let object = pop(&mut stack, span)?;
                    let v = invoke_method(ctx, program, object, &name, args, span)?;
                    stack.push(v);
                }
                Op::GetIndex => {
                    let idx = pop(&mut stack, span)?;
                    let obj = pop(&mut stack, span)?;
                    stack.push(index_get(&obj, &idx, span)?);
                }
                Op::PeekIndex => {
                    if stack.len() < 2 {
                        return Err(runtime_err("stack underflow", span));
                    }
                    let idx = stack[stack.len() - 1].clone();
                    let obj = stack[stack.len() - 2].clone();
                    stack.push(index_get_assign(&obj, &idx, span)?);
                }
                Op::SetIndex => {
                    let val = pop(&mut stack, span)?;
                    let idx = pop(&mut stack, span)?;
                    let obj = pop(&mut stack, span)?;
                    index_set(&obj, &idx, val.clone(), span)?;
                    stack.push(val);
                }
                Op::MakeRange(inclusive) => {
                    let end = pop(&mut stack, span)?;
                    let start = pop(&mut stack, span)?;
                    stack.push(range_value(start, end, *inclusive, span)?);
                }
                Op::IterItems => {
                    let iter = pop(&mut stack, span)?;
                    stack.push(iter_items(iter, span)?);
                }
                Op::Dup => {
                    let v = stack
                        .last()
                        .cloned()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    stack.push(v);
                }
                Op::Format(i) => {
                    let spec = chunk
                        .constants
                        .get(*i as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad constant", span))?;
                    let spec = match spec {
                        Value::String(s) if !s.is_empty() => Some(s),
                        _ => None,
                    };
                    let v = pop(&mut stack, span)?;
                    let s = format_value(&v, spec.as_deref(), span)?;
                    stack.push(Value::String(s));
                }
                Op::Try => {
                    let v = pop(&mut stack, span)?;
                    match try_result(v, span)? {
                        Ok(payload) => stack.push(payload),
                        Err(err) => return Ok(err),
                    }
                }
                Op::IsVariant(m) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let v = stack
                        .last()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    stack.push(Value::Bool(is_variant(v, &name, span)?));
                }
                Op::EnumPayload => {
                    let v = pop(&mut stack, span)?;
                    stack.push(enum_payload(v, span)?);
                }
                Op::GetProp(m) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let obj = pop(&mut stack, span)?;
                    stack.push(member_get(&obj, &name, span)?);
                }
                Op::SetProp(m) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let val = pop(&mut stack, span)?;
                    let obj = pop(&mut stack, span)?;
                    member_set(&obj, &name, val.clone(), span)?;
                    stack.push(val);
                }
                Op::NewStruct(i) => {
                    let def = program
                        .struct_defs
                        .get(*i as usize)
                        .ok_or_else(|| runtime_err("bad struct", span))?;
                    stack.push(new_struct(def));
                }
                Op::MakeClosure(idx, n) => {
                    let n = *n as usize;
                    let mut captures = Vec::with_capacity(n);
                    for _ in 0..n {
                        captures.push(pop(&mut stack, span)?);
                    }
                    captures.reverse();
                    stack.push(Value::BytecodeFn {
                        fn_idx: *idx as usize,
                        captures,
                    });
                }
                Op::CallValue(argc) => {
                    let argc = *argc as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(pop(&mut stack, span)?);
                    }
                    args.reverse();
                    let callee = pop(&mut stack, span)?;
                    let v = match callee {
                        Value::BytecodeFn { fn_idx, captures } => {
                            let mut call_args = captures;
                            call_args.extend(args);
                            traced_call(ctx, program, fn_idx, call_args, span)?
                        }
                        other => ctx.call_value(other, args, span)?,
                    };
                    stack.push(v);
                }
                Op::BoxLocal(slot) => {
                    let slot = *slot as usize;
                    if slot >= slots.len() {
                        return Err(runtime_err("bad local", span));
                    }
                    stack.push(box_local(&mut slots[slot]));
                }
                Op::GetNamedField(m) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let v = stack
                        .last()
                        .ok_or_else(|| runtime_err("stack underflow", span))?;
                    stack.push(named_payload_field(v, &name, &program.enums, span)?);
                }
                Op::SpawnCall(idx, argc) => {
                    let mut args = pop_args(&mut stack, *argc, span)?;
                    args.reverse();
                    stack.push(spawn_call(ctx, program, *idx as usize, args, span)?);
                }
                Op::SpawnHost(h, argc) => {
                    let mut args = pop_args(&mut stack, *argc, span)?;
                    args.reverse();
                    let (module, name) = program
                        .hosts
                        .get(*h as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad host", span))?;
                    stack.push(spawn_host(ctx, module, name, args, span)?);
                }
                Op::SpawnInvoke(m, argc) => {
                    let name = program
                        .methods
                        .get(*m as usize)
                        .cloned()
                        .ok_or_else(|| runtime_err("bad method", span))?;
                    let mut args = pop_args(&mut stack, *argc, span)?;
                    args.reverse();
                    let object = pop(&mut stack, span)?;
                    stack.push(spawn_invoke(ctx, program, object, name, args, span)?);
                }
                Op::SpawnValue(argc) => {
                    let mut args = pop_args(&mut stack, *argc, span)?;
                    args.reverse();
                    let callee = pop(&mut stack, span)?;
                    stack.push(spawn_value(ctx, program, callee, args, span)?);
                }
                Op::Await => {
                    let v = pop(&mut stack, span)?;
                    stack.push(await_task(v, span)?);
                }
            }
            ip += 1;
        }
        Ok(Value::Void)
    })();
    if let Err(ref mut e) = result {
        if e.file.is_empty() {
            e.file = ctx.current_file().to_string();
        }
    }
    ctx.set_vm_file(prev_file);
    result
}

fn pop(stack: &mut Vec<Value>, span: Span) -> Result<Value, RuntimeError> {
    stack
        .pop()
        .ok_or_else(|| runtime_err("stack underflow", span))
}

fn bin(
    stack: &mut Vec<Value>,
    span: Span,
    op: impl FnOnce(Value, Value) -> Result<Value, RuntimeError>,
) -> Result<(), RuntimeError> {
    let r = pop(stack, span)?;
    let l = pop(stack, span)?;
    stack.push(op(l, r)?);
    Ok(())
}

fn add(l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
    match (&l, &r) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
        (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
        _ => Err(runtime_err(
            format!("cannot add {} and {}", l.type_name(), r.type_name()),
            span,
        )),
    }
}

fn pop_args(stack: &mut Vec<Value>, argc: u8, span: Span) -> Result<Vec<Value>, RuntimeError> {
    let argc = argc as usize;
    let mut args = Vec::with_capacity(argc);
    for _ in 0..argc {
        args.push(pop(stack, span)?);
    }
    Ok(args)
}

fn is_missing_method(err: &RuntimeError) -> bool {
    err.message.contains("has no method")
}

fn traced_call(
    ctx: &mut EvalContext,
    program: &Program,
    idx: usize,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let name = program
        .functions
        .get(idx)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "<fn>".into());
    let caller_file = ctx.current_file().to_string();
    call_or_spawn(ctx, program, idx, args, span)
        .map_err(|e| attach_trace(e, &name, span, &caller_file))
}

fn invoke_method(
    ctx: &mut EvalContext,
    program: &Program,
    object: Value,
    name: &str,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    if let Value::Signal {
        name: signal,
        arity,
    } = &object
    {
        return invoke_signal(ctx, program, signal, *arity, name, args, span);
    }
    let type_key = match &object {
        Value::Struct { name: n, .. } => Some(n.clone()),
        Value::Enum { module, .. } => Some(module.clone()),
        _ => None,
    };
    if let Some(ty) = type_key {
        if let Some(&idx) = program.type_methods.get(&(ty, name.to_string())) {
            let mut call_args = vec![object];
            call_args.extend(args);
            return traced_call(ctx, program, idx as usize, call_args, span);
        }
    }
    match ctx.call_member(&object, name, args.clone(), span) {
        Ok(v) => Ok(v),
        Err(e) if is_missing_method(&e) => {
            if let Some(&idx) = program.ufcs.get(name) {
                let mut call_args = vec![object];
                call_args.extend(args);
                traced_call(ctx, program, idx as usize, call_args, span)
            } else {
                Err(e)
            }
        }
        Err(e) => Err(e),
    }
}

fn is_async_fn(program: &Program, idx: usize) -> bool {
    program.functions.get(idx).is_some_and(|f| f.is_async)
}

fn call_or_spawn(
    ctx: &mut EvalContext,
    program: &Program,
    idx: usize,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    if is_async_fn(program, idx) {
        spawn_call(ctx, program, idx, args, span)
    } else {
        run_fn(ctx, program, idx, args)
    }
}

fn invoke_signal(
    ctx: &mut EvalContext,
    program: &Program,
    signal: &str,
    arity: usize,
    name: &str,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    match name {
        "connect" => {
            if args.len() != 1 {
                return Err(runtime_err(
                    format!("signal '{signal}' connect takes 1 argument"),
                    span,
                ));
            }
            ctx.connect_signal(signal, args[0].clone(), arity, span)?;
            Ok(Value::Void)
        }
        "emit" => {
            if args.len() != arity {
                return Err(runtime_err(
                    format!(
                        "signal '{signal}' expected {arity} args, got {}",
                        args.len()
                    ),
                    span,
                ));
            }
            for listener in ctx.signal_listeners(signal) {
                match listener {
                    Value::BytecodeFn { fn_idx, captures } => {
                        let mut call_args = captures;
                        call_args.extend(args.clone());
                        traced_call(ctx, program, fn_idx, call_args, span)?;
                    }
                    other => {
                        ctx.call_value(other, args.clone(), span)?;
                    }
                }
            }
            Ok(Value::Void)
        }
        _ => Err(runtime_err(
            format!("signal '{signal}' has no method '{name}'"),
            span,
        )),
    }
}

fn spawn_job(
    ctx: &mut EvalContext,
    span: Span,
    name: String,
    work: impl FnOnce(&mut EvalContext) -> Result<Value, RuntimeError> + Send + 'static,
) -> Result<Value, RuntimeError> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (ctx, name, work);
        return Err(runtime_err("spawn is not supported on this target", span));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut child = ctx.fork_task();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let _ = tx.send(work(&mut child));
            })
            .map_err(|e| runtime_err(format!("failed to spawn: {e}"), span))?;
        ctx.register_live_task(handle);
        Ok(task_from_rx(rx))
    }
}

fn spawn_call(
    ctx: &mut EvalContext,
    program: &Program,
    idx: usize,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let program = program.clone();
    let name = program
        .functions
        .get(idx)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "<fn>".into());
    spawn_job(ctx, span, name, move |child| {
        run_fn(child, &program, idx, args)
    })
}

fn spawn_host(
    ctx: &mut EvalContext,
    module: String,
    name: String,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let label = if module.is_empty() {
        name.clone()
    } else {
        format!("{module}.{name}")
    };
    spawn_job(ctx, span, label, move |child| {
        if module.is_empty() {
            child.call_builtin_or_fn(&name, args, span)
        } else {
            child.call_qualified(&module, &name, args, span)
        }
    })
}

fn spawn_invoke(
    ctx: &mut EvalContext,
    program: &Program,
    object: Value,
    name: String,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let type_key = match &object {
        Value::Struct { name: n, .. } => Some(n.clone()),
        Value::Enum { module, .. } => Some(module.clone()),
        _ => None,
    };
    if let Some(ty) = type_key {
        if let Some(&idx) = program.type_methods.get(&(ty, name.clone())) {
            let mut call_args = vec![object];
            call_args.extend(args);
            return spawn_call(ctx, program, idx as usize, call_args, span);
        }
    }
    let program = program.clone();
    spawn_job(ctx, span, name.clone(), move |child| {
        match child.call_member(&object, &name, args.clone(), span) {
            Ok(v) => Ok(v),
            Err(e) if is_missing_method(&e) => {
                if let Some(&idx) = program.ufcs.get(&name) {
                    let mut call_args = vec![object];
                    call_args.extend(args);
                    run_fn(child, &program, idx as usize, call_args)
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    })
}

fn spawn_value(
    ctx: &mut EvalContext,
    program: &Program,
    callee: Value,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    match callee {
        Value::BytecodeFn { fn_idx, captures } => {
            let mut call_args = captures;
            call_args.extend(args);
            spawn_call(ctx, program, fn_idx, call_args, span)
        }
        other => ctx.spawn_value(other, args, span),
    }
}
