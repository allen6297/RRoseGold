//! AST → [`Program`]. Unsupported nodes fail compile; `rosegold run` then tree-walks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::interpreter::ops::file_label;
use crate::interpreter::{
    EnumDef, EnumVariantDef, HashMapResolver, ModuleResolver, StructDef, module_lookup_hint,
};
use crate::parser::*;
use crate::{Span, Value};

use super::chunk::{Chunk, FnProto, Op, Program};

#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.span.is_unknown() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.span, self.message)
        }
    }
}

struct Local {
    name: String,
    depth: i32,
}

struct Loop {
    start: usize,
    patch_continue: bool,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Compiler {
    chunk: Chunk,
    locals: Vec<Local>,
    local_count: usize,
    scope_depth: i32,
    loops: Vec<Loop>,
    span: Span,
    fns: HashMap<String, u16>,
    /// `import calc` bind → canonical module name (`calc.add` lives in [`qualified`]).
    modules: HashMap<String, String>,
    /// Exported `module.fn` → function index.
    qualified: HashMap<String, u16>,
    /// `from io import exists` → (`io`, `exists`).
    host_idents: HashMap<String, (String, String)>,
    hosts: Vec<(String, String)>,
    host_index: HashMap<(String, String), u16>,
    methods: Vec<String>,
    method_index: HashMap<String, u16>,
    enums: HashMap<String, Arc<EnumDef>>,
    structs: HashMap<String, Arc<StructDef>>,
    struct_defs: Vec<Arc<StructDef>>,
    struct_index: HashMap<String, u16>,
    extra_fns: Vec<FnProto>,
    fn_base: u16,
    self_fields: Option<Vec<String>>,
    type_methods: HashMap<(String, String), u16>,
    parents: HashMap<String, String>,
    super_type: Option<String>,
    signals: HashMap<String, usize>,
    current_file: String,
    current_type: Option<String>,
    current_module: Option<String>,
    globals: HashMap<String, u16>,
    qualified_globals: HashMap<String, u16>,
    qualified_types: HashMap<String, Value>,
    type_idents: HashMap<String, Value>,
    visible_types: HashSet<String>,
}

impl Compiler {
    fn unsupported(&self, what: &str) -> CompileError {
        CompileError {
            message: format!("bytecode: {what} is not compiled yet"),
            span: self.span,
        }
    }

    fn emit(&mut self, op: Op) {
        self.chunk.ops.push(op);
        self.chunk.spans.push(self.span);
    }

    fn add_constant(&mut self, value: Value) -> Result<u16, CompileError> {
        if self.chunk.constants.len() >= u16::MAX as usize {
            return Err(self.unsupported("too many constants"));
        }
        let i = self.chunk.constants.len() as u16;
        self.chunk.constants.push(value);
        Ok(i)
    }

    fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.scope_depth -= 1;
        while self
            .locals
            .last()
            .is_some_and(|l| l.depth > self.scope_depth)
        {
            self.locals.pop();
        }
    }

    fn add_local(&mut self, name: String) -> Result<u8, CompileError> {
        if self.locals.len() >= 256 {
            return Err(self.unsupported("too many locals"));
        }
        let slot = self.locals.len() as u8;
        self.locals.push(Local {
            name,
            depth: self.scope_depth,
        });
        self.local_count = self.local_count.max(self.locals.len());
        Ok(slot)
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        self.locals
            .iter()
            .enumerate()
            .rev()
            .find(|(_, l)| l.name == name)
            .map(|(i, _)| i as u8)
    }

    fn emit_jump(&mut self, make: fn(u16) -> Op) -> usize {
        let i = self.chunk.ops.len();
        self.emit(make(0));
        i
    }

    fn patch(&mut self, jump_at: usize) {
        let dest = self.chunk.ops.len() as u16;
        match &mut self.chunk.ops[jump_at] {
            Op::Jump(t) | Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => *t = dest,
            _ => {}
        }
    }

    fn intern_host(&mut self, module: &str, name: &str) -> Result<u16, CompileError> {
        let key = (module.to_string(), name.to_string());
        if let Some(&i) = self.host_index.get(&key) {
            return Ok(i);
        }
        if self.hosts.len() >= u16::MAX as usize {
            return Err(self.unsupported("too many host functions"));
        }
        let i = self.hosts.len() as u16;
        self.host_index.insert(key.clone(), i);
        self.hosts.push(key);
        Ok(i)
    }

    fn intern_method(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(&i) = self.method_index.get(name) {
            return Ok(i);
        }
        if self.methods.len() >= u16::MAX as usize {
            return Err(self.unsupported("too many methods"));
        }
        let i = self.methods.len() as u16;
        self.method_index.insert(name.to_string(), i);
        self.methods.push(name.to_string());
        Ok(i)
    }

    fn intern_struct(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(&i) = self.struct_index.get(name) {
            return Ok(i);
        }
        let def = self
            .structs
            .get(name)
            .cloned()
            .ok_or_else(|| self.unsupported(&format!("struct '{name}'")))?;
        if self.struct_defs.len() >= u16::MAX as usize {
            return Err(self.unsupported("too many structs"));
        }
        let i = self.struct_defs.len() as u16;
        self.struct_index.insert(name.to_string(), i);
        self.struct_defs.push(def);
        Ok(i)
    }

