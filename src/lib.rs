//! RoseGold: lexer, parser, typecheck, and tree-walking interpreter.
//!
//! Hosts use [`compile_source`] / [`run_source`] / [`EvalContext`]. Editor
//! metadata lives in [`export`], [`signal`], and [`navigate`].

pub mod export;
pub mod format;
pub mod host;
pub mod interpreter;
pub mod lexer;
pub mod navigate;
pub mod parser;
pub mod signal;
pub mod stdlib;
pub mod typecheck;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub use export::{
    ExportField, NodeClass, collect_exports, collect_nodes, list_exports, list_nodes,
};
pub use format::format_source;
pub use host::{HostEffect, LabeledHostEffects};
pub use interpreter::{
    CombinedResolver, EvalContext, FileModuleResolver, HashMapResolver, Module, ModuleResolver,
    Value, WorldEntry,
};
pub use lexer::{Lexer, Token, TokenKind};
pub use navigate::{SymbolInfo, def_at, hover_at, symbol_at};
pub use parser::{
    Block, ClassDecl, EnumDecl, EnumVariant, Expr, FnDecl, Item, Literal, ModDecl, Parser,
    SignalDecl, Stmt, StructDecl, TraitDecl, Type,
};
pub use signal::{
    FnMeta, SignalField, SignalParam, list_fns, list_signals, list_signals_with_modules,
};
pub use typecheck::{typecheck, typecheck_diagnostics};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime error at {}: {}", self.span, self.message)
    }
}

/// A parse or typecheck finding. Display is `file:line:col: error: message`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub severity: String,
    pub message: String,
}

impl Diagnostic {
    pub fn error(file: impl Into<String>, span: Span, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line: span.line,
            col: span.col,
            severity: "error".into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.file.is_empty() {
            write!(
                f,
                "{}:{}: {}: {}",
                self.line, self.col, self.severity, self.message
            )
        } else {
            write!(
                f,
                "{}:{}:{}: {}: {}",
                self.file, self.line, self.col, self.severity, self.message
            )
        }
    }
}

/// Pull `at line:col` out of lexer/parser error strings.
pub fn diagnostic_from_message(file: &str, err: &str) -> Diagnostic {
    if let Some(idx) = err.rfind(" at ") {
        let after = &err[idx + 4..];
        let loc_end = after
            .find(|c: char| !c.is_ascii_digit() && c != ':')
            .unwrap_or(after.len());
        let loc = &after[..loc_end];
        if let Some((l, c)) = loc.split_once(':') {
            if let (Ok(line), Ok(col)) = (l.parse::<u32>(), c.parse::<u32>()) {
                let mut message = err[..idx].trim().to_string();
                let rest = after[loc_end..].trim();
                if !rest.is_empty() {
                    if !message.is_empty() {
                        message.push(' ');
                    }
                    message.push_str(rest);
                }
                return Diagnostic::error(file, Span { line, col }, message);
            }
        }
    }
    Diagnostic::error(file, Span { line: 1, col: 1 }, err)
}

/// Lex, parse, and typecheck. Does not evaluate. Empty vec means the file is clean.
pub fn check_source(source: &str, file: &str) -> Vec<Diagnostic> {
    check_source_with_resolver(source, file, None)
}

/// Typecheck with in-memory modules (`import utils` → `modules["utils"]` or `utils.rg`).
pub fn check_source_with_modules(
    source: &str,
    file: &str,
    modules: HashMap<String, String>,
) -> Vec<Diagnostic> {
    let resolver = HashMapResolver::new(modules);
    check_source_with_resolver(source, file, Some(&resolver))
}

/// Typecheck a file on disk, resolving sibling `.rg` imports from its directory.
pub fn check_file(path: &Path) -> Vec<Diagnostic> {
    let label = path_label(path);
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return vec![Diagnostic::error(
                &label,
                Span { line: 1, col: 1 },
                format!("failed to read {}: {e}", path.display()),
            )];
        }
    };
    check_source_at(&source, path)
}

/// Typecheck `source` as if it lived at `path` (stdin / unsaved buffers).
pub fn check_source_at(source: &str, path: &Path) -> Vec<Diagnostic> {
    let label = path_label(path);
    let base = path.parent().unwrap_or(Path::new("."));
    let resolver = FileModuleResolver::new(base);
    check_source_with_resolver(source, &label, Some(&resolver))
}

