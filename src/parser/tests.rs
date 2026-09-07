//! Parser unit tests (docs, `@ufcs`, class/trait/enum shape).

use super::*;
use crate::lexer::Lexer;

fn parse(src: &str) -> Vec<Item> {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse().unwrap()
}

#[test]
fn docs_attach_to_var() {
    let items = parse("## Degrees per second.\nvar spin: Float = 8.0;\n");
    let Item::VarDecl(v) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(v.name, "spin");
    assert_eq!(v.doc.as_deref(), Some("Degrees per second."));
}

#[test]
fn docs_join_consecutive_lines() {
    let items = parse("## a\n## b\nfn foo(): Int { return 0; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(f.doc.as_deref(), Some("a\nb"));
}

#[test]
fn hash_comment_does_not_attach() {
    let items = parse("# not a doc\nfn foo(): Int { return 0; }\n");
    assert!(matches!(&items[0], Item::Comment(s) if s == "not a doc"));
    let Item::FnDecl(f) = &items[1] else {
        panic!("{:?}", items[1])
    };
    assert_eq!(f.doc, None);
}

#[test]
fn docs_attach_to_local_var() {
    let items =
        parse("fn main(): Int {\n    ## local counter\n    var n: Int = 1;\n    return n;\n}\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let StmtKind::VarDecl(v) = &f.body.stmts[0].kind else {
        panic!("{:?}", f.body.stmts[0].kind)
    };
    assert_eq!(v.name, "n");
    assert_eq!(v.doc.as_deref(), Some("local counter"));
}

#[test]
fn docs_attach_to_class_field() {
    let items = parse("class Point {\n    ## X component\n    var x: Float = 0.0;\n}\n");
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(c.fields[0].name, "x");
    assert_eq!(c.fields[0].doc.as_deref(), Some("X component"));
}

#[test]
fn docs_inside_body_do_not_fail_parse() {
    let items = parse("fn foo(): Int {\n    ## ignored\n    return 0;\n}\n");
    assert!(matches!(&items[0], Item::FnDecl(_)));
}

#[test]
fn ufcs_attr_sets_flag() {
    let items = parse("@ufcs\nfn clamp(n: Float): Float { return n; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert!(f.is_ufcs);
    assert_eq!(f.name, "clamp");
}

#[test]
fn unknown_attr_is_error() {
    let tokens = Lexer::new("@node\nclass Foo {}\n").tokenize().unwrap();
    let err = Parser::new(tokens).parse().unwrap_err();
    assert!(err.contains("unknown attribute '@node'"), "{err}");
}

#[test]
fn class_fields_parse() {
    let items = parse(
        r#"
class Player {
    var max_health: Float;
    var current_health: Float = 10.0;
}
"#,
    );
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(c.fields.len(), 2);
    assert_eq!(c.fields[0].name, "max_health");
    assert_eq!(c.fields[1].name, "current_health");
}

#[test]
fn class_and_trait_parse() {
    let items = parse(
        r#"
class Vec2 {
    var x: Float = 0.0;
    var y: Float = 0.0;
    fn length(self): Float { return self.x; }
}

trait HasLength {
    fn length(self): Float;
}

impl HasLength for Vec2 {
    fn length(self): Float { return self.x; }
}
"#,
    );
    assert!(
        matches!(&items[0], Item::ClassDecl(c) if c.name == "Vec2" && c.methods.len() == 1 && c.parent.is_none())
    );
    assert!(
        matches!(&items[1], Item::TraitDecl(t) if t.name == "HasLength" && t.methods.len() == 1 && t.signals.is_empty())
    );
    assert!(matches!(
      &items[2],
      Item::ImplDecl {
        type_name,
        trait_name: Some(tr),
        ..
      } if type_name == "Vec2" && tr == "HasLength"
    ));
}

#[test]
fn trait_signal_parse() {
    let items = parse(
        r#"
trait Damageable {
    signal died();
    signal hurt(amount: Float);
    fn take_damage(damage: Float): Float;
}
"#,
    );
    let Item::TraitDecl(t) = &items[0] else {
        panic!("{:?}", items[0]);
    };
    assert_eq!(t.name, "Damageable");
    assert_eq!(t.methods.len(), 1);
    assert_eq!(t.signals.len(), 2);
    assert_eq!(t.signals[0].name, "died");
    assert_eq!(t.signals[1].name, "hurt");
    assert_eq!(t.signals[1].params.len(), 1);
}

#[test]
fn trait_rejects_var() {
    let tokens = Lexer::new("trait Damageable { var hp: Float; }\n")
        .tokenize()
        .unwrap();
    let err = Parser::new(tokens).parse().unwrap_err();
    assert!(err.contains("vars or consts"), "{err}");
}

#[test]
fn class_nested_trait_impl_parse() {
    let items = parse(
        r#"
class Point {
    var x: Float = 0.0;
    impl Named {
        fn label(self): String { return "point"; }
    }
}
"#,
    );
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(c.name, "Point");
    assert_eq!(c.methods.len(), 0);
    assert_eq!(c.trait_impls.len(), 1);
    assert_eq!(c.trait_impls[0].trait_name, "Named");
    assert_eq!(c.trait_impls[0].methods[0].name, "label");
}

#[test]
fn class_header_impl_traits_parse() {
    let items = parse(
        r#"
class Vec3 extends Point impl Named, Drawable {
    var z: Float = 0.0;
    fn label(self): String { return "vec3"; }
}
"#,
    );
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(c.parent.as_deref(), Some("Point"));
    assert_eq!(
        c.impl_traits,
        vec!["Named".to_string(), "Drawable".to_string()]
    );
    assert_eq!(c.methods.len(), 1);
}

#[test]
fn class_extends_parse() {
    let items = parse(
        r#"
class Enemy {
    var hp: Float = 10.0;
}
class Slime extends Enemy {
    var goo: Float = 1.0;
}
"#,
    );
    assert!(matches!(&items[0], Item::ClassDecl(c) if c.name == "Enemy" && c.parent.is_none()));
    assert!(matches!(
      &items[1],
      Item::ClassDecl(c) if c.name == "Slime" && c.parent.as_deref() == Some("Enemy") && c.fields.len() == 1
    ));
}

#[test]
fn arrow_return_type_parses() {
    let items = parse("fn foo() -> Int { return 1; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(f.return_type.as_ref().map(|t| t.name.as_str()), Some("Int"));
}

#[test]
fn pub_and_named_enum_fields() {
    let items = parse(
        r#"
mod shapes {
    pub enum Shape {
        Circle(radius: Float),
        Rect(Float, Float),
    }
    fn helper() {}
    pub fn area() {}
}
"#,
    );
    let Item::Mod(m) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let Item::EnumDecl(e) = &m.items[0] else {
        panic!("{:?}", m.items[0])
    };
    assert!(e.is_pub);
    assert_eq!(e.variants[0].field_names, vec!["radius".to_string()]);
    assert_eq!(
        e.variants[1].field_names,
        vec!["".to_string(), "".to_string()]
    );
    let Item::FnDecl(helper) = &m.items[1] else {
        panic!("{:?}", m.items[1])
    };
    assert!(!helper.is_pub);
    let Item::FnDecl(area) = &m.items[2] else {
        panic!("{:?}", m.items[2])
    };
    assert!(area.is_pub);
}

#[test]
fn spawn_call_parses() {
    let items = parse("fn main(): Int { var t = spawn foo(); return 0; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let StmtKind::VarDecl(v) = &f.body.stmts[0].kind else {
        panic!("{:?}", f.body.stmts[0].kind)
    };
    let ExprKind::Spawn(inner) = &v.value.as_ref().unwrap().kind else {
        panic!("{:?}", v.value)
    };
    assert!(matches!(inner.kind, ExprKind::Call { .. }));
}

#[test]
fn spawn_non_call_is_error() {
    let tokens = Lexer::new("fn main(): Int { spawn x; return 0; }\n")
        .tokenize()
        .unwrap();
    let err = Parser::new(tokens).parse().unwrap_err();
    assert!(err.contains("spawn expects a call"), "{err}");
}

#[test]
fn await_parses() {
    let items = parse("fn main(): Int { await t; return 0; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let StmtKind::Expr(e) = &f.body.stmts[0].kind else {
        panic!("{:?}", f.body.stmts[0].kind)
    };
    assert!(matches!(e.kind, ExprKind::Await(_)));
}

fn first_expr(src: &str) -> ExprKind {
    let items = parse(src);
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    match &f.body.stmts[0].kind {
        StmtKind::Expr(e) => e.kind.clone(),
        StmtKind::VarDecl(v) => v.value.as_ref().unwrap().kind.clone(),
        other => panic!("{:?}", other),
    }
}

fn assert_trailing_call(kind: &ExprKind, arg_count: usize) {
    let ExprKind::Call { args, .. } = kind else {
        panic!("expected Call, got {kind:?}")
    };
    assert_eq!(args.len(), arg_count, "{args:?}");
    let ExprKind::Lambda {
        params,
        return_type,
        ..
    } = &args[arg_count - 1].kind
    else {
        panic!(
            "expected trailing Lambda, got {:?}",
            args[arg_count - 1].kind
        )
    };
    assert!(params.is_empty(), "{params:?}");
    assert!(return_type.is_none());
}

#[test]
fn trailing_closure_after_call() {
    assert_trailing_call(
        &first_expr("fn main(): Int { foo(1) { print(x); }; return 0; }\n"),
        2,
    );
    assert_trailing_call(
        &first_expr("fn main(): Int { foo() { print(1); }; return 0; }\n"),
        1,
    );
    assert_trailing_call(&first_expr("fn main(): Int { foo() {}; return 0; }\n"), 1);
}

#[test]
fn trailing_closure_after_ident() {
    assert_trailing_call(
        &first_expr("fn main(): Int { column { print(1); }; return 0; }\n"),
        1,
    );
}

#[test]
fn trailing_closure_after_member_call() {
    let kind = first_expr("fn main(): Int { ui.button(\"OK\") { print(1); }; return 0; }\n");
    let ExprKind::Call { callee, args } = &kind else {
        panic!("{kind:?}")
    };
    assert!(matches!(callee.kind, ExprKind::Member { .. }), "{callee:?}");
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1].kind, ExprKind::Lambda { .. }));
}

