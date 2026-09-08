//! RoseGold: lexer, parser, typecheck, tree-walking interpreter, and a stack VM.
//!
//! Use [`compile_source`] / [`run_source`] / [`EvalContext`]. Editor metadata
//! lives in [`signal`] and [`navigate`].

pub mod bytecode;
pub mod format;
pub mod interpreter;
pub mod lexer;
pub mod navigate;
pub mod parser;
pub mod repl;
pub mod signal;
pub mod stdlib;
pub mod typecheck;
pub mod vendor;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub use format::format_source;
pub use interpreter::{
    CombinedResolver, EvalContext, FileModuleResolver, HashMapResolver, Module, ModuleResolver,
    ResolverRef, Value,
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

impl Span {
    pub fn is_unknown(self) -> bool {
        self.line == 0 && self.col == 0
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// One caller on a runtime stack: the function entered, the call site, and that file.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceFrame {
    pub name: String,
    pub span: Span,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
    /// Set by `process.exit` — not a crash.
    pub exit_code: Option<i32>,
    pub trace: Vec<TraceFrame>,
    /// Source file of the error site (`helpers.rg`), empty when unknown.
    pub file: String,
}

impl RuntimeError {
    pub fn is_exit(&self) -> Option<i32> {
        self.exit_code
    }
}

fn write_loc(f: &mut std::fmt::Formatter<'_>, file: &str, span: Span) -> std::fmt::Result {
    if file.is_empty() {
        write!(f, "{span}")
    } else {
        write!(f, "{file}:{span}")
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = self.exit_code {
            return write!(f, "exited with {code}");
        }
        write!(f, "runtime error at ")?;
        write_loc(f, &self.file, self.span)?;
        write!(f, ": {}", self.message)?;
        for frame in &self.trace {
            write!(f, "\n  from {}", frame.name)?;
            if frame.span.is_unknown() {
                if !frame.file.is_empty() {
                    write!(f, " at {}", frame.file)?;
                }
            } else {
                write!(f, " at ")?;
                write_loc(f, &frame.file, frame.span)?;
            }
        }
        Ok(())
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
                map.insert(dotted, src.clone());
            }
            if let Some(alias) = vendor_package_alias(rel) {
                map.entry(alias.clone()).or_insert_with(|| src.clone());
                map.entry(format!("{alias}.rg")).or_insert(src);
            }
        }
    }
}

/// `vendor/httpclient/lib.rg` → `httpclient`; `vendor/httpclient/parse.rg` → `httpclient.parse`.
fn vendor_package_alias(rel: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        parts.push(c.as_os_str().to_str()?.to_string());
    }
    if parts.first().map(|s| s.as_str()) != Some("vendor") {
        return None;
    }
    parts.remove(0);
    if parts.is_empty() {
        return None;
    }
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = last.strip_suffix(".rg") {
            *last = stem.to_string();
        }
    }
    if parts.len() == 1 {
        return Some(parts.remove(0));
    }
    if matches!(parts.last().map(|s| s.as_str()), Some("lib" | "main")) {
        parts.pop();
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
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
    pub exit_code: i32,
}

impl RunResult {
    fn fail(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            stdout: String::new(),
            stderr: message.clone(),
            message,
            exit_code: 1,
        }
    }

    fn from_eval(ctx: &EvalContext, result: Result<Value, RuntimeError>) -> Self {
        match result {
            Ok(_) => Self {
                ok: true,
                stdout: ctx.stdout(),
                stderr: String::new(),
                message: "RoseGold finished".to_string(),
                exit_code: 0,
            },
            Err(e) => {
                if let Some(code) = e.exit_code {
                    Self {
                        ok: code == 0,
                        stdout: ctx.stdout(),
                        stderr: String::new(),
                        message: if code == 0 {
                            "RoseGold finished".to_string()
                        } else {
                            format!("exited with {code}")
                        },
                        exit_code: code,
                    }
                } else {
                    let msg = e.to_string();
                    Self {
                        ok: false,
                        stdout: ctx.stdout(),
                        stderr: msg.clone(),
                        message: msg,
                        exit_code: 1,
                    }
                }
            }
        }
    }
}

/// Lex, parse, and typecheck without evaluating.
pub fn compile_source(source: &str) -> Result<Vec<Item>, String> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    typecheck::typecheck(&program)?;
    Ok(program)
}

fn typecheck_error_message(d: &Diagnostic) -> String {
    if d.file.is_empty() {
        format!("type error at {}:{}: {}", d.line, d.col, d.message)
    } else {
        format!(
            "type error at {}:{}:{}: {}",
            d.file, d.line, d.col, d.message
        )
    }
}