fn path_label(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("script.rg")
        .to_string()
}

/// Neighbor `.rg` files for `hover_at` / `def_at` (`utils` / `utils.rg`,
/// and `util.math` for `util/math.rg`).
pub fn sibling_modules(dir: &Path, skip_file: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    walk_rg_modules(dir, dir, skip_file, 0, &mut map);
    map
}

fn walk_rg_modules(
    dir: &Path,
    root: &Path,
    skip_file: Option<&str>,
    depth: usize,
    map: &mut HashMap<String, String>,
) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "target" | "node_modules") {
                continue;
            }
            walk_rg_modules(&path, root, skip_file, depth + 1, map);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rg") {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if skip_file.is_some_and(|s| s.eq_ignore_ascii_case(&name)) && dir == root {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let stem = name.strip_suffix(".rg").unwrap_or(&name).to_string();
        map.insert(name, src.clone());
        map.insert(stem.clone(), src.clone());
        if let Ok(rel) = path.strip_prefix(root) {
            let dotted = rel
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ".");
            if dotted != stem {
                map.insert(dotted, src);
            }
        }
    }
}

fn check_source_with_resolver(
    source: &str,
    file: &str,
    resolver: Option<&dyn ModuleResolver>,
) -> Vec<Diagnostic> {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(t) => t,
        Err(e) => return vec![diagnostic_from_message(file, &e)],
    };
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => return vec![diagnostic_from_message(file, &e)],
    };
    let mut diags = typecheck::typecheck_diagnostics_with(&program, resolver);
    for d in &mut diags {
        if d.file.is_empty() {
            d.file = file.to_string();
        }
    }
    diags
}

pub struct RunResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub message: String,
    pub effects: Vec<HostEffect>,
}

impl RunResult {
    fn fail(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            stdout: String::new(),
            stderr: message.clone(),
            message,
            effects: Vec::new(),
        }
    }
}

/// Lex, parse, and typecheck without evaluating. Hosts cache this across ticks.
pub fn compile_source(source: &str) -> Result<Vec<Item>, String> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    typecheck::typecheck(&program)?;
    Ok(program)
}

fn run_with_context(source: &str, ctx: &mut EvalContext) -> RunResult {
    let program = match compile_source(source) {
        Ok(program) => program,
        Err(e) => return RunResult::fail(e),
    };
    match ctx.run(&program) {
        Ok(_) => RunResult {
            ok: true,
            stdout: ctx.stdout.clone(),
            stderr: String::new(),
            message: "RoseGold finished".to_string(),
            effects: ctx.take_effects(),
        },
        Err(e) => {
            let msg = e.to_string();
            RunResult {
                ok: false,
                stdout: ctx.stdout.clone(),
                stderr: msg.clone(),
                message: msg,
                effects: ctx.take_effects(),
            }
        }
    }
}

pub fn run_source(source: &str) -> RunResult {
    let mut ctx = EvalContext::new();
    run_with_context(source, &mut ctx)
}

pub fn run_source_with_modules(source: &str, modules: HashMap<String, String>) -> RunResult {
    let resolver = Rc::new(RefCell::new(HashMapResolver::new(modules)));
    let mut ctx = EvalContext::with_resolver(resolver);
    run_with_context(source, &mut ctx)
}

/// Script-tab Run: construct a `@node` / class with `on_ready`/`on_create`, or
/// call a free `on_ready`. Does not require `main`.
pub fn run_preview(source: &str, name: &str, x: f64, y: f64) -> RunResult {
    run_preview_with(source, name, x, y, EvalContext::new())
}

pub fn run_preview_with_modules(
    source: &str,
    name: &str,
    x: f64,
    y: f64,
    modules: HashMap<String, String>,
) -> RunResult {
    let resolver = Rc::new(RefCell::new(HashMapResolver::new(modules)));
    run_preview_with(source, name, x, y, EvalContext::with_resolver(resolver))
}