    fn compile_fn(&mut self, decl: &FnDecl, bind_self: bool) -> Result<FnProto, CompileError> {
        self.chunk = Chunk::new();
        self.locals.clear();
        self.local_count = 0;
        self.scope_depth = 0;
        self.loops.clear();
        self.span = decl.body.stmts.first().map(|s| s.span).unwrap_or_default();
        if bind_self {
            self.add_local("self".to_string())?;
            for p in params_after_self(&decl.params) {
                self.add_local(p.name.clone())?;
            }
        } else {
            for p in &decl.params {
                self.add_local(p.name.clone())?;
            }
        }
        self.compile_block(&decl.body)?;
        self.emit(Op::Void);
        self.emit(Op::Return);
        let arity = if bind_self {
            1 + params_after_self(&decl.params).len()
        } else {
            decl.params.len()
        };
        Ok(FnProto {
            name: proto_name(
                decl,
                bind_self,
                self.current_type.as_deref(),
                self.current_module.as_deref(),
            ),
            arity,
            local_count: self.local_count,
            chunk: std::mem::replace(&mut self.chunk, Chunk::new()),
            is_async: decl.is_async,
            file: if decl.file.is_empty() {
                self.current_file.clone()
            } else {
                decl.file.clone()
            },
        })
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), CompileError> {
        self.begin_scope();
        for stmt in &block.stmts {
            self.compile_stmt(stmt)?;
        }
        self.end_scope();
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        self.span = stmt.span;
        match &stmt.kind {
            StmtKind::Expr(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Pop);
            }
            StmtKind::Return(e) => {
                match e {
                    Some(expr) => self.compile_expr(expr)?,
                    None => self.emit(Op::Void),
                }
                self.emit(Op::Return);
            }
            StmtKind::VarDecl(v) => {
                self.add_local(v.name.clone())?;
                match &v.value {
                    Some(e) => self.compile_expr(e)?,
                    None => self.emit(Op::Nil),
                }
                let slot = self.resolve_local(&v.name).unwrap();
                self.emit(Op::SetLocal(slot));
                self.emit(Op::Pop);
            }
            StmtKind::ConstDecl(c) => {
                self.add_local(c.name.clone())?;
                self.compile_expr(&c.value)?;
                let slot = self.resolve_local(&c.name).unwrap();
                self.emit(Op::SetLocal(slot));
                self.emit(Op::Pop);
            }
            StmtKind::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => self.compile_if(cond, then_block, elif_blocks, else_block.as_ref())?,
            StmtKind::While { cond, body } => self.compile_while(cond, body)?,
            StmtKind::Break => {
                if self.loops.is_empty() {
                    return Err(self.unsupported("break outside loop"));
                }
                let jump = self.emit_jump(Op::Jump);
                self.loops.last_mut().unwrap().breaks.push(jump);
            }
            StmtKind::Continue => {
                let Some((patch, start)) = self
                    .loops
                    .last()
                    .map(|l| (l.patch_continue, l.start as u16))
                else {
                    return Err(self.unsupported("continue outside loop"));
                };
                if patch {
                    let jump = self.emit_jump(Op::Jump);
                    self.loops.last_mut().unwrap().continues.push(jump);
                } else {
                    self.emit(Op::Jump(start));
                }
            }
            StmtKind::Pass | StmtKind::Comment(_) => {}
            StmtKind::For { name, iter, body } => self.compile_for(name, iter, body)?,
        }
        Ok(())
    }

    fn compile_if(
        &mut self,
        cond: &Expr,
        then_block: &Block,
        elif_blocks: &[(Expr, Block)],
        else_block: Option<&Block>,
    ) -> Result<(), CompileError> {
        self.compile_expr(cond)?;
        let mut jump_end = Vec::new();
        let else_jump = self.emit_jump(Op::JumpIfFalse);
        self.emit(Op::Pop);
        self.compile_block(then_block)?;
        jump_end.push(self.emit_jump(Op::Jump));
        self.patch(else_jump);
        self.emit(Op::Pop);
        for (elif_cond, elif_body) in elif_blocks {
            self.compile_expr(elif_cond)?;
            let skip = self.emit_jump(Op::JumpIfFalse);
            self.emit(Op::Pop);
            self.compile_block(elif_body)?;
            jump_end.push(self.emit_jump(Op::Jump));
            self.patch(skip);
            self.emit(Op::Pop);
        }
        if let Some(else_block) = else_block {
            self.compile_block(else_block)?;
        }
        for j in jump_end {
            self.patch(j);
        }
        Ok(())
    }

    fn compile_while(&mut self, cond: &Expr, body: &Block) -> Result<(), CompileError> {
        let start = self.chunk.ops.len();
        self.loops.push(Loop {
            start,
            patch_continue: false,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.compile_expr(cond)?;
        let exit = self.emit_jump(Op::JumpIfFalse);
        self.emit(Op::Pop);
        self.compile_block(body)?;
        self.emit(Op::Jump(start as u16));
        self.patch(exit);
        self.emit(Op::Pop);
        let done = self.chunk.ops.len() as u16;
        let brks = self.loops.pop().unwrap().breaks;
        for j in brks {
            if let Op::Jump(t) = &mut self.chunk.ops[j] {
                *t = done;
            }
        }
        Ok(())
    }

    fn compile_for(&mut self, name: &str, iter: &Expr, body: &Block) -> Result<(), CompileError> {
        self.begin_scope();
        let items_slot = self.add_local("\0iter".into())?;
        self.compile_expr(iter)?;
        self.emit(Op::IterItems);
        self.emit(Op::SetLocal(items_slot));
        self.emit(Op::Pop);

        let i_slot = self.add_local("\0i".into())?;
        let zero = self.add_constant(Value::Int(0))?;
        self.emit(Op::Constant(zero));
        self.emit(Op::SetLocal(i_slot));
        self.emit(Op::Pop);

        let name_slot = self.add_local(name.to_string())?;

        self.loops.push(Loop {
            start: 0,
            patch_continue: true,
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        let cond = self.chunk.ops.len();
        self.emit(Op::GetLocal(i_slot));
        self.emit(Op::GetLocal(items_slot));
        let len_m = self.intern_method("len")?;
        self.emit(Op::Invoke(len_m, 0));
        self.emit(Op::Lt);
        let exit = self.emit_jump(Op::JumpIfFalse);
        self.emit(Op::Pop);

        self.emit(Op::GetLocal(items_slot));
        self.emit(Op::GetLocal(i_slot));
        self.emit(Op::GetIndex);
        self.emit(Op::SetLocal(name_slot));
        self.emit(Op::Pop);

        self.compile_block(body)?;

        let cont = self.chunk.ops.len() as u16;
        self.emit(Op::GetLocal(i_slot));
        let one = self.add_constant(Value::Int(1))?;
        self.emit(Op::Constant(one));
        self.emit(Op::Add);
        self.emit(Op::SetLocal(i_slot));
        self.emit(Op::Pop);
        self.emit(Op::Jump(cond as u16));

        self.patch(exit);
        self.emit(Op::Pop);
        let done = self.chunk.ops.len() as u16;
        let loop_ = self.loops.pop().unwrap();
        for j in loop_.breaks {
            if let Op::Jump(t) = &mut self.chunk.ops[j] {
                *t = done;
            }
        }
        for j in loop_.continues {
            if let Op::Jump(t) = &mut self.chunk.ops[j] {
                *t = cont;
            }
        }
        self.end_scope();
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        self.span = expr.span;
        match &expr.kind {
            ExprKind::Literal(l) => {
                let v = match l {
                    Literal::Int(n) => Value::Int(*n),
                    Literal::Float(n) => Value::Float(*n),
                    Literal::String(s) => Value::String(s.clone()),
                    Literal::Bool(b) => Value::Bool(*b),
                    Literal::None => {
                        self.emit(Op::Nil);
                        return Ok(());
                    }
                };
                let i = self.add_constant(v)?;
                self.emit(Op::Constant(i));
            }
            ExprKind::Ident(name) => {
                if let Some(slot) = self.resolve_local(name) {
                    self.emit(Op::GetLocal(slot));
                } else if name == "super" {
                    if self.resolve_local("self").is_none() || self.super_type.is_none() {
                        return Err(CompileError {
                            message:
                                "super is only valid in a method of a class that extends another"
                                    .into(),
                            span: self.span,
                        });
                    }
                    self.emit(Op::GetLocal(0));
                } else if self
                    .self_fields
                    .as_ref()
                    .is_some_and(|f| f.iter().any(|n| n == name))
                {
                    self.emit(Op::GetLocal(0));
                    let idx = self.intern_method(name)?;
                    self.emit(Op::GetProp(idx));
                } else if let Some(&slot) = self.globals.get(name) {
                    self.emit(Op::GetGlobal(slot));
                } else if let Some(&arity) = self.signals.get(name) {
                    let i = self.add_constant(Value::Signal {
                        name: name.clone(),
                        arity,
                    })?;
                    self.emit(Op::Constant(i));
                } else if let Some(&idx) = self.fns.get(name) {
                    self.emit(Op::MakeClosure(idx, 0));
                } else if let Some(val) = self.type_idents.get(name).cloned() {
                    let i = self.add_constant(val)?;
                    self.emit(Op::Constant(i));
                } else if let Some(def) = self.enums.get(name).cloned() {
                    if !self.visible_types.contains(name) {
                        return Err(self.unsupported(&format!("global '{name}'")));
                    }
                    let i = self.add_constant(Value::EnumType(def))?;
                    self.emit(Op::Constant(i));
                } else if let Some(def) = self.structs.get(name).cloned() {
                    if !self.visible_types.contains(name) {
                        return Err(self.unsupported(&format!("global '{name}'")));
                    }
                    let i = self.add_constant(Value::StructType(def))?;
                    self.emit(Op::Constant(i));
                } else {
                    return Err(self.unsupported(&format!("global '{name}'")));
                }
            }
            ExprKind::Unary { op, expr } => {
                self.compile_expr(expr)?;
                match op {
                    UnaryOp::Neg => self.emit(Op::Neg),
                    UnaryOp::Not => self.emit(Op::Not),
                    UnaryOp::BitNot => self.emit(Op::BitNot),
                }
            }
            ExprKind::Binary { op, left, right } => match op {
                BinOp::And => {
                    self.compile_expr(left)?;
                    self.emit(Op::Truthy);
                    let end = self.emit_jump(Op::JumpIfFalse);
                    self.emit(Op::Pop);
                    self.compile_expr(right)?;
                    self.emit(Op::Truthy);
                    self.patch(end);
                }
                BinOp::Or => {
                    self.compile_expr(left)?;
                    self.emit(Op::Truthy);
                    let end = self.emit_jump(Op::JumpIfTrue);
                    self.emit(Op::Pop);
                    self.compile_expr(right)?;
                    self.emit(Op::Truthy);
                    self.patch(end);
                }
                _ => {
                    self.compile_expr(left)?;
                    self.compile_expr(right)?;
                    self.emit(bin_op(op.clone())?);
                }
            },
            ExprKind::Assign { op, left, right } => match &left.kind {
                ExprKind::Ident(name) => {
                    if let Some(slot) = self.resolve_local(name) {
                        if matches!(op, AssignOp::Assign) {
                            self.compile_expr(right)?;
                        } else {
                            self.emit(Op::GetLocal(slot));
                            self.compile_expr(right)?;
                            self.emit(assign_binop(op)?);
                        }
                        self.emit(Op::SetLocal(slot));
                    } else if self
                        .self_fields
                        .as_ref()
                        .is_some_and(|f| f.iter().any(|n| n == name))
                    {
                        self.emit(Op::GetLocal(0));
                        let idx = self.intern_method(name)?;
                        if matches!(op, AssignOp::Assign) {
                            self.compile_expr(right)?;
                        } else {
                            self.emit(Op::Dup);
                            self.emit(Op::GetProp(idx));
                            self.compile_expr(right)?;
                            self.emit(assign_binop(op)?);
                        }
                        self.emit(Op::SetProp(idx));
                    } else if let Some(&slot) = self.globals.get(name) {
                        if matches!(op, AssignOp::Assign) {
                            self.compile_expr(right)?;
                        } else {
                            self.emit(Op::GetGlobal(slot));
                            self.compile_expr(right)?;
                            self.emit(assign_binop(op)?);
                        }
                        self.emit(Op::SetGlobal(slot));
                    } else {
                        return Err(self.unsupported(&format!("global '{name}'")));
                    }
                }
                ExprKind::Index { object, index } => {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    if matches!(op, AssignOp::Assign) {
                        self.compile_expr(right)?;
                    } else {
                        self.emit(Op::PeekIndex);
                        self.compile_expr(right)?;
                        self.emit(assign_binop(op)?);
                    }
                    self.emit(Op::SetIndex);
                }
                ExprKind::Member { object, name } => {
                    if let ExprKind::Ident(obj) = &object.kind {
                        if self.resolve_local(obj).is_none() {
                            if let Some(canonical) = self.modules.get(obj).cloned() {
                                let key = format!("{canonical}.{name}");
                                if let Some(&slot) = self.qualified_globals.get(&key) {
                                    if matches!(op, AssignOp::Assign) {
                                        self.compile_expr(right)?;
                                    } else {
                                        self.emit(Op::GetGlobal(slot));
                                        self.compile_expr(right)?;
                                        self.emit(assign_binop(op)?);
                                    }
                                    self.emit(Op::SetGlobal(slot));
                                    return Ok(());
                                }
                            }
                        }
                    }
                    self.compile_expr(object)?;
                    let idx = self.intern_method(name)?;
                    if matches!(op, AssignOp::Assign) {
                        self.compile_expr(right)?;
                    } else {
                        self.emit(Op::Dup);
                        self.emit(Op::GetProp(idx));
                        self.compile_expr(right)?;
                        self.emit(assign_binop(op)?);
                    }
                    self.emit(Op::SetProp(idx));
                }
                _ => return Err(self.unsupported("assign target")),
            },
            ExprKind::Call { callee, args } => {
                if args.len() > 255 {
                    return Err(self.unsupported("too many arguments"));
                }
                enum Callee {
                    User(u16),
                    Host(u16),
                    Method(u16),
                    Local(u8),
                    Global(u16),
                    Value,
                }
                let mut receiver: Option<&Expr> = None;
                let kind = match &callee.kind {
                    ExprKind::Ident(name) => {
                        if let Some(slot) = self.resolve_local(name) {
                            Callee::Local(slot)
                        } else if let Some(&idx) = self.fns.get(name) {
                            Callee::User(idx)
                        } else if let Some(&slot) = self.globals.get(name) {
                            Callee::Global(slot)
                        } else if let Some((module, n)) = self.host_idents.get(name).cloned() {
                            Callee::Host(self.intern_host(&module, &n)?)
                        } else if is_builtin(name) {
                            Callee::Host(self.intern_host("", name)?)
                        } else {
                            return Err(self.unsupported(&format!("call '{name}'")));
                        }
                    }
                    ExprKind::Member { object, name } => {
                        if let ExprKind::Ident(obj) = &object.kind {
                            if obj == "super" && self.resolve_local("super").is_none() {
                                let idx = self.resolve_super_method(name)?;
                                self.emit(Op::GetLocal(0));
                                for a in args {
                                    self.compile_expr(a)?;
                                }
                                self.emit(Op::Call(idx, args.len() as u8 + 1));
                                return Ok(());
                            }
                            if self.resolve_local(obj).is_none() {
                                if let Some(canonical) = self.modules.get(obj).cloned() {
                                    let key = format!("{canonical}.{name}");
                                    if let Some(&idx) = self.qualified.get(&key) {
                                        Callee::User(idx)
                                    } else if crate::stdlib::is_host_module(&canonical) {
                                        Callee::Host(self.intern_host(&canonical, name)?)
                                    } else {
                                        return Err(
                                            self.unsupported(&format!("call '{obj}.{name}'"))
                                        );
                                    }
                                } else if crate::stdlib::is_internal_host(obj) {
                                    Callee::Host(self.intern_host(obj, name)?)
                                } else {
                                    receiver = Some(object);
                                    Callee::Method(self.intern_method(name)?)
                                }
                            } else {
                                receiver = Some(object);
                                Callee::Method(self.intern_method(name)?)
                            }
                        } else {
                            receiver = Some(object);
                            Callee::Method(self.intern_method(name)?)
                        }
                    }
                    _ => Callee::Value,
                };
                if let Some(obj) = receiver {
                    self.compile_expr(obj)?;
                }
                match &kind {
                    Callee::Local(slot) => self.emit(Op::GetLocal(*slot)),
                    Callee::Global(slot) => self.emit(Op::GetGlobal(*slot)),
                    Callee::Value => self.compile_expr(callee)?,
                    _ => {}
                }
                for a in args {
                    self.compile_expr(a)?;
                }
                match kind {
                    Callee::User(idx) => self.emit(Op::Call(idx, args.len() as u8)),
                    Callee::Host(idx) => self.emit(Op::Host(idx, args.len() as u8)),
                    Callee::Method(idx) => self.emit(Op::Invoke(idx, args.len() as u8)),
                    Callee::Local(_) | Callee::Global(_) | Callee::Value => {
                        self.emit(Op::CallValue(args.len() as u8))
                    }
                }
            }
            ExprKind::Member { object, name } => {
                if let ExprKind::Ident(obj) = &object.kind {
                    if self.resolve_local(obj).is_none() {
                        if let Some(canonical) = self.modules.get(obj).cloned() {
                            let key = format!("{canonical}.{name}");
                            if let Some(&idx) = self.qualified.get(&key) {
                                self.emit(Op::MakeClosure(idx, 0));
                                return Ok(());
                            }
                            if let Some(&slot) = self.qualified_globals.get(&key) {
                                self.emit(Op::GetGlobal(slot));
                                return Ok(());
                            }
                            if let Some(val) = self.qualified_types.get(&key).cloned() {
                                let i = self.add_constant(val)?;
                                self.emit(Op::Constant(i));
                                return Ok(());
                            }
                            return Err(self.unsupported(&format!("member '{obj}.{name}'")));
                        }
                        if crate::stdlib::is_internal_host(obj) {
                            return Err(self.unsupported("member"));
                        }
                    }
                }
                let idx = self.intern_method(name)?;
                self.compile_expr(object)?;
                self.emit(Op::GetProp(idx));
            }
            ExprKind::Index { object, index } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Op::GetIndex);
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                self.compile_expr(start)?;
                self.compile_expr(end)?;
                self.emit(Op::MakeRange(*inclusive));
            }
            ExprKind::StructLiteral { name, fields } => {
                self.compile_struct_literal(name, fields)?
            }
            ExprKind::FString(parts) => self.compile_fstring(parts)?,
            ExprKind::Match { expr, arms } => self.compile_match(expr, arms)?,
            ExprKind::Spawn(inner) => self.compile_spawn(inner)?,
            ExprKind::Await(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Await);
            }
            ExprKind::Try(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Try);
            }
            ExprKind::Lambda { params, body, .. } => self.compile_lambda(params, body)?,
        }
        Ok(())
    }

    fn compile_fstring(&mut self, parts: &[FStringPart]) -> Result<(), CompileError> {
        let empty = self.add_constant(Value::String(String::new()))?;
        self.emit(Op::Constant(empty));
        for part in parts {
            match part {
                FStringPart::Text(t) => {
                    let i = self.add_constant(Value::String(t.clone()))?;
                    self.emit(Op::Constant(i));
                    self.emit(Op::Add);
                }
                FStringPart::Expr { expr, format } => {
                    self.compile_expr(expr)?;
                    let spec = format.clone().unwrap_or_default();
                    let i = self.add_constant(Value::String(spec))?;
                    self.emit(Op::Format(i));
                    self.emit(Op::Add);
                }
            }
        }
        Ok(())
    }

    fn compile_struct_literal(
        &mut self,
        name: &str,
        fields: &[(String, Expr)],
    ) -> Result<(), CompileError> {
        if !self.visible_types.contains(name) && !self.type_idents.contains_key(name) {
            return Err(self.unsupported(&format!("struct '{name}'")));
        }
        let def = self
            .structs
            .get(name)
            .cloned()
            .ok_or_else(|| self.unsupported(&format!("struct '{name}'")))?;
        let idx = self.intern_struct(name)?;
        self.emit(Op::NewStruct(idx));
        let mut provided = HashSet::new();
        for (field, expr) in fields {
            provided.insert(field.clone());
            self.emit(Op::Dup);
            self.compile_expr(expr)?;
            let p = self.intern_method(field)?;
            self.emit(Op::SetProp(p));
            self.emit(Op::Pop);
        }
        for (field, expr) in &def.defaults {
            if provided.contains(field) {
                continue;
            }
            self.emit(Op::Dup);
            self.compile_expr(expr)?;
            let p = self.intern_method(field)?;
            self.emit(Op::SetProp(p));
            self.emit(Op::Pop);
        }
        Ok(())
    }

    fn compile_match(&mut self, expr: &Expr, arms: &[MatchArm]) -> Result<(), CompileError> {
        self.compile_expr(expr)?;
        let mut end_jumps = Vec::new();
        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard => {
                    self.emit(Op::Pop);
                    self.compile_block(&arm.body)?;
                    self.emit(Op::Void);
                    end_jumps.push(self.emit_jump(Op::Jump));
                }
                Pattern::Variant {
                    name,
                    binds,
                    field_binds,
                } => {
                    self.emit(Op::Dup);
                    let v = self.intern_method(name)?;
                    self.emit(Op::IsVariant(v));
                    let skip = self.emit_jump(Op::JumpIfFalse);
                    self.emit(Op::Pop);
                    if !field_binds.is_empty() {
                        self.begin_scope();
                        for (field, bind) in field_binds {
                            if bind == "_" {
                                continue;
                            }
                            self.emit(Op::Dup);
                            let f = self.intern_method(field)?;
                            self.emit(Op::GetNamedField(f));
                            let slot = self.add_local(bind.clone())?;
                            self.emit(Op::SetLocal(slot));
                            self.emit(Op::Pop);
                        }
                        self.emit(Op::Pop);
                        self.compile_block(&arm.body)?;
                        self.end_scope();
                        self.emit(Op::Void);
                        end_jumps.push(self.emit_jump(Op::Jump));
                        self.patch(skip);
                        self.emit(Op::Pop);
                        continue;
                    }
                    match binds.len() {
                        0 => self.emit(Op::Pop),
                        1 if binds[0] == "_" => self.emit(Op::Pop),
                        1 => {
                            self.emit(Op::EnumPayload);
                            self.begin_scope();
                            let slot = self.add_local(binds[0].clone())?;
                            self.emit(Op::SetLocal(slot));
                            self.emit(Op::Pop);
                            self.compile_block(&arm.body)?;
                            self.end_scope();
                            self.emit(Op::Void);
                            end_jumps.push(self.emit_jump(Op::Jump));
                            self.patch(skip);
                            self.emit(Op::Pop);
                            continue;
                        }
                        _ => {
                            self.emit(Op::EnumPayload);
                            self.begin_scope();
                            for (i, bind) in binds.iter().enumerate() {
                                if bind == "_" {
                                    continue;
                                }
                                self.emit(Op::Dup);
                                let ci = self.add_constant(Value::Int(i as i64))?;
                                self.emit(Op::Constant(ci));
                                self.emit(Op::GetIndex);
                                let slot = self.add_local(bind.clone())?;
                                self.emit(Op::SetLocal(slot));
                                self.emit(Op::Pop);
                            }
                            self.emit(Op::Pop);
                            self.compile_block(&arm.body)?;
                            self.end_scope();
                            self.emit(Op::Void);
                            end_jumps.push(self.emit_jump(Op::Jump));
                            self.patch(skip);
                            self.emit(Op::Pop);
                            continue;
                        }
                    }
                    self.compile_block(&arm.body)?;
                    self.emit(Op::Void);
                    end_jumps.push(self.emit_jump(Op::Jump));
                    self.patch(skip);
                    self.emit(Op::Pop);
                }
            }
        }
        self.emit(Op::Pop);
        self.emit(Op::Void);
        for j in end_jumps {
            self.patch(j);
        }
        Ok(())
    }

    fn compile_lambda(&mut self, params: &[Param], body: &Block) -> Result<(), CompileError> {
        let captures = collect_captures(self, params, body);
        for name in &captures {
            let slot = self
                .resolve_local(name)
                .ok_or_else(|| self.unsupported("lambda capture"))?;
            self.emit(Op::BoxLocal(slot));
        }
        if self.fn_base as usize + self.extra_fns.len() >= u16::MAX as usize {
            return Err(self.unsupported("too many functions"));
        }
        let slot = self.extra_fns.len();
        let idx = self.fn_base + slot as u16;
        self.extra_fns.push(FnProto {
            name: "<lambda>".to_string(),
            arity: 0,
            local_count: 0,
            chunk: Chunk::new(),
            is_async: false,
            file: self.current_file.clone(),
        });

        let saved_chunk = std::mem::replace(&mut self.chunk, Chunk::new());
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_count = self.local_count;
        let saved_scope = self.scope_depth;
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_fields = self.self_fields.take();
        let saved_super = self.super_type.take();
        let saved_span = self.span;

        self.local_count = 0;
        self.scope_depth = 0;
        for name in &captures {
            self.add_local(name.clone())?;
        }
        for p in params {
            self.add_local(p.name.clone())?;
        }
        self.compile_block(body)?;
        self.emit(Op::Void);
        self.emit(Op::Return);
        let proto = FnProto {
            name: "<lambda>".to_string(),
            arity: captures.len() + params.len(),
            local_count: self.local_count,
            chunk: std::mem::replace(&mut self.chunk, saved_chunk),
            is_async: false,
            file: self.current_file.clone(),
        };
        self.locals = saved_locals;
        self.local_count = saved_local_count;
        self.scope_depth = saved_scope;
        self.loops = saved_loops;
        self.self_fields = saved_fields;
        self.super_type = saved_super;
        self.span = saved_span;
        self.extra_fns[slot] = proto;
        self.emit(Op::MakeClosure(idx, captures.len() as u8));
        Ok(())
    }

    fn resolve_super_method(&self, name: &str) -> Result<u16, CompileError> {
        let mut current = self.super_type.clone().ok_or_else(|| CompileError {
            message: "super is only valid in a method of a class that extends another".into(),
            span: self.span,
        })?;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.clone()) {
                break;
            }
            if let Some(&idx) = self.type_methods.get(&(current.clone(), name.to_string())) {
                return Ok(idx);
            }
            match self.parents.get(&current) {
                Some(p) => current = p.clone(),
                None => break,
            }
        }
        Err(CompileError {
            message: format!("bytecode: super has no method '{name}'"),
            span: self.span,
        })
    }

    fn compile_spawn(&mut self, inner: &Expr) -> Result<(), CompileError> {
        match &inner.kind {
            ExprKind::Lambda { .. } => {
                self.compile_expr(inner)?;
                self.emit(Op::SpawnValue(0));
                Ok(())
            }
            ExprKind::Call { callee, args } => {
                if args.len() > 255 {
                    return Err(self.unsupported("too many arguments"));
                }
                match &callee.kind {
                    ExprKind::Ident(name) => {
                        if let Some(slot) = self.resolve_local(name) {
                            self.emit(Op::GetLocal(slot));
                            for a in args {
                                self.compile_expr(a)?;
                            }
                            self.emit(Op::SpawnValue(args.len() as u8));
                        } else if let Some(&idx) = self.fns.get(name) {
                            for a in args {
                                self.compile_expr(a)?;
                            }
                            self.emit(Op::SpawnCall(idx, args.len() as u8));
                        } else if let Some(&slot) = self.globals.get(name) {
                            self.emit(Op::GetGlobal(slot));
                            for a in args {
                                self.compile_expr(a)?;
                            }
                            self.emit(Op::SpawnValue(args.len() as u8));
                        } else if let Some((module, n)) = self.host_idents.get(name).cloned() {
                            let idx = self.intern_host(&module, &n)?;
                            for a in args {
                                self.compile_expr(a)?;
                            }
                            self.emit(Op::SpawnHost(idx, args.len() as u8));
                        } else if is_builtin(name) {
                            let idx = self.intern_host("", name)?;
                            for a in args {
                                self.compile_expr(a)?;
                            }
                            self.emit(Op::SpawnHost(idx, args.len() as u8));
                        } else {
                            return Err(self.unsupported(&format!("spawn '{name}'")));
                        }
                    }
                    ExprKind::Member { object, name } => {
                        if let ExprKind::Ident(obj) = &object.kind {
                            if obj == "super" && self.resolve_local("super").is_none() {
                                let idx = self.resolve_super_method(name)?;
                                self.emit(Op::GetLocal(0));
                                for a in args {
                                    self.compile_expr(a)?;
                                }
                                self.emit(Op::SpawnCall(idx, args.len() as u8 + 1));
                                return Ok(());
                            }
                            if self.resolve_local(obj).is_none() {
                                if let Some(canonical) = self.modules.get(obj).cloned() {
                                    let key = format!("{canonical}.{name}");
                                    if let Some(&idx) = self.qualified.get(&key) {
                                        for a in args {
                                            self.compile_expr(a)?;
                                        }
                                        self.emit(Op::SpawnCall(idx, args.len() as u8));
                                        return Ok(());
                                    }
                                    if crate::stdlib::is_host_module(&canonical) {
                                        let idx = self.intern_host(&canonical, name)?;
                                        for a in args {
                                            self.compile_expr(a)?;
                                        }
                                        self.emit(Op::SpawnHost(idx, args.len() as u8));
                                        return Ok(());
                                    }
                                } else if crate::stdlib::is_internal_host(obj) {
                                    let idx = self.intern_host(obj, name)?;
                                    for a in args {
                                        self.compile_expr(a)?;
                                    }
                                    self.emit(Op::SpawnHost(idx, args.len() as u8));
                                    return Ok(());
                                }
                            }
                        }
                        let idx = self.intern_method(name)?;
                        self.compile_expr(object)?;
                        for a in args {
                            self.compile_expr(a)?;
                        }
                        self.emit(Op::SpawnInvoke(idx, args.len() as u8));
                    }
                    _ => {
                        self.compile_expr(callee)?;
                        for a in args {
                            self.compile_expr(a)?;
                        }
                        self.emit(Op::SpawnValue(args.len() as u8));
                    }
                }
                Ok(())
            }
            _ => Err(self.unsupported("spawn (expected call or lambda)")),
        }
    }
}

