//! Hover and go-to-definition at a source `line:col` ([`hover_at`], [`def_at`]).

use std::collections::HashMap;

use crate::Span;
use crate::interpreter::{HashMapResolver, ModuleResolver};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::{Block, ClassDecl, FnDecl, Item, Parser, SignalDecl, StmtKind, Type, VarDecl};

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
/// `Vec2`) resolve into `stdlib/*.rg`. Host APIs (`io.read_text`) stay
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
    if let Some(info) = doc_comment_symbol(source, file, &tokens, line, col) {
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
        return member_symbol(
            source,
            file,
            &program,
            &module,
            &ident.name,
            line,
            col,
            &modules,
        );
    }

    if let Some(bind) = binds.iter().find(|b| b.bind == ident.name) {
        if crate::stdlib::is_host_module(&bind.canonical) {
            return None;
        }
        if let Some(item) = &bind.item {
            return export_in_module(&bind.canonical, item, &modules);
        }
        return module_symbol(&bind.canonical, &modules);
    }

    local_symbol(source, file, &ident.name, line, col)
        .or_else(|| prelude_symbol(&ident.name, &modules))
}

fn prelude_symbol(name: &str, modules: &HashMap<String, String>) -> Option<SymbolInfo> {
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

fn doc_comment_symbol(
    source: &str,
    file: &str,
    tokens: &[Token],
    line: u32,
    col: u32,
) -> Option<SymbolInfo> {
    let idx = tokens.iter().position(|t| token_covers(t, line, col))?;
    if !matches!(tokens[idx].kind, TokenKind::DocComment(_)) {
        return None;
    }
    let i = skip_leading_trivia(tokens, idx);
    let (name, span) = match tokens.get(i).map(|t| &t.kind) {
        Some(
            TokenKind::Var
            | TokenKind::Const
            | TokenKind::Fn
            | TokenKind::Class
            | TokenKind::Struct
            | TokenKind::Trait
            | TokenKind::Enum
            | TokenKind::Mod
            | TokenKind::Signal,
        ) => {
            let name_tok = tokens.get(i + 1)?;
            let TokenKind::Ident(name) = &name_tok.kind else {
                return None;
            };
            (name.as_str(), name_tok.span)
        }
        Some(TokenKind::Ident(name)) => (name.as_str(), tokens[i].span),
        _ => return None,
    };
    local_symbol(source, file, name, span.line, span.col)
}

fn skip_leading_trivia(tokens: &[Token], mut i: usize) -> usize {
    loop {
        match tokens.get(i).map(|t| &t.kind) {
            Some(TokenKind::DocComment(_) | TokenKind::Comment(_) | TokenKind::Pub) => i += 1,
            Some(TokenKind::At) => {
                i += 1;
                if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
                    i += 1;
                }
                if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::LParen)) {
                    let mut depth = 0i32;
                    while i < tokens.len() {
                        match tokens[i].kind {
                            TokenKind::LParen => depth += 1,
                            TokenKind::RParen => {
                                depth -= 1;
                                i += 1;
                                if depth == 0 {
                                    break;
                                }
                                continue;
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            _ => return i,
        }
    }
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
                            var_signature(&v.name, &v.ty),
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

fn local_symbol(source: &str, file: &str, name: &str, line: u32, col: u32) -> Option<SymbolInfo> {
    let program = parse_items(source)?;
    let mut hits = Vec::new();
    collect_item_hits(
        &program,
        source,
        file,
        name,
        Span { line: 1, col: 1 },
        0,
        &mut hits,
    );
    pick_hit(&hits, line, col)
}

struct Hit {
    depth: u32,
    info: SymbolInfo,
}

fn pick_hit(hits: &[Hit], line: u32, col: u32) -> Option<SymbolInfo> {
    if hits.is_empty() {
        return None;
    }
    if let Some(hit) = hits.iter().find(|h| name_covers(&h.info, line, col)) {
        return Some(hit.info.clone());
    }
    hits.iter()
        .filter(|h| pos_le(h.info.line, h.info.col, line, col))
        .max_by_key(|h| (h.depth, h.info.line, h.info.col))
        .or_else(|| {
            hits.iter()
                .max_by_key(|h| (h.depth, h.info.line, h.info.col))
        })
        .map(|h| h.info.clone())
}

fn name_covers(info: &SymbolInfo, line: u32, col: u32) -> bool {
    if info.line != line {
        return false;
    }
    let len = info.name.chars().count() as u32;
    col >= info.col && col < info.col + len
}

fn pos_le(a_line: u32, a_col: u32, b_line: u32, b_col: u32) -> bool {
    a_line < b_line || (a_line == b_line && a_col <= b_col)
}

fn push_hit(
    hits: &mut Vec<Hit>,
    depth: u32,
    kind: &str,
    name: &str,
    signature: String,
    file: &str,
    span: Span,
    doc: Option<String>,
) {
    hits.push(Hit {
        depth,
        info: symbol_info(kind, name, signature, file.to_string(), span, doc),
    });
}

fn collect_item_hits(
    items: &[Item],
    source: &str,
    file: &str,
    name: &str,
    from: Span,
    depth: u32,
    hits: &mut Vec<Hit>,
) {
    for item in items {
        match item {
            Item::FnDecl(f) => {
                if f.name == name {
                    if let Some(span) = decl_span_from(source, TokenKind::Fn, name, from) {
                        push_hit(
                            hits,
                            depth,
                            "fn",
                            name,
                            fn_signature(f),
                            file,
                            span,
                            f.doc.clone(),
                        );
                    }
                }
                collect_block_hits(&f.body, source, file, name, depth + 1, hits);
            }
            Item::VarDecl(v) if v.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Var, name, from) {
                    push_hit(
                        hits,
                        depth,
                        "var",
                        name,
                        var_signature(&v.name, &v.ty),
                        file,
                        span,
                        v.doc.clone(),
                    );
                }
            }
            Item::ConstDecl(c) if c.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Const, name, from) {
                    push_hit(
                        hits,
                        depth,
                        "const",
                        name,
                        format!("const {}", c.name),
                        file,
                        span,
                        c.doc.clone(),
                    );
                }
            }
            Item::StructDecl(s) => {
                if s.name == name {
                    if let Some(span) = decl_span_from(source, TokenKind::Struct, name, from) {
                        push_hit(
                            hits,
                            depth,
                            "struct",
                            name,
                            format!("struct {}", s.name),
                            file,
                            span,
                            s.doc.clone(),
                        );
                    }
                }
                let struct_from =
                    decl_span_from(source, TokenKind::Struct, &s.name, from).unwrap_or(from);
                for f in &s.fields {
                    if f.name == name {
                        if let Some(span) = ident_colon_in_body(source, name, struct_from) {
                            push_hit(
                                hits,
                                depth + 1,
                                "var",
                                name,
                                format!("{}: {}", f.name, type_string(&f.ty)),
                                file,
                                span,
                                f.doc.clone(),
                            );
                        }
                    }
                }
            }
            Item::ClassDecl(c) => {
                if c.name == name {
                    if let Some(span) = decl_span_from(source, TokenKind::Class, name, from) {
                        push_hit(
                            hits,
                            depth,
                            "class",
                            name,
                            format!("class {}", c.name),
                            file,
                            span,
                            c.doc.clone(),
                        );
                    }
                }
                for f in &c.fields {
                    if f.name == name {
                        if let Some(span) = var_ident_in_body(source, name, c.span) {
                            push_hit(
                                hits,
                                depth + 1,
                                "var",
                                name,
                                var_signature(&f.name, &f.ty),
                                file,
                                span,
                                f.doc.clone(),
                            );
                        }
                    }
                }
                for m in c.all_methods() {
                    if m.name == name {
                        if let Some(span) = decl_span_from(source, TokenKind::Fn, name, c.span) {
                            push_hit(
                                hits,
                                depth + 1,
                                "fn",
                                name,
                                fn_signature(m),
                                file,
                                span,
                                m.doc.clone(),
                            );
                        }
                    }
                    collect_block_hits(&m.body, source, file, name, depth + 2, hits);
                }
            }
            Item::TraitDecl(t) => {
                if t.name == name {
                    if let Some(span) = decl_span_from(source, TokenKind::Trait, name, from) {
                        push_hit(
                            hits,
                            depth,
                            "trait",
                            name,
                            format!("trait {}", t.name),
                            file,
                            span,
                            t.doc.clone(),
                        );
                    }
                }
                if let Some(s) = t.signals.iter().find(|s| s.name == name) {
                    if let Some(span) = decl_span_from(source, TokenKind::Signal, name, from) {
                        push_hit(
                            hits,
                            depth + 1,
                            "signal",
                            name,
                            signal_signature(s),
                            file,
                            span,
                            s.doc.clone(),
                        );
                    }
                }
            }
            Item::EnumDecl(e) if e.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Enum, name, from) {
                    push_hit(
                        hits,
                        depth,
                        "enum",
                        name,
                        format!("enum {}", e.name),
                        file,
                        span,
                        e.doc.clone(),
                    );
                }
            }
            Item::SignalDecl(s) if s.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Signal, name, from) {
                    push_hit(
                        hits,
                        depth,
                        "signal",
                        name,
                        signal_signature(s),
                        file,
                        span,
                        s.doc.clone(),
                    );
                }
            }
            Item::Mod(m) => {
                collect_item_hits(&m.items, source, file, name, m.span, depth + 1, hits);
            }
            Item::ImplDecl { methods, span, .. } => {
                for m in methods {
                    if m.name == name {
                        if let Some(name_span) =
                            decl_span_from(source, TokenKind::Fn, name, *span)
                        {
                            push_hit(
                                hits,
                                depth + 1,
                                "fn",
                                name,
                                fn_signature(m),
                                file,
                                name_span,
                                m.doc.clone(),
                            );
                        }
                    }
                    collect_block_hits(&m.body, source, file, name, depth + 2, hits);
                }
            }
            _ => {}
        }
    }
}

