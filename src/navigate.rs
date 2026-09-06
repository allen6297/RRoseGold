//! Hover and go-to-definition at a source `line:col` ([`hover_at`], [`def_at`]).

use std::collections::HashMap;

use crate::Span;
use crate::interpreter::{HashMapResolver, ModuleResolver};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::{FnDecl, Item, Parser, SignalDecl, Type};

/// Definition / hover payload for a name at a source position.
/// `file` is the module key (`utils` / `utils.rg`) or the current file label.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Hover and go-to-def share this query. Crate stdlib *uses* (`math.sin`,
/// `Sprite`) resolve into `stdlib/*.rg`. Host APIs (`strata.move`) stay
/// `None` so the editor catalog can describe them. Import *sites*
/// (`import utils`, `from utils import move_line`) resolve to the module file.
pub fn symbol_at(
    source: &str,
    file: &str,
    line: u32,
    col: u32,
    modules: HashMap<String, String>,
) -> Option<SymbolInfo> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    if let Some(info) = import_site_at(&tokens, line, col, &modules) {
        return Some(info);
    }
    let ident = ident_at(&tokens, line, col)?;
    let program = Parser::new(tokens.clone()).parse().ok()?;
    let binds = import_binds(&program);

    if let Some(module) = ident.qualifier {
        let canonical = binds
            .iter()
            .find(|b| b.bind == module && b.item.is_none())
            .map(|b| b.canonical.as_str())
            .unwrap_or(module.as_str());
        if let Some(info) = export_in_module(canonical, &ident.name, &modules) {
            return Some(info);
        }
        return None;
    }

    if let Some(bind) = binds.iter().find(|b| b.bind == ident.name) {
        if crate::stdlib::is_host_module(&bind.canonical) {
            return None;
        }
        if let Some(item) = &bind.item {
            return export_in_module(&bind.canonical, item, &modules);
        }
        if crate::stdlib::node_kind(&ident.name).is_some() {
            return export_in_module("node", &ident.name, &modules)
                .or_else(|| export_in_module("strata", &ident.name, &modules));
        }
        return module_symbol(&bind.canonical, &modules);
    }

    local_symbol(source, file, &ident.name).or_else(|| prelude_symbol(&ident.name, &modules))
}

fn prelude_symbol(name: &str, modules: &HashMap<String, String>) -> Option<SymbolInfo> {
    if crate::stdlib::node_kind(name).is_some() {
        return export_in_module("node", name, modules)
            .or_else(|| export_in_module("strata", name, modules));
    }
    let stem = crate::stdlib::canonical_module(name)?;
    export_in_module(stem, name, modules)
}

pub fn hover_at(
    source: &str,
    file: &str,
    line: u32,
    col: u32,
    modules: HashMap<String, String>,
) -> Option<SymbolInfo> {
    symbol_at(source, file, line, col, modules)
}

pub fn def_at(
    source: &str,
    file: &str,
    line: u32,
    col: u32,
    modules: HashMap<String, String>,
) -> Option<SymbolInfo> {
    symbol_at(source, file, line, col, modules)
}

struct IdentAt {
    name: String,
    qualifier: Option<String>,
}

struct ImportBind {
    bind: String,
    canonical: String,
    item: Option<String>,
}

fn token_covers(tok: &Token, line: u32, col: u32) -> bool {
    if tok.span.line != line {
        return false;
    }
    let len = tok.text.chars().count() as u32;
    if len == 0 {
        return false;
    }
    let start = tok.span.col;
    // Half-open so `.sin` at the first letter of `sin` is not stolen by `.`.
    col >= start && col < start + len
}