fn bin_op(op: BinOp) -> Result<Op, CompileError> {
    Ok(match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::IDiv => Op::IDiv,
        BinOp::Mod => Op::Mod,
        BinOp::Eq => Op::Eq,
        BinOp::Neq => Op::Neq,
        BinOp::Lt => Op::Lt,
        BinOp::Lte => Op::Lte,
        BinOp::Gt => Op::Gt,
        BinOp::Gte => Op::Gte,
        BinOp::BitAnd => Op::BitAnd,
        BinOp::BitOr => Op::BitOr,
        BinOp::BitXor => Op::BitXor,
        BinOp::Shl => Op::Shl,
        BinOp::Shr => Op::Shr,
        BinOp::And | BinOp::Or => {
            return Err(CompileError {
                message: "bytecode: and/or use jumps".into(),
                span: Span::default(),
            });
        }
    })
}

fn assign_binop(op: &AssignOp) -> Result<Op, CompileError> {
    Ok(match op {
        AssignOp::Assign => {
            return Err(CompileError {
                message: "bytecode: assign".into(),
                span: Span::default(),
            });
        }
        AssignOp::Add => Op::Add,
        AssignOp::Sub => Op::Sub,
        AssignOp::Mul => Op::Mul,
        AssignOp::Div => Op::Div,
        AssignOp::Mod => Op::Mod,
        AssignOp::BitAnd => Op::BitAnd,
        AssignOp::BitOr => Op::BitOr,
        AssignOp::BitXor => Op::BitXor,
    })
}

