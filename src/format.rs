//! Pretty-printer (`rosegold fmt`). Keeps `#` comments and `##` docs.

use crate::lexer::Lexer;
use crate::parser::*;

/// Pretty-print parsed source. `#` line comments and `##` docs are kept.
pub fn format_source(source: &str) -> Result<String, String> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse()?;
    Ok(format_program(&program))
}

pub fn format_program(program: &[Item]) -> String {
    let mut p = Printer::new();
    for (i, item) in program.iter().enumerate() {
        if i > 0 {
            p.newline();
        }
        p.item(item);
        if !p.out.ends_with('\n') {
            p.newline();
        }
    }
    if !p.out.ends_with('\n') {
        p.newline();
    }
    p.out
}

struct Printer {
    out: String,
    indent: usize,
}

impl Printer {
    fn new() -> Self {
        Self {
            out: String::new(),
            indent: 0,
        }
    }

    fn write(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn newline(&mut self) {
        self.out.push('\n');
    }

    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }

    fn docs(&mut self, doc: Option<&str>) {
        if let Some(d) = doc {
            for line in d.lines() {
                self.pad();
                self.write("## ");
                self.write(line);
                self.newline();
            }
        }
    }

    fn hashes(&mut self, comments: &[String]) {
        for c in comments {
            self.pad();
            self.write("# ");
            self.write(c);
            self.newline();
        }
    }