/// Go-to on `import util.math`, `from utils import move_line`, or the `import`/`from` keyword.
fn import_site_at(
    tokens: &[Token],
    line: u32,
    col: u32,
    modules: &HashMap<String, String>,
) -> Option<SymbolInfo> {
    let idx = tokens.iter().position(|t| token_covers(t, line, col))?;
    let tok = &tokens[idx];
    if !matches!(
        tok.kind,
        TokenKind::Import | TokenKind::From | TokenKind::Ident(_) | TokenKind::As | TokenKind::Dot
    ) {
        return None;
    }
    let mut start = idx;
    while start > 0 && tokens[start - 1].kind != TokenKind::Semicolon {
        start -= 1;
    }
    let end = tokens[idx..]
        .iter()
        .position(|t| t.kind == TokenKind::Semicolon)
        .map(|off| idx + off)
        .unwrap_or(tokens.len().saturating_sub(1));
    let stmt = &tokens[start..=end.min(tokens.len().saturating_sub(1))];
    let first = stmt.first()?;
    let is_from = first.kind == TokenKind::From;
    if !is_from && first.kind != TokenKind::Import {
        return None;
    }

    let mut path: Vec<String> = Vec::new();
    let mut item: Option<String> = None;
    let mut alias: Option<String> = None;
    let mut i = 1;
    if is_from {
        while i < stmt.len() {
            match &stmt[i].kind {
                TokenKind::Ident(n) if item.is_none() => path.push(n.clone()),
                TokenKind::Dot if item.is_none() => {}
                TokenKind::Import => {
                    i += 1;
                    if let Some(TokenKind::Ident(n)) = stmt.get(i).map(|t| &t.kind) {
                        item = Some(n.clone());
                    }
                    break;
                }
                _ => break,
            }
            i += 1;
        }
        i += 1;
        while i < stmt.len() {
            match &stmt[i].kind {
                TokenKind::As => {
                    if let Some(TokenKind::Ident(n)) = stmt.get(i + 1).map(|t| &t.kind) {
                        alias = Some(n.clone());
                    }
                    break;
                }
                TokenKind::Semicolon => break,
                _ => {
                    i += 1;
                    continue;
                }
            }
        }
    } else {
        while i < stmt.len() {
            match &stmt[i].kind {
                TokenKind::Ident(n) if alias.is_none() => {
                    if stmt
                        .get(i.saturating_sub(1))
                        .is_some_and(|p| p.kind == TokenKind::As)
                    {
                        alias = Some(n.clone());
                    } else {
                        path.push(n.clone());
                    }
                }
                TokenKind::Dot => {}
                TokenKind::As => {
                    if let Some(TokenKind::Ident(n)) = stmt.get(i + 1).map(|t| &t.kind) {
                        alias = Some(n.clone());
                    }
                    break;
                }
                TokenKind::Semicolon => break,
                _ => break,
            }
            i += 1;
        }
    }
    if path.is_empty() {
        return None;
    }
    let canonical = path.join(".");
    let covering_ident = match &tok.kind {
        TokenKind::Ident(n) => Some(n.as_str()),
        _ => None,
    };
    let last = path.last().unwrap().clone();
    let on_from_item = is_from
        && covering_ident
            .is_some_and(|n| item.as_deref() == Some(n) || alias.as_deref() == Some(n));
    let on_dotted_item = !is_from
        && path.len() >= 2
        && covering_ident.is_some_and(|n| n == last || alias.as_deref() == Some(n));
    if on_from_item || on_dotted_item {
        let name = if is_from {
            item.as_deref().unwrap_or(last.as_str())
        } else {
            last.as_str()
        };
        let parent_owned = if is_from {
            canonical.clone()
        } else {
            path[..path.len() - 1].join(".")
        };
        return export_in_module(&parent_owned, name, modules)
            .or_else(|| module_symbol(&canonical, modules));
    }
    if crate::stdlib::is_host_module(&path[0]) && path.len() == 1 {
        return None;
    }
    module_symbol(&canonical, modules).or_else(|| {
        if path.len() >= 2 {
            module_symbol(&path[0], modules)
        } else {
            None
        }
    })
}

fn ident_at(tokens: &[Token], line: u32, col: u32) -> Option<IdentAt> {
    for (i, tok) in tokens.iter().enumerate() {
        if !token_covers(tok, line, col) {
            continue;
        }
        let TokenKind::Ident(name) = &tok.kind else {
            return None;
        };
        let qualifier = if i >= 2 && tokens[i - 1].kind == TokenKind::Dot {
            match &tokens[i - 2].kind {
                TokenKind::Ident(mod_name) => Some(mod_name.clone()),
                _ => None,
            }
        } else {
            None
        };
        return Some(IdentAt {
            name: name.clone(),
            qualifier,
        });
    }
    None
}

