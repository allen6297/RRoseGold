//! [`EvalContext`]: load a program, eval statements/expressions.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::parser::*;
use crate::{RuntimeError, Span};

use super::ops::*;
use super::resolver::*;
use super::value::*;

pub struct EvalContext {
    pub stdout: String,
    pub env: Environment,
    pub functions: HashMap<String, FnDecl>,
    pub methods: HashMap<String, HashMap<String, FnDecl>>,
    pub stdlib: HashMap<String, HashMap<String, Value>>,
    pub structs: HashMap<String, StructDefRef>,
    pub enums: HashMap<String, EnumDefRef>,
    /// Child type → parent type (`class Child extends Parent`).
    parents: HashMap<String, String>,
    /// Trait name → signal declarations (for `impl` to expose `.emit`).
    trait_signals: HashMap<String, Vec<SignalDecl>>,
    /// Type name → traits it implements (class header, nested impl, `impl Trait for Type`).
    type_traits: HashMap<String, Vec<String>>,
    /// Parent type of the method currently executing, for `super.method`.
    super_type: Option<String>,
    /// Source file label for traces (`script.rg`).
    current_file: String,
    /// Module import path while loading (`util.helpers`).
    current_module: String,
    module_resolver: Rc<RefCell<dyn ModuleResolver>>,
    loaded_modules: Rc<RefCell<HashMap<String, ModuleRef>>>,
    /// Set when `return` runs inside a `match` expression so the enclosing function exits.
    pending_return: Option<Value>,
    /// Host modules (`io`, `time`, `process`, `json`) the program imported.
    imported_host: HashSet<String>,
    /// Capture for `time.elapsed` — this VM, not frame `dt`.
    pub(super) started: Clock,
    /// Function tables of modules whose bodies are on the call stack.
    pub(super) module_fns: Vec<HashMap<String, FnDecl>>,
    /// `process.argv()` — script path first, then user args.
    argv: Vec<String>,
    /// Signal name → connected free-function names, in connect order.
    signal_listeners: HashMap<String, Vec<String>>,
}

pub struct Environment {
    scopes: Vec<HashMap<String, Value>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn define(&mut self, name: &str, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    /// Locals / params in this call (everything above the module scope).
    fn get_local(&self, name: &str) -> Option<Value> {
        if self.scopes.len() < 2 {
            return None;
        }
        for scope in self.scopes.iter().skip(1).rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn set_local(&mut self, name: &str, value: Value) -> bool {
        if self.scopes.len() < 2 {
            return false;
        }
        for scope in self.scopes.iter_mut().skip(1).rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }

    pub fn set(&mut self, name: &str, value: Value, span: Span) -> Result<(), RuntimeError> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return Ok(());
            }
        }
        Err(runtime_err(format!("undefined variable '{}'", name), span))
    }
}

impl EvalContext {
    pub fn new() -> Self {
        Self::with_resolver(Rc::new(RefCell::new(HashMapResolver::new(HashMap::new()))))
    }

    pub fn with_resolver(resolver: Rc<RefCell<dyn ModuleResolver>>) -> Self {
        let mut ctx = Self::unpreloaded(resolver);
        let _ = ctx.preload_embedded();
        ctx
    }

    fn unpreloaded(resolver: Rc<RefCell<dyn ModuleResolver>>) -> Self {
        let mut ctx = Self {
            stdout: String::new(),
            env: Environment::new(),
            functions: HashMap::new(),
            methods: HashMap::new(),
            stdlib: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            parents: HashMap::new(),
            trait_signals: HashMap::new(),
            type_traits: HashMap::new(),
            super_type: None,
            current_file: String::new(),
            current_module: String::new(),
            module_resolver: resolver,
            loaded_modules: Rc::new(RefCell::new(HashMap::new())),
            pending_return: None,
            imported_host: HashSet::new(),
            module_fns: Vec::new(),
            started: Clock::capture(),
            argv: Vec::new(),
            signal_listeners: HashMap::new(),
        };
        ctx.init_stdlib();
        ctx
    }

    pub fn resolver(&self) -> Rc<RefCell<dyn ModuleResolver>> {
        self.module_resolver.clone()
    }

    pub fn set_source_file(&mut self, file: impl Into<String>) {
        self.current_file = file.into();
    }

    fn stamp(&self, mut err: RuntimeError) -> RuntimeError {
        if err.file.is_empty() {
            err.file = self.current_file.clone();
        }
        err
    }

    fn origin_fn(&self, f: &FnDecl) -> FnDecl {
        f.clone()
            .with_origin(&self.current_file, &self.current_module)
    }

