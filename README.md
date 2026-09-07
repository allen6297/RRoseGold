# RoseGold

A general-purpose scripting language: lexer, parser, typecheck, tree-walking interpreter, and a VS Code / Cursor extension.

## Build

```bash
cargo build
```

That produces `target/debug/rosegold` (`rosegold.exe` on Windows). The editor looks for that binary walking up from the workspace.

```bash
cargo test
rosegold check examples/hello.rg
rosegold run examples/hello.rg
rosegold run examples/class_trait.rg
rosegold run examples/process.rg hello
rosegold run examples/tour.rg
rosegold run examples/json.rg
rosegold run examples/signals.rg
rosegold run examples/concurrency.rg
rosegold run examples/ui.rg
rosegold run examples/ui_window.rg
rosegold test examples/tests.rg
rosegold
```

## Layout

| Path | Role |
|---|---|
| `src/` | Language crate and CLI (`check`, `run`, `test`, `fmt`, `hover`, `def`, REPL) |
| `stdlib/` | Embedded `.rg` modules (`math`, `option`, `result`, `str`, `vec`, `checks`, `ui`) |
| `examples/` | Sample scripts |
| `vscode/` | Language support for VS Code / Cursor |

Host modules `io`, `time`, `process`, `json`, `path`, `http`, and `regex` are native. `ui` is `.rg` wrapping `__ui` (egui, native-only). Everything else in stdlib is `.rg`. `examples/ui.rg` is headless (used by tests). `examples/ui_window.rg` opens a real window — run it with `rosegold run examples/ui_window.rg` (or `cargo run --offline -- run examples/ui_window.rg`); do not add it to CI example runners.

`rosegold` with no command starts a REPL (each line is typechecked; `import` uses the current directory). `rosegold run file.rg args…` puts the script path and extra args in `process.argv()`. Runtime errors list the call stack with file names.

Language walkthrough: [docs/tour.md](docs/tour.md) and [`examples/tour.rg`](examples/tour.rg).

## Editor

1. `cargo build`
2. Command Palette → **Developer: Install Extension from Location…**
3. Choose `vscode/`
4. Open a `.rg` file

Details: [vscode/README.md](vscode/README.md).
