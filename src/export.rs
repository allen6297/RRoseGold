//! Inspector metadata: `@export var` fields and `@node class` types.
//! Parse-only; no evaluation.

use serde::Serialize;

use crate::lexer::Lexer;
use crate::parser::{Expr, ExprKind, Item, Literal, Parser, UnaryOp, VarDecl};

/// Inspector metadata for one `@export var`. Parse-only; no eval.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportField {
    pub name: String,
    /// `Int` | `Float` | `Bool` | `Str`
    pub ty: String,
    pub group: Option<String>,
    pub default: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

const EXPORT_TYPES: &[&str] = &["Int", "Float", "Bool", "Str", "String"];

/// Collect `@export var` fields from source. Empty if the file does not parse.
pub fn list_exports(source: &str) -> Vec<ExportField> {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return Vec::new();
    };
    let Ok(program) = Parser::new(tokens).parse() else {
        return Vec::new();
    };
    collect_exports(&program)
}

pub fn collect_exports(program: &[Item]) -> Vec<ExportField> {
    let mut out = Vec::new();
    collect_exports_into(program, &mut out);
    out
}

fn collect_exports_into(program: &[Item], out: &mut Vec<ExportField>) {
    for item in program {
        match item {
            Item::VarDecl(v) => {
                if let Some(field) = export_field(v) {
                    out.push(field);
                }
            }
            Item::ClassDecl(c) => {
                for v in &c.exported_fields {
                    if let Some(field) = export_field(v) {
                        out.push(field);
                    }
                }
            }
            Item::Mod(m) => collect_exports_into(&m.items, out),
            _ => {}
        }
    }
}

fn export_field(v: &VarDecl) -> Option<ExportField> {
    if !v.exported {
        return None;
    }
    let ty = normalize_type(&v.ty.name)?;
    let default = v
        .value
        .as_ref()
        .map(literal_json)
        .unwrap_or(serde_json::Value::Null);
    Some(ExportField {
        name: v.name.clone(),
        ty,
        group: v.export_group.clone(),
        default,
        doc: v.doc.clone().filter(|s| !s.is_empty()),
    })
}

fn normalize_type(name: &str) -> Option<String> {
    if !EXPORT_TYPES.iter().any(|t| *t == name) {
        return None;
    }
    Some(if name == "String" {
        "Str".into()
    } else {
        name.to_string()
    })
}

fn literal_json(expr: &Expr) -> serde_json::Value {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(n)) => serde_json::json!(n),
        ExprKind::Literal(Literal::Float(n)) => serde_json::json!(n),
        ExprKind::Literal(Literal::String(s)) => serde_json::json!(s),
        ExprKind::Literal(Literal::Bool(b)) => serde_json::json!(b),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => match literal_json(expr) {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    serde_json::json!(-i)
                } else if let Some(f) = n.as_f64() {
                    serde_json::json!(-f)
                } else {
                    serde_json::Value::Null
                }
            }
            other => other,
        },
        _ => serde_json::Value::Null,
    }
}

pub fn value_from_json(ty: &str, raw: &serde_json::Value) -> Option<crate::Value> {
    use crate::Value;
    match ty {
        "Int" => match raw {
            serde_json::Value::Number(n) => n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .map(Value::Int),
            _ => None,
        },
        "Float" => match raw {
            serde_json::Value::Number(n) => n.as_f64().map(Value::Float),
            _ => None,
        },
        "Bool" => raw.as_bool().map(Value::Bool),
        "Str" | "String" => match raw {
            serde_json::Value::String(s) => Some(Value::String(s.clone())),
            serde_json::Value::Number(n) => Some(Value::String(n.to_string())),
            serde_json::Value::Bool(b) => Some(Value::String(b.to_string())),
            _ => None,
        },
        _ => None,
    }
}

/// `@node` class metadata for Add Node. Parse-only.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeClass {
    pub name: String,
    pub parent: String,
    /// Engine entity kind: `sprite`, `empty`, …
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

pub fn list_nodes(source: &str) -> Vec<NodeClass> {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return Vec::new();
    };
    let Ok(program) = Parser::new(tokens).parse() else {
        return Vec::new();
    };
    collect_nodes(&program)
}

pub fn collect_nodes(program: &[Item]) -> Vec<NodeClass> {
    let mut out = Vec::new();
    for item in program {
        if let Item::ClassDecl(c) = item {
            if !c.is_node {
                continue;
            }
            let parent = c.parent.clone().unwrap_or_default();
            let kind = crate::stdlib::node_kind(&parent)
                .unwrap_or("sprite")
                .to_string();
            out.push(NodeClass {
                name: c.name.clone(),
                parent,
                kind,
                doc: c.doc.clone().filter(|s| !s.is_empty()),
            });
        }
    }
    out
}