    pub fn set_argv(&mut self, argv: Vec<String>) {
        self.argv = argv;
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    pub(super) fn connect_signal(
        &mut self,
        signal: &str,
        listener: &str,
        arity: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let decl = self
            .lookup_fn(listener)
            .ok_or_else(|| runtime_err(format!("undefined function '{listener}'"), span))?;
        if decl.params.first().is_some_and(|p| p.name == "self") {
            return Err(runtime_err(
                format!(
                    "signal '{signal}' connect expects a free function, got method '{listener}'"
                ),
                span,
            ));
        }
        if decl.params.len() != arity {
            return Err(runtime_err(
                format!(
                    "{listener} expected {arity} args to connect to '{signal}', got {}",
                    decl.params.len()
                ),
                span,
            ));
        }
        let list = self.signal_listeners.entry(signal.to_string()).or_default();
        if !list.iter().any(|n| n == listener) {
            list.push(listener.to_string());
        }
        Ok(())
    }

    pub(super) fn emit_signal(
        &mut self,
        signal: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let listeners = self
            .signal_listeners
            .get(signal)
            .cloned()
            .unwrap_or_default();
        for name in listeners {
            self.call_fn(&name, args.clone(), span)?;
        }
        Ok(Value::Void)
    }

    fn preload_embedded(&mut self) -> Result<(), RuntimeError> {
        let span = Span::default();
        self.load_module("option", span)?;
        self.load_module("result", span)?;
        self.load_module("vec", span)?;
        if let Some(opt) = self.enums.get("Option").cloned() {
            self.enums.insert("option".to_string(), opt);
        }
        if let Some(res) = self.enums.get("Result").cloned() {
            self.enums.insert("result".to_string(), res);
        }
        Ok(())
    }

    fn init_stdlib(&mut self) {
        self.stdlib.insert("__str".to_string(), HashMap::new());
        self.stdlib.insert("__math".to_string(), HashMap::new());
        self.stdlib.insert("io".to_string(), HashMap::new());
        self.stdlib.insert("time".to_string(), HashMap::new());
        self.stdlib.insert("process".to_string(), HashMap::new());
        self.stdlib.insert("json".to_string(), HashMap::new());
        self.stdlib.insert("Array".to_string(), HashMap::new());
    }

    pub fn run(&mut self, program: &[Item]) -> Result<Value, RuntimeError> {
        self.load_program(program)?;
        if self.functions.contains_key("main") {
            self.call_fn("main", vec![], Span::default())
        } else {
            Ok(Value::Void)
        }
    }

    /// Register declarations and evaluate top-level consts/vars without calling `main`.
    pub fn load_program(&mut self, program: &[Item]) -> Result<(), RuntimeError> {
        // First pass: register declarations
        for item in program {
            match item {
                Item::FnDecl(f) => {
                    self.functions
                        .insert(f.name.clone(), self.origin_fn(f));
                }
                Item::StructDecl(s) => {
                    let fields = s.fields.iter().map(|f| f.name.clone()).collect();
                    self.structs.insert(
                        s.name.clone(),
                        Rc::new(StructDef {
                            name: s.name.clone(),
                            fields,
                            defaults: Vec::new(),
                        }),
                    );
                }
                Item::ClassDecl(c) => {
                    self.register_class(c);
                }
                Item::TraitDecl(t) => {
                    self.register_trait_decl(t);
                }
                Item::EnumDecl(e) => {
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
                    self.enums.insert(
                        e.name.clone(),
                        Rc::new(EnumDef {
                            name: e.name.clone(),
                            variants,
                        }),
                    );
                }
                Item::ImplDecl {
                    type_name,
                    trait_name,
                    methods,
                    ..
                } => {
                    let stamped: Vec<FnDecl> =
                        methods.iter().map(|m| self.origin_fn(m)).collect();
                    let entry = self
                        .methods
                        .entry(type_name.clone())
                        .or_insert_with(HashMap::new);
                    for m in stamped {
                        entry.insert(m.name.clone(), m);
                    }
                    if let Some(t) = trait_name {
                        self.record_type_trait(type_name, t);
                    }
                }
                Item::Import(i) => {
                    self.eval_import(i, i.span)?;
                }
                Item::SignalDecl(s) => {
                    self.env.define(
                        &s.name,
                        Value::Signal {
                            name: s.name.clone(),
                            arity: s.params.len(),
                        },
                    );
                }
                _ => {}
            }
        }

        self.bind_all_trait_signals();
        self.apply_inheritance(Span::default())?;

        // Second pass: evaluate top-level consts and vars
        for item in program {
            match item {
                Item::ConstDecl(c) => {
                    let value = self.eval_expr(&c.value)?;
                    self.env.define(&c.name, value);
                }
                Item::VarDecl(v) => {
                    let value = match &v.value {
                        Some(e) => self.eval_expr(e)?,
                        None => Value::None,
                    };
                    self.env.define(&v.name, value);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn register_class(&mut self, c: &ClassDecl) {
        let fields = c.fields.iter().map(|f| f.name.clone()).collect();
        self.structs.insert(
            c.name.clone(),
            Rc::new(StructDef {
                name: c.name.clone(),
                fields,
                defaults: c.defaults.clone(),
            }),
        );
        if let Some(p) = &c.parent {
            self.parents.insert(c.name.clone(), p.clone());
        }
        let stamped: Vec<FnDecl> = c.all_methods().map(|m| self.origin_fn(m)).collect();
        let entry = self
            .methods
            .entry(c.name.clone())
            .or_insert_with(HashMap::new);
        for m in stamped {
            entry.insert(m.name.clone(), m);
        }
        for t in c.implemented_traits() {
            self.record_type_trait(&c.name, &t);
        }
    }

    fn register_trait_decl(&mut self, t: &TraitDecl) {
        self.trait_signals.insert(t.name.clone(), t.signals.clone());
    }

    fn record_type_trait(&mut self, type_name: &str, trait_name: &str) {
        let entry = self.type_traits.entry(type_name.to_string()).or_default();
        if !entry.iter().any(|n| n == trait_name) {
            entry.push(trait_name.to_string());
        }
    }

    fn define_trait_signals(&mut self, trait_name: &str) {
        let Some(sigs) = self.trait_signals.get(trait_name).cloned() else {
            return;
        };
        for s in sigs {
            if self.env.get(&s.name).is_some() {
                continue;
            }
            self.env.define(
                &s.name,
                Value::Signal {
                    name: s.name.clone(),
                    arity: s.params.len(),
                },
            );
        }
    }

    fn bind_all_trait_signals(&mut self) {
        let traits: Vec<String> = self.type_traits.values().flatten().cloned().collect();
        for t in traits {
            self.define_trait_signals(&t);
        }
    }

    fn apply_inheritance(&mut self, span: Span) -> Result<(), RuntimeError> {
        let names: Vec<String> = self.parents.keys().cloned().collect();
        let mut done = HashSet::new();
        let mut stack = Vec::new();
        for name in names {
            self.flatten_type(&name, span, &mut done, &mut stack)?;
        }
        Ok(())
    }

    fn flatten_type(
        &mut self,
        name: &str,
        span: Span,
        done: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), RuntimeError> {
        if done.contains(name) {
            return Ok(());
        }
        if stack.iter().any(|n| n == name) {
            return Err(runtime_err(
                format!("cycle in class inheritance at '{name}'"),
                span,
            ));
        }
        let Some(parent) = self.parents.get(name).cloned() else {
            done.insert(name.to_string());
            return Ok(());
        };
        if !self.structs.contains_key(&parent) {
            if self.methods.contains_key(&parent) {
                done.insert(name.to_string());
                return Ok(());
            }
            return Err(runtime_err(
                format!("class '{name}' extends unknown type '{parent}'"),
                span,
            ));
        }
        if self.enums.contains_key(&parent) {
            return Err(runtime_err(
                format!("class '{name}' cannot extend enum '{parent}'"),
                span,
            ));
        }
        stack.push(name.to_string());
        self.flatten_type(&parent, span, done, stack)?;
        stack.pop();
        let parent_def = self
            .structs
            .get(&parent)
            .cloned()
            .ok_or_else(|| runtime_err(format!("unknown type '{parent}'"), span))?;
        let child_def = self
            .structs
            .get(name)
            .cloned()
            .ok_or_else(|| runtime_err(format!("unknown type '{name}'"), span))?;
        let mut fields = parent_def.fields.clone();
        for f in &child_def.fields {
            if !fields.iter().any(|p| p == f) {
                fields.push(f.clone());
            }
        }
        let mut defaults = parent_def.defaults.clone();
        for (n, e) in &child_def.defaults {
            if let Some(slot) = defaults.iter_mut().find(|(k, _)| k == n) {
                *slot = (n.clone(), e.clone());
            } else {
                defaults.push((n.clone(), e.clone()));
            }
        }
        self.structs.insert(
            name.to_string(),
            Rc::new(StructDef {
                name: name.to_string(),
                fields,
                defaults,
            }),
        );
        done.insert(name.to_string());
        Ok(())
    }

    fn lookup_method(&self, type_name: &str, name: &str) -> Option<(String, FnDecl)> {
        let mut current = Some(type_name.to_string());
        let mut seen = HashSet::new();
        while let Some(ty) = current {
            if !seen.insert(ty.clone()) {
                break;
            }
            if let Some(decl) = self.methods.get(&ty).and_then(|m| m.get(name)) {
                return Some((ty, decl.clone()));
            }
            current = self.parents.get(&ty).cloned();
        }
        None
    }

    pub(super) fn call_type_method(
        &mut self,
        type_name: &str,
        object: &Value,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some((defined_on, decl)) = self.lookup_method(type_name, name) else {
            return Ok(None);
        };
        let prev = self.super_type.take();
        self.super_type = self.parents.get(&defined_on).cloned();
        let result = self.call_method(&decl, object.clone(), args, span, &defined_on);
        self.super_type = prev;
        result.map(Some)
    }

    fn call_method(
        &mut self,
        decl: &FnDecl,
        object: Value,
        extra: Vec<Value>,
        span: Span,
        type_name: &str,
    ) -> Result<Value, RuntimeError> {
        let params = crate::parser::params_after_self(&decl.params);
        if extra.len() != params.len() {
            return Err(runtime_err(
                format!(
                    "{} expected {} args, got {}",
                    decl.name,
                    params.len(),
                    extra.len()
                ),
                span,
            ));
        }
        self.env.push_scope();
        self.env.define("self", object);
        for (param, arg) in params.iter().zip(extra) {
            self.env.define(&param.name, arg);
        }
        let caller_file = self.current_file.clone();
        let prev_file = std::mem::replace(&mut self.current_file, decl.file.clone());
        let prev_mod = std::mem::replace(&mut self.current_module, decl.module.clone());
        let result = self.exec_block(&decl.body);
        self.current_file = prev_file;
        self.current_module = prev_mod;
        self.env.pop_scope();
        let name = decl.method_trace_name(type_name);
        match result {
            Ok(ControlFlow::Return(v)) => Ok(v),
            Ok(ControlFlow::Normal) => Ok(Value::Void),
            Ok(_) => Err(attach_trace(
                self.stamp(runtime_err("break/continue outside loop", span)),
                &name,
                span,
                &caller_file,
            )),
            Err(e) => Err(attach_trace(self.stamp(e), &name, span, &caller_file)),
        }
    }

    fn lookup_ident(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.env.get_local(name) {
            return Some(v);
        }
        if let Some(v) = self.field_on_self(name) {
            return Some(v);
        }
        self.env.get(name)
    }

    fn field_on_self(&self, name: &str) -> Option<Value> {
        let Value::Struct { fields, .. } = self.env.get("self")? else {
            return None;
        };
        fields.borrow().get(name).cloned()
    }

    fn set_field_on_self(&self, name: &str, value: Value) -> bool {
        let Some(Value::Struct { fields, .. }) = self.env.get("self") else {
            return false;
        };
        let mut map = fields.borrow_mut();
        if !map.contains_key(name) {
            return false;
        }
        map.insert(name.to_string(), value);
        true
    }

    fn call_super(&mut self, name: &str, args: &[Expr], span: Span) -> Result<Value, RuntimeError> {
        let parent = self.super_type.clone().ok_or_else(|| {
            runtime_err(
                "super is only valid in a method of a class that extends another".to_string(),
                span,
            )
        })?;
        let object = self
            .env
            .get("self")
            .ok_or_else(|| runtime_err("super requires self".to_string(), span))?;
        let arg_values: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
        let arg_values = arg_values?;
        match self.call_type_method(&parent, &object, name, arg_values, span)? {
            Some(v) => Ok(v),
            None => Err(runtime_err(
                format!("{parent} has no method '{name}'"),
                span,
            )),
        }
    }

    /// Call a registered function by name.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, RuntimeError> {
        self.call_fn(name, args, Span::default())
    }

    fn eval_import(&mut self, import: &Import, span: Span) -> Result<(), RuntimeError> {
        if import.path.is_empty() {
            return Err(runtime_err("empty import path", span));
        }
        let module_name = &import.path[0];

        // Native host modules (`io`, `time`, `process`, `json`) — require `import`.
        if crate::stdlib::is_host_module(module_name)
            && !crate::stdlib::is_internal_host(module_name)
        {
            self.imported_host.insert(module_name.clone());
            if import.path.len() == 1 {
                // import io; — allow io.read_text etc.
            } else if import.is_from && import.path.len() == 2 {
                let item_name = &import.path[1];
                let alias = import.alias.as_ref().unwrap_or(item_name).clone();
                if let Some(e) = self.enums.get(item_name).cloned() {
                    self.env.define(&alias, Value::EnumType(e));
                } else if let Some(e) = self.enums.get(module_name).cloned() {
                    // from option import Option / from result import Result
                    self.env.define(&alias, Value::EnumType(e));
                } else {
                    self.env.define(
                        &alias,
                        Value::NativeFn {
                            module: module_name.clone(),
                            name: item_name.clone(),
                        },
                    );
                }
            } else if !import.is_from && import.path.len() >= 2 {
                // import option.something — treat like from-import of last segment when stdlib
                let item_name = import.path.last().unwrap();
                let alias = import.alias.as_ref().unwrap_or(item_name).clone();
                if let Some(e) = self.enums.get(item_name).cloned() {
                    self.env.define(&alias, Value::EnumType(e));
                } else {
                    self.env.define(
                        &alias,
                        Value::NativeFn {
                            module: module_name.clone(),
                            name: item_name.clone(),
                        },
                    );
                }
            } else {
                return Err(runtime_err(
                    format!("unsupported stdlib import: {:?}", import.path),
                    span,
                ));
            }
            return Ok(());
        }

        if import.is_from {
            let module = self.load_module(module_name, span)?;
            if import.path.len() == 1 {
                let name = import.alias.as_ref().unwrap_or(module_name);
                self.env.define(name, Value::Module(module));
            } else if import.path.len() == 2 {
                let item_name = &import.path[1];
                let m = module.borrow();
                let alias = import.alias.as_ref().unwrap_or(item_name).clone();
                if let Some(decl) = m.functions.get(item_name) {
                    self.functions.insert(alias.clone(), decl.clone());
                } else if let Some(value) = m.values.get(item_name) {
                    self.env.define(&alias, value.clone());
                } else {
                    return Err(runtime_err(
                        format!("module '{}' has no export '{}'", module_name, item_name),
                        span,
                    ));
                }
            } else {
                return Err(runtime_err(
                    format!(
                        "nested from-imports longer than 2 segments are not supported: {:?}",
                        import.path
                    ),
                    span,
                ));
            }
            return Ok(());
        }

        // import a.b.c; → resolve dotted module path, bind last segment
        let full_name = import.path.join(".");
        let module = self.load_module(&full_name, span)?;
        let bind = import
            .alias
            .as_ref()
            .unwrap_or_else(|| import.path.last().unwrap());
        self.env.define(bind, Value::Module(module));
        Ok(())
    }

    fn load_module(&mut self, name: &str, span: Span) -> Result<ModuleRef, RuntimeError> {
        {
            if let Some(m) = self.loaded_modules.borrow().get(name) {
                return Ok(m.clone());
            }
        }
        let parts = self.module_resolver.borrow().resolve_all(name);
        if parts.is_empty() {
            return Err(runtime_err(
                format!(
                    "module '{}' not found (tried {})",
                    name,
                    crate::interpreter::module_lookup_hint(name)
                ),
                span,
            ));
        }

        let mut module = Module::new();
        let mut module_ctx = EvalContext::unpreloaded(self.module_resolver.clone());
        module_ctx.loaded_modules = self.loaded_modules.clone();

        for (_key, source) in &parts {
            let file = file_label(_key);
            let prev_file = std::mem::replace(&mut module_ctx.current_file, file);
            let prev_mod =
                std::mem::replace(&mut module_ctx.current_module, name.to_string());
            let tokens = crate::lexer::Lexer::new(source)
                .tokenize()
                .map_err(|e| runtime_err(e, span))?;
            let program = crate::parser::Parser::new(tokens)
                .parse()
                .map_err(|e| runtime_err(e, span))?;
            let body = crate::parser::module_items(&program, name);
            let from_mod = crate::parser::module_body_from_mod(&program, name);
            let loaded =
                self.ingest_module_items(&mut module, &mut module_ctx, &body, name, from_mod, span);
            module_ctx.current_file = prev_file;
            module_ctx.current_module = prev_mod;
            loaded?;
        }

        let rc = Rc::new(RefCell::new(module));
        self.loaded_modules
            .borrow_mut()
            .insert(name.to_string(), rc.clone());
        Ok(rc)
    }

    fn import_type_chain(&mut self, src: &EvalContext, name: &str) {
        let mut current = Some(name.to_string());
        let mut seen = HashSet::new();
        let mut exported = true;
        while let Some(ty) = current {
            if !seen.insert(ty.clone()) {
                break;
            }
            if exported {
                if let Some(def) = src.structs.get(&ty) {
                    self.structs.insert(ty.clone(), def.clone());
                }
                exported = false;
            }
            if let Some(parent) = src.parents.get(&ty) {
                self.parents.insert(ty.clone(), parent.clone());
            }
            if let Some(methods) = src.methods.get(&ty) {
                let entry = self.methods.entry(ty.clone()).or_insert_with(HashMap::new);
                for (k, v) in methods {
                    entry.insert(k.clone(), v.clone());
                }
            }
            if let Some(traits) = src.type_traits.get(&ty) {
                for t in traits {
                    self.record_type_trait(&ty, t);
                }
            }
            current = src.parents.get(&ty).cloned();
        }
    }

    fn ingest_module_items(
        &mut self,
        module: &mut Module,
        module_ctx: &mut EvalContext,
        body: &[&Item],
        module_name: &str,
        from_mod: bool,
        span: Span,
    ) -> Result<(), RuntimeError> {
        for item in body {
            let exported = crate::parser::item_is_exported(item, from_mod);
            match item {
                Item::FnDecl(f) => {
                    if module_ctx.functions.contains_key(&f.name) {
                        return Err(runtime_err(
                            format!("duplicate export '{}' in module '{}'", f.name, module_name),
                            span,
                        ));
                    }
                    let stamped = module_ctx.origin_fn(f);
                    module_ctx
                        .functions
                        .insert(f.name.clone(), stamped.clone());
                    if exported {
                        module.functions.insert(f.name.clone(), stamped);
                    }
                }
                Item::StructDecl(s) => {
                    if module_ctx.structs.contains_key(&s.name) {
                        return Err(runtime_err(
                            format!("duplicate export '{}' in module '{}'", s.name, module_name),
                            span,
                        ));
                    }
                    let fields = s.fields.iter().map(|f| f.name.clone()).collect();
                    let def = Rc::new(StructDef {
                        name: s.name.clone(),
                        fields,
                        defaults: Vec::new(),
                    });
                    module_ctx.structs.insert(s.name.clone(), def.clone());
                    if exported {
                        self.structs.insert(s.name.clone(), def.clone());
                        module.values.insert(s.name.clone(), Value::StructType(def));
                    }
                }
                Item::ClassDecl(c) => {
                    if module_ctx.structs.contains_key(&c.name) {
                        return Err(runtime_err(
                            format!("duplicate export '{}' in module '{}'", c.name, module_name),
                            span,
                        ));
                    }
                    module_ctx.register_class(c);
                }
                Item::TraitDecl(t) => {
                    module_ctx.register_trait_decl(t);
                    if exported {
                        self.register_trait_decl(t);
                    }
                }
                Item::EnumDecl(e) => {
                    if module_ctx.enums.contains_key(&e.name) {
                        return Err(runtime_err(
                            format!("duplicate export '{}' in module '{}'", e.name, module_name),
                            span,
                        ));
                    }
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
                    let def = Rc::new(EnumDef {
                        name: e.name.clone(),
                        variants,
                    });
                    module_ctx.enums.insert(e.name.clone(), def.clone());
                    if exported {
                        self.enums.insert(e.name.clone(), def.clone());
                        module.values.insert(e.name.clone(), Value::EnumType(def));
                    }
                }
                Item::ImplDecl {
                    type_name,
                    trait_name,
                    methods,
                    ..
                } => {
                    let stamped: Vec<FnDecl> =
                        methods.iter().map(|m| module_ctx.origin_fn(m)).collect();
                    let entry = module_ctx
                        .methods
                        .entry(type_name.clone())
                        .or_insert_with(HashMap::new);
                    for m in stamped {
                        entry.insert(m.name.clone(), m);
                    }
                    if let Some(t) = trait_name {
                        module_ctx.record_type_trait(type_name, t);
                    }
                }
                _ => {}
            }
        }

        for item in body {
            if let Item::Import(i) = item {
                module_ctx.eval_import(i, span)?;
            }
        }

        module_ctx.apply_inheritance(span)?;
        for item in body {
            if !crate::parser::item_is_exported(item, from_mod) {
                continue;
            }
            if let Item::ClassDecl(c) = item {
                if let Some(def) = module_ctx.structs.get(&c.name).cloned() {
                    module.values.insert(c.name.clone(), Value::StructType(def));
                }
                self.import_type_chain(module_ctx, &c.name);
            }
        }

        for item in body {
            let exported = crate::parser::item_is_exported(item, from_mod);
            match item {
                Item::ConstDecl(c) => {
                    if module_ctx.env.get(&c.name).is_some() && module.values.contains_key(&c.name)
                    {
                        return Err(runtime_err(
                            format!("duplicate export '{}' in module '{}'", c.name, module_name),
                            span,
                        ));
                    }
                    let value = module_ctx.eval_expr(&c.value)?;
                    module_ctx.env.define(&c.name, value.clone());
                    if exported {
                        if module.values.contains_key(&c.name) {
                            return Err(runtime_err(
                                format!(
                                    "duplicate export '{}' in module '{}'",
                                    c.name, module_name
                                ),
                                span,
                            ));
                        }
                        module.values.insert(c.name.clone(), value);
                    }
                }
                Item::VarDecl(v) => {
                    let value = match &v.value {
                        Some(e) => module_ctx.eval_expr(e)?,
                        None => Value::None,
                    };
                    module_ctx.env.define(&v.name, value.clone());
                    if exported {
                        if module.values.contains_key(&v.name) {
                            return Err(runtime_err(
                                format!(
                                    "duplicate export '{}' in module '{}'",
                                    v.name, module_name
                                ),
                                span,
                            ));
                        }
                        module.values.insert(v.name.clone(), value);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn bind_match_payload(
        &mut self,
        enum_name: &str,
        variant: &str,
        binds: &[String],
        field_binds: &[(String, String)],
        inner: &Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let parts: Vec<Value> = match inner {
            Value::Array(a) => a.borrow().clone(),
            other => vec![other.clone()],
        };
        if !field_binds.is_empty() {
            let field_names = self
                .enums
                .get(enum_name)
                .and_then(|e| e.variants.get(variant))
                .map(|d| d.field_names.clone())
                .unwrap_or_default();
            if field_names.iter().all(|n| n.is_empty()) {
                return Err(runtime_err(
                    format!("{enum_name}.{variant} has no named payload fields"),
                    span,
                ));
            }
            for (field, bind) in field_binds {
                if bind == "_" {
                    continue;
                }
                let Some(i) = field_names.iter().position(|n| n == field) else {
                    return Err(runtime_err(
                        format!("{enum_name}.{variant} has no field '{field}'"),
                        span,
                    ));
                };
                let Some(item) = parts.get(i) else {
                    return Err(runtime_err(
                        format!("{enum_name}.{variant} field '{field}' is missing"),
                        span,
                    ));
                };
                self.env.define(bind, item.clone());
            }
            return Ok(());
        }
        match binds.len() {
            0 => {}
            1 => {
                if binds[0] != "_" {
                    self.env.define(&binds[0], inner.clone());
                }
            }
            _ => {
                for (i, b) in binds.iter().enumerate() {
                    if b == "_" {
                        continue;
                    }
                    if let Some(item) = parts.get(i) {
                        self.env.define(b, item.clone());
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn lookup_fn(&self, name: &str) -> Option<FnDecl> {
        for map in self.module_fns.iter().rev() {
            if let Some(decl) = map.get(name) {
                return Some(decl.clone());
            }
        }
        self.functions.get(name).cloned()
    }

    pub(super) fn call_fn(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let decl = self
            .lookup_fn(name)
            .ok_or_else(|| runtime_err(format!("undefined function '{}'", name), span))?;
        self.call_fn_decl(&decl, args, span)
    }

    pub(super) fn call_fn_decl(
        &mut self,
        decl: &FnDecl,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if args.len() != decl.params.len() {
            return Err(runtime_err(
                format!(
                    "{} expected {} args, got {}",
                    decl.name,
                    decl.params.len(),
                    args.len()
                ),
                span,
            ));
        }
        self.env.push_scope();
        for (param, arg) in decl.params.iter().zip(args) {
            self.env.define(&param.name, arg);
        }
        let caller_file = self.current_file.clone();
        let prev_file = std::mem::replace(&mut self.current_file, decl.file.clone());
        let prev_mod = std::mem::replace(&mut self.current_module, decl.module.clone());
        let result = self.exec_block(&decl.body);
        self.current_file = prev_file;
        self.current_module = prev_mod;
        self.env.pop_scope();
        match result {
            Ok(ControlFlow::Return(v)) => Ok(v),
            Ok(ControlFlow::Normal) => Ok(Value::Void),
            Ok(_) => Err(attach_trace(
                self.stamp(runtime_err("break/continue outside loop", span)),
                &decl.trace_name(),
                span,
                &caller_file,
            )),
            Err(e) => Err(attach_trace(
                self.stamp(e),
                &decl.trace_name(),
                span,
                &caller_file,
            )),
        }
    }

    fn exec_block(&mut self, block: &Block) -> Result<ControlFlow, RuntimeError> {
        for stmt in &block.stmts {
            match self.exec_stmt(stmt)? {
                ControlFlow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(ControlFlow::Normal)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<ControlFlow, RuntimeError> {
        self.exec_stmt_inner(stmt).map_err(|e| self.stamp(e))
    }

    fn exec_stmt_inner(&mut self, stmt: &Stmt) -> Result<ControlFlow, RuntimeError> {
        let span = stmt.span;
        match &stmt.kind {
            StmtKind::Expr(e) => {
                let _ = self.eval_expr(e)?;
                if let Some(v) = self.pending_return.take() {
                    return Ok(ControlFlow::Return(v));
                }
                Ok(ControlFlow::Normal)
            }
            StmtKind::Return(e) => {
                let value = match e {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Void,
                };
                Ok(ControlFlow::Return(value))
            }
            StmtKind::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                if self.eval_expr(cond)?.truthy() {
                    return self.exec_block(then_block);
                }
                for (elif_cond, elif_block) in elif_blocks {
                    if self.eval_expr(elif_cond)?.truthy() {
                        return self.exec_block(elif_block);
                    }
                }
                if let Some(else_block) = else_block {
                    self.exec_block(else_block)
                } else {
                    Ok(ControlFlow::Normal)
                }
            }
            StmtKind::While { cond, body } => {
                loop {
                    if !self.eval_expr(cond)?.truthy() {
                        break;
                    }
                    match self.exec_block(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal => {}
                    }
                }
                Ok(ControlFlow::Normal)
            }
            StmtKind::For { name, iter, body } => {
                let iter_value = self.eval_expr(iter)?;
                let items = match &iter_value {
                    Value::String(s) => s
                        .chars()
                        .map(|c| Value::String(c.to_string()))
                        .collect::<Vec<_>>(),
                    Value::Array(a) => a.borrow().iter().cloned().collect::<Vec<_>>(),
                    Value::Map(m) => m
                        .borrow()
                        .keys()
                        .map(|k| Value::String(k.clone()))
                        .collect::<Vec<_>>(),
                    Value::Int(n) => (0..*n).map(Value::Int).collect::<Vec<_>>(),
                    Value::Range(start, end, inclusive) => {
                        if *inclusive {
                            (*start..=*end).map(Value::Int).collect::<Vec<_>>()
                        } else {
                            (*start..*end).map(Value::Int).collect::<Vec<_>>()
                        }
                    }
                    _ => {
                        return Err(runtime_err(
                            format!("cannot iterate over {}", iter_value.type_name()),
                            span,
                        ));
                    }
                };
                for item in items {
                    self.env.push_scope();
                    self.env.define(name, item);
                    match self.exec_block(body)? {
                        ControlFlow::Break => {
                            self.env.pop_scope();
                            break;
                        }
                        ControlFlow::Continue => {
                            self.env.pop_scope();
                            continue;
                        }
                        ControlFlow::Return(v) => {
                            self.env.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Normal => {}
                    }
                    self.env.pop_scope();
                }
                Ok(ControlFlow::Normal)
            }
            StmtKind::VarDecl(v) => {
                let value = match &v.value {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::None,
                };
                self.env.define(&v.name, value);
                Ok(ControlFlow::Normal)
            }
            StmtKind::ConstDecl(c) => {
                let value = self.eval_expr(&c.value)?;
                self.env.define(&c.name, value);
                Ok(ControlFlow::Normal)
            }
            StmtKind::Break => Ok(ControlFlow::Break),
            StmtKind::Continue => Ok(ControlFlow::Continue),
            StmtKind::Pass => Ok(ControlFlow::Normal),
            StmtKind::Comment(_) => Ok(ControlFlow::Normal),
        }
    }

    pub(crate) fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        self.eval_expr_inner(expr).map_err(|e| self.stamp(e))
    }

    fn eval_expr_inner(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Literal(l) => match l {
                Literal::Int(n) => Ok(Value::Int(*n)),
                Literal::Float(n) => Ok(Value::Float(*n)),
                Literal::String(s) => Ok(Value::String(s.clone())),
                Literal::Bool(b) => Ok(Value::Bool(*b)),
                Literal::None => Ok(Value::None),
            },
            ExprKind::FString(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        FStringPart::Text(t) => result.push_str(t),
                        FStringPart::Expr { expr, format } => {
                            let value = self.eval_expr(expr)?;
                            result.push_str(&format_value(&value, format.as_deref(), span)?);
                        }
                    }
                }
                Ok(Value::String(result))
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let s = match self.eval_expr(start)? {
                    Value::Int(n) => n,
                    Value::Float(n) => n as i64,
                    v => {
                        return Err(runtime_err(
                            format!("range start must be Int, got {}", v.type_name()),
                            span,
                        ));
                    }
                };
                let e = match self.eval_expr(end)? {
                    Value::Int(n) => n,
                    Value::Float(n) => n as i64,
                    v => {
                        return Err(runtime_err(
                            format!("range end must be Int, got {}", v.type_name()),
                            span,
                        ));
                    }
                };
                Ok(Value::Range(s, e, *inclusive))
            }
            ExprKind::Ident(name) => {
                if let Some(v) = self.lookup_ident(name) {
                    return Ok(v);
                }
                if let Some(s) = self.structs.get(name) {
                    return Ok(Value::StructType(s.clone()));
                }
                if let Some(e) = self.enums.get(name) {
                    return Ok(Value::EnumType(e.clone()));
                }
                if self.lookup_fn(name).is_some() {
                    return Ok(Value::FnRef { name: name.clone() });
                }
                Err(runtime_err(format!("undefined variable '{}'", name), span))
            }
            ExprKind::Unary { op, expr } => {
                let value = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match value {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(n) => Ok(Value::Float(-n)),
                        _ => Err(runtime_err(
                            format!("cannot negate {}", value.type_name()),
                            span,
                        )),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!value.truthy())),
                    UnaryOp::BitNot => match value {
                        Value::Int(n) => Ok(Value::Int(!n)),
                        _ => Err(runtime_err(
                            format!("bitwise not requires Int, got {}", value.type_name()),
                            span,
                        )),
                    },
                }
            }
            ExprKind::Binary { op, left, right } => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    let l = self.eval_expr(left)?;
                    return match op {
                        BinOp::And => {
                            if !l.truthy() {
                                Ok(Value::Bool(false))
                            } else {
                                Ok(Value::Bool(self.eval_expr(right)?.truthy()))
                            }
                        }
                        BinOp::Or => {
                            if l.truthy() {
                                Ok(Value::Bool(true))
                            } else {
                                Ok(Value::Bool(self.eval_expr(right)?.truthy()))
                            }
                        }
                        _ => unreachable!(),
                    };
                }
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                match op {
                    BinOp::Add => match (&l, &r) {
                        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                        (Value::String(a), Value::String(b)) => {
                            Ok(Value::String(format!("{}{}", a, b)))
                        }
                        _ => Err(runtime_err(
                            format!("cannot add {} and {}", l.type_name(), r.type_name()),
                            span,
                        )),
                    },
                    BinOp::Sub => numeric_binop(l, r, |a, b| a - b, span),
                    BinOp::Mul => numeric_binop(l, r, |a, b| a * b, span),
                    BinOp::Div => numeric_binop(l, r, |a, b| a / b, span),
                    BinOp::IDiv => match (&l, &r) {
                        (Value::Int(a), Value::Int(b)) => int_div(*a, *b, span),
                        _ => Err(runtime_err(
                            format!(
                                "integer division requires Int, got {} and {}",
                                l.type_name(),
                                r.type_name()
                            ),
                            span,
                        )),
                    },
                    BinOp::Mod => checked_mod(l, r, span),
                    BinOp::Eq => Ok(Value::Bool(value_eq(&l, &r))),
                    BinOp::Neq => Ok(Value::Bool(!value_eq(&l, &r))),
                    BinOp::Lt => compare_op(l, r, |a, b| a < b, span),
                    BinOp::Lte => compare_op(l, r, |a, b| a <= b, span),
                    BinOp::Gt => compare_op(l, r, |a, b| a > b, span),
                    BinOp::Gte => compare_op(l, r, |a, b| a >= b, span),
                    BinOp::And | BinOp::Or => unreachable!("short-circuit ops handled above"),
                    BinOp::BitAnd => int_bitwise(l, r, |a, b| a & b, span),
                    BinOp::BitOr => int_bitwise(l, r, |a, b| a | b, span),
                    BinOp::BitXor => int_bitwise(l, r, |a, b| a ^ b, span),
                    BinOp::Shl => int_shift(l, r, true, span),
                    BinOp::Shr => int_shift(l, r, false, span),
                }
            }
            ExprKind::Call { callee, args } => {
                if let ExprKind::Member { object, name } = &callee.kind {
                    if let ExprKind::Ident(id) = &object.kind {
                        if id == "super" {
                            return self.call_super(name, args, span);
                        }
                    }
                }
                let arg_values: Result<Vec<_>, _> =
                    args.iter().map(|a| self.eval_expr(a)).collect();
                let arg_values = arg_values?;
                match &callee.kind {
                    ExprKind::Ident(name) => self.call_builtin_or_fn(name, arg_values, span),
                    ExprKind::Member { object, name } => {
                        // Qualified stdlib call: Module.name(args) — but prefer a bound Module value
                        // so `import util.math` (bound as `math`) is not shadowed by the math stdlib.
                        if let ExprKind::Ident(module) = &object.kind {
                            let bound_module =
                                matches!(self.env.get(module), Some(Value::Module(_)));
                            if !bound_module && self.stdlib.contains_key(module) {
                                if crate::stdlib::is_internal_host(module)
                                    || crate::stdlib::is_language_module(module)
                                    || self.imported_host.contains(module)
                                {
                                    return self.call_qualified(module, name, arg_values, span);
                                }
                            }
                        }
                        let obj = self.eval_expr(object)?;
                        self.call_member(&obj, name, arg_values, span)
                    }
                    _ => Err(runtime_err("invalid callee", span)),
                }
            }
            ExprKind::Member { object, name } => {
                let obj = self.eval_expr(object)?;
                match obj {
                    Value::String(s) => match name.as_str() {
                        "len" => Ok(Value::Int(string_char_len(&s))),
                        _ => Err(runtime_err(
                            format!("String has no member '{}'", name),
                            span,
                        )),
                    },
                    Value::Array(a) => match name.as_str() {
                        "len" => Ok(Value::Int(a.borrow().len() as i64)),
                        _ => Err(runtime_err(format!("Array has no member '{}'", name), span)),
                    },
                    Value::Map(m) => match name.as_str() {
                        "len" => Ok(Value::Int(m.borrow().len() as i64)),
                        _ => Err(runtime_err(format!("Map has no member '{}'", name), span)),
                    },
                    Value::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        if let Some(v) = fields.borrow().get(name) {
                            return Ok(v.clone());
                        }
                        Err(runtime_err(
                            format!("struct {} has no field '{}'", struct_name, name),
                            span,
                        ))
                    }
                    Value::EnumType(e) => {
                        if let Some(v) = e.variants.get(name) {
                            if v.arity == 0 {
                                return Ok(Value::Enum {
                                    module: e.name.clone(),
                                    variant: name.clone(),
                                    value: None,
                                });
                            }
                            return Err(runtime_err(
                                format!(
                                    "{}.{} is a constructor and must be called with an argument",
                                    e.name, name
                                ),
                                span,
                            ));
                        }
                        Err(runtime_err(
                            format!("enum {} has no variant '{}'", e.name, name),
                            span,
                        ))
                    }
                    Value::Module(m) => {
                        let m = m.borrow();
                        if m.functions.contains_key(name) {
                            return Err(runtime_err(
                                format!("function '{}' must be called with arguments", name),
                                span,
                            ));
                        }
                        if let Some(value) = m.values.get(name) {
                            return Ok(value.clone());
                        }
                        Err(runtime_err(
                            format!("module has no member '{}'", name),
                            span,
                        ))
                    }
                    _ => Err(runtime_err(
                        format!("type {} has no member '{}'", obj.type_name(), name),
                        span,
                    )),
                }
            }
            ExprKind::Index { object, index } => {
                let obj = self.eval_expr(object)?;
                let idx = self.eval_expr(index)?;
                match (&obj, &idx) {
                    (Value::Array(a), Value::Int(n)) => {
                        let i = *n as usize;
                        a.borrow()
                            .get(i)
                            .cloned()
                            .ok_or_else(|| runtime_err(format!("index {} out of bounds", i), span))
                    }
                    (Value::String(s), Value::Int(n)) => {
                        let i = *n as usize;
                        s.chars()
                            .nth(i)
                            .map(|c| Value::String(c.to_string()))
                            .ok_or_else(|| runtime_err(format!("index {} out of bounds", i), span))
                    }
                    (Value::Map(m), Value::String(k)) => m
                        .borrow()
                        .get(k)
                        .cloned()
                        .ok_or_else(|| runtime_err(format!("key '{}' not found", k), span)),
                    _ => Err(runtime_err(
                        format!("cannot index {} with {}", obj.type_name(), idx.type_name()),
                        span,
                    )),
                }
            }
            ExprKind::Match { expr, arms } => {
                let value = self.eval_expr(expr)?;
                for arm in arms {
                    let matched = match (&arm.pattern, &value) {
                        (Pattern::Wildcard, _) => true,
                        (
                            Pattern::Variant {
                                name,
                                binds: binding,
                                field_binds,
                            },
                            Value::Enum {
                                module: enum_name,
                                variant,
                                value: inner,
                            },
                        ) => {
                            if name == variant {
                                self.env.push_scope();
                                if let Some(v) = inner {
                                    self.bind_match_payload(
                                        enum_name,
                                        variant,
                                        binding,
                                        field_binds,
                                        v,
                                        span,
                                    )?;
                                }
                                let result = self.exec_block(&arm.body)?;
                                self.env.pop_scope();
                                match result {
                                    ControlFlow::Return(v) => {
                                        self.pending_return = Some(v.clone());
                                        return Ok(v);
                                    }
                                    ControlFlow::Normal => return Ok(Value::Void),
                                    ControlFlow::Break => {
                                        return Err(runtime_err("break outside loop", span));
                                    }
                                    ControlFlow::Continue => {
                                        return Err(runtime_err("continue outside loop", span));
                                    }
                                }
                            } else {
                                false
                            }
                        }
                        (Pattern::Variant { name, .. }, _) => {
                            return Err(runtime_err(
                                format!(
                                    "match pattern '{}' does not match value of type {}",
                                    name,
                                    value.type_name()
                                ),
                                span,
                            ));
                        }
                    };
                    if matched {
                        let result = self.exec_block(&arm.body)?;
                        match result {
                            ControlFlow::Return(v) => {
                                self.pending_return = Some(v.clone());
                                return Ok(v);
                            }
                            ControlFlow::Normal => return Ok(Value::Void),
                            ControlFlow::Break => {
                                return Err(runtime_err("break outside loop", span));
                            }
                            ControlFlow::Continue => {
                                return Err(runtime_err("continue outside loop", span));
                            }
                        }
                    }
                }
                Ok(Value::Void)
            }
            ExprKind::StructLiteral { name, fields } => {
                let def = self
                    .structs
                    .get(name)
                    .ok_or_else(|| runtime_err(format!("undefined struct '{}'", name), span))?;
                let field_names = def.fields.clone();
                let defaults = def.defaults.clone();
                let mut values = HashMap::new();
                for (field_name, expr) in fields {
                    if !field_names.iter().any(|n| n == field_name) {
                        return Err(runtime_err(
                            format!("unknown field '{field_name}' on {name}"),
                            span,
                        ));
                    }
                    values.insert(field_name.clone(), self.eval_expr(expr)?);
                }
                for field in field_names {
                    if values.contains_key(&field) {
                        continue;
                    }
                    if let Some((_, expr)) = defaults.iter().find(|(n, _)| n == &field) {
                        values.insert(field, self.eval_expr(expr)?);
                    } else {
                        values.insert(field, Value::None);
                    }
                }
                Ok(Value::Struct {
                    name: name.clone(),
                    fields: Rc::new(RefCell::new(values)),
                })
            }
            ExprKind::Assign { op, left, right } => {
                let value = self.eval_expr(right)?;
                match &left.kind {
                    ExprKind::Ident(name) => {
                        let old = || self.lookup_ident(name);
                        let new_value = if *op == AssignOp::Assign {
                            value.clone()
                        } else {
                            let old = old().ok_or_else(|| {
                                runtime_err(format!("undefined variable '{}'", name), span)
                            })?;
                            apply_assign_op(old, &value, op, span)?
                        };
                        if self.env.set_local(name, new_value.clone()) {
                            return Ok(value);
                        }
                        if self.set_field_on_self(name, new_value.clone()) {
                            return Ok(value);
                        }
                        self.env.set(name, new_value, span)?;
                        Ok(value)
                    }
                    ExprKind::Index { object, index } => {
                        let obj = self.eval_expr(object)?;
                        let idx = self.eval_expr(index)?;
                        match obj {
                            Value::Array(a) => {
                                let i = match idx {
                                    Value::Int(n) => n as usize,
                                    _ => {
                                        return Err(runtime_err(
                                            "array index must be Int".to_string(),
                                            span,
                                        ));
                                    }
                                };
                                let new_value = if *op == AssignOp::Assign {
                                    value.clone()
                                } else {
                                    let old = a
                                        .borrow()
                                        .get(i)
                                        .ok_or_else(|| {
                                            runtime_err("index out of bounds".to_string(), span)
                                        })?
                                        .clone();
                                    apply_assign_op(old, &value, op, span)?
                                };
                                if i < a.borrow().len() {
                                    a.borrow_mut()[i] = new_value;
                                } else {
                                    return Err(runtime_err(
                                        "index out of bounds".to_string(),
                                        span,
                                    ));
                                }
                                Ok(value)
                            }
                            Value::Map(m) => {
                                let k = match idx {
                                    Value::String(s) => s,
                                    _ => {
                                        return Err(runtime_err(
                                            "map key must be String".to_string(),
                                            span,
                                        ));
                                    }
                                };
                                let new_value = if *op == AssignOp::Assign {
                                    value.clone()
                                } else {
                                    let old = m.borrow().get(&k).cloned().unwrap_or(Value::None);
                                    apply_assign_op(old, &value, op, span)?
                                };
                                m.borrow_mut().insert(k, new_value);
                                Ok(value)
                            }
                            _ => Err(runtime_err(
                                format!("cannot assign to {}", obj.type_name()),
                                span,
                            )),
                        }
                    }
                    ExprKind::Member { object, name } => {
                        let obj = self.eval_expr(object)?;
                        match obj {
                            Value::Struct { fields, .. } => {
                                let new_value = if *op == AssignOp::Assign {
                                    value.clone()
                                } else {
                                    let old =
                                        fields.borrow().get(name).cloned().unwrap_or(Value::None);
                                    apply_assign_op(old, &value, op, span)?
                                };
                                fields.borrow_mut().insert(name.clone(), new_value);
                                Ok(value)
                            }
                            _ => Err(runtime_err(
                                format!("cannot assign field on {}", obj.type_name()),
                                span,
                            )),
                        }
                    }
                    _ => Err(runtime_err("invalid assignment target".to_string(), span)),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ControlFlow {
    Normal,
    Return(Value),
    Break,
    Continue,
}