/// Compile every same-file `fn` (not `@test` / async). `main` is required.
pub fn compile_program(program: &[Item]) -> Result<Program, CompileError> {
    let fallback = HashMapResolver::new(HashMap::new());
    compile_program_with(program, Some(&fallback))
}

/// Same as [`compile_program`], and compile imported user `.rg` modules
/// plus crate `math` / `str` / `checks` / `vec`. Native hosts (`io`, `time`, `__math`, …)
/// become [`Op::Host`].
pub fn compile_program_with(
    program: &[Item],
    resolver: Option<&dyn ModuleResolver>,
) -> Result<Program, CompileError> {
    let mut loaded = HashMap::new();
    let mut loading = HashSet::new();
    for item in program {
        if let Item::Import(imp) = item {
            if let Some(name) = import_load_name(imp) {
                load_user_module(&name, imp.span, resolver, &mut loaded, &mut loading)?;
            }
        }
    }

    let mut decls: Vec<FnDecl> = Vec::new();
    let mut origin: Vec<Option<String>> = Vec::new();
    let mut bind_selfs: Vec<bool> = Vec::new();
    let mut method_types: Vec<Option<String>> = Vec::new();
    let mut main_fns: HashMap<String, u16> = HashMap::new();
    let mut main_imports = Vec::new();
    let mut module_locals: HashMap<String, HashMap<String, u16>> = HashMap::new();
    let mut qualified: HashMap<String, u16> = HashMap::new();
    let mut enums = prelude_enums();
    let mut structs: HashMap<String, Arc<StructDef>> = HashMap::new();
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut type_methods: HashMap<(String, String), u16> = HashMap::new();
    let mut signals: HashMap<String, usize> = HashMap::new();
    let mut ufcs: HashMap<String, u16> = HashMap::new();
    let mut trait_signals: HashMap<String, Vec<SignalDecl>> = HashMap::new();
    let mut impl_traits: Vec<String> = Vec::new();
    let mut pending_globals: Vec<PendingGlobal> = Vec::new();
    let mut main_globals: HashMap<String, u16> = HashMap::new();
    let mut module_globals: HashMap<String, HashMap<String, u16>> = HashMap::new();
    let mut qualified_globals: HashMap<String, u16> = HashMap::new();
    let mut qualified_types: HashMap<String, Value> = HashMap::new();
    let mut main_type_names: HashSet<String> = HashSet::new();
    let mut module_type_names: HashMap<String, HashSet<String>> = HashMap::new();

    for item in program {
        match item {
            Item::Comment(_) | Item::Mod(_) => {}
            Item::TraitDecl(t) => {
                trait_signals.insert(t.name.clone(), t.signals.clone());
            }
            Item::SignalDecl(s) => {
                signals.insert(s.name.clone(), s.params.len());
            }
            Item::FnDecl(f) if f.is_test => {}
            Item::FnDecl(f) => {
                let idx = push_fn(f, &mut decls, &mut origin, None)?;
                bind_selfs.push(false);
                method_types.push(None);
                if f.is_ufcs {
                    ufcs.insert(f.name.clone(), idx);
                }
                if main_fns.insert(f.name.clone(), idx).is_some() {
                    return Err(duplicate_fn(&f.name, fn_span(f)));
                }
            }
            Item::Import(i) => main_imports.push(i.clone()),
            Item::EnumDecl(e) => {
                enums.insert(e.name.clone(), enum_from_decl(e));
                main_type_names.insert(e.name.clone());
            }
            Item::StructDecl(s) => {
                structs.insert(s.name.clone(), struct_from_struct(s));
                main_type_names.insert(s.name.clone());
            }
            Item::ClassDecl(c) => {
                register_class(
                    c,
                    None,
                    &mut decls,
                    &mut origin,
                    &mut bind_selfs,
                    &mut method_types,
                    &mut structs,
                    &mut parents,
                    &mut type_methods,
                )?;
                impl_traits.extend(c.implemented_traits());
                main_type_names.insert(c.name.clone());
            }
            Item::ImplDecl {
                type_name,
                trait_name,
                methods,
                ..
            } => {
                register_impl(
                    type_name,
                    methods,
                    None,
                    &mut decls,
                    &mut origin,
                    &mut bind_selfs,
                    &mut method_types,
                    &mut type_methods,
                )?;
                if let Some(t) = trait_name {
                    impl_traits.push(t.clone());
                }
            }
            Item::VarDecl(v) => {
                let slot = push_global(&mut pending_globals, v.value.clone(), None)?;
                main_globals.insert(v.name.clone(), slot);
            }
            Item::ConstDecl(c) => {
                let slot = push_global(&mut pending_globals, Some(c.value.clone()), None)?;
                main_globals.insert(c.name.clone(), slot);
            }
        }
    }

    for (name, src) in &loaded {
        let mut locals = HashMap::new();
        let mut locals_globals = HashMap::new();
        let mut type_names = HashSet::new();
        for (item, from_mod) in &src.items {
            let exported = crate::parser::item_is_exported(item, *from_mod);
            match item {
                Item::Comment(_) | Item::Import(_) | Item::Mod(_) => {}
                Item::TraitDecl(t) => {
                    trait_signals.insert(t.name.clone(), t.signals.clone());
                }
                Item::SignalDecl(s) => {
                    if exported {
                        signals.insert(s.name.clone(), s.params.len());
                    }
                }
                Item::FnDecl(f) if f.is_test => {}
                Item::FnDecl(f) => {
                    if locals.contains_key(&f.name) {
                        return Err(duplicate_fn(&f.name, fn_span(f)));
                    }
                    let idx = push_fn(f, &mut decls, &mut origin, Some(name.clone()))?;
                    bind_selfs.push(false);
                    method_types.push(None);
                    if f.is_ufcs {
                        ufcs.insert(f.name.clone(), idx);
                    }
                    locals.insert(f.name.clone(), idx);
                    if exported {
                        qualified.insert(format!("{name}.{}", f.name), idx);
                    }
                }
                Item::EnumDecl(e) => {
                    let def = enum_from_decl(e);
                    enums.insert(e.name.clone(), def.clone());
                    type_names.insert(e.name.clone());
                    if exported {
                        qualified_types.insert(format!("{name}.{}", e.name), Value::EnumType(def));
                    }
                }
                Item::StructDecl(s) => {
                    let def = struct_from_struct(s);
                    structs.insert(s.name.clone(), def.clone());
                    type_names.insert(s.name.clone());
                    if exported {
                        qualified_types
                            .insert(format!("{name}.{}", s.name), Value::StructType(def));
                    }
                }
                Item::ClassDecl(c) => {
                    register_class(
                        c,
                        Some(name.clone()),
                        &mut decls,
                        &mut origin,
                        &mut bind_selfs,
                        &mut method_types,
                        &mut structs,
                        &mut parents,
                        &mut type_methods,
                    )?;
                    impl_traits.extend(c.implemented_traits());
                    type_names.insert(c.name.clone());
                    if exported {
                        if let Some(def) = structs.get(&c.name) {
                            qualified_types.insert(
                                format!("{name}.{}", c.name),
                                Value::StructType(def.clone()),
                            );
                        }
                    }
                }
                Item::ImplDecl {
                    type_name,
                    trait_name,
                    methods,
                    ..
                } => {
                    register_impl(
                        type_name,
                        methods,
                        Some(name.clone()),
                        &mut decls,
                        &mut origin,
                        &mut bind_selfs,
                        &mut method_types,
                        &mut type_methods,
                    )?;
                    if let Some(t) = trait_name {
                        impl_traits.push(t.clone());
                    }
                }
                Item::VarDecl(v) => {
                    let slot =
                        push_global(&mut pending_globals, v.value.clone(), Some(name.clone()))?;
                    locals_globals.insert(v.name.clone(), slot);
                    if exported {
                        qualified_globals.insert(format!("{name}.{}", v.name), slot);
                    }
                }
                Item::ConstDecl(c) => {
                    let slot = push_global(
                        &mut pending_globals,
                        Some(c.value.clone()),
                        Some(name.clone()),
                    )?;
                    locals_globals.insert(c.name.clone(), slot);
                    if exported {
                        qualified_globals.insert(format!("{name}.{}", c.name), slot);
                    }
                }
            }
        }
        module_locals.insert(name.clone(), locals);
        module_globals.insert(name.clone(), locals_globals);
        module_type_names.insert(name.clone(), type_names);
    }

    preload_vec(
        &mut decls,
        &mut origin,
        &mut bind_selfs,
        &mut method_types,
        &mut structs,
        &mut parents,
        &mut type_methods,
    )?;

    for t in &impl_traits {
        if let Some(sigs) = trait_signals.get(t) {
            for s in sigs {
                signals.entry(s.name.clone()).or_insert(s.params.len());
            }
        }
    }

    flatten_structs(&mut structs, &parents);
    inherit_methods(&parents, &mut type_methods);

    let main_env = file_env(
        main_fns,
        main_globals,
        main_type_names,
        &main_imports,
        &qualified,
        &qualified_globals,
        &qualified_types,
    )?;
    let Some(&main) = main_env.fns.get("main") else {
        return Err(CompileError {
            message: "bytecode: no main".into(),
            span: Span::default(),
        });
    };

    let mut module_envs: HashMap<String, FileEnv> = HashMap::new();
    for (name, src) in &loaded {
        let imports: Vec<Import> = src
            .items
            .iter()
            .filter_map(|(item, _)| match item {
                Item::Import(i) => Some(i.clone()),
                _ => None,
            })
            .collect();
        let locals = module_locals.get(name).cloned().unwrap_or_default();
        let globals = module_globals.get(name).cloned().unwrap_or_default();
        module_envs.insert(
            name.clone(),
            file_env(
                locals,
                globals,
                module_type_names.get(name).cloned().unwrap_or_default(),
                &imports,
                &qualified,
                &qualified_globals,
                &qualified_types,
            )?,
        );
    }

    let mut compiler = Compiler {
        chunk: Chunk::new(),
        locals: Vec::new(),
        local_count: 0,
        scope_depth: 0,
        loops: Vec::new(),
        span: Span::default(),
        fns: HashMap::new(),
        modules: HashMap::new(),
        qualified: qualified.clone(),
        host_idents: HashMap::new(),
        hosts: Vec::new(),
        host_index: HashMap::new(),
        methods: Vec::new(),
        method_index: HashMap::new(),
        enums,
        structs,
        struct_defs: Vec::new(),
        struct_index: HashMap::new(),
        extra_fns: Vec::new(),
        fn_base: decls.len() as u16,
        self_fields: None,
        type_methods: type_methods.clone(),
        parents: parents.clone(),
        super_type: None,
        signals,
        current_file: String::new(),
        current_type: None,
        current_module: None,
        globals: HashMap::new(),
        qualified_globals: qualified_globals.clone(),
        qualified_types: qualified_types.clone(),
        type_idents: HashMap::new(),
        visible_types: HashSet::new(),
    };
    let mut functions = Vec::with_capacity(decls.len());
    for (i, decl) in decls.iter().enumerate() {
        let env = match &origin[i] {
            None => main_env.clone(),
            Some(m) => module_envs.get(m).cloned().unwrap_or_else(FileEnv::empty),
        };
        compiler.fns = env.fns;
        compiler.modules = env.modules;
        compiler.host_idents = env.host_idents;
        compiler.globals = env.globals;
        compiler.type_idents = env.types;
        compiler.visible_types = env.visible_types;
        compiler.self_fields = method_types[i]
            .as_ref()
            .and_then(|t| compiler.structs.get(t).map(|d| d.fields.clone()));
        compiler.super_type = method_types[i]
            .as_ref()
            .and_then(|t| compiler.parents.get(t).cloned());
        compiler.current_type = method_types[i].clone();
        compiler.current_module = origin[i]
            .as_ref()
            .map(|m| m.strip_suffix(".rg").unwrap_or(m).to_string());
        compiler.current_file = origin[i]
            .as_ref()
            .map(|m| file_label(m))
            .unwrap_or_default();
        functions.push(compiler.compile_fn(decl, bind_selfs[i])?);
    }
    functions.extend(compiler.extra_fns.drain(..));

    let init = if pending_globals.is_empty() {
        None
    } else {
        compiler.fn_base = functions.len() as u16;
        compiler.chunk = Chunk::new();
        compiler.locals.clear();
        compiler.local_count = 0;
        compiler.scope_depth = 0;
        compiler.loops.clear();
        compiler.self_fields = None;
        compiler.super_type = None;
        compiler.current_type = None;
        compiler.current_module = None;
        compiler.current_file = String::new();
        for g in &pending_globals {
            let env = match &g.origin {
                None => main_env.clone(),
                Some(m) => module_envs.get(m).cloned().unwrap_or_else(FileEnv::empty),
            };
            compiler.fns = env.fns;
            compiler.modules = env.modules;
            compiler.host_idents = env.host_idents;
            compiler.globals = env.globals;
            compiler.type_idents = env.types;
            compiler.visible_types = env.visible_types;
            compiler.span = Span::default();
            match &g.value {
                Some(e) => compiler.compile_expr(e)?,
                None => compiler.emit(Op::Nil),
            }
            compiler.emit(Op::SetGlobal(g.slot));
            compiler.emit(Op::Pop);
        }
        compiler.emit(Op::Void);
        compiler.emit(Op::Return);
        let proto = FnProto {
            name: "<init>".to_string(),
            arity: 0,
            local_count: compiler.local_count,
            chunk: std::mem::replace(&mut compiler.chunk, Chunk::new()),
            is_async: false,
            file: String::new(),
        };
        let idx = functions.len();
        functions.push(proto);
        functions.extend(compiler.extra_fns.drain(..));
        Some(idx)
    };

    Ok(Program {
        functions,
        hosts: compiler.hosts,
        methods: compiler.methods,
        struct_defs: compiler.struct_defs,
        enums: compiler.enums,
        type_methods,
        ufcs,
        globals: std::sync::Arc::new(std::sync::Mutex::new(
            (0..pending_globals.len()).map(|_| Value::None).collect(),
        )),
        init,
        main: main as usize,
    })
}