#[test]
fn trailing_map_after_member_call() {
    let kind = first_expr("fn main(): Int { ui.theme({ \"bg\": \"x\" }); return 0; }\n");
    let ExprKind::Call { callee, args } = &kind else {
        panic!("{kind:?}")
    };
    assert!(matches!(callee.kind, ExprKind::Member { .. }), "{callee:?}");
    assert_eq!(args.len(), 1);
    assert!(matches!(&args[0].kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Ident(n) if n == "Map")));
}

#[test]
fn trailing_closure_then_method() {
    let kind = first_expr(
        "fn main(): Int { ui.button(\"OK\") { print(1); }.padding(8); return 0; }\n",
    );
    let ExprKind::Call { callee, args } = &kind else {
        panic!("{kind:?}")
    };
    assert_eq!(args.len(), 1);
    let ExprKind::Member { object, name } = &callee.kind else {
        panic!("{:?}", callee.kind)
    };
    assert_eq!(name, "padding");
    assert!(matches!(object.kind, ExprKind::Call { .. }));
}

#[test]
fn struct_literal_not_trailing_closure() {
    assert!(matches!(
        first_expr("fn main(): Int { var p = Point {}; return 0; }\n"),
        ExprKind::StructLiteral { name, fields }
            if name == "Point" && fields.is_empty()
    ));
    assert!(matches!(
        first_expr("fn main(): Int { var p = Point { x: 1 }; return 0; }\n"),
        ExprKind::StructLiteral { name, fields }
            if name == "Point" && fields.len() == 1 && fields[0].0 == "x"
    ));
}

