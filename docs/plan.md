# Plan

Work for RoseGold that is not happening yet. Language and host APIs first; UI and vendor after that; bytecode maybe never.

## 1. Order of work

1. ~~Lambdas / closures~~ — done (`fn() { ... }`, captures)
2. ~~Trailing closures (last arg may be `{ }`)~~ — done (`foo(1) { … }`, `column { … }`)
3. ~~`process.run` + stdin~~ — done (`process.run`, `io.read_stdin`, `io.read_line`)
4. ~~Typecheck imported module bodies~~ — done (`check`/`run` walk import fn and method bodies)
5. ~~Generic `unwrap_or`, array `for`-in types, `?` on `Result`~~ — done
6. ~~Remaining host modules (`path`, `http`, `regex`)~~ — done (`path.join`/`dirname`/`ext`, `http.get`/`post`, `regex.is_match`/`find`)
7. ~~Concurrency polish~~ — done (`Channel.close`, recv-after-close `none`, `recv_timeout` / `Task.wait`)
8. Ship the editor — VSIX sideload done (`vscode/install.ps1` / `install.sh`). Marketplace / Open VSX pending publisher login for **`allen6297.allen6297`**. See `vscode/PUBLISHING.md`.
9. ~~Portable `ui`~~ — done (`import ui`: alert, window, button/text, row, field/checkbox/slider, separator/spacer, `quit`/`invalidate`, `run()`, theme, v1 modifiers). `ui.run()` is native-only and opens a window; tests use theme/alert/handles/`__ui.pump()` and do not call `run()` when a window is registered.
10. ~~`rosegold vendor` when people share `.rg` files~~ — done (`vendor <git-url>`, lock-only `vendor`, `vendor remove`, `rg.toml`, collision folders `vendor/<name>-<version>/`)

Bytecode / JIT only if something is actually slow after that.

## 2. Language

### Holes

- ~~Generic `unwrap_or` (tour still says Int-only).~~ Any fallback type.
- ~~Typecheck **imported** function bodies.~~ `1 + "x"` in `util.rg` is a type diagnostic on `check` / `run`.
- ~~`for x in xs` when `xs` is an array:~~ homogeneous array literals infer the element type; mixed / `Array` variables stay unknown on purpose.

### Closures and `?`

`FnRef` is a global name with no captures. Every callback is a top-level `fn`. Closures unblock `spawn`, signals, and UI.

- **Lambdas / closures** — `fn() { … }` that capture locals.
- **Trailing closures** — last argument may be a `{ }` block: `ui.button("OK") { … }`, `ui.column { … }`. Useful outside UI too.
- ~~**`?` on `Result`**~~ — postfix `?` unwraps `Ok` or returns the `Err` from a `Result`-returning function.

## 3. Host modules

Same pattern as `io`: native (or `.rg` wrapping `__host`), `import` required, tiny surface. Not a framework.

| API | Why |
|---|---|
| `process.run(cmd, args)` | Can `exit` and read `argv`; start another program. `Result.Ok` is stdout. |
| `io` stdin / line input | `io.read_stdin(): Result`, `io.read_line(): Option`. |
| `path` join / dirname / ext | `io` is files, not paths. |
| `http` get/post → `Result` | One useful networked script. |
| `regex` | `str.contains` is not search. |

## 4. Concurrency

Already have `spawn` / `await` / `Mutex` / `Channel` / `Task`.

- ~~`Channel` close + recv-after-close~~ — `send` after close errors; empty `recv` returns `none`
- ~~Timeout on `recv` / `await`~~ — `recv_timeout(seconds)` and `Task.wait(seconds)` return `Option` (`time` already exists)
- Deadlock is documented (`await` cycles, mutex cycles); the VM does not detect it

`ui.run()` stays on the main thread. `spawn` is not for the UI. `fork_task` / `live_tasks` in the interpreter are unused leftovers; do not build on them.

## 5. Editor

The vscode/Cursor extension already has diagnostics, run, tests, hover, def. Ship it: publish or a one-command install. Keep `catalog.json` in sync with host APIs. Helps more than new syntax.

## 6. Portable UI (`import ui`)

After closures, trailing closures, and handles.

Public API is **RoseGold** (`stdlib/ui.rg`, same pattern as `math` wrapping `__math`). Scripts `import ui;` and call `.rg` functions. Those call a thin host (`__ui.*`) that owns **egui + winit**. Do not put padding, theme maps, or `primary_button` in Rust — only primitives the runtime must do (window, run loop, paint a widget, apply a style blob).

Write-once: same script on Windows / macOS / Linux. Widgets will not look native. No `win` / `app` / `gtk`.

Swift-**looking** calls, not SwiftUI (no result builders, `@State`, `$binding`, `some View`). Custom widgets are `.rg` functions. egui is immediate mode (`body` can run every frame); we do not diff a retained `View` tree.

```rg
import ui;

fn main(): Int {
    ui.theme({
        "bg": "#1b1b1b",
        "text": "#f2e6dc",
        "accent": "#c45c26",
        "font_size": 14,
    });
    ui.window("Demo") {
        ui.column {
            ui.text("Hello");
            ui.button("OK") { ui.alert("hi"); }
                .padding(8)
                .color("#c45c26")
                .width(120);
        };
    };
    return ui.run();
}
```