struct LoadedModule {
    items: Vec<(Item, bool)>,
}

fn compile_err(message: impl Into<String>, span: Span) -> CompileError {
    CompileError {
        message: message.into(),
        span,
    }
}

fn duplicate_fn(name: &str, span: Span) -> CompileError {
    compile_err(format!("bytecode: duplicate function '{name}'"), span)
}

fn fn_span(f: &FnDecl) -> Span {
    f.body.stmts.first().map(|s| s.span).unwrap_or_default()
}

fn skip_user_load(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    if crate::stdlib::is_host_module(stem) {
        return true;
    }
    match crate::stdlib::canonical_module(name).or_else(|| crate::stdlib::canonical_module(stem)) {
        Some("math" | "str" | "checks") => false,
        Some(_) => true,
        None => false,
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "print" | "len" | "assert" | "Array" | "Map" | "Mutex" | "Channel"
    )
}

fn import_load_name(imp: &Import) -> Option<String> {
    if imp.path.is_empty() {
        return None;
    }
    let name = if imp.is_from {
        imp.path[0].clone()
    } else {
        imp.path.join(".")
    };
    if skip_user_load(&name) {
        None
    } else {
        Some(name)
    }
}

fn module_not_found(name: &str, span: Span) -> CompileError {
    compile_err(
        format!(
            "bytecode: module '{name}' not found (tried {})",
            module_lookup_hint(name)
        ),
        span,
    )
}