    fn vis(&mut self, is_pub: bool) {
        if is_pub {
            self.write("pub ");
        }
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Import(i) => {
                self.pad();
                if i.is_from {
                    self.write("from ");
                    self.write(&i.path[0]);
                    self.write(" import ");
                    self.write(&i.path[1..].join("."));
                } else {
                    self.write("import ");
                    self.write(&i.path.join("."));
                }
                if let Some(alias) = &i.alias {
                    self.write(" as ");
                    self.write(alias);
                }
                self.write(";");
                self.newline();
            }
            Item::FnDecl(f) => self.fn_decl(f, true),
            Item::VarDecl(v) => {
                self.docs(v.doc.as_deref());
                self.pad();
                self.vis(v.is_pub);
                if v.exported {
                    if let Some(g) = &v.export_group {
                        self.write("@export_group(\"");
                        self.write(g);
                        self.write("\")\n");
                        self.pad();
                    }
                    self.write("@export ");
                }
                self.var_decl(v);
                self.newline();
            }
            Item::ConstDecl(c) => {
                self.docs(c.doc.as_deref());
                self.pad();
                self.vis(c.is_pub);
                self.write("const ");
                self.write(&c.name);
                if let Some(ty) = &c.ty {
                    self.write(": ");
                    self.ty(ty);
                }
                self.write(" = ");
                self.expr(&c.value, 0);
                self.write(";");
                self.newline();
            }
            Item::StructDecl(s) => {
                self.docs(s.doc.as_deref());
                self.pad();
                self.vis(s.is_pub);
                self.write("struct ");
                self.write(&s.name);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for f in &s.fields {
                    self.hashes(&f.leading);
                    self.pad();
                    self.write(&f.name);
                    self.write(": ");
                    self.ty(&f.ty);
                    self.write(",");
                    self.newline();
                }
                self.hashes(&s.trailing);
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::ClassDecl(c) => {
                self.docs(c.doc.as_deref());
                self.pad();
                if c.is_node {
                    self.write("@node");
                    self.newline();
                    self.pad();
                }
                self.vis(c.is_pub);
                self.write("class ");
                self.write(&c.name);
                if let Some(p) = &c.parent {
                    self.write(" extends ");
                    self.write(p);
                }
                if !c.impl_traits.is_empty() {
                    self.write(" impl ");
                    self.write(&c.impl_traits.join(", "));
                }
                self.write(" {");
                self.newline();
                self.indent += 1;
                for f in &c.fields {
                    self.hashes(&f.leading);
                    self.pad();
                    if let Some(v) = c.exported_fields.iter().find(|v| v.name == f.name) {
                        if let Some(g) = &v.export_group {
                            self.write("@export_group(\"");
                            self.write(g);
                            self.write("\")\n");
                            self.pad();
                        }
                        self.write("@export ");
                    }
                    self.write("var ");
                    self.write(&f.name);
                    self.write(": ");
                    self.ty(&f.ty);
                    if let Some((_, e)) = c.defaults.iter().find(|(n, _)| n == &f.name) {
                        self.write(" = ");
                        self.expr(e, 0);
                    }
                    self.write(";");
                    self.newline();
                }
                if !c.fields.is_empty() && (!c.methods.is_empty() || !c.trait_impls.is_empty()) {
                    self.newline();
                }
                for m in &c.methods {
                    self.fn_decl(m, true);
                }
                if !c.trait_impls.is_empty() && !c.methods.is_empty() {
                    self.newline();
                }
                for b in &c.trait_impls {
                    self.hashes(&b.leading);
                    self.pad();
                    self.write("impl ");
                    self.write(&b.trait_name);
                    self.write(" {");
                    self.newline();
                    self.indent += 1;
                    for m in &b.methods {
                        self.fn_decl(m, true);
                    }
                    self.hashes(&b.trailing);
                    self.indent -= 1;
                    self.pad();
                    self.write("}");
                    self.newline();
                }
                self.hashes(&c.trailing);
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::TraitDecl(t) => {
                self.docs(t.doc.as_deref());
                self.pad();
                self.vis(t.is_pub);
                self.write("trait ");
                self.write(&t.name);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for s in &t.signals {
                    self.hashes(&s.leading);
                    self.pad();
                    self.signal_sig(s);
                    self.newline();
                }
                for m in &t.methods {
                    self.hashes(&m.leading);
                    self.pad();
                    self.write("fn ");
                    self.write(&m.name);
                    self.write("(");
                    self.params(&m.params);
                    self.write(")");
                    if let Some(rt) = &m.return_type {
                        self.write(": ");
                        self.ty(rt);
                    }
                    self.write(";");
                    self.newline();
                }
                self.hashes(&t.trailing);
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::EnumDecl(e) => {
                self.docs(e.doc.as_deref());
                self.pad();
                self.vis(e.is_pub);
                self.write("enum ");
                self.write(&e.name);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for v in &e.variants {
                    self.hashes(&v.leading);
                    self.pad();
                    self.write(&v.name);
                    if !v.value_types.is_empty() {
                        self.write("(");
                        for (i, ty) in v.value_types.iter().enumerate() {
                            if i > 0 {
                                self.write(", ");
                            }
                            let fname = v.field_names.get(i).map(|s| s.as_str()).unwrap_or("");
                            if !fname.is_empty() {
                                self.write(fname);
                                self.write(": ");
                            }
                            self.ty(ty);
                        }
                        self.write(")");
                    }
                    self.write(",");
                    self.newline();
                }
                self.hashes(&e.trailing);
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::ImplDecl {
                type_name,
                trait_name,
                methods,
                trailing,
                ..
            } => {
                self.pad();
                self.write("impl ");
                if let Some(tr) = trait_name {
                    self.write(tr);
                    self.write(" for ");
                }
                self.write(type_name);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for m in methods {
                    self.fn_decl(m, true);
                }
                self.hashes(trailing);
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::Mod(m) => {
                self.docs(m.doc.as_deref());
                self.pad();
                self.write("mod ");
                self.write(&m.name);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for (i, inner) in m.items.iter().enumerate() {
                    if i > 0 {
                        self.newline();
                    }
                    self.item(inner);
                }
                self.indent -= 1;
                self.pad();
                self.write("}");
                self.newline();
            }
            Item::SignalDecl(s) => {
                self.hashes(&s.leading);
                self.docs(s.doc.as_deref());
                self.pad();
                self.vis(s.is_pub);
                self.signal_sig(s);
                self.newline();
            }
            Item::Comment(text) => {
                self.pad();
                self.write("# ");
                self.write(text);
                self.newline();
            }
        }
    }

    fn fn_decl(&mut self, f: &FnDecl, newline_after: bool) {
        self.hashes(&f.leading);
        self.docs(f.doc.as_deref());
        self.pad();
        self.vis(f.is_pub);
        if f.is_test {
            self.write("@test\n");
            self.pad();
        }
        if f.is_ufcs {
            self.write("@ufcs\n");
            self.pad();
        }
        self.write("fn ");
        self.write(&f.name);
        self.write("(");
        self.params(&f.params);
        self.write(")");
        if let Some(rt) = &f.return_type {
            self.write(": ");
            self.ty(rt);
        }
        self.write(" ");
        self.block(&f.body);
        if newline_after {
            self.newline();
        }
    }

    fn var_decl(&mut self, v: &VarDecl) {
        self.write("var ");
        self.write(&v.name);
        if v.ty.name != "None" && !v.ty.name.is_empty() {
            self.write(": ");
            self.ty(&v.ty);
        }
        if let Some(e) = &v.value {
            self.write(" = ");
            self.expr(e, 0);
        }
        self.write(";");
    }

    fn signal_sig(&mut self, s: &SignalDecl) {
        self.write("signal ");
        self.write(&s.name);
        self.write("(");
        self.params(&s.params);
        self.write(");");
    }

    fn params(&mut self, params: &[Param]) {
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            if p.name == "self" && p.ty.name == "Self" && p.ty.args.is_empty() {
                self.write("self");
            } else {
                self.write(&p.name);
                self.write(": ");
                self.ty(&p.ty);
            }
        }
    }

    fn ty(&mut self, ty: &Type) {
        self.write(&ty.name);
        if !ty.args.is_empty() {
            self.write("<");
            for (i, a) in ty.args.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.ty(a);
            }
            self.write(">");
        }
        if ty.optional {
            self.write("?");
        }
    }

    fn block(&mut self, block: &Block) {
        self.write("{");
        if block.stmts.is_empty() {
            self.write("}");
            return;
        }
        self.newline();
        self.indent += 1;
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        self.indent -= 1;
        self.pad();
        self.write("}");
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Expr(e) => {
                self.pad();
                self.expr(e, 0);
                if !matches!(e.kind, ExprKind::Match { .. }) {
                    self.write(";");
                }
                self.newline();
            }
            StmtKind::Return(value) => {
                self.pad();
                self.write("return");
                if let Some(e) = value {
                    self.write(" ");
                    self.expr(e, 0);
                }
                self.write(";");
                self.newline();
            }
            StmtKind::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.pad();
                self.write("if ");
                self.expr(cond, 0);
                self.write(" ");
                self.block(then_block);
                for (c, b) in elif_blocks {
                    self.write(" elif ");
                    self.expr(c, 0);
                    self.write(" ");
                    self.block(b);
                }
                if let Some(b) = else_block {
                    self.write(" else ");
                    self.block(b);
                }
                self.newline();
            }
            StmtKind::While { cond, body } => {
                self.pad();
                self.write("while ");
                self.expr(cond, 0);
                self.write(" ");
                self.block(body);
                self.newline();
            }
            StmtKind::For { name, iter, body } => {
                self.pad();
                self.write("for ");
                self.write(name);
                self.write(" in ");
                self.expr(iter, 0);
                self.write(" ");
                self.block(body);
                self.newline();
            }
            StmtKind::VarDecl(v) => {
                self.pad();
                self.var_decl(v);
                self.newline();
            }
            StmtKind::ConstDecl(c) => {
                self.pad();
                self.write("const ");
                self.write(&c.name);
                if let Some(ty) = &c.ty {
                    self.write(": ");
                    self.ty(ty);
                }
                self.write(" = ");
                self.expr(&c.value, 0);
                self.write(";");
                self.newline();
            }
            StmtKind::Break => {
                self.pad();
                self.write("break;");
                self.newline();
            }
            StmtKind::Continue => {
                self.pad();
                self.write("continue;");
                self.newline();
            }
            StmtKind::Pass => {
                self.pad();
                self.write("pass;");
                self.newline();
            }
            StmtKind::Comment(text) => {
                self.pad();
                self.write("# ");
                self.write(text);
                self.newline();
            }
        }
    }