fn import_binds(program: &[Item]) -> Vec<ImportBind> {
    let mut out = Vec::new();
    for item in program {
        let Item::Import(imp) = item else { continue };
        if imp.path.is_empty() {
            continue;
        }
        if imp.is_from {
            let canonical = imp.path[0].clone();
            if imp.path.len() >= 2 {
                let item_name = imp.path.last().unwrap().clone();
                let bind = imp.alias.clone().unwrap_or_else(|| item_name.clone());
                out.push(ImportBind {
                    bind,
                    canonical,
                    item: Some(item_name),
                });
            }
        } else {
            let canonical = imp.path.join(".");
            let bind = imp
                .alias
                .clone()
                .unwrap_or_else(|| imp.path.last().unwrap().clone());
            out.push(ImportBind {
                bind,
                canonical,
                item: None,
            });
        }
    }
    out
}

fn export_in_module(
    canonical: &str,
    name: &str,
    modules: &HashMap<String, String>,
) -> Option<SymbolInfo> {
    let resolver = HashMapResolver::new(modules.clone());
    for (file, source) in resolver.resolve_all(canonical) {
        let Some(program) = parse_items(&source) else {
            continue;
        };
        for item in crate::parser::module_items(&program, canonical) {
            let hit = match item {
                Item::FnDecl(f) if f.name == name => decl_span(&source, TokenKind::Fn, name)
                    .map(|span| ("fn", fn_signature(f), span, f.doc.clone())),
                Item::VarDecl(v) if v.name == name => {
                    decl_span(&source, TokenKind::Var, name).map(|span| {
                        (
                            "var",
                            format!("var {}: {}", v.name, type_string(&v.ty)),
                            span,
                            v.doc.clone(),
                        )
                    })
                }
                Item::ConstDecl(c) if c.name == name => decl_span(&source, TokenKind::Const, name)
                    .map(|span| ("const", format!("const {}", c.name), span, c.doc.clone())),
                Item::StructDecl(s) if s.name == name => {
                    decl_span(&source, TokenKind::Struct, name)
                        .map(|span| ("struct", format!("struct {}", s.name), span, s.doc.clone()))
                }
                Item::ClassDecl(c) if c.name == name => decl_span(&source, TokenKind::Class, name)
                    .map(|span| ("class", format!("class {}", c.name), span, c.doc.clone())),
                Item::TraitDecl(t) => {
                    if t.name == name {
                        decl_span(&source, TokenKind::Trait, name)
                            .map(|span| ("trait", format!("trait {}", t.name), span, t.doc.clone()))
                    } else {
                        t.signals.iter().find(|s| s.name == name).and_then(|s| {
                            decl_span(&source, TokenKind::Signal, name)
                                .map(|span| ("signal", signal_signature(s), span, s.doc.clone()))
                        })
                    }
                }
                Item::EnumDecl(e) if e.name == name => decl_span(&source, TokenKind::Enum, name)
                    .map(|span| ("enum", format!("enum {}", e.name), span, e.doc.clone())),
                Item::SignalDecl(s) if s.name == name => {
                    decl_span(&source, TokenKind::Signal, name)
                        .map(|span| ("signal", signal_signature(s), span, s.doc.clone()))
                }
                _ => None,
            };
            if let Some((kind, signature, span, doc)) = hit {
                return Some(symbol_info(kind, name, signature, file, span, doc));
            }
        }
    }
    None
}

fn module_symbol(canonical: &str, modules: &HashMap<String, String>) -> Option<SymbolInfo> {
    let resolver = HashMapResolver::new(modules.clone());
    let parts = resolver.resolve_all(canonical);
    if parts.is_empty() {
        return None;
    }
    let stem = canonical.strip_suffix(".rg").unwrap_or(canonical);
    let mut file = parts[0].0.clone();
    let mut line = 1u32;
    let mut col = 1u32;
    for (key, source) in &parts {
        if let Some(program) = parse_items(source) {
            if let Some(m) = program.iter().find_map(|item| match item {
                Item::Mod(m) if m.name == stem || m.name == canonical => Some(m),
                _ => None,
            }) {
                file = key.clone();
                line = m.span.line;
                col = m.span.col;
                break;
            }
        }
    }
    Some(symbol_info(
        "module",
        stem,
        format!("mod {stem}"),
        file,
        Span { line, col },
        None,
    ))
}

