//! Host/stdlib arities, return types, and `@node` hook name suggestions.

use crate::lexer::Lexer;
use crate::parser::*;

pub(super) fn native_method_arity(ty: &str, name: &str) -> Option<usize> {
    Some(match (ty, name) {
        ("String" | "Str", "len") => 0,
        ("Array", "len" | "pop" | "first" | "last") => 0,
        ("Array", "push" | "contains") => 1,
        ("Map", "len" | "keys") => 0,
        ("Map", "has" | "remove") => 1,
        ("Map", "insert") => 2,
        ("Option", "is_some" | "is_none" | "unwrap") => 0,
        ("Option", "unwrap_or") => 1,
        ("Result", "is_ok" | "is_err" | "unwrap") => 0,
        ("Result", "unwrap_or") => 1,
        _ => return None,
    })
}

pub(super) fn host_return_type(module: &str, name: &str) -> Option<&'static str> {
    Some(match (module, name) {
        (
            "io",
            "read_text" | "read_lines" | "write_text" | "append_text" | "remove" | "mkdir"
            | "list_dir",
        ) => "Result",
        ("io", "exists" | "is_dir") => "Bool",
        ("input", "pressed" | "held") => "Bool",
        ("time", "now" | "elapsed") => "Float",
        _ => return None,
    })
}

pub(super) fn parse_module_source(source: &str) -> Result<Vec<Item>, String> {
    let tokens = Lexer::new(source).tokenize()?;
    Parser::new(tokens).parse()
}

pub fn is_stdlib_module(name: &str) -> bool {
    crate::stdlib::is_host_module(name) || crate::stdlib::is_embedded_stdlib(name)
}

const NODE_HOOKS: &[&str] = &[
    "on_create",
    "on_ready",
    "on_update",
    "on_destroy",
    "on_enter",
    "on_exit",
];

pub(super) fn is_node_hook(name: &str) -> bool {
    NODE_HOOKS.contains(&name)
}

/// One insertion, deletion, substitution, or adjacent transposition from a known hook.
pub(super) fn suggest_node_hook(name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for hook in NODE_HOOKS {
        let dist = damerau_levenshtein(name, hook);
        if dist == 0 || dist > 1 {
            continue;
        }
        match best {
            Some((_, d)) if d <= dist => {}
            _ => best = Some((*hook, dist)),
        }
    }
    best.map(|(hook, _)| hook)
}

fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n.abs_diff(m) > 1 {
        return 2;
    }
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        dp[i][0] = i;
    }
    for j in 0..=m {
        dp[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                dp[i][j] = dp[i][j].min(dp[i - 2][j - 2] + 1);
            }
        }
    }
    dp[n][m]
}

/// Known native host / primitive arities. Public math/str/checks/option come from `.rg`.
pub fn stdlib_arity(module: &str, name: &str) -> Option<usize> {
    Some(match (module, name) {
        (
            "io",
            "read_text" | "read_lines" | "exists" | "remove" | "mkdir" | "list_dir" | "is_dir",
        ) => 1,
        ("io", "write_text" | "append_text") => 2,
        ("time", "now" | "elapsed") => 0,
        ("ui", "text") => 3,
        ("strata", "rot" | "play_sound" | "spawn") => 1,
        ("strata", "move" | "set" | "after") => 2,
        ("input", "pressed" | "held") => 1,
        ("__math", "sin" | "cos" | "sqrt" | "to_int" | "to_float") => 1,
        ("__math", "pow" | "atan2") => 2,
        ("__str", "contains" | "starts_with" | "ends_with" | "repeat" | "split") => 2,
        ("__str", "length" | "is_empty" | "upper" | "lower" | "trim") => 1,
        ("__str", "slice") => 3,
        ("Array", "first" | "last") => 1,
        ("Array", "contains") => 2,
        _ => return None,
    })
}