    fn expr(&mut self, expr: &Expr, parent_prec: u8) {
        match &expr.kind {
            ExprKind::Literal(l) => self.literal(l),
            ExprKind::Ident(name) => self.write(name),
            ExprKind::Binary { op, left, right } => {
                let prec = bin_prec(*op);
                if prec < parent_prec {
                    self.write("(");
                }
                self.expr(left, prec);
                self.write(" ");
                self.write(bin_sym(*op));
                self.write(" ");
                self.expr(right, prec + 1);
                if prec < parent_prec {
                    self.write(")");
                }
            }
            ExprKind::Unary { op, expr: inner } => {
                self.write(unary_sym(*op));
                self.expr(inner, 13);
            }
            ExprKind::Call { callee, args } => {
                if let ExprKind::Ident(name) = &callee.kind {
                    if name == "Array" {
                        self.write("[");
                        for (i, a) in args.iter().enumerate() {
                            if i > 0 {
                                self.write(", ");
                            }
                            self.expr(a, 0);
                        }
                        self.write("]");
                        return;
                    }
                    if name == "Map" {
                        self.write("{");
                        let mut i = 0;
                        while i + 1 < args.len() {
                            if i > 0 {
                                self.write(", ");
                            }
                            self.expr(&args[i], 0);
                            self.write(": ");
                            self.expr(&args[i + 1], 0);
                            i += 2;
                        }
                        self.write("}");
                        return;
                    }
                }
                self.expr(callee, 14);
                self.write("(");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.expr(a, 0);
                }
                self.write(")");
            }
            ExprKind::Member { object, name } => {
                self.expr(object, 14);
                self.write(".");
                self.write(name);
            }
            ExprKind::Index { object, index } => {
                self.expr(object, 14);
                self.write("[");
                self.expr(index, 0);
                self.write("]");
            }
            ExprKind::StructLiteral { name, fields } => {
                self.write(name);
                self.write(" {");
                if !fields.is_empty() {
                    self.write(" ");
                    for (i, (n, e)) in fields.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write(n);
                        self.write(": ");
                        self.expr(e, 0);
                    }
                    self.write(" ");
                }
                self.write("}");
            }
            ExprKind::Assign { op, left, right } => {
                self.expr(left, 0);
                self.write(" ");
                self.write(assign_sym(*op));
                self.write(" ");
                self.expr(right, 0);
            }
            ExprKind::FString(parts) => {
                self.write("f\"");
                for part in parts {
                    match part {
                        FStringPart::Text(t) => {
                            for c in t.chars() {
                                match c {
                                    '"' => self.write("\\\""),
                                    '\\' => self.write("\\\\"),
                                    '{' => self.write("{{"),
                                    '}' => self.write("}}"),
                                    '\n' => self.write("\\n"),
                                    other => self.out.push(other),
                                }
                            }
                        }
                        FStringPart::Expr { expr, format } => {
                            self.write("{");
                            self.expr(expr, 0);
                            if let Some(fmt) = format {
                                self.write(":");
                                self.write(fmt);
                            }
                            self.write("}");
                        }
                    }
                }
                self.write("\"");
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                self.expr(start, 2);
                self.write(if *inclusive { "..=" } else { ".." });
                self.expr(end, 2);
            }
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.write("match ");
                self.expr(scrutinee, 0);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for arm in arms {
                    self.pad();
                    self.pattern(&arm.pattern);
                    self.write(" ");
                    self.block(&arm.body);
                    self.newline();
                }
                self.indent -= 1;
                self.pad();
                self.write("}");
            }
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard => self.write("_"),
            Pattern::Variant {
                name,
                binds,
                field_binds,
            } => {
                self.write(name);
                if binds.is_empty() && field_binds.is_empty() {
                    return;
                }
                self.write("(");
                let mut first = true;
                for b in binds {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.write(b);
                }
                for (field, bind) in field_binds {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.write(field);
                    self.write(": ");
                    self.write(bind);
                }
                self.write(")");
            }
        }
    }

    fn literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Int(n) => self.write(&n.to_string()),
            Literal::Float(n) => {
                if n.fract() == 0.0 {
                    self.write(&format!("{n:.1}"));
                } else {
                    self.write(&n.to_string());
                }
            }
            Literal::String(s) => {
                self.write("\"");
                for c in s.chars() {
                    match c {
                        '"' => self.write("\\\""),
                        '\\' => self.write("\\\\"),
                        '\n' => self.write("\\n"),
                        '\t' => self.write("\\t"),
                        other => self.out.push(other),
                    }
                }
                self.write("\"");
            }
            Literal::Bool(b) => self.write(if *b { "true" } else { "false" }),
            Literal::None => self.write("none"),
        }
    }
}