#[test]
fn if_while_for_match_keep_their_blocks() {
    let items = parse(
        "fn main(): Int {\n    if foo() { return 1; }\n    while bar() { break; }\n    for x in xs { pass; }\n    match self {\n        None { return 0; }\n    }\n    return 0;\n}\n",
    );
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert!(matches!(f.body.stmts[0].kind, StmtKind::If { .. }));
    assert!(matches!(f.body.stmts[1].kind, StmtKind::While { .. }));
    assert!(matches!(f.body.stmts[2].kind, StmtKind::For { .. }));
    assert!(matches!(
        f.body.stmts[3].kind,
        StmtKind::Expr(Expr {
            kind: ExprKind::Match { .. },
            ..
        })
    ));
}

#[test]
fn map_literal_trailing_comma() {
    let kind = first_expr("fn main(): Int { var m = { \"a\": 1, }; return 0; }\n");
    let ExprKind::Call { callee, args } = &kind else {
        panic!("{kind:?}")
    };
    assert!(matches!(&callee.kind, ExprKind::Ident(n) if n == "Map"));
    assert_eq!(args.len(), 2);
}

#[test]
fn map_literal_stays_map() {
    let kind = first_expr("fn main(): Int { var m = { \"a\": 1 }; return 0; }\n");
    let ExprKind::Call { callee, args } = &kind else {
        panic!("{kind:?}")
    };
    assert!(matches!(&callee.kind, ExprKind::Ident(n) if n == "Map"));
    assert_eq!(args.len(), 2);
}

