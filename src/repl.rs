//! Line-oriented eval for the CLI REPL. Typechecks each line against prior bindings.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::Diagnostic;
use crate::interpreter::{CombinedResolver, EvalContext, ResolverRef, Value};
use crate::lexer::Lexer;
use crate::parser::{Item, Parser, parse_expr_from_str};
use crate::typecheck::{typecheck_diagnostics_with, typecheck_expr_diagnostics_with};

#[derive(Debug)]
pub enum LineResult {
    Empty,
    Silent,
    Value(Value),
    Exit(i32),
    Error(String),
}

/// REPL state: persistent eval plus items accepted so far (for typecheck).
pub struct Session {
    pub ctx: EvalContext,
    items: Vec<Item>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self::with_context(EvalContext::new())
    }

    /// Resolve `import` against the process working directory.
    pub fn with_cwd() -> Self {
        let cwd = std::env::current_dir().ok();
        let resolver: ResolverRef =
            Arc::new(Mutex::new(CombinedResolver::new(HashMap::new(), cwd)));
        let mut ctx = EvalContext::with_resolver(resolver);
        ctx.set_argv(vec!["<repl>".into()]);
        Self::with_context(ctx)
    }

    pub fn with_context(ctx: EvalContext) -> Self {
        Self {
            ctx,
            items: Vec::new(),
        }
    }
}

/// Unclosed `(`, `[`, `{`, or string → keep reading.
pub fn is_complete(source: &str) -> bool {
    let mut depth: i32 = 0;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                while let Some(n) = chars.peek() {
                    if *n == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '"' | '\'' => {
                let q = c;
                let mut closed = false;
                while let Some(n) = chars.next() {
                    if n == '\\' {
                        chars.next();
                        continue;
                    }
                    if n == q {
                        closed = true;
                        break;
                    }
                    if n == '\n' {
                        break;
                    }
                }
                if !closed {
                    return false;
                }
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

pub fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s.escape_default()),
        other => other.to_string(),
    }
}

pub fn eval_line(session: &mut Session, source: &str) -> LineResult {
    let source = source.trim();
    if source.is_empty() {
        return LineResult::Empty;
    }
    try_eval(session, source)
}

fn map_eval(err: crate::RuntimeError) -> LineResult {
    if let Some(code) = err.exit_code {
        LineResult::Exit(code)
    } else {
        LineResult::Error(err.to_string())
    }
}

fn first_type_error(diags: &[Diagnostic]) -> LineResult {
    let d = &diags[0];
    LineResult::Error(format!("type error at {}:{}: {}", d.line, d.col, d.message))
}

fn check_with_session(session: &Session, extra: &[Item]) -> Vec<Diagnostic> {
    let mut program = session.items.clone();
    program.extend(extra.iter().cloned());
    let resolver = session.ctx.resolver();
    let borrowed = resolver.lock().unwrap_or_else(|p| p.into_inner());
    typecheck_diagnostics_with(&program, Some(&*borrowed))
}

fn check_expr_with_session(session: &Session, expr: &crate::parser::Expr) -> Vec<Diagnostic> {
    let resolver = session.ctx.resolver();
    let borrowed = resolver.lock().unwrap_or_else(|p| p.into_inner());
    typecheck_expr_diagnostics_with(&session.items, expr, Some(&*borrowed))
}

fn try_eval(session: &mut Session, source: &str) -> LineResult {
    if looks_like_item(source) {
        return eval_items(session, source);
    }
    match parse_expr_line(source) {
        Ok(expr) => {
            let diags = check_expr_with_session(session, &expr);
            if !diags.is_empty() {
                return first_type_error(&diags);
            }
            match session.ctx.eval_expr(&expr) {
                Ok(Value::Void) => LineResult::Silent,
                Ok(value) => LineResult::Value(value),
                Err(e) => map_eval(e),
            }
        }
        Err(expr_err) => match eval_items(session, source) {
            LineResult::Error(item_err) if item_err.contains("expected top-level") => {
                LineResult::Error(expr_err)
            }
            other => other,
        },
    }
}

fn eval_items(session: &mut Session, source: &str) -> LineResult {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(t) => t,
        Err(e) => return LineResult::Error(e),
    };
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => return LineResult::Error(e),
    };
    if program.is_empty() {
        return LineResult::Empty;
    }
    let diags = check_with_session(session, &program);
    if !diags.is_empty() {
        return first_type_error(&diags);
    }
    match session.ctx.load_program(&program) {
        Ok(()) => {
            session.items.extend(program);
            LineResult::Silent
        }
        Err(e) => map_eval(e),
    }
}

fn parse_expr_line(source: &str) -> Result<crate::parser::Expr, String> {
    let trimmed = source.trim().trim_end_matches(';').trim();
    parse_expr_from_str(trimmed)
}

fn looks_like_item(source: &str) -> bool {
    let t = source.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t).trim_start();
    let t = t
        .strip_prefix('@')
        .map(|rest| {
            rest.split_once(|c: char| c.is_whitespace())
                .map(|(_, r)| r.trim_start())
                .unwrap_or("")
        })
        .unwrap_or(t);
    [
        "import ", "from ", "fn ", "async ", "var ", "const ", "struct ", "class ", "trait ",
        "enum ", "impl ", "mod ", "signal ",
    ]
    .iter()
    .any(|p| t.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::{EvalContext, HashMapResolver, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn incomplete_fn_body() {
        assert!(!is_complete("fn foo() {"));
        assert!(is_complete("fn foo() { return 1; }"));
        assert!(!is_complete("print(\"hi"));
        assert!(is_complete("print(\"hi\")"));
    }

    #[test]
    fn expr_and_binding_persist() {
        let mut session = Session::new();
        match eval_line(&mut session, "1 + 2") {
            LineResult::Value(Value::Int(3)) => {}
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "var x: Int = 10;") {
            LineResult::Silent => {}
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "x") {
            LineResult::Value(Value::Int(10)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typecheck_rejects_undefined_and_keeps_session() {
        let mut session = Session::new();
        match eval_line(&mut session, "y") {
            LineResult::Error(e) => assert!(e.contains("undefined"), "{e}"),
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "var x: Int = 1;") {
            LineResult::Silent => {}
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "x") {
            LineResult::Value(Value::Int(1)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn host_import_then_call() {
        let mut session = Session::new();
        match eval_line(&mut session, "json.parse(\"1\")") {
            LineResult::Error(e) => assert!(e.contains("undefined") || e.contains("json"), "{e}"),
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "import json;") {
            LineResult::Silent => {}
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "json.parse(\"1\")") {
            LineResult::Value(v) => {
                let s = format_value(&v);
                assert!(s.contains("1"), "{s}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn import_user_module_from_resolver() {
        let mut modules = HashMap::new();
        modules.insert(
            "util".into(),
            "mod util {\n    pub fn add(a: Int, b: Int): Int { return a + b; }\n}\n".into(),
        );
        let resolver: ResolverRef = Arc::new(Mutex::new(HashMapResolver::new(modules)));
        let mut session = Session::with_context(EvalContext::with_resolver(resolver));
        match eval_line(&mut session, "import util;") {
            LineResult::Silent => {}
            other => panic!("{other:?}"),
        }
        match eval_line(&mut session, "util.add(2, 3)") {
            LineResult::Value(Value::Int(5)) => {}
            other => panic!("{other:?}"),
        }
    }
}
