# RoseGold for VS Code / Cursor

Syntax highlighting, diagnostics, navigation, testing, and snippets for `.rg` sources. Uses the **Rust** `rosegold` CLI in this repo (`cargo build`) plus the host/stdlib catalog in `catalog.json`.

## Install (local, in this repo)

1. Build the CLI:

   ```bash
   cargo build
   ```

2. Command Palette → **Developer: Install Extension from Location…**
3. Choose `vscode`
4. Reload the window if prompted
5. Open a `.rg` file

The extension looks for `target/debug/rosegold` or `target/release/rosegold` walking up from the workspace. If the binary is elsewhere, set **RoseGold › Cli Path**.

### Package a `.vsix`

```bash
cd vscode
npm run package
# installs as: cursor --install-extension rosegold-0.7.4.vsix
```

## Features

| Feature | How |
|---|---|
| Highlighting | TextMate grammar matching the lexer (`#` comments, `##` docs, `@test` / `@ufcs`, bitwise `&` `|` `^` `<<` `>>` `~`, f-strings, `signal`, `class` / `trait` / `extends` / `impl`) |
| Diagnostics | `rosegold check --json --stdin` (unsaved buffers included) |
| Go to Definition | F12 / Cmd-click via `rosegold def` (`import utils` → that file; `utils.move_line` → the fn) |
| Hover | Host/stdlib from `catalog.json`; user symbols from `rosegold hover` (`##` text when present) |
| Signature help | Catalog params while typing `(` / `,` (`math.clamp`, `io.read_text`, `time.now`, …) |
| Completions | keywords, builtins, `math.` / `str.` / `io.` / `time.` / `process.` / `json.` / `checks.` / `option` / `result` members, crate types (`Vec2`, `Vec3`), locals |
| Outline | `fn` / `struct` / `class` / `trait` / `impl` / `enum` / `signal` / `var` |
| Snippets | `fn`, `class`, `trait`, `@ufcs`, `.emit`, `signal`, `match`, `@test`, … |
| Run File | play button / **RoseGold: Run File** (`rosegold run`) |
| Run Tests | beaker / **RoseGold: Run Tests** (`rosegold test`) |
| Test Explorer | Testing sidebar lists `@test` functions |
| Format | Format Document via `rosegold fmt --stdin` (`#` comments and `##` docs are kept) |

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `rosegold.cliPath` | *(empty)* | CLI override; empty = `target/{debug,release}/rosegold` then `PATH` |
| `rosegold.diagnosticsDelayMs` | `400` | Debounce for live diagnostics |

## Develop

Edit files here, then **Developer: Reload Window**. There is no hot reload. Keep `catalog.json` in sync with host APIs in `src/interpreter/dispatch.rs` and `stdlib/*.rg`.

```bash
npm test
```

## Changelog

**0.7.4** — Snippets for `import process` / `import json`. Host catalog already covers `argv` / `env` / `exit` and `parse` / `stringify`.
**0.7.3** — `signal.connect(fn)`: emit runs listeners in connect order.
**0.7.2** — `json.parse` / `json.stringify`; language tour (`docs/tour.md`, `examples/tour.rg`).
**0.7.1** — `process.argv` / `env` / `exit`, `rosegold` REPL, extra args after `run <file>`.
**0.7.0** — Standalone language: dropped Strata/`input`/`ui`/`@node`/`@export` host APIs. Kept `io`, `time`, and crate stdlib.
**0.6.3** — F12 / Cmd-click on `import utils`, `from utils import move_line`, and dotted `import util.math`.
**0.6.2** — `@node class` snippet and completions; catalog covers crate `Node` / `Sprite` / …; `@node` highlights like `@ufcs`.
**0.6.1** — After `name.`, complete `emit`; `on_update` snippet matches the short hook; squiggles cover the whole token.
**0.6.0** — Catalog covers crate `option` / `result` / `Vec2`, `Some`/`None`/`Ok`/`Err`, `strata.find`, and WASM `io` as an in-memory VFS.
