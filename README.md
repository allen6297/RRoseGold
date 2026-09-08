# RoseGold

A small scripting language: lexer, parser, typecheck, tree-walking interpreter, and a VS Code / Cursor extension.

Walkthrough: [docs/tour.md](docs/tour.md) and [`examples/tour.rg`](examples/tour.rg). UI: [docs/ui.md](docs/ui.md). Plan: [docs/plan.md](docs/plan.md).

## Build

```bash
cargo build
cargo test
```

That produces `target/debug/rosegold` (`rosegold.exe` on Windows). The editor looks for that binary walking up from the workspace.

## CLI

```bash
rosegold                         # REPL (each line is typechecked; import from cwd)
rosegold check examples/hello.rg
rosegold run examples/hello.rg
rosegold run examples/process.rg hello   # extra args → process.argv()
rosegold test examples/tests.rg
rosegold fmt examples/hello.rg
```

`run file.rg args…` puts the script path and extra args in `process.argv()`. Runtime errors list the call stack.

`cargo build` writes `target/debug/rosegold`. That is not the same as `rosegold` on PATH (often an older `cargo install --path .`). After you change the interpreter, run with `cargo run --offline -- run examples/…` or `./target/debug/rosegold`.

Other examples: `tour.rg`, `concurrency.rg`, `json.rg`, `signals.rg`. `examples/ui.rg` is headless (tests). Live window:

```bash
cargo run --offline -- run examples/ui_window.rg
```

## Vendor

Run these from the project root (the directory with `main.rg` / `vendor.lock`). The command writes `./vendor/` and `./vendor.lock` in the current directory. `import` looks for `vendor/` next to the script you pass to `run` / `check`.

```bash
rosegold vendor https://github.com/you/httpclient.git
rosegold vendor                    # restore from vendor.lock
rosegold vendor remove httpclient
```

`import httpclient` loads `vendor/httpclient/lib.rg`. Name comes from optional `rg.toml`, else the repo (`httpclient.git` → `httpclient`). Lock lines are `name url sha` or `name url sha version`. A second version of the same name keeps the old tree as `vendor/httpclient-0.1/`; import still hits the current pin. Commit `vendor/` and `vendor.lock` with the project. No registry.

A library is ordinary RoseGold (`lib.rg` plus optional `rg.toml`):

```toml
name = "httpclient"
version = "0.1"
files = ["lib.rg", "parse.rg"]
```

## Editor

Extension id **`allen6297.rosegold-language`**. Sideload:

```powershell
.\vscode\install.ps1
```

Marketplace / Open VSX: [vscode/PUBLISHING.md](vscode/PUBLISHING.md). Details: [vscode/README.md](vscode/README.md).

## Layout

| Path | Role |
|---|---|
| `src/` | Language crate and CLI (`check`, `run`, `test`, `fmt`, `hover`, `def`, `vendor`, REPL) |
| `stdlib/` | Embedded `.rg` modules (`math`, `option`, `result`, `str`, `vec`, `checks`, `ui`) |
| `examples/` | Sample scripts |
| `vscode/` | Language support for VS Code / Cursor |

Host modules `io`, `time`, `process`, `json`, `path`, `http`, and `regex` are native. `ui` is `.rg` wrapping `__ui` (egui, native-only). Everything else in stdlib is `.rg`.