fn run_with_context(source: &str, ctx: &mut EvalContext) -> RunResult {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(t) => t,
        Err(e) => return RunResult::fail(e),
    };
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => return RunResult::fail(e),
    };
    {
        let resolver = ctx.resolver();
        let r = resolver.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(d) = typecheck::typecheck_diagnostics_with(&program, Some(&*r))
            .into_iter()
            .next()
        {
            return RunResult::fail(typecheck_error_message(&d));
        }
        let compiled = crate::bytecode::compile_program_with(&program, Some(&*r));
        drop(r);
        if let Ok(proto) = compiled {
            let result = crate::bytecode::interpret_in(ctx, &proto, Vec::new());
            return RunResult::from_eval(ctx, result);
        }
    }
    let result = ctx.run(&program);
    RunResult::from_eval(ctx, result)
}

pub fn run_source(source: &str) -> RunResult {
    let mut ctx = EvalContext::new();
    run_with_context(source, &mut ctx)
}

/// Same as [`run_source`], with `process.argv()` set to `argv`.
pub fn run_source_with_argv(source: &str, argv: Vec<String>) -> RunResult {
    let mut ctx = EvalContext::new();
    ctx.set_argv(argv);
    run_with_context(source, &mut ctx)
}

pub fn run_source_with_modules(source: &str, modules: HashMap<String, String>) -> RunResult {
    let resolver = Arc::new(Mutex::new(HashMapResolver::new(modules)));
    let mut ctx = EvalContext::with_resolver(resolver);
    run_with_context(source, &mut ctx)
}

pub fn run_file(path: &Path) -> RunResult {
    run_file_with_argv(path, vec![path.display().to_string()])
}

/// Run a file with `process.argv()` set (script path should be `argv[0]`).
pub fn run_file_with_argv(path: &Path, argv: Vec<String>) -> RunResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return RunResult::fail(format!("failed to read file: {}", e)),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let resolver = Arc::new(Mutex::new(FileModuleResolver::new(base)));
    let mut ctx = EvalContext::with_resolver(resolver);
    ctx.set_argv(argv);
    ctx.set_source_file(source_label(path));
    run_with_context(&source, &mut ctx)
}

/// Run all `@test` functions in `source`. Does not call `main`.
pub fn run_tests(source: &str) -> RunResult {
    let resolver: ResolverRef = Arc::new(Mutex::new(HashMapResolver::new(HashMap::new())));
    run_tests_with_resolver(&source, resolver, String::new())
}

/// Same as `run_tests`, with in-memory `{ "utils": "…" }` modules.
pub fn run_tests_with_modules(source: &str, modules: HashMap<String, String>) -> RunResult {
    let resolver: ResolverRef = Arc::new(Mutex::new(HashMapResolver::new(modules)));
    run_tests_with_resolver(source, resolver, String::new())
}

/// Run `@test` functions in a file, resolving sibling `.rg` imports from its directory.
pub fn run_tests_file(path: &Path) -> RunResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return RunResult::fail(format!("failed to read file: {}", e)),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let resolver: ResolverRef = Arc::new(Mutex::new(FileModuleResolver::new(base)));
    run_tests_with_resolver(&source, resolver, source_label(path))
}

fn source_label(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("script.rg")
        .to_string()
}

fn run_tests_with_resolver(source: &str, resolver: ResolverRef, file: String) -> RunResult {
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
        let r = resolver.lock().unwrap_or_else(|p| p.into_inner());
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
            exit_code: 0,
        };
    }

    let mut ctx = EvalContext::with_resolver(resolver);
    ctx.set_source_file(file);
    if let Err(e) = ctx.load_program(&program) {
        let msg = e.to_string();
        return RunResult {
            ok: false,
            stdout: ctx.stdout(),
            stderr: msg.clone(),
            message: msg,
            exit_code: 1,
        };
    }

    let mut failed = 0usize;
    for name in &tests {
        match ctx.call(name, vec![]) {
            Ok(_) => {
                ctx.append_stdout(&format!("ok {name}\n"));
            }
            Err(e) => {
                failed += 1;
                ctx.append_stdout(&format!("FAIL {name}: {e}\n"));
            }
        }
    }

    let total = tests.len();
    let passed = total - failed;
    let summary = format!("{passed}/{total} tests passed");
    ctx.append_stdout(&summary);
    ctx.append_stdout("\n");
    RunResult {
        ok: failed == 0,
        stdout: ctx.stdout(),
        stderr: if failed == 0 {
            String::new()
        } else {
            summary.clone()
        },
        message: summary,
        exit_code: if failed == 0 { 0 } else { 1 },
    }
}

#[cfg(test)]
mod tests;
