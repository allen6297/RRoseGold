//! Host/stdlib arities and return types.

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
        ("Mutex", "lock" | "unlock") => 0,
        ("Channel", "send") => 1,
        ("Channel", "recv" | "close") => 0,
        ("Channel", "recv_timeout") => 1,
        ("Task", "wait") => 1,
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
            | "list_dir" | "read_stdin",
        ) => "Result",
        ("io", "exists" | "is_dir") => "Bool",
        ("io", "read_line") => "Option",
        ("time", "now" | "elapsed") => "Float",
        ("process", "argv") => "Array",
        ("process", "env") => "Option",
        ("process", "exit") => "Void",
        ("process", "run") => "Result",
        ("json", "parse" | "stringify") => "Result",
        ("path", "join" | "dirname" | "ext") => "String",
        ("http", "get" | "post") => "Result",
        ("regex", "is_match" | "find") => "Result",
        ("__ui", "theme" | "alert" | "window" | "begin" | "end" | "widget" | "bind" | "pump") => {
            "Void"
        }
        ("__ui", "theme_get") => "Map",
        ("__ui", "last_alert") => "Option",
        ("__ui", "run") => "Int",
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

/// Known native host / primitive arities. Public math/str/checks/option come from `.rg`.
pub fn stdlib_arity(module: &str, name: &str) -> Option<usize> {
    Some(match (module, name) {
        (
            "io",
            "read_text" | "read_lines" | "exists" | "remove" | "mkdir" | "list_dir" | "is_dir",
        ) => 1,
        ("io", "write_text" | "append_text") => 2,
        ("io", "read_stdin" | "read_line") => 0,
        ("time", "now" | "elapsed") => 0,
        ("process", "argv") => 0,
        ("process", "env" | "exit") => 1,
        ("process", "run") => 2,
        ("json", "parse" | "stringify") => 1,
        ("path", "join") => 2,
        ("path", "dirname" | "ext") => 1,
        ("http", "get") => 1,
        ("http", "post") => 2,
        ("regex", "is_match" | "find") => 2,
        ("__math", "sin" | "cos" | "sqrt" | "to_int" | "to_float") => 1,
        ("__math", "pow" | "atan2") => 2,
        ("__str", "contains" | "starts_with" | "ends_with" | "repeat" | "split") => 2,
        ("__str", "length" | "is_empty" | "upper" | "lower" | "trim") => 1,
        ("__str", "slice") => 3,
        ("__ui", "theme" | "alert" | "begin" | "widget") => 1,
        ("__ui", "window" | "bind") => 2,
        ("__ui", "end" | "run" | "pump" | "last_alert" | "theme_get") => 0,
        ("Array", "first" | "last") => 1,
        ("Array", "contains") => 2,
        _ => return None,
    })
}
