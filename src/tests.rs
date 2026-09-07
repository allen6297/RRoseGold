//! Language and runtime tests for the public crate API (`run_source`, `check_*`, …).

use super::*;
use std::path::{Path, PathBuf};

fn example(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(rel)
}

fn assert_ok(source: &str) -> String {
    let result = run_source(source);
    assert!(result.ok, "{}\nstderr: {}", result.message, result.stderr);
    result.stdout
}

#[test]
fn hello_main() {
    let out = assert_ok(r#"fn main(): Int { print("hello"); return 0; }"#);
    assert_eq!(out, "hello\n");
}

#[test]
fn host_module_requires_import() {
    let diags = check_source(
        "fn main(): Int { print(io.exists(\".\")); return 0; }\n",
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("undefined") || d.message.contains("io")),
        "{diags:?}"
    );
}

#[test]
fn check_hello_rg_is_clean() {
    let diags = check_file(&example("hello.rg"));
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn check_io_time_rg_is_clean() {
    let diags = check_file(&example("io_time.rg"));
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn check_tour_and_json_rg_are_clean() {
    for name in ["tour.rg", "json.rg", "tests.rg", "signals.rg", "process.rg"] {
        let diags = check_file(&example(name));
        assert!(diags.is_empty(), "{name}: {diags:?}");
    }
}

#[test]
fn str_alias_is_string() {
    let out = assert_ok(
        r#"
var s: Str = "hello";
fn greet(name: Str): Str {
    return name;
}
fn main(): Int {
    print(greet(s));
    return 0;
}
"#,
    );
    assert_eq!(out, "hello\n");
}

#[test]
fn eval_context_persists_module_vars() {
    let program = compile_source(
        r#"
var n: Int = 0;
fn bump(): Int {
    n = n + 1;
    return n;
}
"#,
    )
    .expect("compile");
    let mut ctx = EvalContext::new();
    ctx.load_program(&program).expect("load");
    assert_eq!(ctx.call("bump", vec![]).unwrap(), Value::Int(1));
    assert_eq!(ctx.call("bump", vec![]).unwrap(), Value::Int(2));
}

#[test]
fn arithmetic_and_loops() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var i: Int = 0;
    var sum: Int = 0;
    while i < 5 {
        sum = sum + i;
        i = i + 1;
    }
    print(sum);
    return 0;
}
"#,
    );
    assert_eq!(out, "10\n");
}

#[test]
fn int_equals_whole_float() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var hp: Float = 100.0;
    hp = hp - 100.0;
    if hp == 0.0 {
        print("dead");
    }
    if 0 == 0.0 {
        print("mixed");
    }
    return 0;
}
"#,
    );
    assert!(out.contains("dead"), "{out}");
    assert!(out.contains("mixed"), "{out}");
}

#[test]
fn fstring_interpolation() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var x: Int = 7;
    print(f"x = {x}, double = {x + x}");
    return 0;
}
"#,
    );
    assert_eq!(out, "x = 7, double = 14\n");
}

#[test]
fn range_for_loops() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var sum: Int = 0;
    for i in 0..5 {
        sum = sum + i;
    }
    print(sum);
    var prod: Int = 1;
    for j in 1..=3 {
        prod = prod * j;
    }
    print(prod);
    return 0;
}
"#,
    );
    assert!(out.contains("10"));
    assert!(out.contains("6"));
}

#[test]
fn math_stdlib() {
    let out = assert_ok(
        r#"
import math;

fn main(): Int {
    print(math.clamp(5, 0, 3));
    print(math.pow(2, 10));
    print(math.abs(-7));
    print(math.sign(-42));
    print(math.min(3, 8));
    print(math.max(3, 8));
    print(math.to_int(3.14));
    print(math.sqrt(4));
    print(math.sin(0));
    print(math.cos(0));
    print(math.atan2(0, 1));
    print(math.lerp(0.0, 10.0, 0.5));
    print(math.move_toward(0.0, 10.0, 3.0));
    return 0;
}
"#,
    );
    assert!(out.contains("3"));
    assert!(out.contains("1024"));
    assert!(out.contains("7"));
    assert!(out.contains("-1"));
    assert!(out.contains("3"));
    assert!(out.contains("8"));
    assert!(out.contains("3"));
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines.iter().any(|l| *l == "2" || *l == "2.0"),
        "sqrt(4) missing in {out:?}"
    );
    assert!(
        lines.iter().any(|l| *l == "0" || *l == "0.0"),
        "sin(0)/atan2 missing in {out:?}"
    );
    assert!(
        lines.iter().any(|l| *l == "1" || *l == "1.0"),
        "cos(0) missing in {out:?}"
    );
    assert!(
        lines.iter().any(|l| *l == "5" || *l == "5.0"),
        "lerp(0, 10, 0.5) missing in {out:?}"
    );
    assert!(
        lines.iter().any(|l| *l == "3" || *l == "3.0"),
        "move_toward missing in {out:?}"
    );
}

#[test]
fn embedded_stdlib_is_crate_rg() {
    let math = crate::stdlib::SOURCES
        .iter()
        .find(|(n, _)| *n == "math")
        .map(|(_, src)| *src)
        .expect("math.rg");
    assert!(math.contains("fn lerp"), "lerp should live in math.rg");
    assert!(
        math.contains("__math.sin"),
        "math.sin should wrap the native primitive"
    );
    assert!(crate::stdlib::SOURCES.iter().any(|(n, _)| *n == "option"));
    assert!(crate::stdlib::SOURCES.iter().any(|(n, _)| *n == "result"));
    assert!(crate::stdlib::SOURCES.iter().any(|(n, _)| *n == "str"));
    assert!(crate::stdlib::SOURCES.iter().any(|(n, _)| *n == "vec"));
    assert!(!crate::stdlib::SOURCES.iter().any(|(n, _)| *n == "node"));
}

#[test]
fn project_math_rg_does_not_override_crate() {
    let mut modules = HashMap::new();
    modules.insert(
        "math.rg".into(),
        "mod math { pub fn lerp(a: Float, b: Float, t: Float): Float { return 0.0; } }\n".into(),
    );
    let out = run_source_with_modules(
        "import math;\nfn main(): Int { print(math.lerp(0.0, 10.0, 0.5)); return 0; }\n",
        modules,
    );
    assert!(out.ok, "{}", out.message);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines.iter().any(|l| *l == "5" || *l == "5.0"),
        "crate math.lerp should win, got {:?}",
        out.stdout
    );
}

#[test]
fn checks_stdlib() {
    let out = assert_ok(
        r#"
import checks;

fn main(): Int {
    checks.that(true);
    checks.eq(2 + 2, 4);
    checks.eq_string("hi", "hi");
    return 0;
}
"#,
    );
    assert_eq!(out, "");
}

#[test]
fn elif_branch() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var x: Int = 2;
    if x == 1 {
        print("one");
    } elif x == 2 {
        print("two");
    } else {
        print("other");
    }
    return 0;
}
"#,
    );
    assert_eq!(out, "two\n");
}

#[test]
fn map_literals_and_methods() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var scores: Map<String, Int> = {"ada": 10, "grace": 12};
    scores["linus"] = 9;
    print(scores.len());
    print(scores.has("ada"));
    print(scores.has("missing"));
    print(scores["grace"]);
    var keys = scores.keys();
    print(keys.len());
    print(scores.remove("ada"));
    print(scores.has("ada"));
    return 0;
}
"#,
    );
    assert!(out.contains("3"));
    assert!(out.contains("true"));
    assert!(out.contains("false"));
    assert!(out.contains("12"));
    assert!(out.contains("10"));
    assert!(out.contains("false"));
}

#[test]
fn match_simple() {
    let out = assert_ok(
        r#"
fn main(): Int {
    const x = Option.Some(1);
    match x {
        Some(v) { print(v); }
        None { print("none"); }
    }
    return 0;
}
"#,
    );
    assert!(out.contains("1"));
}

#[test]
fn enum_option_and_match() {
    let out = assert_ok(
        r#"
fn main(): Int {
    const maybe: Option<Int> = Option.Some(42);
    print(maybe.unwrap_or(0));
    print(Option.None.unwrap_or(7));
    match maybe {
        Some(v) { print(v); }
        None { print("none"); }
    }
    return 0;
}
"#,
    );
    assert!(out.contains("42"));
    assert!(out.contains("7"));
}