fn parse_items(source: &str, span: Span) -> Result<Vec<Item>, CompileError> {
    let tokens = crate::lexer::Lexer::new(source)
        .tokenize()
        .map_err(|e| compile_err(e, span))?;
    crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| compile_err(e, span))
}

fn load_user_module(
    name: &str,
    span: Span,
    resolver: Option<&dyn ModuleResolver>,
    loaded: &mut HashMap<String, LoadedModule>,
    loading: &mut HashSet<String>,
) -> Result<(), CompileError> {
    if skip_user_load(name) || loaded.contains_key(name) || loading.contains(name) {
        return Ok(());
    }
    let Some(resolver) = resolver else {
        return Err(module_not_found(name, span));
    };
    let parts = resolver.resolve_all(name);
    if parts.is_empty() {
        return Err(module_not_found(name, span));
    }
    loading.insert(name.to_string());
    let mut items = Vec::new();
    let mut deps = Vec::new();
    for (_, source) in &parts {
        let program = parse_items(source, span)?;
        let from_mod = crate::parser::module_body_from_mod(&program, name);
        for item in crate::parser::module_items(&program, name) {
            if let Item::Import(imp) = item {
                deps.push(imp.clone());
            }
            items.push(((*item).clone(), from_mod));
        }
    }
    loaded.insert(name.to_string(), LoadedModule { items });
    loading.remove(name);
    for imp in deps {
        if let Some(dep) = import_load_name(&imp) {
            load_user_module(&dep, imp.span, Some(resolver), loaded, loading)?;
        }
    }
    Ok(())
}