fn run_preview_with(source: &str, name: &str, x: f64, y: f64, mut ctx: EvalContext) -> RunResult {
    let program = match compile_source(source) {
        Ok(program) => program,
        Err(e) => return RunResult::fail(e),
    };
    if let Err(e) = ctx.load_program(&program) {
        let msg = e.to_string();
        return RunResult {
            ok: false,
            stdout: ctx.stdout.clone(),
            stderr: msg.clone(),
            message: msg,
            effects: ctx.take_effects(),
        };
    }
    if let Err(e) = ctx.adopt_preview_class(&program) {
        let msg = e.to_string();
        return RunResult {
            ok: false,
            stdout: ctx.stdout.clone(),
            stderr: msg.clone(),
            message: msg,
            effects: ctx.take_effects(),
        };
    }
    match ctx.run_ready_preview(name, x, y) {
        Ok(_) => RunResult {
            ok: true,
            stdout: ctx.stdout.clone(),
            stderr: String::new(),
            message: "RoseGold finished".to_string(),
            effects: ctx.take_effects(),
        },
        Err(e) => {
            let msg = e.to_string();
            RunResult {
                ok: false,
                stdout: ctx.stdout.clone(),
                stderr: msg.clone(),
                message: msg,
                effects: ctx.take_effects(),
            }
        }
    }
}

pub fn run_file(path: &Path) -> RunResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return RunResult::fail(format!("failed to read file: {}", e)),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let resolver = Rc::new(RefCell::new(FileModuleResolver::new(base)));
    let mut ctx = EvalContext::with_resolver(resolver);
    run_with_context(&source, &mut ctx)
}

/// Run all `@test` functions in `source`. Does not call `main`.
pub fn run_tests(source: &str) -> RunResult {
    let resolver: Rc<RefCell<dyn ModuleResolver>> =
        Rc::new(RefCell::new(HashMapResolver::new(HashMap::new())));
    run_tests_with_resolver(source, resolver)
}

/// Same as `run_tests`, with in-memory `{ "utils": "…" }` modules.
pub fn run_tests_with_modules(source: &str, modules: HashMap<String, String>) -> RunResult {
    let resolver: Rc<RefCell<dyn ModuleResolver>> =
        Rc::new(RefCell::new(HashMapResolver::new(modules)));
    run_tests_with_resolver(source, resolver)
}

/// Run `@test` functions in a file, resolving sibling `.rg` imports from its directory.
pub fn run_tests_file(path: &Path) -> RunResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return RunResult::fail(format!("failed to read file: {}", e)),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let resolver: Rc<RefCell<dyn ModuleResolver>> =
        Rc::new(RefCell::new(FileModuleResolver::new(base)));
    run_tests_with_resolver(&source, resolver)
}

fn run_tests_with_resolver(source: &str, resolver: Rc<RefCell<dyn ModuleResolver>>) -> RunResult {
    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(e) => return RunResult::fail(e),
    };
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => return RunResult::fail(e),
    };
    {
        let r = resolver.borrow();
        if let Some(d) = typecheck::typecheck_diagnostics_with(&program, Some(&*r))
            .into_iter()
            .next()
        {
            return RunResult::fail(format!("type error at {}:{}: {}", d.line, d.col, d.message));
        }
    }

    let tests: Vec<_> = program
        .iter()
        .filter_map(|item| match item {
            Item::FnDecl(f) if f.is_test => Some(f.name.clone()),
            _ => None,
        })
        .collect();

    if tests.is_empty() {
        return RunResult {
            ok: true,
            stdout: String::new(),
            stderr: String::new(),
            message: "no @test functions".into(),
            effects: Vec::new(),
        };
    }

    let mut ctx = EvalContext::with_resolver(resolver);
    if let Err(e) = ctx.load_program(&program) {
        let msg = e.to_string();
        return RunResult {
            ok: false,
            stdout: ctx.stdout.clone(),
            stderr: msg.clone(),
            message: msg,
            effects: ctx.take_effects(),
        };
    }

    let mut failed = 0usize;
    for name in &tests {
        match ctx.call(name, vec![]) {
            Ok(_) => {
                ctx.stdout.push_str(&format!("ok {name}\n"));
            }
            Err(e) => {
                failed += 1;
                ctx.stdout.push_str(&format!("FAIL {name}: {e}\n"));
            }
        }
    }

    let total = tests.len();
    let passed = total - failed;
    let summary = format!("{passed}/{total} tests passed");
    ctx.stdout.push_str(&summary);
    ctx.stdout.push('\n');
    RunResult {
        ok: failed == 0,
        stdout: ctx.stdout.clone(),
        stderr: if failed == 0 {
            String::new()
        } else {
            summary.clone()
        },
        message: summary,
        effects: ctx.take_effects(),
    }
}

#[cfg(test)]
mod tests;