fn collect_block_hits(
    block: &Block,
    source: &str,
    file: &str,
    name: &str,
    depth: u32,
    hits: &mut Vec<Hit>,
) {
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::VarDecl(v) if v.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Var, name, stmt.span) {
                    push_hit(
                        hits,
                        depth,
                        "var",
                        name,
                        var_signature(&v.name, &v.ty),
                        file,
                        span,
                        v.doc.clone(),
                    );
                }
            }
            StmtKind::ConstDecl(c) if c.name == name => {
                if let Some(span) = decl_span_from(source, TokenKind::Const, name, stmt.span) {
                    push_hit(
                        hits,
                        depth,
                        "const",
                        name,
                        format!("const {}", c.name),
                        file,
                        span,
                        c.doc.clone(),
                    );
                }
            }
            StmtKind::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                collect_block_hits(then_block, source, file, name, depth + 1, hits);
                for (_, b) in elif_blocks {
                    collect_block_hits(b, source, file, name, depth + 1, hits);
                }
                if let Some(b) = else_block {
                    collect_block_hits(b, source, file, name, depth + 1, hits);
                }
            }
            StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                collect_block_hits(body, source, file, name, depth + 1, hits);
            }
            _ => {}
        }
    }
}

fn member_symbol(
    source: &str,
    file: &str,
    program: &[Item],
    receiver: &str,
    name: &str,
    line: u32,
    col: u32,
    modules: &HashMap<String, String>,
) -> Option<SymbolInfo> {
    if receiver == "self" || receiver == "super" {
        let class = class_containing(program, source, line, col)?;
        let start = if receiver == "super" {
            class
                .parent
                .as_ref()
                .and_then(|p| find_class(program, p))
                .unwrap_or(class)
        } else {
            class
        };
        return lookup_class_member(source, file, program, start, name).or_else(|| {
            start
                .parent
                .as_ref()
                .and_then(|p| member_in_modules(modules, p, name))
        });
    }
    if let Some(ty) = type_of_binding(program, receiver) {
        return class_or_struct_member(source, file, program, &ty, name)
            .or_else(|| member_in_modules(modules, &ty, name));
    }
    unique_named_member(source, file, program, name)
}