fn push_fn(
    f: &FnDecl,
    decls: &mut Vec<FnDecl>,
    origin: &mut Vec<Option<String>>,
    module: Option<String>,
) -> Result<u16, CompileError> {
    if decls.len() >= u16::MAX as usize {
        return Err(compile_err("bytecode: too many functions", fn_span(f)));
    }
    let idx = decls.len() as u16;
    decls.push(f.clone());
    origin.push(module);
    Ok(idx)
}

fn proto_name(
    decl: &FnDecl,
    bind_self: bool,
    type_name: Option<&str>,
    module: Option<&str>,
) -> String {
    if bind_self {
        if let Some(ty) = type_name {
            return match module {
                Some(m) => format!("{m}.{ty}.{}", decl.name),
                None => format!("{ty}.{}", decl.name),
            };
        }
    }
    match module {
        Some(m) => format!("{m}.{}", decl.name),
        None => decl.name.clone(),
    }
}

struct PendingGlobal {
    slot: u16,
    value: Option<Expr>,
    origin: Option<String>,
}

fn push_global(
    pending: &mut Vec<PendingGlobal>,
    value: Option<Expr>,
    origin: Option<String>,
) -> Result<u16, CompileError> {
    if pending.len() >= u16::MAX as usize {
        return Err(compile_err("bytecode: too many globals", Span::default()));
    }
    let slot = pending.len() as u16;
    pending.push(PendingGlobal {
        slot,
        value,
        origin,
    });
    Ok(slot)
}

fn register_class(
    c: &ClassDecl,
    origin: Option<String>,
    decls: &mut Vec<FnDecl>,
    origin_vec: &mut Vec<Option<String>>,
    bind_selfs: &mut Vec<bool>,
    method_types: &mut Vec<Option<String>>,
    structs: &mut HashMap<String, Arc<StructDef>>,
    parents: &mut HashMap<String, String>,
    type_methods: &mut HashMap<(String, String), u16>,
) -> Result<(), CompileError> {
    structs.insert(c.name.clone(), struct_from_class(c));
    if let Some(p) = &c.parent {
        parents.insert(c.name.clone(), p.clone());
    }
    for m in c.all_methods() {
        if m.is_test {
            continue;
        }
        let idx = push_fn(m, decls, origin_vec, origin.clone())?;
        bind_selfs.push(true);
        method_types.push(Some(c.name.clone()));
        type_methods.insert((c.name.clone(), m.name.clone()), idx);
    }
    Ok(())
}

fn register_impl(
    type_name: &str,
    methods: &[FnDecl],
    origin: Option<String>,
    decls: &mut Vec<FnDecl>,
    origin_vec: &mut Vec<Option<String>>,
    bind_selfs: &mut Vec<bool>,
    method_types: &mut Vec<Option<String>>,
    type_methods: &mut HashMap<(String, String), u16>,
) -> Result<(), CompileError> {
    for m in methods {
        if m.is_test {
            continue;
        }
        let idx = push_fn(m, decls, origin_vec, origin.clone())?;
        bind_selfs.push(true);
        method_types.push(Some(type_name.to_string()));
        type_methods.insert((type_name.to_string(), m.name.clone()), idx);
    }
    Ok(())
}

