//! [`TypeChecker`]: walk the AST and collect [`Diagnostic`]s.

use std::collections::{HashMap, HashSet};

use crate::interpreter::ModuleResolver;
use crate::parser::*;
use crate::{Diagnostic, Span};

use super::helpers::*;

#[derive(Clone)]
struct FnSig {
    param_count: Option<usize>,
    /// Types of call arguments (excludes `self`).
    arg_types: Vec<String>,
    return_type: Option<Type>,
    is_ufcs: bool,
    takes_self: bool,
    is_async: bool,
}

impl FnSig {
    fn from_fn(f: &FnDecl) -> Self {
        let takes_self = f.params.first().is_some_and(|p| p.name == "self");
        let skip = if takes_self { 1 } else { 0 };
        Self {
            param_count: Some(f.params.len()),
            arg_types: f
                .params
                .iter()
                .skip(skip)
                .map(|p| p.ty.name.clone())
                .collect(),
            return_type: f.return_type.clone(),
            is_ufcs: f.is_ufcs,
            takes_self,
            is_async: f.is_async,
        }
    }

    /// Instance method: `self` is always the receiver, even when omitted in source.
    fn from_method(f: &FnDecl) -> Self {
        let extras = params_after_self(&f.params);
        Self {
            param_count: Some(extras.len() + 1),
            arg_types: extras.iter().map(|p| p.ty.name.clone()).collect(),
            return_type: f.return_type.clone(),
            is_ufcs: f.is_ufcs,
            takes_self: true,
            is_async: f.is_async,
        }
    }

    fn from_trait_method(m: &TraitMethod) -> Self {
        let extras = params_after_self(&m.params);
        Self {
            param_count: Some(extras.len() + 1),
            arg_types: extras.iter().map(|p| p.ty.name.clone()).collect(),
            return_type: m.return_type.clone(),
            is_ufcs: false,
            takes_self: true,
            is_async: m.is_async,
        }
    }

    fn loose(param_count: Option<usize>) -> Self {
        Self {
            param_count,
            arg_types: Vec::new(),
            return_type: None,
            is_ufcs: false,
            takes_self: false,
            is_async: false,
        }
    }

    fn inner_return(&self) -> Option<String> {
        self.return_type.as_ref().map(|t| t.name.clone())
    }

    fn call_return(&self) -> Option<String> {
        if self.is_async {
            Some("Task".to_string())
        } else {
            self.inner_return()
        }
    }

    fn call_arity(&self) -> Option<usize> {
        self.param_count.map(|n| {
            if self.takes_self || self.is_ufcs {
                n.saturating_sub(1)
            } else {
                n
            }
        })
    }
}

struct TypeChecker<'a> {
    functions: HashMap<String, FnSig>,
    structs: HashMap<String, ()>,
    builtins: HashMap<&'static str, Option<usize>>,
    diagnostics: Vec<Diagnostic>,
    resolver: Option<&'a dyn ModuleResolver>,
    /// Canonical module name → exported function signatures.
    user_modules: HashMap<String, HashMap<String, FnSig>>,
    /// Import bind name (`utils`, `m`) → canonical module name.
    module_aliases: HashMap<String, String>,
    /// Enum name → variant → payload arity (from embedded + user modules).
    enums: HashMap<String, HashMap<String, usize>>,
    /// Trait name → method signature (arity includes self).
    traits: HashMap<String, HashMap<String, FnSig>>,
    /// Trait name → signal contract (name + payload arity).
    trait_signals: HashMap<String, Vec<(String, usize, Span)>>,
    /// Type name → inherent + trait-impl methods.
    type_methods: HashMap<String, HashMap<String, FnSig>>,
    /// Type name → field name → type.
    class_fields: HashMap<String, HashMap<String, String>>,
    /// Child type → parent type.
    parents: HashMap<String, String>,
    class_spans: HashMap<String, Span>,
    /// Parent of the method currently being checked (`super.method`).
    current_super: Option<String>,
    /// Return type name of the function / lambda currently being checked.
    current_return: Option<String>,
    /// Module-scope `signal` name → payload arity.
    signals: HashMap<String, usize>,
    loading: HashSet<String>,
    /// Error on missing user modules. False for `compile_source` (stdlib still loads).
    strict_imports: bool,
    /// Function / block bindings (type names).
    scopes: Vec<HashMap<String, String>>,
    /// Module-scope `var` / `const` types.
    globals: HashMap<String, String>,
    /// Diagnostic file label while checking an imported module (`util.rg`).
    current_file: String,
    /// Parsed user modules waiting for body checks (not host / crate stdlib).
    pending_bodies: HashMap<String, Vec<(String, Vec<Item>)>>,
}

impl<'a> TypeChecker<'a> {
    fn new(resolver: Option<&'a dyn ModuleResolver>, strict_imports: bool) -> Self {
        let mut builtins = HashMap::new();
        // None = any arity
        builtins.insert("print", None);
        builtins.insert("len", Some(1));
        builtins.insert("assert", Some(1));
        builtins.insert("Array", None);
        builtins.insert("Map", None);
        builtins.insert("Mutex", Some(0));
        builtins.insert("Channel", Some(0));
        Self {
            functions: HashMap::new(),
            structs: HashMap::new(),
            builtins,
            diagnostics: Vec::new(),
            resolver,
            user_modules: HashMap::new(),
            module_aliases: HashMap::new(),
            enums: HashMap::new(),
            traits: HashMap::new(),
            trait_signals: HashMap::new(),
            type_methods: HashMap::new(),
            class_fields: HashMap::new(),
            parents: HashMap::new(),
            class_spans: HashMap::new(),
            current_super: None,
            current_return: None,
            signals: HashMap::new(),
            loading: HashSet::new(),
            strict_imports,
            scopes: Vec::new(),
            globals: HashMap::new(),
            current_file: String::new(),
            pending_bodies: HashMap::new(),
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(&self.current_file, span, message));
    }