fn class_containing<'a>(
    program: &'a [Item],
    source: &str,
    line: u32,
    col: u32,
) -> Option<&'a ClassDecl> {
    let mut best: Option<&ClassDecl> = None;
    for c in classes_in(program) {
        if !contains_from(source, c.span, line, col) {
            continue;
        }
        let closer = best.map_or(true, |b| {
            b.span.line < c.span.line || (b.span.line == c.span.line && b.span.col <= c.span.col)
        });
        if closer {
            best = Some(c);
        }
    }
    best
}

fn classes_in(items: &[Item]) -> Vec<&ClassDecl> {
    let mut out = Vec::new();
    collect_classes(items, &mut out);
    out
}

fn collect_classes<'a>(items: &'a [Item], out: &mut Vec<&'a ClassDecl>) {
    for item in items {
        match item {
            Item::ClassDecl(c) => out.push(c),
            Item::Mod(m) => collect_classes(&m.items, out),
            _ => {}
        }
    }
}

fn find_class<'a>(program: &'a [Item], name: &str) -> Option<&'a ClassDecl> {
    classes_in(program).into_iter().find(|c| c.name == name)
}

fn contains_from(source: &str, from: Span, line: u32, col: u32) -> bool {
    let Some(end) = matching_rbrace_span(source, from) else {
        return false;
    };
    pos_le(from.line, from.col, line, col) && pos_le(line, col, end.line, end.col)
}

