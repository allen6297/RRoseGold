//! Stack VM: compile a subset of RoseGold to a chunk, then interpret it.
//!
//! `rosegold run` uses this when the program compiles, and tree-walks otherwise.
//! JIT and a register machine wait until this instruction set is boring and
//! something is actually slow.

mod chunk;
mod compiler;
mod vm;

pub use chunk::{Chunk, FnProto, Op, Program};
pub use compiler::{CompileError, compile_program, compile_program_with};
pub use vm::{interpret, interpret_in};

use std::collections::HashMap;

use crate::Value;

/// Parse, typecheck, compile `main`, and run it on the stack VM.
pub fn eval_source(source: &str) -> Result<Value, String> {
    let program = crate::compile_source(source)?;
    let proto = compile_program(&program).map_err(|e| e.to_string())?;
    interpret(&proto, Vec::new()).map_err(|e| e.to_string())
}

/// Same as [`eval_source`], with in-memory user modules (`import calc`).
pub fn eval_source_with_modules(
    source: &str,
    modules: HashMap<String, String>,
) -> Result<Value, String> {
    let tokens = crate::lexer::Lexer::new(source)
        .tokenize()
        .map_err(|e| e.to_string())?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| e.to_string())?;
    let resolver = crate::interpreter::HashMapResolver::new(modules);
    if let Some(d) = crate::typecheck::typecheck_diagnostics_with(&program, Some(&resolver))
        .into_iter()
        .next()
    {
        let err = if d.file.is_empty() {
            format!("type error at {}:{}: {}", d.line, d.col, d.message)
        } else {
            format!(
                "type error at {}:{}:{}: {}",
                d.file, d.line, d.col, d.message
            )
        };
        return Err(err);
    }
    let proto = compile_program_with(&program, Some(&resolver)).map_err(|e| e.to_string())?;
    interpret(&proto, Vec::new()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use std::collections::HashMap;

    fn v(src: &str) -> Value {
        eval_source(src).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn arithmetic() {
        assert_eq!(v("fn main(): Int { return 1 + 2 * 3; }"), Value::Int(7));
    }

    #[test]
    fn locals_and_while() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var n: Int = 0;\n  while n < 10 {\n    n = n + 1;\n  }\n  return n;\n}"
            ),
            Value::Int(10)
        );
    }

    #[test]
    fn if_else() {
        assert_eq!(
            v("fn main(): Int {\n  if 1 < 2 {\n    return 3;\n  } else {\n    return 4;\n  }\n}"),
            Value::Int(3)
        );
    }

    #[test]
    fn and_or() {
        assert_eq!(
            v("fn main(): Bool { return 1 < 2 && 3 > 4; }"),
            Value::Bool(false)
        );
        assert_eq!(
            v("fn main(): Bool { return 1 > 2 || 3 < 4; }"),
            Value::Bool(true)
        );
    }

    #[test]
    fn string_add() {
        assert_eq!(
            v("fn main(): String { return \"a\" + \"b\"; }"),
            Value::String("ab".into())
        );
    }

    #[test]
    fn index_array() {
        assert_eq!(
            v("fn main(): Int { return [10, 20, 30][1]; }"),
            Value::Int(20)
        );
    }

    #[test]
    fn index_string() {
        assert_eq!(
            v("fn main(): String { return \"ab\"[1]; }"),
            Value::String("b".into())
        );
    }

    #[test]
    fn index_map() {
        assert_eq!(
            v("fn main(): Int { return { \"a\": 7 }[\"a\"]; }"),
            Value::Int(7)
        );
    }

    #[test]
    fn index_nested() {
        assert_eq!(
            v("fn main(): Int { return [[1, 2], [3, 4]][1][0]; }"),
            Value::Int(3)
        );
    }

    #[test]
    fn index_assign() {
        assert_eq!(
            v("fn main(): Int {\n  var a = [1, 2];\n  a[0] = 9;\n  return a[0];\n}"),
            Value::Int(9)
        );
    }

    #[test]
    fn index_compound() {
        assert_eq!(
            v("fn main(): Int {\n  var a = [1, 2];\n  a[0] += 3;\n  return a[0];\n}"),
            Value::Int(4)
        );
    }

    #[test]
    fn index_map_assign() {
        assert_eq!(
            v("fn main(): Int {\n  var m = { \"a\": 1 };\n  m[\"b\"] = 2;\n  return m[\"b\"];\n}"),
            Value::Int(2)
        );
    }

    #[test]
    fn index_oob() {
        let err = eval_source("fn main(): Int { return [1][3]; }").unwrap_err();
        assert!(err.contains("out of bounds"), "{err}");
    }

    #[test]
    fn for_range() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var sum: Int = 0;\n  for i in 0..5 {\n    sum = sum + i;\n  }\n  return sum;\n}"
            ),
            Value::Int(10)
        );
    }

    #[test]
    fn for_inclusive() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var prod: Int = 1;\n  for j in 1..=3 {\n    prod = prod * j;\n  }\n  return prod;\n}"
            ),
            Value::Int(6)
        );
    }

    #[test]
    fn for_array() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var sum: Int = 0;\n  for x in [1, 2, 3] {\n    sum = sum + x;\n  }\n  return sum;\n}"
            ),
            Value::Int(6)
        );
    }

    #[test]
    fn for_string() {
        assert_eq!(
            v(
                "fn main(): String {\n  var out: String = \"\";\n  for c in \"ab\" {\n    out = out + c;\n  }\n  return out;\n}"
            ),
            Value::String("ab".into())
        );
    }

    #[test]
    fn for_break() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var n: Int = 0;\n  for i in 0..10 {\n    if i == 3 {\n      break;\n    }\n    n = n + 1;\n  }\n  return n;\n}"
            ),
            Value::Int(3)
        );
    }

    #[test]
    fn for_continue() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var n: Int = 0;\n  for i in 0..5 {\n    if i == 2 {\n      continue;\n    }\n    n = n + i;\n  }\n  return n;\n}"
            ),
            Value::Int(8)
        );
    }

    #[test]
    fn call_helper() {
        assert_eq!(
            v(
                "fn add(a: Int, b: Int): Int {\n  return a + b;\n}\nfn main(): Int {\n  return add(2, 3);\n}"
            ),
            Value::Int(5)
        );
    }

    #[test]
    fn call_recursive() {
        assert_eq!(
            v(
                "fn fact(n: Int): Int {\n  if n <= 1 {\n    return 1;\n  }\n  return n * fact(n - 1);\n}\nfn main(): Int {\n  return fact(5);\n}"
            ),
            Value::Int(120)
        );
    }

    fn vm(src: &str, modules: HashMap<String, String>) -> Value {
        eval_source_with_modules(src, modules).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn import_unused_host() {
        assert_eq!(v("import io;\nfn main(): Int { return 1; }"), Value::Int(1));
    }

    #[test]
    fn builtin_len() {
        assert_eq!(v("fn main(): Int { return len(\"ab\"); }"), Value::Int(2));
    }

    #[test]
    fn builtin_print() {
        let program = crate::compile_source("fn main(): Int {\n  print(1, 2);\n  return 0;\n}")
            .unwrap_or_else(|e| panic!("{e}"));
        let proto = compile_program(&program).unwrap_or_else(|e| panic!("{e}"));
        let mut ctx = crate::EvalContext::new();
        interpret_in(&mut ctx, &proto, Vec::new()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ctx.stdout(), "1 2\n");
    }

    #[test]
    fn host_io_exists() {
        assert_eq!(
            v(
                "import io;\nfn main(): Bool { return io.exists(\"definitely-missing-rosegold-xyz\"); }"
            ),
            Value::Bool(false)
        );
    }

    #[test]
    fn from_host_import() {
        assert_eq!(
            v(
                "from io import exists;\nfn main(): Bool { return exists(\"definitely-missing-rosegold-xyz\"); }"
            ),
            Value::Bool(false)
        );
    }

    #[test]
    fn host_math_wrapper() {
        assert_eq!(
            v("import math;\nfn main(): Float { return math.abs(-3.0); }"),
            Value::Float(3.0)
        );
    }

    #[test]
    fn host_math_sin() {
        assert_eq!(
            v("import math;\nfn main(): Float { return math.sin(0.0); }"),
            Value::Float(0.0)
        );
    }

    #[test]
    fn host_internal_math() {
        assert_eq!(
            v("fn main(): Float { return __math.sin(0.0); }"),
            Value::Float(0.0)
        );
    }

    #[test]
    fn host_str_contains() {
        assert_eq!(
            v("import str;\nfn main(): Bool { return str.contains(\"abc\", \"b\"); }"),
            Value::Bool(true)
        );
    }

    #[test]
    fn method_string_len() {
        assert_eq!(v("fn main(): Int { return \"ab\".len(); }"), Value::Int(2));
    }

    #[test]
    fn method_string_len_property() {
        assert_eq!(
            v("fn main(): Int {\n  var s: String = \"ab\";\n  return s.len;\n}"),
            Value::Int(2)
        );
    }

    #[test]
    fn method_array_len() {
        assert_eq!(
            v("fn main(): Int { return [1, 2, 3].len(); }"),
            Value::Int(3)
        );
    }

    #[test]
    fn method_array_push_pop() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var a = [1];\n  a.push(2);\n  a.push(3);\n  a.pop();\n  return a.len();\n}"
            ),
            Value::Int(2)
        );
    }

    #[test]
    fn method_map_has() {
        assert_eq!(
            v("fn main(): Bool {\n  var m = { \"a\": 1 };\n  return m.has(\"a\");\n}"),
            Value::Bool(true)
        );
    }

    #[test]
    fn method_result_is_err() {
        assert_eq!(
            v(
                "import io;\nfn main(): Bool {\n  return io.read_text(\"definitely-missing-rosegold-xyz\").is_err();\n}"
            ),
            Value::Bool(true)
        );
    }

    #[test]
    fn import_module() {
        let mut modules = HashMap::new();
        modules.insert(
            "calc".into(),
            r#"
fn add(a: Int, b: Int): Int {
    return a + b;
}

fn double(n: Int): Int {
    return n * 2;
}
"#
            .into(),
        );
        assert_eq!(
            vm(
                "import calc;\nfn main(): Int {\n  return calc.add(2, 3) + calc.double(4);\n}",
                modules
            ),
            Value::Int(13)
        );
    }

    #[test]
    fn from_import_function() {
        let mut modules = HashMap::new();
        modules.insert(
            "strings".into(),
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
            .into(),
        );
        assert_eq!(
            vm(
                "from strings import repeat;\nfn main(): String {\n  return repeat(\"a\", 3);\n}",
                modules
            ),
            Value::String("aaa".into())
        );
    }

    #[test]
    fn import_module_internal_call() {
        let mut modules = HashMap::new();
        modules.insert(
            "calc".into(),
            r#"
fn add(a: Int, b: Int): Int {
    return a + b;
}

fn double(n: Int): Int {
    return add(n, n);
}
"#
            .into(),
        );
        assert_eq!(
            vm(
                "import calc;\nfn main(): Int {\n  return calc.double(4);\n}",
                modules
            ),
            Value::Int(8)
        );
    }

    #[test]
    fn import_nested_user_module() {
        let mut modules = HashMap::new();
        modules.insert(
            "util".into(),
            "fn inc(n: Int): Int {\n  return n + 1;\n}\n".into(),
        );
        modules.insert(
            "calc".into(),
            "import util;\nfn bump(n: Int): Int {\n  return util.inc(n);\n}\n".into(),
        );
        assert_eq!(
            vm(
                "import calc;\nfn main(): Int {\n  return calc.bump(1);\n}",
                modules
            ),
            Value::Int(2)
        );
    }

    #[test]
    fn import_named_mod() {
        let mut modules = HashMap::new();
        modules.insert(
            "helpers.rg".into(),
            r#"
mod utils {
    fn helper(n: Int): Int {
        return n + 1;
    }
    pub fn add(a: Int, b: Int): Int {
        return helper(a) + b;
    }
}
"#
            .into(),
        );
        assert_eq!(
            vm(
                "import utils;\nfn main(): Int {\n  return utils.add(2, 3);\n}",
                modules
            ),
            Value::Int(6)
        );
    }

    #[test]
    fn rejects_private_mod_export() {
        let mut modules = HashMap::new();
        modules.insert(
            "helpers.rg".into(),
            r#"
mod utils {
    fn helper(n: Int): Int {
        return n + 1;
    }
    pub fn add(a: Int, b: Int): Int {
        return helper(a) + b;
    }
}
"#
            .into(),
        );
        let err = eval_source_with_modules(
            "import utils;\nfn main(): Int {\n  return utils.helper(1);\n}",
            modules,
        )
        .unwrap_err();
        assert!(
            err.contains("no function") || err.contains("not compiled yet"),
            "{err}"
        );
    }

    #[test]
    fn missing_module() {
        let err = eval_source("import calc;\nfn main(): Int { return 1; }").unwrap_err();
        assert!(
            err.contains("not found") || err.contains("not compiled yet"),
            "{err}"
        );
    }

    #[test]
    fn fstring() {
        assert_eq!(
            v("fn main(): String {\n  var n: Int = 3;\n  return f\"n={n}\";\n}"),
            Value::String("n=3".into())
        );
    }

    #[test]
    fn fstring_format() {
        assert_eq!(
            v("fn main(): String {\n  return f\"{3.14159:.2f}\";\n}"),
            Value::String("3.14".into())
        );
    }

    #[test]
    fn try_ok() {
        assert_eq!(
            v(
                "fn inner(): Result {\n  var r: Result = Result.Ok(7);\n  var n = r?;\n  return Result.Ok(n);\n}\nfn main(): Int {\n  match inner() {\n    Ok(n) { return n; }\n    Err(_) { return 0; }\n  }\n}"
            ),
            Value::Int(7)
        );
    }

    #[test]
    fn try_err() {
        let v = v(
            "fn boom(): Result {\n  return Result.Err(\"no\");\n}\nfn main(): Result {\n  return boom()?;\n}",
        );
        match v {
            Value::Enum { variant, .. } => assert_eq!(variant, "Err"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn option_match() {
        assert_eq!(
            v(
                "fn main(): Int {\n  match Option.Some(4) {\n    Some(n) { return n; }\n    None { return 0; }\n  }\n}"
            ),
            Value::Int(4)
        );
    }

    #[test]
    fn option_none_match() {
        assert_eq!(
            v(
                "fn main(): Int {\n  match Option.None {\n    Some(_) { return 1; }\n    None { return 2; }\n  }\n}"
            ),
            Value::Int(2)
        );
    }

    #[test]
    fn struct_fields() {
        assert_eq!(
            v(
                "struct Point {\n  x: Int,\n  y: Int,\n}\nfn main(): Int {\n  var p: Point = Point { x: 1, y: 2 };\n  p.x = p.x + 10;\n  return p.x + p.y;\n}"
            ),
            Value::Int(13)
        );
    }

    #[test]
    fn impl_method() {
        assert_eq!(
            v(
                "struct Point {\n  x: Int,\n  y: Int,\n}\nimpl Point {\n  fn get(self): Int {\n    return self.x;\n  }\n}\nfn main(): Int {\n  var p: Point = Point { x: 3, y: 4 };\n  return p.get();\n}"
            ),
            Value::Int(3)
        );
    }

    #[test]
    fn class_method() {
        assert_eq!(
            v(
                "class Counter {\n  var n: Int = 0;\n  fn inc(self) {\n    self.n = self.n + 1;\n  }\n  fn get(self): Int {\n    return self.n;\n  }\n}\nfn main(): Int {\n  var c: Counter = Counter {};\n  c.inc();\n  c.inc();\n  return c.get();\n}"
            ),
            Value::Int(2)
        );
    }

    #[test]
    fn lambda_call() {
        assert_eq!(
            v("fn main(): Int {\n  var f = fn(x: Int): Int { return x + 1; };\n  return f(3);\n}"),
            Value::Int(4)
        );
    }

    #[test]
    fn lambda_capture() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var n: Int = 1;\n  var f = fn(x: Int): Int { return x + n; };\n  return f(2);\n}"
            ),
            Value::Int(3)
        );
    }

    #[test]
    fn lambda_capture_mutates() {
        assert_eq!(
            v(
                "fn main(): Int {\n  var n: Int = 1;\n  var f = fn(x: Int): Int { return x + n; };\n  n = 10;\n  return f(2);\n}"
            ),
            Value::Int(12)
        );
    }

    #[test]
    fn super_method() {
        assert_eq!(
            v(
                "class Enemy {\n  var hp: Int = 10;\n  fn hurt(self, dmg: Int): Int {\n    self.hp = self.hp - dmg;\n    return self.hp;\n  }\n}\nclass Slime extends Enemy {\n  fn hurt(self, dmg: Int): Int {\n    return super.hurt(dmg / 2);\n  }\n}\nfn main(): Int {\n  var s: Slime = Slime { hp: 10 };\n  return s.hurt(4);\n}"
            ),
            Value::Int(8)
        );
    }

    #[test]
    fn named_match_binds() {
        assert_eq!(
            v(
                "enum Shape {\n  Rect(width: Int, height: Int),\n}\nfn main(): Int {\n  match Shape.Rect(2, 3) {\n    Rect(width: w, height: h) { return w + h; }\n    _ { return 0; }\n  }\n}"
            ),
            Value::Int(5)
        );
    }

    #[test]
    fn spawn_await_fn() {
        assert_eq!(
            v(
                "fn add(a: Int, b: Int): Int {\n  return a + b;\n}\nfn main(): Int {\n  var t = spawn add(2, 3);\n  return await t;\n}"
            ),
            Value::Int(5)
        );
    }

    #[test]
    fn spawn_await_lambda() {
        assert_eq!(
            v("fn main(): Int {\n  var t = spawn fn(): Int { return 7; };\n  return await t;\n}"),
            Value::Int(7)
        );
    }

    #[test]
    fn async_fn_await() {
        assert_eq!(
            v(
                "async fn add(a: Int, b: Int): Int {\n  return a + b;\n}\nfn main(): Int {\n  return await add(2, 3);\n}"
            ),
            Value::Int(5)
        );
    }

    #[test]
    fn async_method_await() {
        assert_eq!(
            v(
                "class Box {\n  var n: Int = 0;\n  async fn get(self): Int {\n    return self.n;\n  }\n}\nfn main(): Int {\n  var b: Box = Box { n: 9 };\n  return await b.get();\n}"
            ),
            Value::Int(9)
        );
    }

    #[test]
    fn async_main_inline() {
        assert_eq!(v("async fn main(): Int {\n  return 4;\n}"), Value::Int(4));
    }

    #[test]
    fn signal_connect_emit() {
        let program = crate::compile_source(
            "signal ping();\nfn on_ping() {\n  print(1);\n}\nfn main(): Int {\n  ping.connect(on_ping);\n  ping.emit();\n  return 0;\n}",
        )
        .unwrap();
        let proto = compile_program(&program).unwrap();
        let mut ctx = crate::EvalContext::new();
        interpret_in(&mut ctx, &proto, Vec::new()).unwrap();
        assert_eq!(ctx.stdout().trim(), "1");
    }

    #[test]
    fn top_level_const() {
        assert_eq!(
            v("const n = 3;\nfn main(): Int {\n  return n;\n}"),
            Value::Int(3)
        );
    }

    #[test]
    fn top_level_var_mutates() {
        assert_eq!(
            v(
                "var n = 1;\nfn bump(): Int {\n  n = n + 1;\n  return n;\n}\nfn main(): Int {\n  bump();\n  return n;\n}"
            ),
            Value::Int(2)
        );
    }

    #[test]
    fn trait_signal_global() {
        let program = crate::compile_source(
            "trait Hit {\n  signal died();\n  fn go(self);\n}\nclass P impl Hit {\n  fn go(self) {\n    died.emit();\n  }\n}\nfn on_died() {\n  print(1);\n}\nfn main(): Int {\n  died.connect(on_died);\n  var p = P {};\n  p.go();\n  return 0;\n}",
        )
        .unwrap();
        let proto = compile_program(&program).unwrap();
        let mut ctx = crate::EvalContext::new();
        interpret_in(&mut ctx, &proto, Vec::new()).unwrap();
        assert_eq!(ctx.stdout().trim(), "1");
    }

    #[test]
    fn imported_enum_member() {
        let mut modules = HashMap::new();
        modules.insert(
            "util.math".into(),
            "pub enum Shape {\n  Circle(radius: Float),\n}\nfn main_unused(): Int { return 0; }\n"
                .into(),
        );
        assert_eq!(
            eval_source_with_modules(
                "import util.math;\nfn main(): Float {\n  match math.Shape.Circle(10.0) {\n    Circle(r) { return r; }\n    _ { return 0.0; }\n  }\n}",
                modules,
            )
            .unwrap(),
            Value::Float(10.0)
        );
    }

    #[test]
    fn crate_vec2_length() {
        assert_eq!(
            v("fn main(): Float {\n  return Vec2 { x: 3.0, y: 4.0 }.length();\n}"),
            Value::Float(5.0)
        );
    }

    #[test]
    fn super_as_value_field() {
        assert_eq!(
            v(
                "class Enemy {\n  var hp: Int = 10;\n}\nclass Slime extends Enemy {\n  fn hp_of(self): Int {\n    return super.hp;\n  }\n}\nfn main(): Int {\n  var s: Slime = Slime { hp: 8 };\n  return s.hp_of();\n}"
            ),
            Value::Int(8)
        );
    }

    #[test]
    fn main_file_mod_is_ignored() {
        assert_eq!(
            v(
                "mod unused {\n  fn helper(): Int { return 1; }\n}\nfn main(): Int {\n  return 2;\n}"
            ),
            Value::Int(2)
        );
    }
}