#[test]
fn lambda_parses() {
    let items =
        parse("fn main(): Int { var f = fn(x: Int): Int { return x + 1; }; return f(1); }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let StmtKind::VarDecl(v) = &f.body.stmts[0].kind else {
        panic!("{:?}", f.body.stmts[0].kind)
    };
    assert!(matches!(
        v.value.as_ref().unwrap().kind,
        ExprKind::Lambda { .. }
    ));
}

#[test]
fn spawn_lambda_parses() {
    let items = parse("fn main(): Int { spawn fn() { pass; }; return 0; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    let StmtKind::Expr(e) = &f.body.stmts[0].kind else {
        panic!("{:?}", f.body.stmts[0].kind)
    };
    let ExprKind::Spawn(inner) = &e.kind else {
        panic!("{:?}", e.kind)
    };
    assert!(matches!(inner.kind, ExprKind::Lambda { .. }));
}

#[test]
fn try_postfix_parses() {
    let kind = first_expr("fn go(): Result { var n = Result.Ok(1)?; return n; }\n");
    let ExprKind::Try(inner) = kind else {
        panic!("expected Try, got {kind:?}")
    };
    assert!(matches!(
        inner.kind,
        ExprKind::Call { .. } | ExprKind::Member { .. }
    ));
}

#[test]
fn try_postfix_after_call() {
    let kind = first_expr("fn go(): Result { load()?; return Result.Ok(0); }\n");
    assert!(matches!(kind, ExprKind::Try(_)), "{kind:?}");
}

#[test]
fn async_fn_sets_flag() {
    let items = parse("async fn add(a: Int, b: Int): Int { return a + b; }\n");
    let Item::FnDecl(f) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert!(f.is_async);
    assert_eq!(f.name, "add");
    assert!(
        !parse("fn add(a: Int, b: Int): Int { return a + b; }\n")
            .into_iter()
            .any(|item| matches!(item, Item::FnDecl(f) if f.is_async))
    );
}

#[test]
fn async_method_and_trait_signature() {
    let items = parse(
        r#"
trait Loader {
    async fn load(): Int;
}
class Box {
    async fn get(): Int {
        return 1;
    }
}
"#,
    );
    let Item::TraitDecl(t) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert!(t.methods[0].is_async);
    let Item::ClassDecl(c) = &items[1] else {
        panic!("{:?}", items[1])
    };
    assert!(c.methods[0].is_async);
}

#[test]
fn async_without_fn_is_error() {
    let tokens = Lexer::new("async add(): Int { return 1; }\n")
        .tokenize()
        .unwrap();
    let err = Parser::new(tokens).parse().unwrap_err();
    assert!(
        err.contains("expected 'fn' after 'async'") || err.contains("expected fn"),
        "{err}"
    );
}