fn bin_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 3,
        BinOp::And => 4,
        BinOp::BitOr => 5,
        BinOp::BitXor => 6,
        BinOp::BitAnd => 7,
        BinOp::Eq | BinOp::Neq => 8,
        BinOp::Lt | BinOp::Lte | BinOp::Gt | BinOp::Gte => 9,
        BinOp::Shl | BinOp::Shr => 10,
        BinOp::Add | BinOp::Sub => 11,
        BinOp::Mul | BinOp::Div | BinOp::IDiv | BinOp::Mod => 12,
    }
}

fn bin_sym(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::IDiv => "//",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Lte => "<=",
        BinOp::Gt => ">",
        BinOp::Gte => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
    }
}

fn unary_sym(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Not => "!",
        UnaryOp::BitNot => "~",
    }
}

fn assign_sym(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::Add => "+=",
        AssignOp::Sub => "-=",
        AssignOp::Mul => "*=",
        AssignOp::Div => "/=",
        AssignOp::Mod => "%=",
        AssignOp::BitAnd => "&=",
        AssignOp::BitOr => "|=",
        AssignOp::BitXor => "^=",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_roundtrip_hello() {
        let src = "fn main(): Int {\n    print(\"hi\");\n    return 0;\n}\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("fn main(): Int"));
        assert!(out.contains("print(\"hi\");"));
        let again = format_source(&out).unwrap();
        assert_eq!(out, again);
    }

    #[test]
    fn format_keeps_parens_for_precedence() {
        let out = format_source("fn main(): Int { print((1 + 2) * 3); return 0; }").unwrap();
        assert!(out.contains("(1 + 2) * 3"), "{out}");
    }

    #[test]
    fn format_trait_signal() {
        let out = format_source(
            "trait Damageable { signal died(); fn take_damage(damage: Float): Float; }\n",
        )
        .unwrap();
        assert!(out.contains("signal died();"), "{out}");
        assert!(out.contains("fn take_damage"), "{out}");
    }
}
