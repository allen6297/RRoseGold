# Language tour

RoseGold is a small scripting language: `#` comments, `##` docs, `fn main()`, classes, `match`, and `@test`. Build the CLI with `cargo build`, then:

```bash
rosegold                  # REPL
rosegold run examples/tour.rg
rosegold test examples/tests.rg
```

Types: `Int`, `Float`, `String` (alias `Str`), `Bool`, `Void`, `Array`, `Map`, `Option`, `Result`, `Mutex`, `Channel`, `Task`. `none` is the missing value.

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

`0..5` is exclusive of 5; `1..=3` is inclusive. Arrays and maps: `[1, 2]`, `{"a": 1}`. `len(xs)`, `xs.len()`, `xs.push(x)`, `m.has(key)`, `m[key] = v`. F-strings: `f"x={x}"`, `f"{pi:.2f}"`. `for x in [1, 2]` types `x` as `Int` (homogeneous array literal). `for x in xs` when `xs` is `Array` leaves `x` untyped, so operator checks stay quiet.

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

`unwrap` / `is_some` / `is_ok` / `unwrap_or` exist. `unwrap_or` accepts any fallback type. On a `Result`, postfix `?` unwraps `Ok` or returns that `Err` from the enclosing function (which should return `Result`).

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

`rosegold vendor <git-url>` clones a library into `vendor/<name>/` and pins the git SHA in `vendor.lock` (lines: `name url sha` or `name url sha version`). Run it from the project root (`main.rg` / `vendor.lock`); `import` looks for `vendor/` next to the script passed to `run`/`check`. `rosegold vendor` with no URL restores the lock. `rosegold vendor remove <name>` drops that folder and lock line. `import httpclient` loads `vendor/httpclient/lib.rg`. `import vendor.httpclient` still works.

A library may ship `rg.toml` (`name`, `version`, optional `files`). If you vendor the same name at a different version, the previous tree is kept as `vendor/httpclient-0.1/` and the new pin stays at `vendor/httpclient/` (what `import httpclient` loads). Extra folders are stored and locked, not imported (`httpclient-0.1` is not an identifier); `rosegold vendor remove httpclient-0.1` drops one.

## Host modules

Require `import`:

| Module | Role |
|---|---|
| `io` | `read_text` / `write_text` / `exists` / `mkdir` / `read_stdin()` / `read_line()` / … (many return `Result`) |
| `time` | `now()` Unix seconds; `elapsed()` since this VM started |
| `process` | `argv()`, `env(name)`, `exit(code)`, `run(cmd, args)` → `Result` (stdout or error) |
| `json` | `parse(text)` / `stringify(value)` → `Result` |
| `path` | `join(a, b)`, `dirname(p)`, `ext(p)` |
| `http` | `get(url)` / `post(url, body)` → `Result` (body or error) |
| `regex` | `is_match(pattern, text)` / `find(pattern, text)` → `Result` |
| `ui` | Window, inputs, scroll, file picker — see [ui.md](ui.md) (native-only) |

JSON objects become `Map`, arrays `Array`, `null` becomes `none`. `Option.None` stringifies as `null`.

Crate `.rg` stdlib: `math`, `str`, `vec`, `option`, `result`, `checks`, `ui`.

## Concurrency

`spawn call(...)` starts an OS thread and returns a `Task`. `await` waits for that task and yields its return value. Calling an `async fn` as an expression does the same thing: it returns a `Task` without writing `spawn`. `main` and `@test` still run inline, so `await` inside them yields the inner value. Heap objects passed as arguments are shared (arrays, maps, instances). Use `Mutex` for multi-step updates and `Channel` to send values between tasks.

`ch.close()` stops new sends (`send` after close is a runtime error). `recv` still drains queued values, then returns `none` instead of blocking. `ch.recv_timeout(seconds)` and `task.wait(seconds)` return `Option` (`None` on timeout). Deadlock is possible: two tasks `await` each other, or two mutexes locked in opposite order — the VM does not detect that.

```rg
fn hang(ch: Channel) {
    ch.recv();
}

fn main(): Int {
    var ch = Channel();
    ch.send(1);
    ch.close();
    print(ch.recv());
    print(ch.recv());

    var stuck = Channel();
    var t = spawn hang(stuck);
    print(t.wait(0.05).is_none());
    stuck.close();
    await t;
    return 0;
}
```

```rg
async fn add(a: Int, b: Int): Int {
    return a + b;
}

fn bump(xs: Array, lock: Mutex) {
    lock.lock();
    xs.push(1);
    lock.unlock();
}

fn main(): Int {
    print(await add(2, 3));
    var t = add(1, 1);
    print(await t);

    var xs = [];
    var lock = Mutex();
    var u = spawn bump(xs, lock);
    bump(xs, lock);
    await u;
    print(xs.len());
    return 0;
}
```

`spawn` accepts a call (`spawn bump(xs, lock)`) or a lambda (`spawn fn() { … }`). `spawn async_fn()` still starts one task (not a Task of a Task). If `main` returns while tasks are still running, the VM joins them. `io` stays blocking: it only stalls that thread. `await` is allowed in any function.

## Closures

`fn(params) { … }` is an expression. It closes over locals: reads and writes the same bindings as the enclosing function. Pass it to `connect`, store it in a `var`, or `spawn` it. The last argument of a call may be a `{ }` block (`foo(1) { … }`, `column { … }`) — a zero-param lambda.

```rg
fn main(): Int {
    var n: Int = 0;
    var bump = fn() { n = n + 1; };
    bump();
    print(n);
    var t = spawn fn() { n = n + 1; };
    await t;
    print(n);
    return 0;
}
```

## UI

Full reference: [ui.md](ui.md). `import ui` is write-once egui. `ui.run()` stays on the main thread — do not `spawn` the UI. Inputs are immediate mode (`name = ui.field(name)`). `ui.open()` / `ui.save()` are native file dialogs; call them from a button during `run()`.

PATH `rosegold` may be an older install. For a live window:

```bash
cargo run --offline -- run examples/ui_window.rg
```

```rg
import ui;

fn main(): Int {
    ui.theme({
        "bg": "#1b1b1b",
        "text": "#f2e6dc",
        "accent": "#c45c26",
        "font_size": 14,
    });
    var name = "hi";
    var on = false;
    var vol = 0.5;
    var choice = "a";
    ui.window("Demo") {
        ui.column {
            ui.row {
                ui.text("Name");
                name = ui.field(name);
            };
            on = ui.checkbox("Loud", on);
            vol = ui.slider(vol, 0.0, 1.0);
            choice = ui.select(["a", "b", "c"], choice);
            ui.progress(vol);
            ui.scroll {
                ui.text("long");
            }.width(180).height(120);
            ui.button("OK") { ui.alert("hi"); }
                .padding(8)
                .color("#c45c26")
                .width(120);
            ui.button("Quit") { ui.quit(); };
        };
    };
    return ui.run();
}
```

`ui.column { … }` / `ui.row { … }` / `ui.scroll { … }` / `ui.window("Demo") { … }` is a trailing-closure scope. `.padding` / `.color` / `.bg` / `.width` / `.height` / `.font_size` / `.disabled` are handle modifiers.

Runnable walkthrough: [`examples/tour.rg`](../examples/tour.rg). JSON: [`examples/json.rg`](../examples/json.rg). Concurrency: [`examples/concurrency.rg`](../examples/concurrency.rg). Headless UI: [`examples/ui.rg`](../examples/ui.rg). Live window: [`examples/ui_window.rg`](../examples/ui_window.rg).