fn preload_vec(
    decls: &mut Vec<FnDecl>,
    origin: &mut Vec<Option<String>>,
    bind_selfs: &mut Vec<bool>,
    method_types: &mut Vec<Option<String>>,
    structs: &mut HashMap<String, Arc<StructDef>>,
    parents: &mut HashMap<String, String>,
    type_methods: &mut HashMap<(String, String), u16>,
) -> Result<(), CompileError> {
    if structs.contains_key("Vec2") && structs.contains_key("Vec3") {
        return Ok(());
    }
    let Some((_, source)) = crate::stdlib::file_source("vec") else {
        return Ok(());
    };
    let items = parse_items(source, Span::default())?;
    for item in &items {
        if let Item::ClassDecl(c) = item {
            if structs.contains_key(&c.name) {
                continue;
            }
            register_class(
                c,
                Some("vec".into()),
                decls,
                origin,
                bind_selfs,
                method_types,
                structs,
                parents,
                type_methods,
            )?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct FileEnv {
    fns: HashMap<String, u16>,
    modules: HashMap<String, String>,
    host_idents: HashMap<String, (String, String)>,
    globals: HashMap<String, u16>,
    types: HashMap<String, Value>,
    visible_types: HashSet<String>,
}

impl FileEnv {
    fn empty() -> Self {
        Self {
            fns: HashMap::new(),
            modules: HashMap::new(),
            host_idents: HashMap::new(),
            globals: HashMap::new(),
            types: HashMap::new(),
            visible_types: prelude_type_names(),
        }
    }
}

fn prelude_type_names() -> HashSet<String> {
    ["Option", "Result", "Vec2", "Vec3"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn file_env(
    mut ident: HashMap<String, u16>,
    mut ident_globals: HashMap<String, u16>,
    mut visible_types: HashSet<String>,
    imports: &[Import],
    qualified: &HashMap<String, u16>,
    qualified_globals: &HashMap<String, u16>,
    qualified_types: &HashMap<String, Value>,
) -> Result<FileEnv, CompileError> {
    visible_types.extend(prelude_type_names());
    let mut modules = HashMap::new();
    let mut host_idents = HashMap::new();
    let mut types = HashMap::new();
    for imp in imports {
        apply_import(
            imp,
            &mut ident,
            &mut modules,
            &mut host_idents,
            &mut ident_globals,
            &mut types,
            &mut visible_types,
            qualified,
            qualified_globals,
            qualified_types,
        )?;
    }
    Ok(FileEnv {
        fns: ident,
        modules,
        host_idents,
        globals: ident_globals,
        types,
        visible_types,
    })
}

fn apply_import(
    import: &Import,
    ident_fns: &mut HashMap<String, u16>,
    modules: &mut HashMap<String, String>,
    host_idents: &mut HashMap<String, (String, String)>,
    ident_globals: &mut HashMap<String, u16>,
    ident_types: &mut HashMap<String, Value>,
    visible_types: &mut HashSet<String>,
    qualified: &HashMap<String, u16>,
    qualified_globals: &HashMap<String, u16>,
    qualified_types: &HashMap<String, Value>,
) -> Result<(), CompileError> {
    if import.path.is_empty() {
        return Err(compile_err("bytecode: empty import path", import.span));
    }
    let first = &import.path[0];
    if crate::stdlib::is_host_module(first) {
        if import.is_from {
            if import.path.len() == 2 {
                let item_name = &import.path[1];
                let alias = import.alias.as_ref().unwrap_or(item_name);
                host_idents.insert(alias.clone(), (first.clone(), item_name.clone()));
            }
            return Ok(());
        }
        let bind = import
            .alias
            .as_ref()
            .unwrap_or_else(|| import.path.last().unwrap());
        modules.insert(bind.clone(), first.clone());
        return Ok(());
    }
    if skip_user_load(first) {
        if import.is_from {
            return Ok(());
        }
        let bind = import
            .alias
            .as_ref()
            .unwrap_or_else(|| import.path.last().unwrap());
        modules.insert(bind.clone(), first.clone());
        return Ok(());
    }

    let canonical = if import.is_from {
        first.clone()
    } else {
        import.path.join(".")
    };

    if import.is_from {
        if import.path.len() == 1 {
            let bind = import.alias.as_ref().unwrap_or(first);
            modules.insert(bind.clone(), canonical);
        } else if import.path.len() == 2 {
            let item_name = &import.path[1];
            let alias = import.alias.as_ref().unwrap_or(item_name);
            let key = format!("{canonical}.{item_name}");
            if let Some(&idx) = qualified.get(&key) {
                if let Some(prev) = ident_fns.insert(alias.clone(), idx) {
                    if prev != idx {
                        return Err(duplicate_fn(alias, import.span));
                    }
                }
            } else if let Some(&slot) = qualified_globals.get(&key) {
                ident_globals.insert(alias.clone(), slot);
            } else if let Some(val) = qualified_types.get(&key) {
                ident_types.insert(alias.clone(), val.clone());
                visible_types.insert(alias.clone());
            } else {
                return Err(compile_err(
                    format!("bytecode: module '{canonical}' has no export '{item_name}'"),
                    import.span,
                ));
            }
        } else {
            return Err(compile_err(
                format!(
                    "bytecode: nested from-imports longer than 2 segments are not supported: {:?}",
                    import.path
                ),
                import.span,
            ));
        }
    } else {
        let bind = import
            .alias
            .as_ref()
            .unwrap_or_else(|| import.path.last().unwrap());
        modules.insert(bind.clone(), canonical);
    }
    Ok(())
}

fn prelude_enums() -> HashMap<String, Arc<EnumDef>> {
    let mut m = HashMap::new();
    let mut option = HashMap::new();
    option.insert(
        "Some".into(),
        EnumVariantDef {
            arity: 1,
            field_names: vec![String::new()],
        },
    );
    option.insert(
        "None".into(),
        EnumVariantDef {
            arity: 0,
            field_names: vec![],
        },
    );
    m.insert(
        "Option".into(),
        Arc::new(EnumDef {
            name: "Option".into(),
            variants: option,
        }),
    );
    let mut result = HashMap::new();
    result.insert(
        "Ok".into(),
        EnumVariantDef {
            arity: 1,
            field_names: vec![String::new()],
        },
    );
    result.insert(
        "Err".into(),
        EnumVariantDef {
            arity: 1,
            field_names: vec![String::new()],
        },
    );
    m.insert(
        "Result".into(),
        Arc::new(EnumDef {
            name: "Result".into(),
            variants: result,
        }),
    );
    m
}

fn enum_from_decl(e: &EnumDecl) -> Arc<EnumDef> {
    let mut variants = HashMap::new();
    for v in &e.variants {
        variants.insert(
            v.name.clone(),
            EnumVariantDef {
                arity: v.value_types.len(),
                field_names: v.field_names.clone(),
            },
        );
    }
    Arc::new(EnumDef {
        name: e.name.clone(),
        variants,
    })
}

fn struct_from_struct(s: &StructDecl) -> Arc<StructDef> {
    Arc::new(StructDef {
        name: s.name.clone(),
        fields: s.fields.iter().map(|f| f.name.clone()).collect(),
        defaults: Vec::new(),
    })
}

fn struct_from_class(c: &ClassDecl) -> Arc<StructDef> {
    Arc::new(StructDef {
        name: c.name.clone(),
        fields: c.fields.iter().map(|f| f.name.clone()).collect(),
        defaults: c.defaults.clone(),
    })
}

fn flatten_structs(
    structs: &mut HashMap<String, Arc<StructDef>>,
    parents: &HashMap<String, String>,
) {
    let names: Vec<String> = structs.keys().cloned().collect();
    let mut done = HashSet::new();
    for name in names {
        flatten_one(&name, structs, parents, &mut done);
    }
}

fn flatten_one(
    name: &str,
    structs: &mut HashMap<String, Arc<StructDef>>,
    parents: &HashMap<String, String>,
    done: &mut HashSet<String>,
) {
    if done.contains(name) {
        return;
    }
    if let Some(parent) = parents.get(name).cloned() {
        flatten_one(&parent, structs, parents, done);
        let Some(p) = structs.get(&parent).cloned() else {
            done.insert(name.to_string());
            return;
        };
        let Some(c) = structs.get(name).cloned() else {
            done.insert(name.to_string());
            return;
        };
        let mut fields = p.fields.clone();
        for f in &c.fields {
            if !fields.iter().any(|x| x == f) {
                fields.push(f.clone());
            }
        }
        let mut defaults = p.defaults.clone();
        for (n, e) in &c.defaults {
            if let Some(slot) = defaults.iter_mut().find(|(k, _)| k == n) {
                *slot = (n.clone(), e.clone());
            } else {
                defaults.push((n.clone(), e.clone()));
            }
        }
        structs.insert(
            name.to_string(),
            Arc::new(StructDef {
                name: name.to_string(),
                fields,
                defaults,
            }),
        );
    }
    done.insert(name.to_string());
}

fn inherit_methods(
    parents: &HashMap<String, String>,
    type_methods: &mut HashMap<(String, String), u16>,
) {
    let children: Vec<(String, String)> = parents
        .iter()
        .map(|(c, p)| (c.clone(), p.clone()))
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (child, parent) in &children {
            let inherited: Vec<(String, u16)> = type_methods
                .iter()
                .filter(|((t, _), _)| t == parent)
                .map(|((_, name), idx)| (name.clone(), *idx))
                .collect();
            for (name, idx) in inherited {
                let key = (child.clone(), name);
                if let std::collections::hash_map::Entry::Vacant(e) = type_methods.entry(key) {
                    e.insert(idx);
                    changed = true;
                }
            }
        }
    }
}

fn collect_captures(compiler: &Compiler, params: &[Param], body: &Block) -> Vec<String> {
    let mut declared = HashSet::new();
    for p in params {
        declared.insert(p.name.clone());
    }
    collect_block_declared(body, &mut declared);
    let mut idents = Vec::new();
    collect_block_idents(body, &mut idents);
    let mut captures = Vec::new();
    for name in idents {
        if declared.contains(&name) {
            continue;
        }
        if compiler.resolve_local(&name).is_none() {
            continue;
        }
        if !captures.contains(&name) {
            captures.push(name);
        }
    }
    captures
}

fn collect_block_declared(block: &Block, names: &mut HashSet<String>) {
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::VarDecl(v) => {
                names.insert(v.name.clone());
            }
            StmtKind::ConstDecl(c) => {
                names.insert(c.name.clone());
            }
            StmtKind::For { name, body, .. } => {
                names.insert(name.clone());
                collect_block_declared(body, names);
            }
            StmtKind::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                collect_block_declared(then_block, names);
                for (_, b) in elif_blocks {
                    collect_block_declared(b, names);
                }
                if let Some(b) = else_block {
                    collect_block_declared(b, names);
                }
            }
            StmtKind::While { body, .. } => collect_block_declared(body, names),
            StmtKind::Expr(e) | StmtKind::Return(Some(e)) => collect_expr_declared(e, names),
            _ => {}
        }
    }
}

fn collect_expr_declared(expr: &Expr, names: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Match { expr, arms } => {
            collect_expr_declared(expr, names);
            for arm in arms {
                match &arm.pattern {
                    Pattern::Wildcard => {}
                    Pattern::Variant {
                        binds, field_binds, ..
                    } => {
                        for b in binds {
                            if b != "_" {
                                names.insert(b.clone());
                            }
                        }
                        for (_, b) in field_binds {
                            if b != "_" {
                                names.insert(b.clone());
                            }
                        }
                    }
                }
                collect_block_declared(&arm.body, names);
            }
        }
        ExprKind::Lambda { params, body, .. } => {
            for p in params {
                names.insert(p.name.clone());
            }
            collect_block_declared(body, names);
        }
        ExprKind::Binary { left, right, .. } | ExprKind::Assign { left, right, .. } => {
            collect_expr_declared(left, names);
            collect_expr_declared(right, names);
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Member { object: expr, .. }
        | ExprKind::Try(expr)
        | ExprKind::Spawn(expr)
        | ExprKind::Await(expr) => collect_expr_declared(expr, names),
        ExprKind::Call { callee, args } => {
            collect_expr_declared(callee, names);
            for a in args {
                collect_expr_declared(a, names);
            }
        }
        ExprKind::Index { object, index } => {
            collect_expr_declared(object, names);
            collect_expr_declared(index, names);
        }
        ExprKind::Range { start, end, .. } => {
            collect_expr_declared(start, names);
            collect_expr_declared(end, names);
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_expr_declared(e, names);
            }
        }
        ExprKind::FString(parts) => {
            for part in parts {
                if let FStringPart::Expr { expr, .. } = part {
                    collect_expr_declared(expr, names);
                }
            }
        }
        _ => {}
    }
}

fn collect_block_idents(block: &Block, out: &mut Vec<String>) {
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::Expr(e) | StmtKind::Return(Some(e)) => collect_expr_idents(e, out),
            StmtKind::VarDecl(v) => {
                if let Some(e) = &v.value {
                    collect_expr_idents(e, out);
                }
            }
            StmtKind::ConstDecl(c) => collect_expr_idents(&c.value, out),
            StmtKind::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                collect_expr_idents(cond, out);
                collect_block_idents(then_block, out);
                for (c, b) in elif_blocks {
                    collect_expr_idents(c, out);
                    collect_block_idents(b, out);
                }
                if let Some(b) = else_block {
                    collect_block_idents(b, out);
                }
            }
            StmtKind::While { cond, body } => {
                collect_expr_idents(cond, out);
                collect_block_idents(body, out);
            }
            StmtKind::For { iter, body, .. } => {
                collect_expr_idents(iter, out);
                collect_block_idents(body, out);
            }
            _ => {}
        }
    }
}

fn collect_expr_idents(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Ident(n) => out.push(n.clone()),
        ExprKind::Binary { left, right, .. } | ExprKind::Assign { left, right, .. } => {
            collect_expr_idents(left, out);
            collect_expr_idents(right, out);
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Member { object: expr, .. }
        | ExprKind::Try(expr)
        | ExprKind::Spawn(expr)
        | ExprKind::Await(expr) => collect_expr_idents(expr, out),
        ExprKind::Call { callee, args } => {
            collect_expr_idents(callee, out);
            for a in args {
                collect_expr_idents(a, out);
            }
        }
        ExprKind::Index { object, index } => {
            collect_expr_idents(object, out);
            collect_expr_idents(index, out);
        }
        ExprKind::Range { start, end, .. } => {
            collect_expr_idents(start, out);
            collect_expr_idents(end, out);
        }
        ExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_expr_idents(e, out);
            }
        }
        ExprKind::FString(parts) => {
            for part in parts {
                if let FStringPart::Expr { expr, .. } = part {
                    collect_expr_idents(expr, out);
                }
            }
        }
        ExprKind::Match { expr, arms } => {
            collect_expr_idents(expr, out);
            for arm in arms {
                collect_block_idents(&arm.body, out);
            }
        }
        ExprKind::Lambda { body, .. } => collect_block_idents(body, out),
        _ => {}
    }
}