fn matching_rbrace_span(source: &str, from: Span) -> Option<Span> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    let mut i = skip_before(&tokens, from);
    let mut depth = 0i32;
    let mut started = false;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::LBrace => {
                depth += 1;
                started = true;
            }
            TokenKind::RBrace => {
                depth -= 1;
                if started && depth == 0 {
                    return Some(tokens[i].span);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn lookup_class_member(
    source: &str,
    file: &str,
    program: &[Item],
    class: &ClassDecl,
    name: &str,
) -> Option<SymbolInfo> {
    if let Some(info) = direct_class_member(source, file, class, name) {
        return Some(info);
    }
    class
        .parent
        .as_ref()
        .and_then(|p| find_class(program, p))
        .and_then(|parent| lookup_class_member(source, file, program, parent, name))
}

fn direct_class_member(
    source: &str,
    file: &str,
    class: &ClassDecl,
    name: &str,
) -> Option<SymbolInfo> {
    if let Some(f) = class.fields.iter().find(|f| f.name == name) {
        let span = var_ident_in_body(source, name, class.span)?;
        return Some(symbol_info(
            "var",
            name,
            var_signature(&f.name, &f.ty),
            file.to_string(),
            span,
            f.doc.clone(),
        ));
    }
    if let Some(m) = class.all_methods().find(|m| m.name == name) {
        let span = decl_span_from(source, TokenKind::Fn, name, class.span)?;
        return Some(symbol_info(
            "fn",
            name,
            fn_signature(m),
            file.to_string(),
            span,
            m.doc.clone(),
        ));
    }
    None
}

fn class_or_struct_member(
    source: &str,
    file: &str,
    program: &[Item],
    type_name: &str,
    name: &str,
) -> Option<SymbolInfo> {
    if let Some(c) = find_class(program, type_name) {
        return lookup_class_member(source, file, program, c, name);
    }
    for item in program {
        match item {
            Item::StructDecl(s) if s.name == type_name => {
                if let Some(f) = s.fields.iter().find(|f| f.name == name) {
                    let from = decl_span(source, TokenKind::Struct, &s.name)?;
                    let span = ident_colon_in_body(source, name, from)?;
                    return Some(symbol_info(
                        "var",
                        name,
                        format!("{}: {}", f.name, type_string(&f.ty)),
                        file.to_string(),
                        span,
                        f.doc.clone(),
                    ));
                }
            }
            Item::Mod(m) => {
                if let Some(info) =
                    class_or_struct_member(source, file, &m.items, type_name, name)
                {
                    return Some(info);
                }
            }
            _ => {}
        }
    }
    None
}

fn unique_named_member(
    source: &str,
    file: &str,
    program: &[Item],
    name: &str,
) -> Option<SymbolInfo> {
    let mut found = Vec::new();
    for c in classes_in(program) {
        if let Some(info) = direct_class_member(source, file, c, name) {
            found.push(info);
        }
    }
    if found.len() == 1 {
        found.pop()
    } else {
        None
    }
}

fn member_in_modules(
    modules: &HashMap<String, String>,
    type_name: &str,
    name: &str,
) -> Option<SymbolInfo> {
    for (file, source) in modules {
        let Some(program) = parse_items(source) else {
            continue;
        };
        if let Some(info) = class_or_struct_member(source, file, &program, type_name, name) {
            return Some(info);
        }
    }
    None
}

fn type_of_binding(items: &[Item], name: &str) -> Option<String> {
    for item in items {
        match item {
            Item::VarDecl(v) if v.name == name => return type_of_var(v),
            Item::FnDecl(f) => {
                if let Some(t) = type_in_block(&f.body, name) {
                    return Some(t);
                }
            }
            Item::ClassDecl(c) => {
                for m in c.all_methods() {
                    if let Some(t) = type_in_block(&m.body, name) {
                        return Some(t);
                    }
                }
            }
            Item::ImplDecl { methods, .. } => {
                for m in methods {
                    if let Some(t) = type_in_block(&m.body, name) {
                        return Some(t);
                    }
                }
            }
            Item::Mod(m) => {
                if let Some(t) = type_of_binding(&m.items, name) {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

fn type_of_var(v: &VarDecl) -> Option<String> {
    if v.ty.name != "None" && !v.ty.name.is_empty() {
        return Some(v.ty.name.clone());
    }
    match v.value.as_ref().map(|e| &e.kind) {
        Some(crate::parser::ExprKind::StructLiteral { name, .. }) => Some(name.clone()),
        _ => None,
    }
}

fn type_in_block(block: &Block, name: &str) -> Option<String> {
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::VarDecl(v) if v.name == name => return type_of_var(v),
            StmtKind::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                if let Some(t) = type_in_block(then_block, name) {
                    return Some(t);
                }
                for (_, b) in elif_blocks {
                    if let Some(t) = type_in_block(b, name) {
                        return Some(t);
                    }
                }
                if let Some(b) = else_block {
                    if let Some(t) = type_in_block(b, name) {
                        return Some(t);
                    }
                }
            }
            StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                if let Some(t) = type_in_block(body, name) {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

fn skip_before(tokens: &[Token], from: Span) -> usize {
    tokens
        .iter()
        .position(|t| {
            t.span.line > from.line || (t.span.line == from.line && t.span.col >= from.col)
        })
        .unwrap_or(tokens.len())
}

fn decl_span_from(source: &str, kw: TokenKind, name: &str, from: Span) -> Option<Span> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    let start = skip_before(&tokens, from);
    for i in start..tokens.len() {
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

fn var_ident_in_body(source: &str, name: &str, from: Span) -> Option<Span> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    let mut i = skip_before(&tokens, from);
    let mut depth = 0i32;
    let mut started = false;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::LBrace => {
                depth += 1;
                started = true;
            }
            TokenKind::RBrace => {
                depth -= 1;
                if started && depth == 0 {
                    return None;
                }
            }
            TokenKind::Var if started && depth == 1 => {
                if let Some(tok) = tokens.get(i + 1) {
                    if let TokenKind::Ident(n) = &tok.kind {
                        if n == name {
                            return Some(tok.span);
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn ident_colon_in_body(source: &str, name: &str, from: Span) -> Option<Span> {
    let tokens = Lexer::new(source).tokenize().ok()?;
    let mut i = skip_before(&tokens, from);
    let mut depth = 0i32;
    let mut started = false;
    while i < tokens.len() {
        match &tokens[i].kind {
            TokenKind::LBrace => {
                depth += 1;
                started = true;
            }
            TokenKind::RBrace => {
                depth -= 1;
                if started && depth == 0 {
                    return None;
                }
            }
            TokenKind::Ident(n) if started && depth == 1 && n == name => {
                if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                    return Some(tokens[i].span);
                }
            }
            _ => {}
        }
        i += 1;
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
    decl_span_from(source, kw, name, Span { line: 1, col: 1 })
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

fn var_signature(name: &str, ty: &Type) -> String {
    if ty.name == "None" && ty.args.is_empty() && !ty.optional {
        format!("var {name}")
    } else {
        format!("var {name}: {}", type_string(ty))
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

    fn utils() -> String {
        "# Shared helpers.\n\nmod utils {\n    pub fn move_line(dx: Float, dy: Float) {\n        print(dx);\n    }\n}\n"
            .to_string()
    }

    fn hero() -> String {
        "import utils;\n\nfn main(): Int {\n    utils.move_line(1.0, 0.0);\n    return 0;\n}\n"
            .to_string()
    }

    fn modules() -> HashMap<String, String> {
        HashMap::from([("utils".into(), utils()), ("utils.rg".into(), utils())])
    }

    #[test]
    fn def_at_imported_fn() {
        let src = hero();
        // `utils.move_line` on the on_update body
        let info = def_at(&src, "Hero.rg", 4, 12, modules()).expect("def");
        assert_eq!(info.kind, "fn");
        assert_eq!(info.name, "move_line");
        assert!(info.file.starts_with("utils"), "{}", info.file);
        assert_eq!(info.line, 4);
        assert!(info.signature.contains("move_line"));
        assert!(info.signature.contains("dx"));
    }

    #[test]
    fn hover_at_imported_fn_signature() {
        let info = hover_at(&hero(), "Hero.rg", 4, 12, modules()).expect("hover");
        assert_eq!(info.signature, "fn move_line(dx: Float, dy: Float)");
    }

    #[test]
    fn def_at_import_module_name() {
        let info = def_at(&hero(), "Hero.rg", 1, 8, modules()).expect("module");
        assert_eq!(info.kind, "module");
        assert!(info.file.starts_with("utils"), "{}", info.file);
        assert_eq!(info.signature, "mod utils");
        assert_eq!(info.line, 3);
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
        let src = "import math;\n";
        let info = def_at(src, "t.rg", 1, 8, HashMap::new()).expect("math");
        assert_eq!(info.kind, "module");
        assert!(info.file.contains("math"), "{}", info.file);
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
    fn host_member_is_none() {
        let src = "fn main(): Int {\n    io.exists(\"x\");\n    return 0;\n}\n";
        assert!(def_at(src, "t.rg", 2, 7, HashMap::new()).is_none());
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
        assert_eq!(info.line, 4);
    }

    #[test]
    fn hover_at_prefers_doc_comment() {
        let src = "## Degrees per second.\nvar spin: Float = 8.0;\nfn main(): Int { return 0; }\n";
        let info = hover_at(src, "t.rg", 2, 5, HashMap::new()).expect("hover");
        assert_eq!(info.name, "spin");
        assert_eq!(info.doc.as_deref(), Some("Degrees per second."));
        assert!(info.signature.contains("spin"));
    }

    #[test]
    fn hover_at_var_use_shows_docs() {
        let src = "## Degrees per second.\nvar spin: Float = 8.0;\nfn main(): Int {\n    print(spin);\n    return 0;\n}\n";
        let info = hover_at(src, "t.rg", 4, 11, HashMap::new()).expect("hover use");
        assert_eq!(info.name, "spin");
        assert_eq!(info.doc.as_deref(), Some("Degrees per second."));
    }

    #[test]
    fn hover_at_local_var_docs() {
        let src = "fn main(): Int {\n    ## local counter\n    var n: Int = 1;\n    print(n);\n    return 0;\n}\n";
        let on_decl = hover_at(src, "t.rg", 3, 9, HashMap::new()).expect("decl");
        assert_eq!(on_decl.name, "n");
        assert_eq!(on_decl.doc.as_deref(), Some("local counter"));
        let on_use = hover_at(src, "t.rg", 4, 11, HashMap::new()).expect("use");
        assert_eq!(on_use.doc.as_deref(), Some("local counter"));
    }

    #[test]
    fn hover_at_class_field_docs() {
        let src = "class Point {\n    ## X component\n    var x: Float = 0.0;\n    fn get(self): Float {\n        return self.x;\n    }\n}\nfn main(): Int {\n    var p = Point { x: 1.0 };\n    print(p.x);\n    return 0;\n}\n";
        let on_field = hover_at(src, "t.rg", 3, 9, HashMap::new()).expect("field");
        assert_eq!(on_field.name, "x");
        assert_eq!(on_field.doc.as_deref(), Some("X component"));
        let on_self = hover_at(src, "t.rg", 5, 21, HashMap::new()).expect("self.x");
        assert_eq!(on_self.doc.as_deref(), Some("X component"));
        let on_p = hover_at(src, "t.rg", 10, 13, HashMap::new()).expect("p.x");
        assert_eq!(on_p.doc.as_deref(), Some("X component"));
    }

    #[test]
    fn hover_at_doc_line_shows_following_var() {
        let src = "fn main(): Int {\n    ## Helper functions for JSON\n    var round = 1;\n    return round;\n}\n";
        let on_hashes = hover_at(src, "t.rg", 2, 5, HashMap::new()).expect("##");
        assert_eq!(on_hashes.name, "round");
        assert_eq!(on_hashes.doc.as_deref(), Some("Helper functions for JSON"));
        let on_word = hover_at(src, "t.rg", 2, 12, HashMap::new()).expect("Helper");
        assert_eq!(on_word.doc.as_deref(), Some("Helper functions for JSON"));
        assert_eq!(on_word.signature, "var round");
    }

    #[test]
    fn hover_at_joins_multiline_docs() {
        let src = "## Clamp to unit range.\n## Inclusive on both ends.\nfn clamp01(n: Float): Float { return n; }\n";
        let info = hover_at(src, "t.rg", 3, 4, HashMap::new()).expect("hover");
        assert_eq!(info.name, "clamp01");
        assert_eq!(
            info.doc.as_deref(),
            Some("Clamp to unit range.\nInclusive on both ends.")
        );
    }

    #[test]
    fn line_comment_is_not_hover_doc() {
        let src = "# not a doc\nfn foo(): Int { return 0; }\n";
        let info = hover_at(src, "t.rg", 2, 4, HashMap::new()).expect("hover");
        assert_eq!(info.doc, None);
    }
}