    fn bind(&mut self, name: String, ty: String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        } else {
            self.globals.insert(name, ty);
        }
    }

    fn lookup_binding(&self, name: &str) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        if name != "self" {
            if let Some(recv) = self.receiver_type() {
                if let Some(ft) = self.lookup_field(&recv, name) {
                    return Some(ft);
                }
            }
        }
        self.globals.get(name).cloned()
    }

    fn register_methods(&mut self, type_name: &str, methods: &[FnDecl]) {
        let entry = self.type_methods.entry(type_name.to_string()).or_default();
        for m in methods {
            entry.insert(m.name.clone(), FnSig::from_method(m));
        }
    }

    fn register_fields(&mut self, type_name: &str, fields: &[FieldDecl]) {
        let entry = self.class_fields.entry(type_name.to_string()).or_default();
        for f in fields {
            entry.insert(f.name.clone(), f.ty.name.clone());
        }
    }

    fn lookup_field(&self, type_name: &str, name: &str) -> Option<String> {
        let mut current = Some(type_name.to_string());
        let mut seen = HashSet::new();
        while let Some(ty) = current {
            if !seen.insert(ty.clone()) {
                break;
            }
            if let Some(ft) = self.class_fields.get(&ty).and_then(|f| f.get(name)) {
                return Some(ft.clone());
            }
            current = self.parents.get(&ty).cloned();
        }
        None
    }

    fn receiver_type(&self) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get("self") {
                return if ty.is_empty() {
                    None
                } else {
                    Some(ty.clone())
                };
            }
        }
        None
    }

    fn register_type_name(&mut self, name: &str) {
        if self.structs.insert(name.to_string(), ()).is_some() {
            self.error(Span::default(), format!("duplicate type '{name}'"));
        }
    }

    fn register_item_types(&mut self, item: &Item) {
        match item {
            Item::FnDecl(f) => {
                self.functions.insert(f.name.clone(), FnSig::from_fn(f));
            }
            Item::ImplDecl {
                type_name, methods, ..
            } => {
                self.register_methods(type_name, methods);
            }
            Item::StructDecl(s) => {
                self.register_type_name(&s.name);
                self.register_fields(&s.name, &s.fields);
            }
            Item::ClassDecl(c) => {
                self.register_type_name(&c.name);
                self.register_fields(&c.name, &c.fields);
                self.register_methods(&c.name, &c.methods);
                for b in &c.trait_impls {
                    self.register_methods(&c.name, &b.methods);
                }
                self.class_spans.insert(c.name.clone(), c.span);
                if let Some(p) = &c.parent {
                    self.parents.insert(c.name.clone(), p.clone());
                }
            }
            Item::TraitDecl(t) => self.register_trait(t, true),
            Item::EnumDecl(e) => {
                let mut variants = HashMap::new();
                for v in &e.variants {
                    variants.insert(v.name.clone(), v.value_types.len());
                }
                self.enums.insert(e.name.clone(), variants);
            }
            Item::VarDecl(v) => {
                if let Some(ty) = self.binding_type_for_var(v) {
                    self.globals.insert(v.name.clone(), ty);
                }
            }
            Item::ConstDecl(c) => {
                if let Some(ty) = c.ty.as_ref().filter(|t| !Self::type_is_skipped(t)) {
                    self.globals.insert(c.name.clone(), ty.name.clone());
                } else if let Some(ty) = self.infer_expr_type(&c.value) {
                    self.globals.insert(c.name.clone(), ty);
                }
            }
            Item::Mod(m) => {
                for inner in &m.items {
                    self.register_item_types(inner);
                }
            }
            _ => {}
        }
    }

    fn binding_type_for_var(&self, v: &VarDecl) -> Option<String> {
        if !Self::type_is_skipped(&v.ty) {
            return Some(v.ty.name.clone());
        }
        v.value.as_ref().and_then(|e| self.infer_expr_type(e))
    }

    fn preload_crate_types(&mut self) {
        if !self.structs.contains_key("Vec2") || !self.structs.contains_key("Vec3") {
            self.try_load_user_module("vec", Span::default());
        }
        if !self.enums.contains_key("Option") {
            self.try_load_user_module("option", Span::default());
        }
        if !self.enums.contains_key("Result") {
            self.try_load_user_module("result", Span::default());
        }
    }

    fn check_program(&mut self, program: &[Item]) {
        for item in program {
            match item {
                Item::Import(i) => self.register_import(i),
                Item::SignalDecl(s) => {
                    if self
                        .signals
                        .insert(s.name.clone(), s.params.len())
                        .is_some()
                    {
                        self.error(s.span, format!("duplicate signal '{}'", s.name));
                    }
                }
                Item::Mod(m) => {
                    self.module_aliases.insert(m.name.clone(), m.name.clone());
                    let mut dups = Vec::new();
                    {
                        let entry = self.user_modules.entry(m.name.clone()).or_default();
                        for inner in &m.items {
                            if let Item::FnDecl(f) = inner {
                                if !f.is_pub {
                                    continue;
                                }
                                if entry.insert(f.name.clone(), FnSig::from_fn(f)).is_some() {
                                    dups.push(f.name.clone());
                                }
                            }
                        }
                    }
                    for fn_name in dups {
                        self.error(
                            m.span,
                            format!("duplicate export '{fn_name}' in module '{}'", m.name),
                        );
                    }
                    self.register_item_types(item);
                }
                _ => self.register_item_types(item),
            }
        }

        self.preload_crate_types();
        self.check_inheritance();

        for item in program {
            self.check_trait_impl(item);
            if let Item::Mod(m) = item {
                for inner in &m.items {
                    self.check_trait_impl(inner);
                }
            }
        }

        for item in program {
            self.check_item(item);
        }

        self.check_imported_module_bodies();
    }

    fn check_inheritance(&mut self) {
        let pairs: Vec<(String, String)> = self
            .parents
            .iter()
            .map(|(c, p)| (c.clone(), p.clone()))
            .collect();
        for (child, parent) in pairs {
            let span = self.class_spans.get(&child).copied().unwrap_or_default();
            if self.enums.contains_key(&parent) {
                self.error(
                    span,
                    format!("class '{child}' cannot extend enum '{parent}'"),
                );
                continue;
            }
            if !self.structs.contains_key(&parent) {
                if self.class_fields.contains_key(&parent)
                    || self.type_methods.contains_key(&parent)
                {
                    continue;
                }
                self.error(
                    span,
                    format!("class '{child}' extends unknown type '{parent}'"),
                );
                continue;
            }
            let mut seen = HashSet::new();
            let mut current = Some(parent);
            while let Some(ty) = current {
                if !seen.insert(ty.clone()) {
                    self.error(span, format!("cycle in class inheritance at '{child}'"));
                    break;
                }
                if ty == child {
                    self.error(span, format!("cycle in class inheritance at '{child}'"));
                    break;
                }
                current = self.parents.get(&ty).cloned();
            }
        }
    }

    fn lookup_type_method(&self, type_name: &str, name: &str) -> Option<FnSig> {
        let mut current = Some(type_name.to_string());
        let mut seen = HashSet::new();
        while let Some(ty) = current {
            if !seen.insert(ty.clone()) {
                break;
            }
            if let Some(sig) = self.type_methods.get(&ty).and_then(|m| m.get(name)) {
                return Some(sig.clone());
            }
            current = self.parents.get(&ty).cloned();
        }
        None
    }

    fn register_trait(&mut self, t: &TraitDecl, warn_dup: bool) {
        if warn_dup && self.traits.contains_key(&t.name) {
            self.error(Span::default(), format!("duplicate trait '{}'", t.name));
        }
        let mut methods = HashMap::new();
        for m in &t.methods {
            methods.insert(m.name.clone(), FnSig::from_trait_method(m));
        }
        let mut sigs = Vec::new();
        let mut seen = HashSet::new();
        for s in &t.signals {
            if methods.contains_key(&s.name) {
                self.error(
                    s.span,
                    format!(
                        "trait '{}' cannot have both a method and signal '{}'",
                        t.name, s.name
                    ),
                );
            }
            if !seen.insert(s.name.clone()) {
                self.error(
                    s.span,
                    format!("duplicate signal '{}' in trait '{}'", s.name, t.name),
                );
            }
            sigs.push((s.name.clone(), s.params.len(), s.span));
        }
        self.traits.insert(t.name.clone(), methods);
        self.trait_signals.insert(t.name.clone(), sigs);
    }

    fn bind_trait_signals(&mut self, trait_name: &str) {
        let Some(sigs) = self.trait_signals.get(trait_name).cloned() else {
            return;
        };
        for (name, arity, sig_span) in sigs {
            if let Some(&existing) = self.signals.get(&name) {
                if existing != arity {
                    self.error(
            sig_span,
            format!(
              "signal '{name}' from trait '{trait_name}' expected {arity} args, already declared with {existing}"
            ),
          );
                }
                continue;
            }
            self.signals.insert(name, arity);
        }
    }

    fn check_trait_impl(&mut self, item: &Item) {
        match item {
            Item::ImplDecl {
                type_name,
                trait_name: Some(trait_name),
                methods,
                span,
                ..
            } => {
                self.bind_trait_signals(trait_name);
                self.check_trait_methods(type_name, trait_name, methods, *span);
            }
            Item::ClassDecl(c) => {
                for trait_name in c.implemented_traits() {
                    self.bind_trait_signals(&trait_name);
                    self.check_type_implements_trait(&c.name, &trait_name, c.span);
                }
            }
            _ => {}
        }
    }

    fn check_trait_methods(
        &mut self,
        type_name: &str,
        trait_name: &str,
        methods: &[FnDecl],
        span: Span,
    ) {
        match self.traits.get(trait_name).cloned() {
            Some(required) => {
                for (method, required_sig) in required {
                    match methods.iter().find(|m| m.name == method) {
                        None => self.error(
                            span,
                            format!("impl {trait_name} for {type_name} is missing '{method}'"),
                        ),
                        Some(provided) => {
                            if required_sig.is_async != provided.is_async {
                                self.error(
                                    span,
                                    format!(
                                        "{trait_name}.{method} expected {}fn",
                                        if required_sig.is_async { "async " } else { "" }
                                    ),
                                );
                            }
                            if let Some(expected) = required_sig.param_count {
                                if provided.params.len() != expected {
                                    self.error(
                    span,
                    format!(
                      "{trait_name}.{method} expected {expected} params, got {}",
                      provided.params.len()
                    ),
                  );
                                }
                            }
                            if let (Some(rt), Some(pt)) =
                                (&required_sig.return_type, &provided.return_type)
                            {
                                if !Self::type_is_skipped(rt)
                                    && !Self::type_is_skipped(pt)
                                    && !Self::types_compatible(&rt.name, &pt.name)
                                {
                                    self.error(
                                        span,
                                        format!(
                                            "{trait_name}.{method} expected return {}, got {}",
                                            rt.name, pt.name
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            None => self.error(span, format!("unknown trait '{trait_name}'")),
        }
    }

    fn check_type_implements_trait(&mut self, type_name: &str, trait_name: &str, span: Span) {
        match self.traits.get(trait_name).cloned() {
            Some(required) => {
                for (method, required_sig) in required {
                    match self.lookup_type_method(type_name, &method) {
                        None => self.error(
                            span,
                            format!("impl {trait_name} for {type_name} is missing '{method}'"),
                        ),
                        Some(provided) => {
                            if required_sig.is_async != provided.is_async {
                                self.error(
                                    span,
                                    format!(
                                        "{trait_name}.{method} expected {}fn",
                                        if required_sig.is_async { "async " } else { "" }
                                    ),
                                );
                            }
                            if let (Some(expected), Some(got)) =
                                (required_sig.param_count, provided.param_count)
                            {
                                if got != expected {
                                    self.error(
                    span,
                    format!("{trait_name}.{method} expected {expected} params, got {got}"),
                  );
                                }
                            }
                            if let (Some(rt), Some(pt)) =
                                (&required_sig.return_type, &provided.return_type)
                            {
                                if !Self::type_is_skipped(rt)
                                    && !Self::type_is_skipped(pt)
                                    && !Self::types_compatible(&rt.name, &pt.name)
                                {
                                    self.error(
                                        span,
                                        format!(
                                            "{trait_name}.{method} expected return {}, got {}",
                                            rt.name, pt.name
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            None => self.error(span, format!("unknown trait '{trait_name}'")),
        }
    }

    fn check_item(&mut self, item: &Item) {
        match item {
            Item::FnDecl(f) => self.check_fn(f, None),
            Item::ImplDecl {
                type_name, methods, ..
            } => {
                for m in methods {
                    self.check_fn(m, Some(type_name.as_str()));
                }
            }
            Item::ClassDecl(c) => {
                for (_, e) in &c.defaults {
                    self.walk_expr(e);
                }
                for m in &c.methods {
                    self.check_fn(m, Some(c.name.as_str()));
                }
                for b in &c.trait_impls {
                    for m in &b.methods {
                        self.check_fn(m, Some(c.name.as_str()));
                    }
                }
            }
            Item::VarDecl(v) => self.check_var_decl(v),
            Item::ConstDecl(c) => {
                if let Some(ty) = &c.ty {
                    self.check_type_vs_expr(ty, &c.value);
                }
                self.walk_expr(&c.value);
                if let Some(ty) = c.ty.as_ref().filter(|t| !Self::type_is_skipped(t)) {
                    self.bind(c.name.clone(), ty.name.clone());
                } else if let Some(ty) = self.infer_expr_type(&c.value) {
                    self.bind(c.name.clone(), ty);
                }
            }
            Item::Mod(m) => {
                for inner in &m.items {
                    self.check_item(inner);
                }
            }
            _ => {}
        }
    }

    fn register_import(&mut self, import: &Import) {
        if import.path.is_empty() {
            return;
        }
        let module_name = import.path[0].as_str();

        if crate::stdlib::is_host_module(module_name) {
            if import.is_from && import.path.len() >= 2 {
                let item_name = import.path.last().unwrap();
                let bind = import.alias.as_ref().unwrap_or(item_name);
                if let Some(arity) = stdlib_arity(module_name, item_name) {
                    self.functions
                        .insert(bind.clone(), FnSig::loose(Some(arity)));
                } else {
                    self.functions.insert(bind.clone(), FnSig::loose(None));
                }
            } else if !import.is_from {
                let bind = import
                    .alias
                    .as_ref()
                    .unwrap_or_else(|| import.path.last().unwrap());
                self.module_aliases
                    .insert(bind.clone(), module_name.to_string());
            }
            return;
        }

        let canonical = if import.is_from {
            module_name.to_string()
        } else {
            import.path.join(".")
        };
        self.try_load_user_module(&canonical, import.span);

        if import.is_from {
            if import.path.len() >= 2 {
                let item_name = import.path.last().unwrap();
                let bind = import.alias.as_ref().unwrap_or(item_name);
                if let Some(sig) = self
                    .user_modules
                    .get(&canonical)
                    .and_then(|fns| fns.get(item_name))
                    .cloned()
                {
                    self.functions.insert(bind.clone(), sig);
                } else if self.structs.contains_key(item_name) || self.enums.contains_key(item_name)
                {
                    // Exported class/struct/enum — already registered on this checker.
                } else if self.strict_imports && self.user_modules.contains_key(&canonical) {
                    self.error(
                        import.span,
                        format!("module '{canonical}' has no export '{item_name}'"),
                    );
                } else {
                    self.functions.insert(bind.clone(), FnSig::loose(None));
                }
            }
        } else {
            let bind = import
                .alias
                .as_ref()
                .unwrap_or_else(|| import.path.last().unwrap());
            self.module_aliases.insert(bind.clone(), canonical);
        }
    }

    fn register_class_shape(&mut self, c: &ClassDecl) {
        self.register_fields(&c.name, &c.fields);
        self.register_methods(&c.name, &c.methods);
        for b in &c.trait_impls {
            self.register_methods(&c.name, &b.methods);
        }
        self.class_spans.insert(c.name.clone(), c.span);
    }

    fn ensure_class_ancestor(&mut self, name: &str, items: &[&Item]) {
        if self.structs.contains_key(name) || self.class_fields.contains_key(name) {
            return;
        }
        let Some(c) = items.iter().find_map(|item| match item {
            Item::ClassDecl(c) if c.name == name => Some(c),
            _ => None,
        }) else {
            return;
        };
        self.register_class_shape(c);
        if let Some(parent) = &c.parent {
            self.parents.insert(c.name.clone(), parent.clone());
            self.ensure_class_ancestor(parent, items);
        }
    }

    fn register_exported_class(&mut self, c: &ClassDecl, items: &[&Item]) {
        self.structs.insert(c.name.clone(), ());
        self.register_class_shape(c);
        if let Some(parent) = &c.parent {
            self.parents.insert(c.name.clone(), parent.clone());
            self.ensure_class_ancestor(parent, items);
        }
    }

    fn try_load_user_module(&mut self, name: &str, span: Span) {
        let name = crate::stdlib::canonical_module(name).unwrap_or(name);
        if crate::stdlib::is_host_module(name) {
            return;
        }
        if self.user_modules.contains_key(name) || self.loading.contains(name) {
            return;
        }
        let Some(resolver) = self.resolver else {
            return;
        };
        let parts = resolver.resolve_all(name);
        if parts.is_empty() {
            if self.strict_imports {
                self.error(
                    span,
                    format!(
                        "module '{name}' not found (tried {})",
                        crate::interpreter::module_lookup_hint(name)
                    ),
                );
            }
            self.user_modules.insert(name.to_string(), HashMap::new());
            return;
        }

        self.loading.insert(name.to_string());
        let mut fns = HashMap::new();
        let mut parsed: Vec<(String, Vec<Item>)> = Vec::new();
        for (key, source) in &parts {
            let program = match parse_module_source(source) {
                Ok(p) => p,
                Err(e) => {
                    self.error(span, format!("failed to parse module '{name}': {e}"));
                    self.loading.remove(name);
                    self.user_modules.insert(name.to_string(), HashMap::new());
                    return;
                }
            };

            for item in crate::parser::module_items(&program, name) {
                if let Item::Import(imp) = item {
                    let dep = if imp.is_from {
                        imp.path.first().cloned().unwrap_or_default()
                    } else {
                        imp.path.join(".")
                    };
                    if !dep.is_empty() && !crate::stdlib::is_host_module(&dep) {
                        self.try_load_user_module(&dep, imp.span);
                    }
                }
            }

            for item in crate::parser::module_items(&program, name) {
                let from_mod = crate::parser::module_body_from_mod(&program, name);
                if let Item::FnDecl(f) = item {
                    if from_mod && !f.is_pub {
                        continue;
                    }
                    if fns.insert(f.name.clone(), FnSig::from_fn(f)).is_some() {
                        self.error(
                            span,
                            format!("duplicate export '{}' in module '{name}'", f.name),
                        );
                    }
                }
                if !crate::parser::item_is_exported(item, from_mod) {
                    continue;
                }
                if let Item::StructDecl(s) = item {
                    self.structs.insert(s.name.clone(), ());
                    self.register_fields(&s.name, &s.fields);
                }
                if let Item::ClassDecl(c) = item {
                    if !self.structs.contains_key(&c.name) {
                        let items = crate::parser::module_items(&program, name);
                        self.register_exported_class(c, &items);
                    }
                }
                if let Item::ImplDecl {
                    type_name, methods, ..
                } = item
                {
                    self.register_methods(type_name, methods);
                }
                if let Item::TraitDecl(t) = item {
                    self.register_trait(t, false);
                }
                if let Item::EnumDecl(e) = item {
                    let mut variants = HashMap::new();
                    for v in &e.variants {
                        variants.insert(v.name.clone(), v.value_types.len());
                    }
                    self.enums.insert(e.name.clone(), variants);
                    fns.insert(e.name.clone(), FnSig::loose(None));
                }
            }
            parsed.push((key.clone(), program));
        }
        self.user_modules.insert(name.to_string(), fns);
        self.loading.remove(name);
        // Crate stdlib is already clean; checking it would re-report noise on every script.
        if !crate::stdlib::is_embedded_stdlib(name) {
            self.pending_bodies.insert(name.to_string(), parsed);
        }
    }

    fn check_imported_module_bodies(&mut self) {
        while !self.pending_bodies.is_empty() {
            let pending = std::mem::take(&mut self.pending_bodies);
            for (_name, parts) in pending {
                self.check_user_module_parts(&parts);
            }
        }
    }

    /// Walk imported fn / method bodies with the same operator checks as the main file.
    fn check_user_module_parts(&mut self, parts: &[(String, Vec<Item>)]) {
        let saved_functions = std::mem::take(&mut self.functions);
        let saved_aliases = std::mem::take(&mut self.module_aliases);
        let saved_globals = std::mem::take(&mut self.globals);
        let saved_signals = std::mem::take(&mut self.signals);
        let saved_file = std::mem::take(&mut self.current_file);

        for (_file, program) in parts {
            for item in program {
                self.register_module_check_env(item);
            }
        }
        for (file, program) in parts {
            self.current_file = file.clone();
            for item in program {
                self.check_item(item);
            }
        }

        self.functions = saved_functions;
        self.module_aliases = saved_aliases;
        self.globals = saved_globals;
        self.signals = saved_signals;
        self.current_file = saved_file;
    }

    /// Locals needed to check a foreign module (imports, fns, globals). Types stay shared.
    fn register_module_check_env(&mut self, item: &Item) {
        match item {
            Item::Import(i) => self.register_import(i),
            Item::FnDecl(f) => {
                self.functions.insert(f.name.clone(), FnSig::from_fn(f));
            }
            Item::VarDecl(v) => {
                if let Some(ty) = self.binding_type_for_var(v) {
                    self.globals.insert(v.name.clone(), ty);
                }
            }
            Item::ConstDecl(c) => {
                if let Some(ty) = c.ty.as_ref().filter(|t| !Self::type_is_skipped(t)) {
                    self.globals.insert(c.name.clone(), ty.name.clone());
                } else if let Some(ty) = self.infer_expr_type(&c.value) {
                    self.globals.insert(c.name.clone(), ty);
                }
            }
            Item::SignalDecl(s) => {
                self.signals.insert(s.name.clone(), s.params.len());
            }
            Item::Mod(m) => {
                for inner in &m.items {
                    self.register_module_check_env(inner);
                }
            }
            _ => {}
        }
    }

    fn check_fn(&mut self, f: &FnDecl, self_type: Option<&str>) {
        let prev_super = self.current_super.take();
        self.current_super = self_type.and_then(|t| self.parents.get(t).cloned());
        let prev_return = self.current_return.take();
        self.current_return = f.return_type.as_ref().map(|t| t.name.clone());
        self.scopes.push(HashMap::new());
        if let Some(ty) = self_type {
            self.bind("self".to_string(), ty.to_string());
            for p in params_after_self(&f.params) {
                if p.name == "self" {
                    continue;
                } else if !Self::type_is_skipped(&p.ty) {
                    self.bind(p.name.clone(), p.ty.name.clone());
                } else {
                    self.bind(p.name.clone(), String::new());
                }
            }
        } else {
            for p in &f.params {
                if p.name == "self" {
                    self.bind("self".to_string(), String::new());
                } else if !Self::type_is_skipped(&p.ty) {
                    self.bind(p.name.clone(), p.ty.name.clone());
                } else {
                    self.bind(p.name.clone(), String::new());
                }
            }
        }
        self.check_block(&f.body, f.return_type.as_ref());
        self.scopes.pop();
        self.current_super = prev_super;
        self.current_return = prev_return;
    }

    fn check_var_decl(&mut self, v: &VarDecl) {
        if let Some(expr) = &v.value {
            self.check_type_vs_expr(&v.ty, expr);
            self.walk_expr(expr);
        }
        if let Some(ty) = self.binding_type_for_var(v) {
            self.bind(v.name.clone(), ty);
        } else {
            self.bind(v.name.clone(), String::new());
        }
    }

    fn type_is_skipped(ty: &Type) -> bool {
        ty.name == "None" || ty.name.is_empty() || ty.name == "Self"
    }

    fn check_type_vs_expr(&mut self, ty: &Type, expr: &Expr) {
        if Self::type_is_skipped(ty) {
            return;
        }
        if let Some(inferred) = self.infer_expr_type(expr) {
            if !Self::types_compatible(&ty.name, &inferred) {
                self.error(
                    expr.span,
                    format!(
                        "variable annotated as {} but initializer looks like {}",
                        ty.name, inferred
                    ),
                );
            }
        }
    }

    fn types_compatible(annotated: &str, inferred: &str) -> bool {
        if annotated == inferred {
            return true;
        }
        if matches!((annotated, inferred), ("Str", "String") | ("String", "Str")) {
            return true;
        }
        // Loose numeric compatibility
        matches!((annotated, inferred), ("Float", "Int") | ("Int", "Float"))
    }

    fn type_known(ty: Option<String>) -> Option<String> {
        match ty {
            Some(s) if !s.is_empty() && s != "None" && s != "Self" => Some(s),
            _ => None,
        }
    }

    fn is_numeric(ty: &str) -> bool {
        matches!(ty, "Int" | "Float")
    }

    fn is_string(ty: &str) -> bool {
        matches!(ty, "String" | "Str")
    }

    fn is_int(ty: &str) -> bool {
        ty == "Int"
    }

    fn numeric_result(lt: &str, rt: &str) -> String {
        if lt == "Float" || rt == "Float" {
            "Float".to_string()
        } else {
            "Int".to_string()
        }
    }

    fn infer_binop_type(&self, op: BinOp, lt: &str, rt: &str) -> Option<String> {
        match op {
            BinOp::Add if Self::is_numeric(lt) && Self::is_numeric(rt) => {
                Some(Self::numeric_result(lt, rt))
            }
            BinOp::Add if Self::is_string(lt) && Self::is_string(rt) => Some("String".to_string()),
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
                if Self::is_numeric(lt) && Self::is_numeric(rt) =>
            {
                Some(Self::numeric_result(lt, rt))
            }
            BinOp::IDiv if Self::is_int(lt) && Self::is_int(rt) => Some("Int".to_string()),
            BinOp::Eq
            | BinOp::Neq
            | BinOp::Lt
            | BinOp::Lte
            | BinOp::Gt
            | BinOp::Gte
            | BinOp::And
            | BinOp::Or => Some("Bool".to_string()),
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
                if Self::is_int(lt) && Self::is_int(rt) =>
            {
                Some("Int".to_string())
            }
            _ => None,
        }
    }

    fn check_binop(&mut self, span: Span, op: BinOp, left: &Expr, right: &Expr) {
        let lt = Self::type_known(self.infer_expr_type(left));
        let rt = Self::type_known(self.infer_expr_type(right));
        match op {
            BinOp::Add => match (lt.as_deref(), rt.as_deref()) {
                (Some(l), Some(r))
                    if (Self::is_numeric(l) && Self::is_numeric(r))
                        || (Self::is_string(l) && Self::is_string(r)) => {}
                (Some(l), Some(r)) => {
                    self.error(span, format!("cannot add {l} and {r}"));
                }
                _ => {}
            },
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                match (lt.as_deref(), rt.as_deref()) {
                    (Some(l), Some(r)) if Self::is_numeric(l) && Self::is_numeric(r) => {}
                    (Some(l), Some(r)) => {
                        let verb = match op {
                            BinOp::Sub => "subtract",
                            BinOp::Mul => "multiply",
                            BinOp::Div => "divide",
                            _ => "modulo",
                        };
                        self.error(span, format!("cannot {verb} {l} and {r}"));
                    }
                    _ => {}
                }
            }
            BinOp::IDiv => {
                if lt.as_deref().is_some_and(|t| !Self::is_int(t))
                    || rt.as_deref().is_some_and(|t| !Self::is_int(t))
                {
                    let l = lt.as_deref().unwrap_or("unknown");
                    let r = rt.as_deref().unwrap_or("unknown");
                    self.error(
                        span,
                        format!("integer division requires Int, got {l} and {r}"),
                    );
                }
            }
            BinOp::Lt | BinOp::Lte | BinOp::Gt | BinOp::Gte => match (lt.as_deref(), rt.as_deref())
            {
                (Some(l), Some(r)) if Self::is_numeric(l) && Self::is_numeric(r) => {}
                (Some(l), Some(r)) => {
                    self.error(span, format!("cannot compare {l} and {r}"));
                }
                _ => {}
            },
            BinOp::Eq | BinOp::Neq | BinOp::And | BinOp::Or => {}
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                if lt.as_deref().is_some_and(|t| !Self::is_int(t))
                    || rt.as_deref().is_some_and(|t| !Self::is_int(t))
                {
                    self.error(span, "bitwise operators require Int");
                }
            }
        }
    }

    fn check_index(&mut self, span: Span, object: &Expr, index: &Expr) {
        let obj = Self::type_known(self.infer_expr_type(object));
        let idx = Self::type_known(self.infer_expr_type(index));
        match obj.as_deref() {
            Some("Array" | "String" | "Str") => {
                if let Some(i) = idx.as_deref() {
                    if !Self::is_int(i) {
                        self.error(
                            span,
                            format!("cannot index {} with {i}", obj.as_deref().unwrap()),
                        );
                    }
                }
            }
            Some("Map") => {
                if let Some(i) = idx.as_deref() {
                    if !Self::is_string(i) {
                        self.error(span, format!("cannot index Map with {i}"));
                    }
                }
            }
            Some(o) => {
                let i = idx.as_deref().unwrap_or("unknown");
                self.error(span, format!("cannot index {o} with {i}"));
            }
            None => {}
        }
    }

    fn infer_iter_item_type(&self, iter: &Expr) -> String {
        if matches!(&iter.kind, ExprKind::Range { .. }) {
            return "Int".to_string();
        }
        if let ExprKind::Call { callee, args } = &iter.kind {
            if let ExprKind::Ident(name) = &callee.kind {
                if name == "Array" {
                    return self.infer_array_elem_type(args);
                }
            }
        }
        match Self::type_known(self.infer_expr_type(iter)).as_deref() {
            Some("Map" | "String" | "Str") => "String".to_string(),
            Some("Int") => "Int".to_string(),
            _ => String::new(),
        }
    }

    /// Homogeneous array literal of known types → that element type.
    /// Mixed or unknown elements stay untyped on purpose.
    fn infer_array_elem_type(&self, args: &[Expr]) -> String {
        if args.is_empty() {
            return String::new();
        }
        let mut elem: Option<String> = None;
        for a in args {
            let Some(t) = Self::type_known(self.infer_expr_type(a)) else {
                return String::new();
            };
            match &elem {
                None => elem = Some(t),
                Some(prev) if Self::types_compatible(prev, &t) => {
                    if prev == "Int" && t == "Float" {
                        elem = Some(t);
                    }
                }
                Some(_) => return String::new(),
            }
        }
        elem.unwrap_or_default()
    }

    fn check_assign_op(&mut self, span: Span, op: AssignOp, left: &Expr, right: &Expr) {
        match op {
            AssignOp::Assign => {
                let lt = Self::type_known(self.infer_expr_type(left));
                let rt = Self::type_known(self.infer_expr_type(right));
                if let (Some(l), Some(r)) = (lt.as_deref(), rt.as_deref()) {
                    if !Self::types_compatible(l, r) {
                        self.error(span, format!("cannot assign {r} to {l}"));
                    }
                }
            }
            AssignOp::Add => self.check_binop(span, BinOp::Add, left, right),
            AssignOp::Sub => self.check_binop(span, BinOp::Sub, left, right),
            AssignOp::Mul => self.check_binop(span, BinOp::Mul, left, right),
            AssignOp::Div => self.check_binop(span, BinOp::Div, left, right),
            AssignOp::Mod => self.check_binop(span, BinOp::Mod, left, right),
            AssignOp::BitAnd => self.check_binop(span, BinOp::BitAnd, left, right),
            AssignOp::BitOr => self.check_binop(span, BinOp::BitOr, left, right),
            AssignOp::BitXor => self.check_binop(span, BinOp::BitXor, left, right),
        }
    }

    fn infer_expr_type(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Literal(Literal::Int(_)) => Some("Int".to_string()),
            ExprKind::Literal(Literal::Float(_)) => Some("Float".to_string()),
            ExprKind::Literal(Literal::String(_)) => Some("String".to_string()),
            ExprKind::Literal(Literal::Bool(_)) => Some("Bool".to_string()),
            ExprKind::Literal(Literal::None) => Some("None".to_string()),
            ExprKind::Ident(name) => self.lookup_binding(name).filter(|t| !t.is_empty()),
            ExprKind::StructLiteral { name, .. } => Some(name.clone()),
            ExprKind::FString(_) => Some("String".to_string()),
            ExprKind::Call { callee, .. } => self.infer_call_type(callee),
            ExprKind::Binary { op, left, right } => {
                let lt = self.infer_expr_type(left)?;
                let rt = self.infer_expr_type(right)?;
                self.infer_binop_type(*op, &lt, &rt)
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnaryOp::Neg => {
                    let ty = self.infer_expr_type(inner)?;
                    Self::is_numeric(&ty).then_some(ty)
                }
                UnaryOp::Not => Some("Bool".to_string()),
                UnaryOp::BitNot => {
                    let ty = self.infer_expr_type(inner)?;
                    Self::is_int(&ty).then_some("Int".to_string())
                }
            },
            ExprKind::Spawn(_) => Some("Task".to_string()),
            ExprKind::Await(inner) => self.infer_await_type(inner),
            ExprKind::Lambda { .. } => Some("Fn".to_string()),
            ExprKind::Try(_) => None,
            ExprKind::Index { object, .. } => {
                let obj = self.infer_expr_type(object)?;
                match obj.as_str() {
                    "String" | "Str" => Some("String".to_string()),
                    "Array" | "Map" => None,
                    _ => None,
                }
            }
            ExprKind::Member { object, .. } => {
                if let ExprKind::Ident(mod_name) = &object.kind {
                    if self.enums.contains_key(mod_name) {
                        return Some(mod_name.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn infer_call_type(&self, callee: &Expr) -> Option<String> {
        match &callee.kind {
            ExprKind::Ident(name) => match name.as_str() {
                "Array" => Some("Array".to_string()),
                "Map" => Some("Map".to_string()),
                "Mutex" => Some("Mutex".to_string()),
                "Channel" => Some("Channel".to_string()),
                _ => {
                    if let Some(ty) = self.functions.get(name).and_then(|s| s.call_return()) {
                        return Some(ty);
                    }
                    if let Some(recv) = self.receiver_type() {
                        if let Some(sig) = self.lookup_type_method(&recv, name) {
                            return sig.call_return();
                        }
                    }
                    None
                }
            },
            ExprKind::Member { object, name } => {
                if let ExprKind::Ident(mod_name) = &object.kind {
                    if self.enums.contains_key(mod_name) {
                        return Some(mod_name.clone());
                    }
                    if let Some(sig) = self.qualified_fn_sig(mod_name, name) {
                        return sig.call_return();
                    }
                    let canonical = self
                        .module_aliases
                        .get(mod_name)
                        .cloned()
                        .unwrap_or_else(|| mod_name.to_string());
                    if let Some(ty) = host_return_type(&canonical, name) {
                        return Some(ty.to_string());
                    }
                }
                let obj_ty = self.infer_expr_type(object)?;
                if let Some(sig) = self.lookup_type_method(&obj_ty, name) {
                    return sig.call_return();
                }
                match (obj_ty.as_str(), name.as_str()) {
                    ("Array" | "String" | "Str" | "Map", "len") => Some("Int".to_string()),
                    ("Array", "first" | "last" | "pop") => None,
                    ("Map", "keys") => Some("Array".to_string()),
                    ("Mutex", "lock" | "unlock") => Some("Void".to_string()),
                    ("Channel", "send" | "close") => Some("Void".to_string()),
                    ("Channel", "recv") => None,
                    ("Channel", "recv_timeout") => Some("Option".to_string()),
                    ("Task", "wait") => Some("Option".to_string()),
                    ("Option", "is_some" | "is_none") | ("Result", "is_ok" | "is_err") => {
                        Some("Bool".to_string())
                    }
                    _ => None,
                }
            }
            ExprKind::Lambda { return_type, .. } => Some(
                return_type
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Void".to_string()),
            ),
            _ => None,
        }
    }

    fn infer_await_type(&self, inner: &Expr) -> Option<String> {
        match &inner.kind {
            ExprKind::Spawn(call) => match &call.kind {
                ExprKind::Call { callee, .. } => self.callee_inner_return(callee),
                ExprKind::Lambda { return_type, .. } => Some(
                    return_type
                        .as_ref()
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| "Void".to_string()),
                ),
                _ => None,
            },
            ExprKind::Call { callee, .. } => self.callee_inner_return(callee),
            ExprKind::Lambda { return_type, .. } => Some(
                return_type
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Void".to_string()),
            ),
            _ => None,
        }
    }

    fn callee_inner_return(&self, callee: &Expr) -> Option<String> {
        match &callee.kind {
            ExprKind::Ident(name) => self.functions.get(name).and_then(|s| s.inner_return()),
            ExprKind::Lambda { return_type, .. } => Some(
                return_type
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Void".to_string()),
            ),
            ExprKind::Member { object, name } => {
                if let ExprKind::Ident(mod_name) = &object.kind {
                    if let Some(sig) = self.qualified_fn_sig(mod_name, name) {
                        return sig.inner_return();
                    }
                }
                let obj_ty = self.infer_expr_type(object)?;
                self.lookup_type_method(&obj_ty, name)
                    .and_then(|s| s.inner_return())
            }
            _ => None,
        }
    }

    fn qualified_fn_sig(&self, mod_name: &str, name: &str) -> Option<&FnSig> {
        let canonical = self
            .module_aliases
            .get(mod_name)
            .cloned()
            .unwrap_or_else(|| mod_name.to_string());
        let load_name = crate::stdlib::canonical_module(&canonical).unwrap_or(canonical.as_str());
        self.user_modules
            .get(load_name)
            .or_else(|| self.user_modules.get(&canonical))
            .and_then(|fns| fns.get(name))
    }

    fn known_receiver(&self, ty: &str) -> bool {
        !ty.is_empty()
            && (self.structs.contains_key(ty)
                || self.type_methods.contains_key(ty)
                || self.enums.contains_key(ty)
                || matches!(
                    ty,
                    "Array"
                        | "String"
                        | "Str"
                        | "Map"
                        | "Option"
                        | "Result"
                        | "Vec2"
                        | "Vec3"
                        | "Int"
                        | "Float"
                        | "Bool"
                        | "None"
                        | "Mutex"
                        | "Channel"
                        | "Task"
                        | "Fn"
                ))
    }

    fn check_block(&mut self, block: &Block, return_type: Option<&Type>) {
        for stmt in &block.stmts {
            self.check_stmt(stmt, return_type);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt, return_type: Option<&Type>) {
        match &stmt.kind {
            StmtKind::Expr(e) => self.walk_expr(e),
            StmtKind::Return(value) => {
                if let Some(rt) = return_type {
                    if !Self::type_is_skipped(rt) && rt.name != "Void" && value.is_none() {
                        self.error(
                            stmt.span,
                            format!("function with return type {} must return a value", rt.name),
                        );
                    }
                }
                if let Some(e) = value {
                    self.walk_expr(e);
                }
            }
            StmtKind::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.walk_expr(cond);
                self.check_block(then_block, return_type);
                for (c, b) in elif_blocks {
                    self.walk_expr(c);
                    self.check_block(b, return_type);
                }
                if let Some(b) = else_block {
                    self.check_block(b, return_type);
                }
            }
            StmtKind::While { cond, body } => {
                self.walk_expr(cond);
                self.check_block(body, return_type);
            }
            StmtKind::For { name, iter, body } => {
                self.walk_expr(iter);
                self.scopes.push(HashMap::new());
                self.bind(name.clone(), self.infer_iter_item_type(iter));
                self.check_block(body, return_type);
                self.scopes.pop();
            }
            StmtKind::VarDecl(v) => self.check_var_decl(v),
            StmtKind::ConstDecl(c) => {
                if let Some(ty) = &c.ty {
                    self.check_type_vs_expr(ty, &c.value);
                }
                self.walk_expr(&c.value);
                if let Some(ty) = c.ty.as_ref().filter(|t| !Self::type_is_skipped(t)) {
                    self.bind(c.name.clone(), ty.name.clone());
                } else if let Some(ty) = self.infer_expr_type(&c.value) {
                    self.bind(c.name.clone(), ty);
                } else {
                    self.bind(c.name.clone(), String::new());
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Pass | StmtKind::Comment(_) => {}
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Ident(name) => {
                if !self.name_is_defined(name) {
                    self.error(expr.span, format!("undefined '{name}'"));
                }
            }
            ExprKind::Binary { op, left, right } => {
                self.walk_expr(left);
                self.walk_expr(right);
                self.check_binop(expr.span, *op, left, right);
            }
            ExprKind::Unary { op, expr: inner } => {
                self.walk_expr(inner);
                if let Some(ty) = Self::type_known(self.infer_expr_type(inner)) {
                    match op {
                        UnaryOp::Neg if !Self::is_numeric(&ty) => {
                            self.error(expr.span, format!("cannot negate {ty}"));
                        }
                        UnaryOp::BitNot if !Self::is_int(&ty) => {
                            self.error(expr.span, "bitwise not requires Int");
                        }
                        _ => {}
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                for a in args {
                    self.walk_expr(a);
                }
                self.check_call(callee, args);
                if !matches!(&callee.kind, ExprKind::Ident(_)) {
                    self.walk_expr(callee);
                }
            }
            ExprKind::Member { object, name } => {
                if matches!(name.as_str(), "emit" | "connect")
                    && matches!(&object.kind, ExprKind::Ident(_))
                {
                    // check_call reports unknown signals; don't also flag the name as undefined
                } else {
                    self.walk_expr(object);
                }
            }
            ExprKind::Index { object, index } => {
                self.walk_expr(object);
                self.walk_expr(index);
                self.check_index(expr.span, object, index);
            }
            ExprKind::StructLiteral { name, fields } => {
                let known = self.class_fields.contains_key(name) || self.structs.contains_key(name);
                for (fname, e) in fields {
                    if known && self.lookup_field(name, fname).is_none() {
                        self.error(expr.span, format!("unknown field '{fname}' on {name}"));
                    }
                    self.walk_expr(e);
                }
            }
            ExprKind::Assign { op, left, right } => {
                self.walk_expr(left);
                self.walk_expr(right);
                self.check_assign_op(expr.span, *op, left, right);
            }
            ExprKind::FString(parts) => {
                for part in parts {
                    if let FStringPart::Expr { expr: e, .. } = part {
                        self.walk_expr(e);
                    }
                }
            }
            ExprKind::Range { start, end, .. } => {
                self.walk_expr(start);
                self.walk_expr(end);
                for (side, e) in [("start", start.as_ref()), ("end", end.as_ref())] {
                    if let Some(ty) = Self::type_known(self.infer_expr_type(e)) {
                        if !Self::is_int(&ty) {
                            self.error(expr.span, format!("range {side} must be Int, got {ty}"));
                        }
                    }
                }
            }
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.walk_expr(scrutinee);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.bind_pattern(&arm.pattern);
                    self.check_block(&arm.body, None);
                    self.scopes.pop();
                }
            }
            ExprKind::Spawn(inner) => self.walk_expr(inner),
            ExprKind::Await(inner) => {
                self.walk_expr(inner);
                if let Some(ty) = Self::type_known(self.infer_expr_type(inner)) {
                    if ty != "Task" {
                        self.error(expr.span, format!("await expects Task, got {ty}"));
                    }
                }
            }
            ExprKind::Try(inner) => {
                self.walk_expr(inner);
                if let Some(ty) = Self::type_known(self.infer_expr_type(inner)) {
                    if ty != "Result" {
                        self.error(expr.span, format!("? expects Result, got {ty}"));
                    }
                }
                match self.current_return.as_deref() {
                    Some("Result") => {}
                    Some(rt) => self.error(
                        expr.span,
                        format!("? requires the function to return Result, got {rt}"),
                    ),
                    None => self.error(expr.span, "? requires the function to return Result"),
                }
            }
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => self.check_lambda(params, return_type.as_ref(), body),
        }
    }

    fn check_lambda(&mut self, params: &[Param], return_type: Option<&Type>, body: &Block) {
        let prev_return = self.current_return.take();
        self.current_return = return_type.map(|t| t.name.clone());
        self.scopes.push(HashMap::new());
        for p in params {
            if !Self::type_is_skipped(&p.ty) {
                self.bind(p.name.clone(), p.ty.name.clone());
            } else {
                self.bind(p.name.clone(), String::new());
            }
        }
        self.check_block(body, return_type);
        self.scopes.pop();
        self.current_return = prev_return;
    }

    fn check_call(&mut self, callee: &Expr, args: &[Expr]) {
        let arg_count = args.len();
        match &callee.kind {
            ExprKind::Ident(name) => {
                if let Some(arity) = self.builtins.get(name.as_str()) {
                    if let Some(expected) = arity {
                        if arg_count != *expected {
                            self.error(
                                callee.span,
                                format!("{} expected {} args, got {}", name, expected, arg_count),
                            );
                        }
                    }
                    return;
                }
                if let Some(sig) = self.functions.get(name).cloned() {
                    if let Some(expected) = sig.param_count {
                        if arg_count != expected {
                            self.error(
                                callee.span,
                                format!("{} expected {} args, got {}", name, expected, arg_count),
                            );
                        }
                    }
                    return;
                }
                if let Some(ty) = self.receiver_type() {
                    if self.check_method_call(callee.span, &ty, name, args) {
                        return;
                    }
                }
                if self.lookup_binding(name).as_deref() == Some("Fn") {
                    return;
                }
                self.error(callee.span, format!("undefined function '{}'", name));
            }
            ExprKind::Lambda { params, .. } => {
                if arg_count != params.len() {
                    self.error(
                        callee.span,
                        format!("fn() expected {} args, got {}", params.len(), arg_count),
                    );
                }
            }
            ExprKind::Member { object, name } => {
                if matches!(name.as_str(), "emit" | "connect") {
                    if let ExprKind::Ident(sig) = &object.kind {
                        self.check_signal_call(callee.span, sig, name, args);
                        return;
                    }
                }
                if let ExprKind::Ident(mod_name) = &object.kind {
                    if mod_name == "super" {
                        match self.current_super.clone() {
                            Some(parent) => {
                                if !self.check_method_call(callee.span, &parent, name, args) {
                                    self.error(
                                        callee.span,
                                        format!("{parent} has no method '{name}'"),
                                    );
                                }
                            }
                            None => self.error(
                                callee.span,
                                "super is only valid in a method of a class that extends another",
                            ),
                        }
                        return;
                    }
                    if self.is_qualifiable(mod_name) && self.lookup_binding(mod_name).is_none() {
                        self.check_qualified_call(callee.span, mod_name, name, arg_count);
                        return;
                    }
                }
                if let Some(obj_ty) = self.infer_expr_type(object).filter(|t| !t.is_empty()) {
                    if self.check_method_call(callee.span, &obj_ty, name, args) {
                        return;
                    }
                }
                if let Some(sig) = self.functions.get(name).cloned() {
                    if sig.is_ufcs {
                        if let Some(expected) = sig.call_arity() {
                            if arg_count != expected {
                                self.error(
                                    callee.span,
                                    format!("{name} expected {expected} args, got {arg_count}"),
                                );
                            }
                        }
                        return;
                    }
                }
                if let ExprKind::Ident(id) = &object.kind {
                    if self.lookup_binding(id).as_deref() == Some("") {
                        self.error(
                            callee.span,
                            format!("cannot call '{name}' on a value of unknown type"),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn check_signal_call(&mut self, span: Span, sig: &str, method: &str, args: &[Expr]) {
        let Some(&arity) = self.signals.get(sig) else {
            self.error(span, format!("unknown signal '{sig}'"));
            return;
        };
        if method == "emit" {
            if args.len() != arity {
                self.error(
                    span,
                    format!("{sig}.emit expected {arity} args, got {}", args.len()),
                );
            }
            return;
        }
        if method == "connect" {
            if args.len() != 1 {
                self.error(
                    span,
                    format!("{sig}.connect expected 1 arg, got {}", args.len()),
                );
                return;
            }
            let arg = args.first().unwrap();
            match &arg.kind {
                ExprKind::Ident(listener) => {
                    let Some(fn_sig) = self.functions.get(listener).cloned() else {
                        if self.lookup_binding(listener).as_deref() != Some("Fn") {
                            self.error(span, format!("undefined function '{listener}'"));
                        }
                        return;
                    };
                    if fn_sig.takes_self {
                        self.error(
                            span,
                            format!(
                                "{sig}.connect expects a free function, got method '{listener}'"
                            ),
                        );
                        return;
                    }
                    if let Some(got) = fn_sig.call_arity() {
                        if got != arity {
                            self.error(
                                span,
                                format!(
                                    "{listener} expected {arity} args to connect to '{sig}', got {got}"
                                ),
                            );
                        }
                    }
                }
                ExprKind::Lambda { params, .. } => {
                    if params.len() != arity {
                        self.error(
                            span,
                            format!(
                                "fn() expected {arity} args to connect to '{sig}', got {}",
                                params.len()
                            ),
                        );
                    }
                }
                _ => {
                    if self.infer_expr_type(arg).as_deref() != Some("Fn") {
                        self.error(span, format!("{sig}.connect expects a function"));
                    }
                }
            }
        }
    }

    /// Returns true if this was a known method (or a known type with a missing method).
    fn check_method_call(&mut self, span: Span, obj_ty: &str, name: &str, args: &[Expr]) -> bool {
        let arg_count = args.len();
        if let Some(sig) = self.lookup_type_method(obj_ty, name) {
            self.check_sig_call(span, &format!("{obj_ty}.{name}"), &sig, args);
            return true;
        }
        if let Some(expected) = native_method_arity(obj_ty, name) {
            if arg_count != expected {
                self.error(
                    span,
                    format!("{obj_ty}.{name} expected {expected} args, got {arg_count}"),
                );
            }
            return true;
        }
        if self.known_receiver(obj_ty) {
            if let Some(sig) = self.functions.get(name).cloned() {
                if sig.is_ufcs {
                    self.check_sig_call(span, name, &sig, args);
                    return true;
                }
            }
            self.error(span, format!("{obj_ty} has no method '{name}'"));
            return true;
        }
        false
    }

    fn check_sig_call(&mut self, span: Span, label: &str, sig: &FnSig, args: &[Expr]) {
        if let Some(expected) = sig.call_arity() {
            if args.len() != expected {
                self.error(
                    span,
                    format!("{label} expected {expected} args, got {}", args.len()),
                );
                return;
            }
        }
        for (arg, expected_ty) in args.iter().zip(sig.arg_types.iter()) {
            if expected_ty.is_empty()
                || expected_ty == "None"
                || expected_ty == "Self"
                || expected_ty == "T"
                || expected_ty == "E"
            {
                continue;
            }
            if let Some(got) = self.infer_expr_type(arg) {
                if !Self::types_compatible(expected_ty, &got) {
                    self.error(
                        arg.span,
                        format!("{label} expected {expected_ty}, got {got}"),
                    );
                }
            }
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard => {}
            Pattern::Variant {
                binds, field_binds, ..
            } => {
                for b in binds {
                    if b != "_" {
                        self.bind(b.clone(), String::new());
                    }
                }
                for (_, b) in field_binds {
                    if b != "_" {
                        self.bind(b.clone(), String::new());
                    }
                }
            }
        }
    }

    fn name_is_defined(&self, name: &str) -> bool {
        if matches!(name, "super" | "self") {
            return true;
        }
        if self.lookup_binding(name).is_some() {
            return true;
        }
        if self.functions.contains_key(name) || self.builtins.contains_key(name) {
            return true;
        }
        if self.structs.contains_key(name)
            || self.enums.contains_key(name)
            || self.traits.contains_key(name)
            || self.signals.contains_key(name)
        {
            return true;
        }
        self.is_qualifiable(name)
    }

    fn is_qualifiable(&self, name: &str) -> bool {
        self.module_aliases.contains_key(name)
            || self.user_modules.contains_key(name)
            || self.enums.contains_key(name)
            || crate::stdlib::is_internal_host(name)
            || crate::stdlib::is_language_module(name)
    }

    fn check_qualified_call(&mut self, span: Span, mod_name: &str, name: &str, arg_count: usize) {
        let canonical = self
            .module_aliases
            .get(mod_name)
            .cloned()
            .unwrap_or_else(|| mod_name.to_string());

        if crate::stdlib::is_host_module(&canonical)
            || crate::stdlib::is_language_module(&canonical)
        {
            if let Some(expected) = stdlib_arity(&canonical, name) {
                if arg_count != expected {
                    self.error(
                        span,
                        format!("{canonical}.{name} expected {expected} args, got {arg_count}"),
                    );
                }
                return;
            }
            self.error(span, format!("unknown function '{canonical}.{name}'"));
            return;
        }

        // `Shape.Circle` is an enum variant, not `import Shape`.
        if !self.enums.contains_key(&canonical)
            && !self.structs.contains_key(&canonical)
            && !self.traits.contains_key(&canonical)
        {
            self.try_load_user_module(&canonical, span);
        }
        let load_name = crate::stdlib::canonical_module(&canonical).unwrap_or(canonical.as_str());

        if let Some(variants) = self
            .enums
            .get(&canonical)
            .or_else(|| self.enums.get(load_name))
        {
            if let Some(expected) = variants.get(name).copied() {
                if arg_count != expected {
                    self.error(
                        span,
                        format!("{canonical}.{name} expected {expected} args, got {arg_count}"),
                    );
                }
                return;
            }
        }

        if let Some(fns) = self
            .user_modules
            .get(load_name)
            .or_else(|| self.user_modules.get(&canonical))
        {
            if let Some(sig) = fns.get(name) {
                if let Some(expected) = sig.param_count {
                    if arg_count != expected {
                        self.error(
                            span,
                            format!("{canonical}.{name} expected {expected} args, got {arg_count}"),
                        );
                    }
                }
                return;
            }
            if !fns.is_empty() {
                self.error(
                    span,
                    format!("module '{canonical}' has no function '{name}'"),
                );
            }
        }
    }
}

/// All typecheck findings (may be empty). Spans are filled; `file` is left blank.
/// Uses the crate-embedded stdlib; user imports are lenient without a resolver.
pub fn typecheck_diagnostics(program: &[Item]) -> Vec<Diagnostic> {
    typecheck_diagnostics_with(program, None)
}

pub fn typecheck_diagnostics_with(
    program: &[Item],
    resolver: Option<&dyn ModuleResolver>,
) -> Vec<Diagnostic> {
    let fallback = crate::interpreter::HashMapResolver::new(HashMap::new());
    let strict = resolver.is_some();
    let resolver: &dyn ModuleResolver = match resolver {
        Some(r) => r,
        None => &fallback,
    };
    let mut checker = TypeChecker::new(Some(resolver), strict);
    checker.check_program(program);
    checker.diagnostics
}

/// Typecheck `expr` after registering `prelude` (REPL lines, prior bindings).
pub fn typecheck_expr_diagnostics(prelude: &[Item], expr: &Expr) -> Vec<Diagnostic> {
    typecheck_expr_diagnostics_with(prelude, expr, None)
}

pub fn typecheck_expr_diagnostics_with(
    prelude: &[Item],
    expr: &Expr,
    resolver: Option<&dyn ModuleResolver>,
) -> Vec<Diagnostic> {
    let fallback = crate::interpreter::HashMapResolver::new(HashMap::new());
    let strict = resolver.is_some();
    let resolver: &dyn ModuleResolver = match resolver {
        Some(r) => r,
        None => &fallback,
    };
    let mut checker = TypeChecker::new(Some(resolver), strict);
    checker.check_program(prelude);
    checker.walk_expr(expr);
    checker.diagnostics
}

/// Static type check. Hard-fails on wrong arity, undefined names, and method calls with no receiver type.
pub fn typecheck(program: &[Item]) -> Result<(), String> {
    match typecheck_diagnostics(program).into_iter().next() {
        None => Ok(()),
        Some(d) => Err(format!("type error at {}:{}: {}", d.line, d.col, d.message)),
    }
}