fn local_symbol(source: &str, file: &str, name: &str) -> Option<SymbolInfo> {
    let program = parse_items(source)?;
    for item in &program {
        match item {
            Item::FnDecl(f) if f.name == name => {
                let span = decl_span(source, TokenKind::Fn, name)?;
                return Some(symbol_info(
                    "fn",
                    name,
                    fn_signature(f),
                    file.to_string(),
                    span,
                    f.doc.clone(),
                ));
            }
            Item::VarDecl(v) if v.name == name => {
                let span = decl_span(source, TokenKind::Var, name)?;
                return Some(symbol_info(
                    "var",
                    name,
                    format!("var {}: {}", v.name, type_string(&v.ty)),
                    file.to_string(),
                    span,
                    v.doc.clone(),
                ));
            }
            Item::ConstDecl(c) if c.name == name => {
                let span = decl_span(source, TokenKind::Const, name)?;
                return Some(symbol_info(
                    "const",
                    name,
                    format!("const {}", c.name),
                    file.to_string(),
                    span,
                    c.doc.clone(),
                ));
            }
            Item::StructDecl(s) if s.name == name => {
                let span = decl_span(source, TokenKind::Struct, name)?;
                return Some(symbol_info(
                    "struct",
                    name,
                    format!("struct {}", s.name),
                    file.to_string(),
                    span,
                    s.doc.clone(),
                ));
            }
            Item::ClassDecl(c) if c.name == name => {
                let span = decl_span(source, TokenKind::Class, name)?;
                return Some(symbol_info(
                    "class",
                    name,
                    format!("class {}", c.name),
                    file.to_string(),
                    span,
                    c.doc.clone(),
                ));
            }
            Item::TraitDecl(t) => {
                if t.name == name {
                    let span = decl_span(source, TokenKind::Trait, name)?;
                    return Some(symbol_info(
                        "trait",
                        name,
                        format!("trait {}", t.name),
                        file.to_string(),
                        span,
                        t.doc.clone(),
                    ));
                }
                if let Some(s) = t.signals.iter().find(|s| s.name == name) {
                    let span = decl_span(source, TokenKind::Signal, name)?;
                    return Some(symbol_info(
                        "signal",
                        name,
                        signal_signature(s),
                        file.to_string(),
                        span,
                        s.doc.clone(),
                    ));
                }
            }
            Item::EnumDecl(e) if e.name == name => {
                let span = decl_span(source, TokenKind::Enum, name)?;
                return Some(symbol_info(
                    "enum",
                    name,
                    format!("enum {}", e.name),
                    file.to_string(),
                    span,
                    e.doc.clone(),
                ));
            }
            Item::SignalDecl(s) if s.name == name => {
                let span = decl_span(source, TokenKind::Signal, name)?;
                return Some(symbol_info(
                    "signal",
                    name,
                    signal_signature(s),
                    file.to_string(),
                    span,
                    s.doc.clone(),
                ));
            }
            _ => {}
        }
    }
    None
}

fn signal_signature(s: &SignalDecl) -> String {
    let params = s
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, type_string(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("signal {}({params})", s.name)
}

fn symbol_info(
    kind: &str,
    name: &str,
    signature: String,
    file: String,
    span: Span,
    doc: Option<String>,
) -> SymbolInfo {
    SymbolInfo {
        kind: kind.into(),
        name: name.to_string(),
        signature,
        file,
        line: span.line,
        col: span.col,
        doc: doc.filter(|s| !s.is_empty()),
    }
}

fn parse_items(source: &str) -> Option<Vec<Item>> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    Parser::new(tokens).parse().ok()
}

