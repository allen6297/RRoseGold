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
```

## Layout

| Path | Role |
|---|---|
| `src/` | Language crate and CLI (`check`, `run`, `test`, `fmt`, `hover`, `def`) |
| `stdlib/` | Embedded `.rg` modules (`math`, `option`, `result`, `str`, `vec`, `checks`) |
| `examples/` | Sample scripts |
| `vscode/` | Language support for VS Code / Cursor |

Host modules `io` and `time` are native. Everything else in stdlib is `.rg`.

## Editor

1. `cargo build`
2. Command Palette → **Developer: Install Extension from Location…**
3. Choose `vscode/`
4. Open a `.rg` file

Details: [vscode/README.md](vscode/README.md).