`ui.column { … }` / `ui.window("Demo") { … }` is a trailing-closure scope (egui parent). `.padding` / `.color` are UFCS modifiers that return the same handle.

**Customization**

- **Modifiers (v1):** `padding`, `color` / `bg`, `width` / `height`, `font_size`, `disabled`. No shadow, material, or CSS.
- **Theme:** one `ui.theme({ … })` map → egui `Visuals`. Widgets inherit unless a modifier overrides.
- **Custom widgets:** functions, not a `View` protocol:

```rg
fn primary_button(u: Ui, label: String, action: Fn) {
    u.button(label, action).color("#c45c26").padding(10);
}
```

Callbacks on the interpreter thread. **First surface:** alert → window → button/label → `run()`, then theme + v1 modifiers. Not a layout engine, design system, drawing API, or OS dark/light sync.

**Next surface**

v1 opens a window and takes a click. Tiny apps still need input, the other parent, and a way to leave. Same rule: `.rg` wrappers, host only for the primitive. Immediate mode — pass current, get next. No bindings.

- ~~**`ui.row { … }`**~~ — done (`column` exists; this is the other parent).
- ~~**`ui.field(s)`**~~ — done (text; `String` in, next `String` out).
- ~~**`ui.checkbox(label, on)`**~~ — done (`Bool`).
- ~~**`ui.slider(value, min, max)`**~~ — done (`Float` + range).
- ~~**`ui.separator` / `ui.spacer`**~~ — done (a line or a gap. Not flexbox).
- ~~**`ui.quit()`**~~ — done (close from a button).
- ~~**Idle**~~ — done (wait for egui events; `ui.invalidate()` when a script needs a frame).

Later, still one call each:

- **`ui.select(options, current)`** — list of strings → chosen string.
- **`ui.scroll { … }`** — wrap a column/row.
- **File picker** — `ui.open` / `ui.save` → `Option`/`Result` path. Scripts care.
- **`ui.progress(t)`** — `0..1` if cheap.
- A second window or a dialog besides `alert` is optional. Do not invent a window manager.

Still not: canvas, animation, routing, menus-as-a-platform, tray, shaders, an a11y tree as a project, OS dark/light sync.

## 7. Vendor, not a registry

After people are copying `.rg` libraries around. No crates.io / npm.

1. ~~**`rosegold vendor <git-url>`**~~ — clone into `vendor/<name>/`. `import httpclient` → `vendor/httpclient/lib.rg`. Pins are git SHAs in `vendor.lock`.
2. ~~**Tiny `rg.toml`**~~ — `name`, `version`, optional `files`. If two versions of the same name would collide, the previous tree moves to `vendor/<name>-<version>/` and the new pin stays at `vendor/<name>/` (what `import httpclient` loads). Extra folders are stored and locked, not imported (`httpclient-0.1` is not an identifier); `vendor remove httpclient-0.1` drops one. No registry, no semver solver.
3. ~~**`rosegold vendor` (no URL)**~~ — restore every pin from `vendor.lock` into the matching folder. Missing lock is an error. Idempotent.
4. ~~**`rosegold vendor remove <name>`**~~ — drop that folder and lock line. Unknown name is an error.

Run `rosegold vendor` from the project root (`main.rg` / `vendor.lock`). `run`/`check` resolve `vendor/` from the entry script’s directory (same as sibling imports). `import vendor.httpclient` still works as a dotted path.

Lock lines are greppable: `name url sha` or `name url sha version`. `name` is the folder and the remove key.

```text
scores/
  main.rg
  util.rg
  vendor/
    httpclient/
      rg.toml
      lib.rg
      parse.rg
    regexish/
      rg.toml
      lib.rg
  vendor.lock
```

```rg
import httpclient;
import util;
```

The library repo is ordinary RoseGold (`lib.rg` + `rg.toml`).

## 8. Runtime: stay on the tree-walker

RoseGold executes by walking the AST (`src/interpreter/`). Comments that say “VM” mean this process’s `EvalContext` (for example `time.elapsed()`). Do not add bytecode, a JIT, or a register machine while the language is still moving.

Bytecode is the program as a compact instruction list (`LOAD_CONST 1; LOAD_CONST 2; ADD`) instead of `Expr` nodes. A **stack** VM is the right first backend if we ever compile (after typecheck: emit a chunk, interpret it). A **register** VM comes after that instruction set is boring. **JIT** is last and only if the interpreter is the measured bottleneck.

It helps **large runs** (hot loops), not **large codebases** (`check` / hover still need the AST). Cached on-disk bytecode can help startup; compiling on every `rosegold run` can be slower. Host-bound scripts (`io` / `json`) barely care.

If something feels slow before that, fix **values** first: arrays/maps sit behind `Arc<Mutex<_>>`, names get cloned on lookup. Flatten call dispatch; stop cloning `Value` on every local read.

## Not this

- Package registry. Vendor + git SHAs first.
- Framework stdlib until people share `.rg` files.
- Per-OS native UI (`win` / `app` / `gtk`).
- SwiftUI-the-language (result builders, `@State`, `$binding`, `some View`).
- Wrapping all of Win32 / AppKit / GTK.
- Bytecode / JIT / register VM until the surface settles and something is actually slow.
- CSS, material, layout engine, or OS appearance sync in `ui` v1.