#[test]
fn enum_result_and_match() {
    let out = assert_ok(
        r#"
fn lookup(m: Map<String, Int>, key: String): Result<Int, String> {
    if m.has(key) {
        return Result.Ok(m[key]);
    }
    return Result.Err("missing key");
}

fn main(): Int {
    var scores: Map<String, Int> = {"alice": 10, "bob": 7};
    const found = lookup(scores, "alice");
    match found {
        Ok(v) { print(v); }
        Err(_) { print("not found"); }
    }
    const missing = lookup(scores, "zed");
    if missing.is_err() {
        print("err");
    }
    return 0;
}
"#,
    );
    assert!(out.contains("10"));
    assert!(out.contains("err"));
}

#[test]
fn fstring_format_spec() {
    let out = assert_ok(
        r#"
fn main(): Int {
    const pi: Float = 3.14159;
    print(f"pi≈{pi:.2f}");
    return 0;
}
"#,
    );
    assert!(out.contains("pi≈3.14"));
}

#[test]
fn runtime_error_includes_span() {
    let result = run_source(
        r#"
fn main(): Int {
    var x: Int = 5;
    print(x + "hello");
    return 0;
}
"#,
    );
    assert!(!result.ok);
    assert!(
        result.stderr.contains("runtime error at"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("cannot add"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn array_mutation_and_shared_references() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var a = [1, 2, 3];
    a.push(4);
    print(a.len());
    print(a[3]);
    a[0] = 42;
    print(a[0]);
    var b = a;
    b.push(5);
    print(a.len());
    print(b.pop());
    print(a.len());
    print(b[b.len() - 1]);
    return 0;
}
"#,
    );
    assert!(out.contains("4"));
    assert!(out.contains("4")); // a[3]
    assert!(out.contains("42"));
    assert!(out.contains("5")); // a.len after b.push
    assert!(out.contains("5")); // b.pop()
    assert!(out.contains("4")); // a.len after b.pop
    assert!(out.contains("4")); // b[b.len() - 1]
}

#[test]
fn import_module_in_memory() {
    let mut modules = HashMap::new();
    modules.insert(
        "calc".to_string(),
        r#"
fn add(a: Int, b: Int): Int {
    return a + b;
}

fn double(n: Int): Int {
    return n * 2;
}
"#
        .to_string(),
    );
    let result = run_source_with_modules(
        r#"
import calc;
fn main(): Int {
    print(calc.add(2, 3));
    print(calc.double(4));
    return 0;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("5"));
    assert!(result.stdout.contains("8"));
}

#[test]
fn from_module_import_function() {
    let mut modules = HashMap::new();
    modules.insert(
        "strings".to_string(),
        r#"
fn repeat(s: String, n: Int): String {
    var out: String = "";
    var i: Int = 0;
    while i < n {
        out = out + s;
        i = i + 1;
    }
    return out;
}
"#
        .to_string(),
    );
    let result = run_source_with_modules(
        r#"
from strings import repeat;
fn main(): Int {
    print(repeat("a", 3));
    return 0;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("aaa"));
}

#[test]
fn import_named_mod_not_filename() {
    let mut modules = HashMap::new();
    modules.insert(
        "helpers.rg".to_string(),
        r#"
mod utils {
    pub fn add(a: Int, b: Int): Int {
        return a + b;
    }
}
"#
        .to_string(),
    );
    let result = run_source_with_modules(
        r#"
import utils;
fn main(): Int {
    print(utils.add(2, 3));
    return 0;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("5"));
}

#[test]
fn import_merges_mod_blocks_across_files() {
    let mut modules = HashMap::new();
    modules.insert(
        "helpers.rg".into(),
        r#"
mod utils {
    pub fn add(a: Int, b: Int): Int {
        return a + b;
    }
}
"#
        .into(),
    );
    modules.insert(
        "extra.rg".into(),
        r#"
mod utils {
    pub fn mul(a: Int, b: Int): Int {
        return a * b;
    }
}
"#
        .into(),
    );
    let result = run_source_with_modules(
        r#"
import utils;
fn main(): Int {
    print(utils.add(2, 3));
    print(utils.mul(2, 3));
    return 0;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("5"));
    assert!(result.stdout.contains("6"));
}

#[test]
fn duplicate_mod_export_is_an_error() {
    let mut modules = HashMap::new();
    modules.insert(
        "a.rg".into(),
        "mod utils { pub fn add(a: Int, b: Int): Int { return a + b; } }\n".into(),
    );
    modules.insert(
        "b.rg".into(),
        "mod utils { pub fn add(a: Int, b: Int): Int { return a + b; } }\n".into(),
    );
    let result = run_source_with_modules(
        "import utils;\nfn main(): Int { return utils.add(1, 2); }\n",
        modules,
    );
    assert!(
        !result.ok,
        "expected duplicate export, got {}",
        result.stdout
    );
    assert!(
        result.stderr.contains("duplicate export 'add'"),
        "{}",
        result.stderr
    );
}

#[test]
fn check_merged_mod_arity() {
    let mut modules = HashMap::new();
    modules.insert(
        "helpers.rg".into(),
        "mod utils { pub fn add(a: Int, b: Int): Int { return a + b; } }\n".into(),
    );
    modules.insert(
        "extra.rg".into(),
        "mod utils { pub fn mul(a: Int, b: Int): Int { return a * b; } }\n".into(),
    );
    let diags = check_source_with_modules(
        "import utils;\nfn main(): Int { print(utils.mul(1)); return 0; }\n",
        "t.rg",
        modules,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 2 args"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn import_module_from_file() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_module_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("greet.rg"),
        r#"
fn greet(name: String): String {
    return f"hello {name}";
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("main.rg"),
        r#"
import greet;
fn main(): Int {
    print(greet.greet("world"));
    return 0;
}
"#,
    )
    .unwrap();
    let result = run_file(&dir.join("main.rg"));
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("hello world"));
}

#[test]
fn struct_decl_and_field_access() {
    let out = assert_ok(
        r#"
struct Point {
    x: Float,
    y: Float,
}

fn main(): Int {
    var p = Point { x: 1.5, y: 2.5 };
    print(p.x);
    print(p.y);
    return 0;
}
"#,
    );
    assert!(out.contains("1.5"));
    assert!(out.contains("2.5"));
}

#[test]
fn struct_literal_missing_field_defaults_to_none() {
    let out = assert_ok(
        r#"
struct Point {
    x: Float,
    y: Float,
}

fn main(): Int {
    var p = Point { x: 3.0 };
    print(p.x);
    print(p.y);
    return 0;
}
"#,
    );
    assert!(out.contains("3"));
    assert!(out.contains("none"));
}

#[test]
fn struct_literal_unknown_field_is_error() {
    let src = r#"
struct Point {
    x: Float,
    y: Float,
}

fn main(): Int {
    var p = Point { x: 1.0, yy: 2.0 };
    return 0;
}
"#;
    let diags = check_source(src, "t.rg");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown field") && d.message.contains("yy")),
        "{diags:?}"
    );
    let result = run_source(src);
    assert!(!result.ok, "runtime should reject unknown fields");
    assert!(
        result.stderr.contains("unknown field") || result.message.contains("unknown field"),
        "{}",
        result.stderr
    );
}

#[test]
fn and_or_short_circuit() {
    let out = assert_ok(
        r#"
fn main(): Int {
    if false && (1 // 0 == 1) {
        print("and-bad");
    }
    if true || (1 // 0 == 1) {
        print("or-ok");
    }
    print("done");
    return 0;
}
"#,
    );
    assert!(out.contains("or-ok"));
    assert!(out.contains("done"));
    assert!(!out.contains("and-bad"));
}

#[test]
fn integer_division_by_zero_is_runtime_error() {
    let result = run_source("fn main(): Int { print(1 // 0); return 0; }\n");
    assert!(!result.ok);
    assert!(
        result.stderr.contains("division by zero") || result.message.contains("division by zero"),
        "{}",
        result.stderr
    );
}

#[test]
fn enum_truthiness() {
    let out = assert_ok(
        r#"
enum Color {
    Red,
    Green,
}

fn main(): Int {
    if Color.Red {
        print("red");
    }
    if Option.None {
        print("none");
    } else {
        print("not-none");
    }
    if Option.Some(1) {
        print("some");
    }
    if Result.Err("x") {
        print("err");
    } else {
        print("not-err");
    }
    if Result.Ok(1) {
        print("ok");
    }
    return 0;
}
"#,
    );
    assert!(out.contains("red"));
    assert!(out.contains("not-none"));
    assert!(out.contains("some"));
    assert!(out.contains("not-err"));
    assert!(out.contains("ok"));
    assert!(!out.contains("\nnone\n") && !out.starts_with("none"));
}

#[test]
fn string_len_is_chars() {
    let out = assert_ok(
        r#"
fn main(): Int {
    print("é".len);
    print(len("é"));
    return 0;
}
"#,
    );
    assert_eq!(out, "1\n1\n");
}

#[test]
fn module_sibling_fn_calls() {
    let mut modules = HashMap::new();
    modules.insert(
        "utils.rg".into(),
        r#"
fn helper(): Int {
    return 7;
}
fn add_one(): Int {
    return helper() + 1;
}
"#
        .into(),
    );
    let result = run_source_with_modules(
        r#"
import utils;
fn main(): Int {
    print(utils.add_one());
    return 0;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("8"));
}

#[test]
fn enum_decl_and_match() {
    let out = assert_ok(
        r#"
enum Color {
    Red,
    Green,
    Blue,
}

fn main(): Int {
    var c = Color.Red;
    match c {
        Red { print("red"); }
        Green { print("green"); }
        Blue { print("blue"); }
    }
    return 0;
}
"#,
    );
    assert!(out.contains("red"));
}

#[test]
fn enum_variant_with_args() {
    let out = assert_ok(
        r#"
enum Shape {
    Circle(Float),
    Rect(Float, Float),
}

fn main(): Int {
    var s = Shape.Circle(5.0);
    match s {
        Circle(r) { print(r); }
        Rect(dims) { print(dims[0]); print(dims[1]); }
    }
    var t = Shape.Rect(2.0, 3.0);
    match t {
        Circle(r) { print(r); }
        Rect(dims) { print(dims[0]); print(dims[1]); }
    }
    return 0;
}
"#,
    );
    assert!(out.contains("5"));
    assert!(out.contains("2"));
    assert!(out.contains("3"));
}

#[test]
fn enum_variant_positional_and_named_binds() {
    let out = assert_ok(
        r#"
enum Shape {
    Circle(radius: Float),
    Rect(width: Float, height: Float),
}

fn main(): Int {
    var t = Shape.Rect(2.0, 3.0);
    match t {
        Circle(r) { print(r); }
        Rect(w, h) { print(w); print(h); }
    }
    match t {
        Rect(width: w, height: h) { print(w); print(h); }
        _ { }
    }
    return 0;
}
"#,
    );
    assert!(out.contains("2"));
    assert!(out.contains("3"));
}

#[test]
fn arrow_return_type_runs() {
    let out = assert_ok(
        r#"
fn answer() -> Int {
    return 7;
}
fn main(): Int {
    print(answer());
    return 0;
}
"#,
    );
    assert!(out.contains("7"));
}

#[test]
fn typecheck_undefined_ident() {
    let diags = check_source(
        r#"
fn main(): Int {
    print(nope);
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("undefined 'nope'")),
        "{diags:?}"
    );
}

#[test]
fn mod_private_fn_is_not_exported() {
    let mut modules = HashMap::new();
    modules.insert(
        "helpers.rg".into(),
        r#"
mod utils {
    fn secret(): Int { return 1; }
    pub fn add(a: Int, b: Int): Int { return a + b; }
}
"#
        .into(),
    );
    let result = run_source_with_modules(
        r#"
import utils;
fn main(): Int {
    print(utils.add(2, 3));
    return 0;
}
"#,
        modules.clone(),
    );
    assert!(result.ok, "{}", result.stderr);
    let hidden = run_source_with_modules(
        r#"
import utils;
fn main(): Int {
    print(utils.secret());
    return 0;
}
"#,
        modules,
    );
    assert!(!hidden.ok, "private fn should not be callable");
    assert!(
        hidden.stderr.contains("no member")
            || hidden.stderr.contains("no export")
            || hidden.stderr.contains("secret"),
        "{}",
        hidden.stderr
    );
}

#[test]
fn format_source_roundtrip() {
    let src = "fn main(): Int { print(1 + 2); return 0; }\n";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("fn main(): Int"));
    assert!(run_source(&formatted).ok);
}

#[test]
fn format_source_keeps_hash_comments() {
    let src = "# keep me\nfn main(): Int {\n    # inside\n    return 0;\n}\n";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("# keep me"), "{formatted}");
    assert!(formatted.contains("# inside"), "{formatted}");
    assert!(
        run_source(&formatted).ok,
        "{}",
        run_source(&formatted).stderr
    );
}

#[test]
fn format_source_keeps_body_hash_comments() {
    let src = r#"
struct Point {
    # x coord
    x: Float,
    y: Float,
}

trait Named {
    # identity
    fn label(self): String;
}

class Player {
    # health
    var hp: Float = 10.0;
    # boom
    fn explode() {
        pass;
    }
}
"#;
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("# x coord"), "{formatted}");
    assert!(formatted.contains("# identity"), "{formatted}");
    assert!(formatted.contains("# health"), "{formatted}");
    assert!(formatted.contains("# boom"), "{formatted}");
}

#[test]
fn impl_method_call_on_struct() {
    let out = assert_ok(
        r#"
struct Point {
    x: Float,
    y: Float,
}

impl Point {
    fn length(self): Float {
        return self.x;
    }

    fn scaled(self, factor: Float): Float {
        return self.x * factor;
    }
}

fn main(): Int {
    var p = Point { x: 3.0, y: 4.0 };
    print(p.length());
    print(p.scaled(2.0));
    return 0;
}
"#,
    );
    assert!(out.contains("3"));
    assert!(out.contains("6"));
}

/// Port of a typical RoseGold-PY class example (`class Vec2` + `length`).
#[test]
fn class_methods_and_field_defaults() {
    let out = assert_ok(
        r#"
import math;

class Vec2 {
    var x: Float = 0.0;
    var y: Float = 0.0;

    fn length(self): Float {
        return math.sqrt(self.x * self.x + self.y * self.y);
    }
}

fn main(): Int {
    var v = Vec2 { x: 3.0, y: 4.0 };
    print(v.length());
    var z = Vec2 {};
    print(z.x);
    print(z.y);
    return 0;
}
"#,
    );
    assert!(out.contains("5"), "{out}");
    assert!(out.contains("0"), "{out}");
}

#[test]
fn class_extends_inherits_fields_and_methods() {
    let out = assert_ok(
        r#"
class Enemy {
    var hp: Float = 10.0;
    fn hurt(self, dmg: Float): Float {
        self.hp = self.hp - dmg;
        return self.hp;
    }
}
class Slime extends Enemy {
    var goo: Float = 1.0;
}
fn main(): Int {
    var s = Slime {};
    print(s.hp);
    print(s.goo);
    print(s.hurt(3.0));
    return 0;
}
"#,
    );
    assert!(out.contains("10"), "{out}");
    assert!(out.contains("1"), "{out}");
    assert!(out.contains("7"), "{out}");
}

#[test]
fn class_implicit_fields_and_self_param() {
    let out = assert_ok(
        r#"
var hp: Float = 1.0;
class Enemy {
    var hp: Float = 10.0;
    fn hurt(self, dmg: Float): Float {
        hp = hp - dmg;
        return hp;
    }
}
fn main(): Int {
    var e = Enemy {};
    print(e.hurt(3.0));
    print(hp);
    return 0;
}
"#,
    );
    assert!(out.contains("7"), "{out}");
    assert!(out.contains("1"), "{out}");
}

#[test]
fn class_method_omits_self_param() {
    let out = assert_ok(
        r#"
trait Damageable {
    fn take_damage(damage: Float): Float;
}
class Player impl Damageable {
    var current_health: Float = 100.0;
    fn take_damage(damage: Float): Float {
        current_health = current_health - damage;
        return current_health;
    }
}
fn main(): Int {
    var p = Player {};
    print(p.take_damage(25.0));
    return 0;
}
"#,
    );
    assert!(out.contains("75"), "{out}");
}

#[test]
fn class_local_shadows_field() {
    let out = assert_ok(
        r#"
class Enemy {
    var hp: Float = 10.0;
    fn peek(self): Float {
        var hp = 0.0;
        return hp;
    }
}
fn main(): Int {
    var e = Enemy {};
    print(e.peek());
    print(e.hp);
    return 0;
}
"#,
    );
    assert!(out.contains("0"), "{out}");
    assert!(out.contains("10"), "{out}");
}

#[test]
fn class_implicit_method_call() {
    let out = assert_ok(
        r#"
class Player {
    var hp: Float = 100.0;
    fn on_ready(): Int {
        take_damage(10.0);
        print(hp);
        return 0;
    }
    fn take_damage(damage: Float): Float {
        hp = hp - damage;
        return hp;
    }
}
fn main(): Int {
    var p = Player {};
    return p.on_ready();
}
"#,
    );
    assert!(out.contains("90"), "{out}");
}

#[test]
fn class_implicit_method_call_typechecks() {
    let diags = check_source(
        r#"
trait Damageable {
    fn take_damage(damage: Float): Float;
}
class Player impl Damageable {
    var current_health: Float = 100.0;
    fn on_ready(): Int {
        take_damage(10.0);
        return 0;
    }
    fn take_damage(damage: Float): Float {
        current_health = current_health - damage;
        return current_health;
    }
}
"#,
        "t.rg",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn class_extends_super_method() {
    let out = assert_ok(
        r#"
class Enemy {
    var hp: Float = 10.0;
    fn hurt(self, dmg: Float): Float {
        self.hp = self.hp - dmg;
        return self.hp;
    }
}
class Slime extends Enemy {
    fn hurt(self, dmg: Float): Float {
        return super.hurt(dmg * 0.5);
    }
}
fn main(): Int {
    var s = Slime { hp: 10.0 };
    print(s.hurt(4.0));
    return 0;
}
"#,
    );
    assert!(out.contains("8"), "{out}");
}

#[test]
fn import_pub_class_extends_private_parent() {
    let mut modules = HashMap::new();
    modules.insert(
        "pets".into(),
        r#"
mod pets {
    class Animal {
        var hp: Float = 10.0;
        fn hurt(self, dmg: Float): Float {
            self.hp = self.hp - dmg;
            return self.hp;
        }
    }
    pub class Dog extends Animal {
        var name: String = "Rex";
        fn hurt(self, dmg: Float): Float {
            return super.hurt(dmg);
        }
    }
}
"#
        .into(),
    );
    let src = r#"
from pets import Dog;
fn main(): Int {
    var d = Dog {};
    print(d.hp);
    print(d.name);
    print(d.hurt(3.0));
    return 0;
}
"#;
    let diags = check_source_with_modules(src, "main.rg", modules.clone());
    assert!(diags.is_empty(), "{diags:?}");
    let result = run_source_with_modules(src, modules);
    assert!(result.ok, "{}", result.message);
    assert!(result.stdout.contains("10"), "{}", result.stdout);
    assert!(result.stdout.contains("Rex"), "{}", result.stdout);
    assert!(result.stdout.contains("7"), "{}", result.stdout);
}

#[test]
fn import_hides_private_parent_class() {
    let mut modules = HashMap::new();
    modules.insert(
        "pets".into(),
        r#"
mod pets {
    class Animal {
        var hp: Float = 10.0;
    }
    pub class Dog extends Animal {}
}
"#
        .into(),
    );
    let result = run_source_with_modules(
        r#"
from pets import Dog;
fn main(): Int {
    var a = Animal {};
    return 0;
}
"#,
        modules,
    );
    assert!(!result.ok, "private parent should not be constructable");
    assert!(
        result.message.contains("Animal") || result.stderr.contains("Animal"),
        "{} {}",
        result.message,
        result.stderr
    );
}

#[test]
fn typecheck_extends_unknown_parent() {
    let diags = check_source(
        r#"
class Slime extends Missing {
    var goo: Float = 1.0;
}
fn main(): Int { return 0; }
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown type 'Missing'")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_extends_cycle() {
    let diags = check_source(
        r#"
class A extends B {}
class B extends A {}
fn main(): Int { return 0; }
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("cycle")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_super_outside_method() {
    let diags = check_source(
        r#"
fn main(): Int {
    return super.hurt(1.0);
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("super is only valid")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_inherited_method_arity() {
    let diags = check_source(
        r#"
class Enemy {
    fn hurt(self, dmg: Float): Float { return dmg; }
}
class Slime extends Enemy {}
fn main(): Int {
    var s = Slime {};
    return s.hurt();
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("hurt") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
}

#[test]
fn trait_impl_for_type() {
    let out = assert_ok(
        r#"
class Vec2 {
    var x: Float = 3.0;
    var y: Float = 4.0;
}

import math;

trait HasLength {
    fn length(self): Float;
}

impl HasLength for Vec2 {
    fn length(self): Float {
        return self.x;
    }
}

fn main(): Int {
    var v = Vec2 {};
    print(v.length());
    return 0;
}
"#,
    );
    assert!(out.contains("3"), "{out}");
}

#[test]
fn class_nested_trait_impl() {
    let out = assert_ok(
        r#"
trait Named {
    fn label(self): String;
}

class Point {
    var x: Float = 3.0;
    impl Named {
        fn label(self): String {
            return "point";
        }
    }
}

fn main(): Int {
    var p = Point {};
    print(p.label());
    return 0;
}
"#,
    );
    assert!(out.contains("point"), "{out}");
}

#[test]
fn class_header_impl_traits() {
    let out = assert_ok(
        r#"
trait Named {
    fn label(self): String;
}
trait Drawable {
    fn draw(self): String;
}

class Point impl Named, Drawable {
    var x: Float = 3.0;
    fn label(self): String {
        return "point";
    }
    fn draw(self): String {
        return "dot";
    }
}

class Vec3 extends Point impl Named {
    var z: Float = 1.0;
    fn label(self): String {
        return "vec3";
    }
}

fn main(): Int {
    var p = Point {};
    print(p.label());
    print(p.draw());
    var q = Vec3 {};
    print(q.label());
    return 0;
}
"#,
    );
    assert!(out.contains("point"), "{out}");
    assert!(out.contains("dot"), "{out}");
    assert!(out.contains("vec3"), "{out}");
}

#[test]
fn math_module_requires_import() {
    let diags = check_source(
        "fn main(): Int { return math.clamp(1.0, 0.0, 1.0); }\n",
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("undefined") || d.message.contains("math")),
        "{diags:?}"
    );
}

#[test]
fn class_header_impl_missing_method_is_type_error() {
    let diags = check_source(
        r#"
trait Named {
    fn label(self): String;
}
class Point impl Named {
    var x: Float = 0.0;
}
fn main(): Int { return 0; }
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("missing 'label'")),
        "{diags:?}"
    );
}

#[test]
fn trait_impl_missing_method_is_type_error() {
    let diags = check_source(
        r#"
import math;

trait HasLength {
    fn length(self): Float;
}
struct Point {
    x: Float,
}
impl HasLength for Point {
    fn other(self): Float { return 0.0; }
}
fn main(): Int { return 0; }
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("missing 'length'")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_method_arity_at_call_span() {
    let diags = check_source(
        r#"
class Point {
    var x: Float = 0.0;
    fn length(self): Float {
        return self.x;
    }
}
fn main(): Int {
    var p = Point { x: 3.0 };
    return p.length(1);
}
"#,
        "t.rg",
    );
    let d = diags
        .iter()
        .find(|d| d.message.contains("length") && d.message.contains("expected 0 args"));
    assert!(d.is_some(), "{diags:?}");
    assert_eq!(d.unwrap().line, 10, "{d:?}");
}

#[test]
fn typecheck_method_on_untyped_is_error() {
    let diags = check_source(
        r#"
fn main(): Int {
    var foo;
    foo.bar();
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("cannot call 'bar'") && d.message.contains("unknown type")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_method_on_untyped_param_is_error() {
    let diags = check_source(
        r#"
fn take(foo: None) {
    foo.bar();
}
fn main(): Int {
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("cannot call 'bar'") && d.message.contains("unknown type")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_unknown_method_on_int() {
    let diags = check_source(
        r#"
fn main(): Int {
    var n = 1;
    n.nope();
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("Int has no method 'nope'")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_ufcs_on_untyped_is_ok() {
    let diags = check_source(
        r#"
@ufcs
fn clamp(n: Float, lo: Float, hi: Float): Float {
    return n;
}
fn take(n: None) {
    n.clamp(0.0, 1.0);
}
fn main(): Int {
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        !diags.iter().any(|d| d.message.contains("unknown type")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_unknown_method_on_class() {
    let diags = check_source(
        r#"
class Point {
    var x: Float = 0.0;
}
fn main(): Int {
    var p = Point { x: 1.0 };
    return p.nope();
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("has no method 'nope'")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_crate_vec2_method_arity() {
    let diags = check_source(
        r#"
fn main(): Int {
    return Vec2 { x: 3.0, y: 4.0 }.length(1);
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("Vec2.length") && d.message.contains("expected 0 args")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_method_arg_type() {
    let diags = check_source(
        r#"
fn main(): Int {
    var v = Vec2 { x: 1.0, y: 2.0 };
    v.add(1);
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(|d| d.message.contains("expected Vec2")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_array_method_arity() {
    let diags = check_source(
        r#"
fn main(): Int {
    var a = [1];
    a.push();
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("Array.push") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
}

#[test]
fn trait_impl_wrong_arity_is_type_error() {
    let diags = check_source(
        r#"
import math;

trait HasLength {
    fn length(self): Float;
}
struct Point {
    x: Float,
}
impl HasLength for Point {
    fn length(self, extra: Int): Float { return 0.0; }
}
fn main(): Int { return 0; }
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("HasLength.length")
                && d.message.contains("expected 1 params")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_option_method_arity() {
    let diags = check_source(
        r#"
fn main(): Int {
    return Option.Some(1).unwrap_or();
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unwrap_or") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
}

#[test]
fn stdlib_vec2_and_str_split() {
    let out = assert_ok(
        r#"
import str;

fn main(): Int {
    var v = Vec2 { x: 3.0, y: 4.0 };
    print(v.length());
    var parts = str.split("a,b,c", ",");
    print(parts.len());
    print(parts[1]);
    print(str.slice("rosegold", 4, 8));
    return 0;
}
"#,
    );
    assert!(out.contains("5"), "{out}");
    assert!(out.contains("3"), "{out}");
    assert!(out.contains("b"), "{out}");
    assert!(out.contains("gold"), "{out}");
}

#[test]
fn stdlib_vec3() {
    let out = assert_ok(
        r#"
fn main(): Int {
    var a = Vec3 { x: 3.0, y: 4.0, z: 12.0 };
    print(a.length());
    print(a.z);
    var b = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
    var c = a.add(b);
    print(c.x);
    print(c.z);
    return 0;
}
"#,
    );
    assert!(out.contains("13"), "{out}");
    assert!(out.contains("12"), "{out}");
    assert!(out.contains("4"), "{out}");
    assert!(out.contains("15"), "{out}");
}

#[test]
fn bitwise_ops_on_int() {
    let out = assert_ok(
        r#"
fn layer_bit(n: Int): Int {
    return 1 << n;
}

fn main(): Int {
    print(1 << 3);
    var mask = layer_bit(0) | layer_bit(2);
    print(mask);
    print(mask & layer_bit(2));
    print(mask ^ 255);
    var n: Int = 1;
    n |= 2;
    print(n);
    print(16 >> 3);
    print(~0);
    print(true && false);
    print(true || false);
    return 0;
}
"#,
    );
    assert!(out.contains("8\n"), "{out}");
    assert!(out.contains("5\n"), "{out}");
    assert!(out.contains("4\n"), "{out}");
    assert!(out.contains("250\n"), "{out}");
    assert!(out.contains("3\n"), "{out}");
    assert!(out.contains("2\n"), "{out}");
    assert!(out.contains("-1\n"), "{out}");
    assert!(out.contains("false\n"), "{out}");
    assert!(out.contains("true\n"), "{out}");
}

#[test]
fn bitwise_float_is_type_error() {
    let diags = check_source("fn main(): Int { return 1.0 << 3; }\n", "t.rg");
    assert!(
        diags.iter().any(|d| d.message.contains("bitwise")),
        "{diags:?}"
    );
    let diags = check_source("fn main(): Int { return ~1.0; }\n", "t.rg");
    assert!(
        diags.iter().any(|d| d.message.contains("bitwise")),
        "{diags:?}"
    );
}

#[test]
fn ufcs_member_call() {
    let out = assert_ok(
        r#"
import math;

@ufcs
fn clamp(n: Float, lo: Float, hi: Float): Float {
    return math.clamp(n, lo, hi);
}

fn main(): Int {
    print(400.0.clamp(0.0, 200.0));
    print(clamp(50.0, 0.0, 200.0));
    return 0;
}
"#,
    );
    assert!(out.contains("200"), "{out}");
    assert!(out.contains("50"), "{out}");
}

#[test]
fn ufcs_does_not_steal_inherent_method() {
    let out = assert_ok(
        r#"
struct Point {
    x: Float,
    y: Float,
}

impl Point {
    fn length(self): Float {
        return 9.0;
    }
}

@ufcs
fn length(n: Float): Float {
    return n;
}

fn main(): Int {
    var p = Point { x: 3.0, y: 4.0 };
    print(p.length());
    print(3.0.length());
    return 0;
}
"#,
    );
    assert!(out.contains("9"), "{out}");
    assert!(out.contains("3"), "{out}");
}

#[test]
fn ufcs_arity_at_call_span() {
    let diags = check_source(
        r#"
@ufcs
fn clamp(n: Float, lo: Float, hi: Float): Float {
    return n;
}
fn main(): Int {
    return 1.0.clamp(0.0);
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("clamp expected 2 args")),
        "{diags:?}"
    );
}

#[test]
fn ufcs_does_not_steal_array_push() {
    let out = assert_ok(
        r#"
@ufcs
fn push(n: Int, x: Int): Int {
    return n + x;
}

fn main(): Int {
    var a = [1];
    a.push(2);
    print(a.len());
    print(3.push(4));
    return 0;
}
"#,
    );
    assert!(out.contains("2"), "{out}");
    assert!(out.contains("7"), "{out}");
}

#[test]
fn typecheck_catches_wrong_arity() {
    let src = r#"
fn add(a: Int, b: Int): Int {
    return a + b;
}

fn main(): Int {
    return add(1);
}
"#;
    let result = run_source(src);
    assert!(!result.ok);
    assert!(
        result.stderr.contains("expected 2 args") || result.stderr.contains("type error"),
        "stderr: {}",
        result.stderr
    );
    let diags = check_source(src, "t.rg");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("expected 2 args"));
    assert_eq!(diags[0].line, 7, "arity error should be at the call site");
    let rendered = diags[0].to_string();
    assert!(
        rendered.starts_with("t.rg:7:") && rendered.contains("error:"),
        "{rendered}"
    );
}

#[test]
fn typecheck_catches_stdlib_arity() {
    let src = r#"
import math;
fn main(): Int {
    return math.clamp(1);
}
"#;
    let diags = check_source(src, "t.rg");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 3 args"),
        "{:?}",
        diags[0]
    );
    assert_eq!(diags[0].line, 4, "arity error should be at the call site");
    assert!(diags[0].col >= 1);
    let rendered = diags[0].to_string();
    assert!(
        rendered.starts_with("t.rg:4:") && rendered.contains("error:"),
        "{rendered}"
    );
}

#[test]
fn typecheck_from_math_import_arity() {
    let src = r#"
from math import clamp;
fn main(): Int {
    return clamp(1);
}
"#;
    let diags = check_source(src, "t.rg");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 3 args"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn check_source_with_modules_catches_import_arity() {
    let mut modules = HashMap::new();
    modules.insert(
        "utils".into(),
        "mod utils {\npub fn move_line(dx: Float, dy: Float): String { return \"x\"; }\n}\n".into(),
    );
    let diags = check_source_with_modules(
        "import utils;\nfn main(): Int { print(utils.move_line(1.0)); return 0; }\n",
        "Hero.rg",
        modules,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 2 args"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn list_signals_and_emit() {
    let src = r#"
signal collected(amount: Int);
fn main(): Int {
    collected.emit(1);
    return 0;
}
"#;
    let fields = list_signals(src);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "collected");
    assert_eq!(fields[0].params.len(), 1);
    assert_eq!(fields[0].params[0].name, "amount");
    assert_eq!(fields[0].params[0].ty, "Int");

    let result = run_source(src);
    assert!(result.ok, "{}\nstderr: {}", result.message, result.stderr);
}

#[test]
fn signal_connect_runs_listeners_in_order() {
    let out = assert_ok(
        r#"
signal collected(amount: Int);
fn log_coin(amount: Int) {
    print(f"log {amount}");
}
fn total_coin(amount: Int) {
    print(f"total {amount}");
}
fn main(): Int {
    collected.connect(log_coin);
    collected.connect(total_coin);
    collected.connect(log_coin);
    collected.emit(5);
    return 0;
}
"#,
    );
    assert_eq!(out, "log 5\ntotal 5\n");
}

#[test]
fn signal_listener_error_stops_later_listeners() {
    let result = run_source(
        r#"
signal ping();
fn boom() {
    assert(false);
}
fn later() {
    print("later");
}
fn main(): Int {
    ping.connect(boom);
    ping.connect(later);
    ping.emit();
    return 0;
}
"#,
    );
    assert!(!result.ok);
    assert!(!result.stdout.contains("later"), "{}", result.stdout);
}

#[test]
fn typecheck_signal_connect_arity_and_unknown() {
    let diags = check_source(
        r#"
signal collected(amount: Int);
fn wrong() {}
fn ok(amount: Int) {}
fn main(): Int {
    collected.connect();
    collected.connect(wrong);
    nope.connect(ok);
    collected.connect(1);
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags.iter().any(
            |d| d.message.contains("collected.connect") && d.message.contains("expected 1 arg")
        ),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("wrong") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown signal") && d.message.contains("nope")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("expects a function name")),
        "{diags:?}"
    );
}

#[test]
fn trait_signal_lists_and_emits() {
    let src = r#"
trait Damageable {
    signal died();
    fn take_damage(damage: Float): Float;
}
class Player impl Damageable {
    var hp: Float = 10.0;
    fn take_damage(damage: Float): Float {
        hp = hp - damage;
        if hp <= 0.0 {
            died.emit();
        }
        return hp;
    }
}
fn main(): Int {
    var p = Player {};
    print(p.take_damage(10.0));
    return 0;
}
"#;
    let fields = list_signals(src);
    assert_eq!(fields.len(), 1, "{fields:?}");
    assert_eq!(fields[0].name, "died");
    assert!(fields[0].params.is_empty());

    let diags = check_source(src, "t.rg");
    assert!(diags.is_empty(), "{diags:?}");

    let result = run_source(src);
    assert!(result.ok, "{}\nstderr: {}", result.message, result.stderr);
}

#[test]
fn trait_signal_unknown_without_impl() {
    let src = r#"
trait Damageable {
    signal died();
}
class Player {
    fn on_ready() {
        died.emit();
    }
}
"#;
    assert!(list_signals(src).is_empty());
    let diags = check_source(src, "t.rg");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("unknown signal"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn trait_signal_emit_arity() {
    let diags = check_source(
        r#"
trait Damageable {
    signal died();
    fn take_damage(damage: Float): Float;
}
class Player impl Damageable {
    fn take_damage(damage: Float): Float {
        died.emit(1);
        return 0.0;
    }
}
"#,
        "t.rg",
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 0 args"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn trait_signal_imported() {
    let mut modules = HashMap::new();
    modules.insert(
        "damage.rg".into(),
        r#"
trait Damageable {
    signal died();
    fn take_damage(damage: Float): Float;
}
"#
        .into(),
    );
    let src = r#"
import damage;
class Player impl Damageable {
    fn take_damage(damage: Float): Float {
        died.emit();
        return 0.0;
    }
}
fn main(): Int {
    var p = Player {};
    p.take_damage(1.0);
    return 0;
}
"#;
    let diags = check_source_with_modules(src, "player.rg", modules.clone());
    assert!(diags.is_empty(), "{diags:?}");
    let fields = list_signals_with_modules(src, modules.clone());
    assert!(
        fields.iter().any(|f| f.name == "died"),
        "imported trait signals should list: {fields:?}"
    );
    assert!(
        list_signals(src).is_empty(),
        "without the module, imported trait signals stay hidden"
    );

    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse().unwrap();
    let resolver = Rc::new(RefCell::new(HashMapResolver::new(modules)));
    let mut ctx = EvalContext::with_resolver(resolver);
    ctx.load_program(&program).unwrap();
    ctx.call("main", vec![]).unwrap();
}

#[test]
fn typecheck_unknown_signal_emit() {
    let diags = check_source("fn main(): Int { collected.emit(1); return 0; }\n", "t.rg");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("unknown signal"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn typecheck_signal_emit_arity() {
    let diags = check_source(
        "signal collected(amount: Int);\nfn main(): Int { collected.emit(); return 0; }\n",
        "t.rg",
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected 1 args"),
        "{:?}",
        diags[0]
    );
}

#[test]
fn unknown_signal_emit_is_runtime_error() {
    let src = "fn main(): Int { var x: Int = 1; x.emit(1); return 0; }\n";
    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse().unwrap();
    let mut ctx = EvalContext::new();
    ctx.load_program(&program).unwrap();
    let err = ctx.call("main", vec![]).unwrap_err();
    assert!(err.message.contains("unknown signal"), "{}", err.message);
}

#[test]
fn typecheck_io_arity_and_unknown() {
    let diags = check_source(
        r#"
import io;
fn main(): Int {
    io.append_text("a.txt");
    io.list_dir();
    io.chmod("a.txt");
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("append_text") && d.message.contains("expected 2 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("list_dir") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown function") && d.message.contains("io.chmod")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_time_arity() {
    let diags = check_source(
        r#"
import time;
fn main(): Int {
    time.now(1);
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("time.now") && d.message.contains("expected 0 args")),
        "{diags:?}"
    );
}

#[test]
fn diagnostic_from_parser_message() {
    let d = diagnostic_from_message(
        "x.rg",
        "expected ';' after variable declaration at 4:10 (got Ident)",
    );
    assert_eq!(d.line, 4);
    assert_eq!(d.col, 10);
    assert_eq!(
        d.to_string(),
        "x.rg:4:10: error: expected ';' after variable declaration (got Ident)"
    );
}

#[test]
fn io_read_write_roundtrip() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_io_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.txt");
    let missing = dir.join("gone.txt");
    let path_str = path.to_string_lossy().replace('\\', "\\\\");
    let missing_str = missing.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        r#"
import io;

fn main(): Int {{
    var w = io.write_text("{path}", "hello");
    print(w.is_ok());
    var a = io.append_text("{path}", " rosegold");
    print(a.is_ok());
    var text = io.read_text("{path}").unwrap();
    print(text);
    var lines: Array = io.read_lines("{path}").unwrap();
    print(lines.len());
    print(lines[0]);
    print(io.exists("{path}"));
    var r = io.remove("{path}");
    print(r.is_ok());
    print(io.exists("{path}"));
    print(io.read_text("{path}").is_err());
    print(io.remove("{missing}").is_err());
    return 0;
}}
"#,
        path = path_str,
        missing = missing_str
    );
    let out = assert_ok(&source);
    assert!(out.contains("true"));
    assert!(out.contains("hello rosegold"));
    assert!(!path.exists(), "io.remove should delete the file");
}

#[test]
fn io_dirs_roundtrip() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_io_dir_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let nested = dir.join("saves");
    let nested_str = nested.to_string_lossy().replace('\\', "\\\\");
    let missing = dir.join("nope");
    let missing_str = missing.to_string_lossy().replace('\\', "\\\\");
    let file_str = nested
        .join("slot.txt")
        .to_string_lossy()
        .replace('\\', "\\\\");
    let source = format!(
        r#"
import io;

fn main(): Int {{
    print(io.mkdir("{nested}").is_ok());
    print(io.is_dir("{nested}"));
    print(io.exists("{nested}"));
    print(io.write_text("{file}", "ok").is_ok());
    var names: Array = io.list_dir("{nested}").unwrap();
    print(names.len());
    print(names[0]);
    print(io.is_dir("{file}"));
    print(io.list_dir("{missing}").is_err());
    return 0;
}}
"#,
        nested = nested_str,
        file = file_str,
        missing = missing_str
    );
    let out = assert_ok(&source);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "true", "true", "true", "true", "1", "slot.txt", "false", "true"
        ],
        "{out}"
    );
}

#[test]
fn time_now_is_unix_not_delta() {
    let out = assert_ok(
        r#"
import time;

fn main(): Int {
    var n = time.now();
    print(n > 1000000000.0);
    var e = time.elapsed();
    print(e >= 0.0);
    print(e < 60.0);
    return 0;
}
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, ["true", "true", "true"], "{out}");
}

#[test]
fn array_first_last_and_str_helpers() {
    let out = assert_ok(
        r#"
import str;

fn main(): Int {
    var a = [10, 20, 30];
    print(a.first());
    print(a.last());
    print(a.contains(20));
    print(a.contains(99));
    print(Array.first(a));
    print(str.upper("Hi"));
    print(str.lower("Hi"));
    print(str.trim("  yo  "));
    return 0;
}
"#,
    );
    assert!(out.contains("10"));
    assert!(out.contains("30"));
    assert!(out.contains("true"));
    assert!(out.contains("false"));
    assert!(out.contains("HI"));
    assert!(out.contains("hi"));
    assert!(out.contains("yo"));
}

/// Port of RoseGold-PY `examples/class/main.rg` (`class` + `trait` + crate `Vec2`).
#[test]
fn example_class() {
    let path = example("class_trait.rg");
    let result = run_file(&path);
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("5"), "{}", result.stdout);
    assert!(result.stdout.contains("point"), "{}", result.stdout);
    assert!(result.stdout.contains("10"), "{}", result.stdout);
    assert!(result.stdout.contains("13"), "{}", result.stdout);
    assert!(result.stdout.contains("vec3"), "{}", result.stdout);
}

/// Port of RoseGold-PY `examples/hello.rg`
#[test]
fn example_hello() {
    let out = assert_ok(
        r#"
fn main(): Int {
    print("hello from RoseGold");
    return 0;
}
"#,
    );
    assert_eq!(out, "hello from RoseGold\n");
}

/// Port of RoseGold-PY `examples/map_result/main.rg`
#[test]
fn example_map_result() {
    let out = assert_ok(
        r#"
from result import Result;
from option import Option;

fn lookup(m: Map<String, Int>, key: String): Result<Int, String> {
    if m.has(key) {
        return Result.Ok(m[key]);
    }
    return Result.Err("missing key");
}

fn main(): Int {
    var scores: Map<String, Int> = {"alice": 10, "bob": 7};
    scores["cara"] = 12;

    var total: Int = 0;
    for name in scores {
        total += scores[name];
    }
    assert(total == 29);
    assert(scores.len() == 3);

    match lookup(scores, "bob") {
        Ok(v) { assert(v == 7); }
        Err(_) { assert(false); }
    }

    const missing = lookup(scores, "zed");
    assert(missing.is_err());

    const maybe: Option<Int> = Option.Some(3);
    assert(maybe.unwrap_or(0) == 3);
    assert(Option.None.unwrap_or(9) == 9);

    print(scores);
    return scores.remove("alice");
}
"#,
    );
    assert!(out.contains("alice") || out.contains("10"));
}

/// Port of RoseGold-PY `examples/tests/main.rg` (stdlib checks without @test runner)
#[test]
fn example_stdlib_tests() {
    let out = assert_ok(
        r#"
import math;
import str;
import checks;

fn main(): Int {
    checks.eq(math.pow(2, 10), 1024);
    checks.eq(math.gcd(12, 18), 6);
    checks.eq(math.sign(-3), -1);
    checks.eq_bool(str.starts_with("rosegold", "rose"), true);
    checks.eq_bool(str.ends_with("rosegold", "gold"), true);
    checks.eq_bool(str.contains("hello", "ell"), true);
    checks.eq_string(str.repeat("ab", 3), "ababab");
    const n = 7;
    checks.eq_string(f"n={n}", "n=7");
    print("ok");
    return 0;
}
"#,
    );
    assert_eq!(out, "ok\n");
}

/// Port of RoseGold-PY `examples/tour/main.rg` core (maps, Result, f-string, io.exists)
#[test]
fn example_tour_core() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_tour_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("present.txt");
    fs::write(&marker, "x").unwrap();
    let present = marker.to_string_lossy().replace('\\', "\\\\");
    let missing = dir
        .join("nope_missing.rg")
        .to_string_lossy()
        .replace('\\', "\\\\");
    let source = format!(
        r#"
from result import Result;
from option import Option;
import io;
import checks;

fn score_of(scores: Map<String, Int>, name: String): Result<Int, String> {{
    if scores.has(name) {{
        return Result.Ok(scores[name]);
    }}
    return Result.Err(f"unknown player: {{name}}");
}}

fn main(): Int {{
    var scores: Map<String, Int> = {{"ada": 10, "grace": 12}};
    scores["linus"] = 9;
    checks.eq(scores.len(), 3);
    match score_of(scores, "grace") {{
        Ok(v) {{ checks.eq(v, 12); }}
        Err(_) {{ checks.eq_bool(false, true); }}
    }}
    checks.eq_bool(score_of(scores, "zed").is_err(), true);

    const maybe: Option<Int> = Option.Some(42);
    checks.eq(maybe.unwrap_or(0), 42);
    checks.eq(Option.None.unwrap_or(7), 7);

    const pi: Float = 3.14159;
    checks.eq_string(f"pi≈{{pi:.2f}}", "pi≈3.14");

    checks.eq_bool(io.exists("{present}"), true);
    checks.eq_bool(io.exists("{missing}"), false);

    const grace = scores["grace"];
    print(f"players={{scores.len()}} grace={{grace}}");
    return 0;
}}
"#,
        present = present,
        missing = missing
    );
    let out = assert_ok(&source);
    assert!(out.contains("players=3 grace=12"));
}

/// Port of RoseGold-PY `examples/multi` (dotted import + named enum fields)
#[test]
fn example_multi_module() {
    let mut modules = HashMap::new();
    modules.insert(
        "util.math".to_string(),
        r#"
pub fn add(a: Int, b: Int): Int {
    return a + b;
}

pub enum Shape {
    Circle(radius: Float),
    Rectangle(width: Float, height: Float),
}

pub fn area(shape: Shape): Float {
    match shape {
        Shape.Circle(radius) {
            return 3.14 * radius * radius;
        }
        Shape.Rectangle(width, height) {
            return width * height;
        }
        _ {
            print("Unknown shape");
        }
    }
}
"#
        .to_string(),
    );
    let result = run_source_with_modules(
        r#"
import util.math;

fn main(): Float {
    const v = math.area(math.Shape.Circle(10.0));
    print(v);
    return v;
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("314"));
}

#[test]
fn example_multi_from_files() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_multi_file_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("util").join("math")).unwrap();
    fs::write(
        dir.join("util").join("math").join("lib.rg"),
        r#"
pub enum Shape {
    Circle(radius: Float),
    Rectangle(width: Float, height: Float),
}

pub fn area(shape: Shape): Float {
    match shape {
        Shape.Circle(radius) {
            return 3.14 * radius * radius;
        }
        Shape.Rectangle(width, height) {
            return width * height;
        }
        _ {
            print("Unknown shape");
        }
    }
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("main.rg"),
        r#"
import util.math;

fn main(): Float {
    const v = math.area(math.Shape.Circle(10.0));
    print(v);
    return v;
}
"#,
    )
    .unwrap();
    let result = run_file(&dir.join("main.rg"));
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("314"));
}

#[test]
fn attribute_test_is_skipped() {
    let out = assert_ok(
        r#"
@test
fn helper(): Int {
    return 1;
}

fn main(): Int {
    print(helper());
    return 0;
}
"#,
    );
    assert_eq!(out, "1\n");
}

#[test]
fn run_tests_executes_test_fns() {
    let result = run_tests(
        r#"
import checks;

@test
fn test_add() {
    checks.eq(2 + 2, 4);
}

@test
fn test_fail_example() {
    checks.eq(1, 2);
}

fn main(): Int {
    return 0;
}
"#,
    );
    assert!(!result.ok);
    assert!(result.stdout.contains("ok test_add"));
    assert!(result.stdout.contains("FAIL test_fail_example"));
    assert!(result.stdout.contains("1/2 tests passed"));
}

#[test]
fn run_tests_all_pass() {
    let result = run_tests(
        r#"
import checks;

@test
fn test_ok() {
    checks.eq(1, 1);
}
"#,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("ok test_ok"));
    assert!(result.stdout.contains("1/1 tests passed"));
}

#[test]
fn run_tests_resolves_imported_modules() {
    let mut modules = HashMap::new();
    modules.insert(
        "utils.rg".into(),
        r#"
fn add(a: Int, b: Int): Int {
    return a + b;
}
"#
        .into(),
    );
    let result = run_tests_with_modules(
        r#"
import checks;
import utils;

@test
fn test_add() {
    checks.eq(utils.add(2, 3), 5);
}
"#,
        modules,
    );
    assert!(result.ok, "{}", result.stderr);
    assert!(result.stdout.contains("ok test_add"));
}

#[test]
fn list_fns_includes_class_methods() {
    let fields = list_fns(
        r#"
class Player {
    fn explode(power: Float) {
        pass;
    }
}
fn after() {
    pass;
}
"#,
    );
    let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"explode"), "{names:?}");
    assert!(names.contains(&"after"), "{names:?}");
    let explode = fields.iter().find(|f| f.name == "explode").unwrap();
    assert_eq!(explode.params.len(), 1);
    assert_eq!(explode.params[0].name, "power");
}

#[test]
fn run_examples_ok() {
    for name in [
        "hello.rg",
        "class_trait.rg",
        "io_time.rg",
        "process.rg",
        "tour.rg",
        "json.rg",
        "signals.rg",
    ] {
        let result = run_file(&example(name));
        assert!(result.ok, "{name}: {}", result.stderr);
        assert_eq!(result.exit_code, 0, "{name}");
    }
}

#[test]
fn process_argv_and_env() {
    let result = run_source_with_argv(
        r#"
import process;
fn main(): Int {
    var args = process.argv();
    print(args.len());
    print(args[0]);
    print(args[1]);
    return 0;
}
"#,
        vec!["script.rg".into(), "hello".into()],
    );
    assert!(result.ok, "{}", result.stderr);
    let lines: Vec<&str> = result.stdout.lines().collect();
    assert_eq!(lines[0], "2", "{}", result.stdout);
    assert_eq!(lines[1], "script.rg");
    assert_eq!(lines[2], "hello");
}

#[test]
fn process_env_missing_is_none() {
    let out = assert_ok(
        r#"
import process;
fn main(): Int {
    match process.env("ROSEGOLD_NO_SUCH_ENV_VAR_XYZ") {
        Some(_) { print("hit"); }
        None { print("none"); }
    }
    return 0;
}
"#,
    );
    assert_eq!(out.trim(), "none");
}

#[test]
fn process_env_path_is_some() {
    let out = assert_ok(
        r#"
import process;
fn main(): Int {
    match process.env("PATH") {
        Some(_) { print("path"); }
        None { print("no-path"); }
    }
    return 0;
}
"#,
    );
    assert_eq!(out.trim(), "path");
}

#[test]
fn process_exit_stops_and_sets_code() {
    let result = run_source(
        r#"
import process;
fn main(): Int {
    print("bye");
    process.exit(7);
    print("nope");
    return 0;
}
"#,
    );
    assert!(!result.ok);
    assert_eq!(result.exit_code, 7);
    assert_eq!(result.stdout, "bye\n");
    assert!(result.stderr.is_empty(), "{}", result.stderr);
}

#[test]
fn process_requires_import() {
    let diags = check_source("fn main(): Int { process.exit(0); return 0; }\n", "t.rg");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("process") || d.message.contains("undefined")),
        "{diags:?}"
    );
}

#[test]
fn typecheck_process_arity() {
    let diags = check_source(
        r#"
import process;
fn main(): Int {
    process.argv(1);
    process.env();
    process.exit();
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("process.argv") && d.message.contains("expected 0 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("process.env") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("process.exit") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
}

#[test]
fn repl_import_and_call() {
    let mut session = crate::repl::Session::new();
    match crate::repl::eval_line(&mut session, "import math;") {
        crate::repl::LineResult::Silent => {}
        other => panic!("{other:?}"),
    }
    match crate::repl::eval_line(&mut session, "math.sqrt(4.0)") {
        crate::repl::LineResult::Value(Value::Float(n)) => {
            assert!((n - 2.0).abs() < 1e-9, "{n}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn json_parse_stringify_roundtrip() {
    let out = assert_ok(
        r#"
import json;
fn main(): Int {
    var t = json.stringify({"n": 1, "ok": true, "xs": [1, 2]}).unwrap();
    var d = json.parse(t).unwrap();
    print(d["n"]);
    print(d["ok"]);
    print(len(d["xs"]));
    print(json.parse("[1,2,3]").unwrap());
    print(json.parse("not json").is_err());
    print(json.stringify(none).unwrap());
    return 0;
}
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "1", "{out}");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], "2");
    assert_eq!(lines[3], "[1, 2, 3]");
    assert_eq!(lines[4], "true");
    assert_eq!(lines[5], "null");
}

#[test]
fn json_option_none_is_null() {
    let out = assert_ok(
        r#"
import json;
fn main(): Int {
    print(json.stringify(Option.None).unwrap());
    print(json.stringify(Option.Some(4)).unwrap());
    print(json.parse("null").unwrap());
    return 0;
}
"#,
    );
    assert_eq!(out, "null\n4\nnone\n");
}

#[test]
fn typecheck_json_arity() {
    let diags = check_source(
        r#"
import json;
fn main(): Int {
    json.parse();
    json.stringify(1, 2);
    json.pretty("{}");
    return 0;
}
"#,
        "t.rg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("json.parse") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("json.stringify") && d.message.contains("expected 1 args")),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown function") && d.message.contains("json.pretty")),
        "{diags:?}"
    );
}

#[test]
fn json_requires_import() {
    let diags = check_source("fn main(): Int { json.parse(\"{}\"); return 0; }\n", "t.rg");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("json") || d.message.contains("undefined")),
        "{diags:?}"
    );
}

#[test]
fn runtime_error_includes_call_trace() {
    let result = run_source(
        r#"
fn inner(): Int {
    return 1 + "x";
}
fn outer(): Int {
    return inner();
}
fn main(): Int {
    return outer();
}
"#,
    );
    assert!(!result.ok);
    assert!(
        result.stderr.contains("from inner"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("from outer"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn runtime_error_traces_method() {
    let result = run_source(
        r#"
class Boom {
    fn bang(): Int {
        return 1 + "x";
    }
}
fn main(): Int {
    var b = Boom {};
    return b.bang();
}
"#,
    );
    assert!(!result.ok);
    assert!(
        result.stderr.contains("from Boom.bang"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn runtime_error_trace_includes_module_file() {
    let mut modules = HashMap::new();
    modules.insert(
        "util.rg".into(),
        r#"
mod util {
    pub fn boom(): Int {
        return 1 + "x";
    }
}
"#
        .into(),
    );
    let result = run_source_with_modules(
        r#"
import util;
fn main(): Int {
    return util.boom();
}
"#,
        modules,
    );
    assert!(!result.ok);
    assert!(
        result.stderr.contains("from util.boom"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("util.rg"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn runtime_error_trace_from_imported_file() {
    use std::fs;
    let dir = std::env::temp_dir().join("rosegold_trace_mod");
    let _ = fs::create_dir_all(&dir);
    fs::write(
        dir.join("util.rg"),
        "mod util {\n    pub fn boom(): Int {\n        return 1 + \"x\";\n    }\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.rg"),
        "import util;\nfn main(): Int {\n    return util.boom();\n}\n",
    )
    .unwrap();
    let result = run_file(&dir.join("main.rg"));
    assert!(!result.ok, "{}", result.stderr);
    assert!(
        result.stderr.contains("from util.boom"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("util.rg"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("main.rg"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn missing_module_names_lookup_paths() {
    let diags = check_source_with_modules(
        "import no_such_mod;\nfn main(): Int { return 0; }\n",
        "t.rg",
        HashMap::new(),
    );
    assert!(
        diags.iter().any(|d| d.message.contains("no_such_mod")
            && d.message.contains("tried")
            && d.message.contains("no_such_mod.rg")),
        "{diags:?}"
    );
}

#[test]
fn examples_tests_rg_passes() {
    let result = run_tests_file(&example("tests.rg"));
    assert!(result.ok, "{}", result.stderr);
    assert!(
        result.stdout.contains("2/2 tests passed"),
        "{}",
        result.stdout
    );
}
