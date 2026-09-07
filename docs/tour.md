# Language tour

RoseGold is a small scripting language: `#` comments, `##` docs, `fn main()`, classes, `match`, and `@test`. Build the CLI with `cargo build`, then:

```bash
rosegold                  # REPL
rosegold run examples/tour.rg
rosegold test examples/tests.rg
```

Types: `Int`, `Float`, `String` (alias `Str`), `Bool`, `Void`, `Array`, `Map`, `Option`, `Result`. `none` is the missing value.

## Values and control flow

```rg
fn main(): Int {
    var n: Int = 3;
    const name = "RoseGold";
    print(f"{name} n={n}");

    if n > 2 {
        print("big");
    } elif n == 0 {
        print("zero");
    } else {
        print("small");
    }

    var sum: Int = 0;
    for i in 0..5 {
        sum = sum + i;
    }
    while n > 0 {
        n = n - 1;
    }
    return 0;
}
```

`0..5` is exclusive of 5; `1..=3` is inclusive. Arrays and maps: `[1, 2]`, `{"a": 1}`. `len(xs)`, `xs.len()`, `xs.push(x)`, `m.has(key)`, `m[key] = v`. F-strings: `f"x={x}"`, `f"{pi:.2f}"`.

## Functions, tests, UFCS

```rg
fn add(a: Int, b: Int): Int {
    return a + b;
}

@test
fn add_works() {
    assert(add(2, 2) == 4);
}

@ufcs
fn doubled(n: Int): Int {
    return n * 2;
}

fn main(): Int {
    print((3).doubled());
    return 0;
}
```

`rosegold test file.rg` runs `@test` functions and does not call `main`. `@ufcs` lets `x.fn(args)` mean `fn(x, args)`.

## Option, Result, match

```rg
fn score_of(scores: Map<String, Int>, name: String): Result<Int, String> {
    if scores.has(name) {
        return Result.Ok(scores[name]);
    }
    return Result.Err(f"unknown: {name}");
}

fn main(): Int {
    match Option.Some(1) {
        Some(v) { print(v); }
        None { print("none"); }
    }
    match score_of({"ada": 10}, "ada") {
        Ok(v) { print(v); }
        Err(e) { print(e); }
    }
    return 0;
}
```

`unwrap` / `is_some` / `is_ok` exist. Today `unwrap_or` is Int-only.

## Signals

Typed events. `connect` adds one free function; `emit` calls every listener in connect order. Several subscribers means several `connect` calls.

```rg
signal collected(amount: Int);

fn log_coin(amount: Int) {
    print(amount);
}

fn main(): Int {
    collected.connect(log_coin);
    collected.emit(5);
    return 0;
}
```

A `signal` on a trait is a contract: types that `impl` it can `died.emit()`. Connect still uses the signal name (`died.connect(on_died)`).

## Classes and traits

```rg
trait Named {
    fn label(self): String;
}

class Point impl Named {
    var x: Float = 0.0;
    var y: Float = 0.0;

    fn length(self): Float {
        return math.sqrt(self.x * self.x + self.y * self.y);
    }

    fn label(self): String {
        return "point";
    }
}

class Offset extends Point {
    fn length(self): Float {
        return super.length();
    }
}
```

`import math;` is required for `math.sqrt`. Crate `Vec2` / `Vec3` are always in scope. `pub` on items inside `mod { }` is required to export; a file module exports everything.

## Modules

```rg
import math;
import util.helpers;
from str import contains;
```

`import util.math` loads `util/math.rg` or `util/math/lib.rg` (also `util.math.rg` and `util/math/main.rg`). Missing imports name those paths. `from m import item` binds `item` in the current file. Runtime errors print a call stack (`from util.boom at main.rg:3:12`) and the file of the error site.

## Host modules

Require `import`:

| Module | Role |
|---|---|
| `io` | `read_text` / `write_text` / `exists` / `mkdir` / … (many return `Result`) |
| `time` | `now()` Unix seconds; `elapsed()` since this VM started |
| `process` | `argv()`, `env(name)`, `exit(code)` |
| `json` | `parse(text)` / `stringify(value)` → `Result` |

JSON objects become `Map`, arrays `Array`, `null` becomes `none`. `Option.None` stringifies as `null`.

Crate `.rg` stdlib: `math`, `str`, `vec`, `option`, `result`, `checks`.

Runnable walkthrough: [`examples/tour.rg`](../examples/tour.rg). JSON only: [`examples/json.rg`](../examples/json.rg).