fn decl_span(source: &str, kw: TokenKind, name: &str) -> Option<Span> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    for i in 0..tokens.len() {
        if tokens[i].kind != kw {
            continue;
        }
        let Some(next) = tokens.get(i + 1) else {
            continue;
        };
        if let TokenKind::Ident(n) = &next.kind {
            if n == name {
                return Some(next.span);
            }
        }
    }
    None
}

fn fn_signature(f: &FnDecl) -> String {
    let params = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, type_string(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    match &f.return_type {
        Some(ty) => format!("fn {}({}): {}", f.name, params, type_string(ty)),
        None => format!("fn {}({})", f.name, params),
    }
}

fn type_string(ty: &Type) -> String {
    let mut s = ty.name.clone();
    if !ty.args.is_empty() {
        s.push('<');
        s.push_str(
            &ty.args
                .iter()
                .map(type_string)
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push('>');
    }
    if ty.optional {
        s.push('?');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero() -> String {
        include_str!("../../../examples/demo-project/scripts/Hero.rg").to_string()
    }

    fn utils() -> String {
        include_str!("../../../examples/demo-project/scripts/utils.rg").to_string()
    }

    fn modules() -> HashMap<String, String> {
        HashMap::from([("utils".into(), utils()), ("utils.rg".into(), utils())])
    }

    #[test]
    fn def_at_imported_fn() {
        let src = hero();
        // `utils.move_line` on the on_update body
        let info = def_at(&src, "Hero.rg", 32, 18, modules()).expect("def");
        assert_eq!(info.kind, "fn");
        assert_eq!(info.name, "move_line");
        assert!(info.file.starts_with("utils"), "{}", info.file);
        assert_eq!(info.line, 6);
        assert!(info.signature.contains("move_line"));
        assert!(info.signature.contains("dx"));
    }

    #[test]
    fn hover_at_imported_fn_signature() {
        let info = hover_at(&hero(), "Hero.rg", 32, 18, modules()).expect("hover");
        assert_eq!(info.signature, "fn move_line(dx: Float, dy: Float)");
    }

    #[test]
    fn def_at_import_module_name() {
        let info = def_at(&hero(), "Hero.rg", 1, 8, modules()).expect("module");
        assert_eq!(info.kind, "module");
        assert!(info.file.starts_with("utils"), "{}", info.file);
        assert_eq!(info.signature, "mod utils");
        assert_eq!(info.line, 5);
    }

    #[test]
    fn def_at_import_keyword() {
        let info = def_at(&hero(), "Hero.rg", 1, 1, modules()).expect("import kw");
        assert_eq!(info.kind, "module");
        assert!(info.file.starts_with("utils"), "{}", info.file);
    }

    #[test]
    fn def_at_from_import_module_name() {
        let src = "from utils import move_line;\nfn on_update(): Int {\n    move_line(1.0, 0.0);\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 1, 6, modules()).expect("from module");
        assert_eq!(info.kind, "module");
        assert!(info.file.starts_with("utils"), "{}", info.file);
    }

    #[test]
    fn def_at_from_import_item_on_import_line() {
        let src = "from utils import move_line;\nfn on_update(): Int {\n    move_line(1.0, 0.0);\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 1, 20, modules()).expect("from item");
        assert_eq!(info.kind, "fn");
        assert_eq!(info.name, "move_line");
    }

    #[test]
    fn def_at_dotted_import_path() {
        let math = "pub fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n";
        let map = HashMap::from([
            ("util.math".into(), math.to_string()),
            ("util/math.rg".into(), math.to_string()),
        ]);
        let src = "import util.math;\nfn main(): Int {\n    return math.add(1, 2);\n}\n";
        let head = def_at(src, "t.rg", 1, 8, map.clone()).expect("dotted head");
        assert_eq!(head.kind, "module");
        let tail = def_at(src, "t.rg", 1, 13, map).expect("dotted tail");
        assert_eq!(tail.kind, "module");
    }

    #[test]
    fn def_at_import_stdlib_type() {
        let src = "import strata.Sprite;\n";
        let info = def_at(src, "t.rg", 1, 16, HashMap::new()).expect("Sprite");
        assert_eq!(info.kind, "class");
        assert_eq!(info.name, "Sprite");
        assert!(info.file.contains("node"), "{}", info.file);
    }

    #[test]
    fn def_at_stdlib_math_fn() {
        let src = "import math;\nfn main(): Int {\n    return math.sin(0);\n}\n";
        let info = def_at(src, "t.rg", 3, 17, HashMap::new()).expect("sin");
        assert_eq!(info.kind, "fn");
        assert_eq!(info.name, "sin");
        assert!(info.file.contains("math"), "{}", info.file);
    }

    #[test]
    fn def_at_prelude_vec2() {
        let src = "fn main(): Int {\n    var v = Vec2 { x: 1.0, y: 2.0 };\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 2, 13, HashMap::new()).expect("Vec2");
        assert_eq!(info.kind, "class");
        assert_eq!(info.name, "Vec2");
        assert!(info.file.contains("vec"), "{}", info.file);
    }

    #[test]
    fn def_at_prelude_vec3() {
        let src =
            "fn main(): Int {\n    var v = Vec3 { x: 1.0, y: 2.0, z: 3.0 };\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 2, 13, HashMap::new()).expect("Vec3");
        assert_eq!(info.kind, "class");
        assert_eq!(info.name, "Vec3");
        assert!(info.file.contains("vec"), "{}", info.file);
    }

    #[test]
    fn def_at_node_type_use() {
        let src = "import strata.Sprite;\n@node\nclass Foo extends Sprite {\n    fn on_create() { pass; }\n}\n";
        let info = def_at(src, "t.rg", 3, 20, HashMap::new()).expect("Sprite");
        assert_eq!(info.kind, "class");
        assert_eq!(info.name, "Sprite");
        assert!(info.file.contains("node"), "{}", info.file);
    }

    #[test]
    fn host_member_is_none() {
        let src = "fn on_update(): Int {\n    strata.move(1.0, 0.0);\n    return 0;\n}\n";
        assert!(def_at(src, "t.rg", 2, 12, HashMap::new()).is_none());
    }

    #[test]
    fn def_at_merged_mod_other_file() {
        let extra = "mod utils {\n    pub fn extra_fn(): Int {\n        return 1;\n    }\n}\n";
        let mut map = modules();
        map.insert("extra.rg".into(), extra.into());
        let src = "import utils;\nfn main(): Int {\n    utils.extra_fn();\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 3, 14, map).expect("def");
        assert_eq!(info.name, "extra_fn");
        assert_eq!(info.file, "extra.rg");
        assert_eq!(info.line, 2);
    }

    #[test]
    fn from_import_bare_name() {
        let src = "from utils import move_line;\nfn on_update(): Int {\n    move_line(1.0, 0.0);\n    return 0;\n}\n";
        let info = def_at(src, "t.rg", 3, 5, modules()).expect("from-import");
        assert_eq!(info.name, "move_line");
        assert_eq!(info.line, 6);
    }

    #[test]
    fn hover_at_prefers_doc_comment() {
        let src = "## Degrees per second.\n@export var spin: Float = 8.0;\nfn main(): Int { return 0; }\n";
        let info = hover_at(src, "t.rg", 2, 14, HashMap::new()).expect("hover");
        assert_eq!(info.name, "spin");
        assert_eq!(info.doc.as_deref(), Some("Degrees per second."));
        assert!(info.signature.contains("spin"));
    }

    #[test]
    fn hover_at_joins_multiline_docs() {
        let src = "## Called every Play frame.\n## dt is seconds.\nfn on_update(dt: Float): Int { return 0; }\n";
        let info = hover_at(src, "t.rg", 3, 4, HashMap::new()).expect("hover");
        assert_eq!(info.name, "on_update");
        assert_eq!(
            info.doc.as_deref(),
            Some("Called every Play frame.\ndt is seconds.")
        );
    }

    #[test]
    fn line_comment_is_not_hover_doc() {
        let src = "# not a doc\nfn foo(): Int { return 0; }\n";
        let info = hover_at(src, "t.rg", 2, 4, HashMap::new()).expect("hover");
        assert_eq!(info.doc, None);
    }
}
