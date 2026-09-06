//! Parser unit tests (docs, `@ufcs` / `@node`, class/trait/enum shape).

use super::*;
use crate::lexer::Lexer;

fn parse(src: &str) -> Vec<Item> {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse().unwrap()
}

#[test]
fn docs_attach_to_export_var() {
    let items = parse("## Degrees per second.\n@export var spin: Float = 8.0;\n");
    let Item::VarDecl(v) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(v.name, "spin");
    assert_eq!(v.doc.as_deref(), Some("Degrees per second."));
    assert!(v.exported);
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
fn node_attr_sets_flag() {
    let items =
        parse("@node\nclass MyNode extends Sprite {\n    fn on_create(self) { pass; }\n}\n");
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert!(c.is_node);
    assert_eq!(c.name, "MyNode");
    assert_eq!(c.parent.as_deref(), Some("Sprite"));
}

#[test]
fn class_export_fields_parse() {
    let items = parse(
        r#"
@node
class Player extends Node {
    @export_group("Health")
    @export var max_health: Float;
    @export var current_health: Float = 10.0;
}
"#,
    );
    let Item::ClassDecl(c) = &items[0] else {
        panic!("{:?}", items[0])
    };
    assert_eq!(c.exported_fields.len(), 2);
    assert!(c.exported_fields[0].exported);
    assert_eq!(c.exported_fields[0].name, "max_health");
    assert_eq!(c.exported_fields[0].export_group.as_deref(), Some("Health"));
    assert_eq!(c.exported_fields[1].name, "current_health");
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
