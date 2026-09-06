//! Inspector connection metadata: `signal` decls and callable `fn`s.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::interpreter::{HashMapResolver, ModuleResolver};
use crate::lexer::Lexer;
use crate::parser::{Item, Parser, SignalDecl, Type};

/// Inspector metadata for one `signal` declaration. Parse-only; no eval.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalField {
    pub name: String,
    pub params: Vec<SignalParam>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalParam {
    pub name: String,
    pub ty: String,
}

/// Top-level `fn` names for Inspector method pickers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FnMeta {
    pub name: String,
    pub params: Vec<SignalParam>,
}

pub fn list_signals(source: &str) -> Vec<SignalField> {
    list_signals_with_modules(source, HashMap::new())
}

/// Same as `list_signals`, plus traits from in-memory imported modules.
pub fn list_signals_with_modules(
    source: &str,
    modules: HashMap<String, String>,
) -> Vec<SignalField> {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return Vec::new();
    };
    let Ok(program) = Parser::new(tokens).parse() else {
        return Vec::new();
    };
    let resolver = HashMapResolver::new(modules);
    collect_signals_resolved(&program, &resolver)
}

pub fn list_fns(source: &str) -> Vec<FnMeta> {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return Vec::new();
    };
    let Ok(program) = Parser::new(tokens).parse() else {
        return Vec::new();
    };
    collect_fns(&program)
}

pub fn collect_signals(program: &[Item]) -> Vec<SignalField> {
    collect_signals_resolved(program, &HashMapResolver::new(HashMap::new()))
}

fn collect_signals_resolved(program: &[Item], resolver: &dyn ModuleResolver) -> Vec<SignalField> {
    let mut trait_signals: HashMap<String, Vec<SignalDecl>> = HashMap::new();
    index_trait_signals(program, &mut trait_signals);
    let mut seen_mods = HashSet::new();
    load_imported_traits(program, resolver, &mut trait_signals, &mut seen_mods);

    let mut impld = HashSet::new();
    collect_impld_traits(program, &mut impld);

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_toplevel_signals(program, &mut out, &mut seen);
    for trait_name in impld {
        if let Some(sigs) = trait_signals.get(&trait_name) {
            for s in sigs {
                push_signal(&mut out, &mut seen, s);
            }
        }
    }
    out
}

fn load_imported_traits(
    program: &[Item],
    resolver: &dyn ModuleResolver,
    out: &mut HashMap<String, Vec<SignalDecl>>,
    seen: &mut HashSet<String>,
) {
    for item in program {
        match item {
            Item::Import(imp) => {
                if imp.path.is_empty() {
                    continue;
                }
                let name = if imp.is_from {
                    imp.path[0].clone()
                } else {
                    imp.path.join(".")
                };
                if crate::stdlib::is_host_module(&name) {
                    continue;
                }
                index_module_traits(&name, resolver, out, seen);
            }
            Item::Mod(m) => load_imported_traits(&m.items, resolver, out, seen),
            Item::Comment(_) => {}
            _ => {}
        }
    }
}

fn index_module_traits(
    name: &str,
    resolver: &dyn ModuleResolver,
    out: &mut HashMap<String, Vec<SignalDecl>>,
    seen: &mut HashSet<String>,
) {
    let canonical = crate::stdlib::canonical_module(name).unwrap_or(name);
    if !seen.insert(canonical.to_string()) {
        return;
    }
    for (_, source) in resolver.resolve_all(canonical) {
        let Ok(tokens) = Lexer::new(&source).tokenize() else {
            continue;
        };
        let Ok(program) = Parser::new(tokens).parse() else {
            continue;
        };
        index_trait_signals(&program, out);
        load_imported_traits(&program, resolver, out, seen);
    }
}

fn index_trait_signals(program: &[Item], out: &mut HashMap<String, Vec<SignalDecl>>) {
    for item in program {
        match item {
            Item::TraitDecl(t) => {
                out.insert(t.name.clone(), t.signals.clone());
            }
            Item::Mod(m) => index_trait_signals(&m.items, out),
            _ => {}
        }
    }
}

fn collect_impld_traits(program: &[Item], out: &mut HashSet<String>) {
    for item in program {
        match item {
            Item::ClassDecl(c) => {
                for t in c.implemented_traits() {
                    out.insert(t);
                }
            }
            Item::ImplDecl {
                trait_name: Some(t),
                ..
            } => {
                out.insert(t.clone());
            }
            Item::Mod(m) => collect_impld_traits(&m.items, out),
            _ => {}
        }
    }
}

fn collect_toplevel_signals(
    program: &[Item],
    out: &mut Vec<SignalField>,
    seen: &mut HashSet<String>,
) {
    for item in program {
        match item {
            Item::SignalDecl(s) => push_signal(out, seen, s),
            Item::Mod(m) => collect_toplevel_signals(&m.items, out, seen),
            _ => {}
        }
    }
}

fn push_signal(out: &mut Vec<SignalField>, seen: &mut HashSet<String>, s: &SignalDecl) {
    if !seen.insert(s.name.clone()) {
        return;
    }
    out.push(signal_field(s));
}

fn signal_field(s: &SignalDecl) -> SignalField {
    SignalField {
        name: s.name.clone(),
        params: s
            .params
            .iter()
            .map(|p| SignalParam {
                name: p.name.clone(),
                ty: display_type(&p.ty),
            })
            .collect(),
    }
}

pub fn collect_fns(program: &[Item]) -> Vec<FnMeta> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_fns_into(program, &mut out, &mut seen);
    out
}

fn collect_fns_into(program: &[Item], out: &mut Vec<FnMeta>, seen: &mut HashSet<String>) {
    for item in program {
        match item {
            Item::FnDecl(f) => push_fn(out, seen, f),
            Item::ClassDecl(c) => {
                for m in c.all_methods() {
                    push_fn(out, seen, m);
                }
            }
            Item::ImplDecl { methods, .. } => {
                for m in methods {
                    push_fn(out, seen, m);
                }
            }
            Item::Mod(m) => collect_fns_into(&m.items, out, seen),
            _ => {}
        }
    }
}

fn push_fn(out: &mut Vec<FnMeta>, seen: &mut HashSet<String>, f: &crate::parser::FnDecl) {
    if !seen.insert(f.name.clone()) {
        return;
    }
    out.push(FnMeta {
        name: f.name.clone(),
        params: f
            .params
            .iter()
            .filter(|p| p.name != "self")
            .map(|p| SignalParam {
                name: p.name.clone(),
                ty: display_type(&p.ty),
            })
            .collect(),
    });
}

fn display_type(ty: &Type) -> String {
    if ty.name == "String" {
        "Str".into()
    } else {
        ty.name.clone()
    }
}
