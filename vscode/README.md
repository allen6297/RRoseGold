# RoseGold for VS Code / Cursor

Syntax highlighting, diagnostics, navigation, testing, and snippets for `.rg` sources. Uses the **Rust** `rosegold` CLI in this repo (`cargo build -p rosegold`) plus the same host/stdlib catalog as Script mode (`catalog.json`).

## Install (local, in this repo)

1. Build the CLI:

   ```bash
   cargo build -p rosegold
   ```

2. Command Palette → **Developer: Install Extension from Location…**
3. Choose `editors/vscode`
4. Reload the window if prompted
5. Open a `.rg` file

The extension looks for `target/debug/rosegold` or `target/release/rosegold` walking up from the workspace. If the binary is elsewhere, set **RoseGold › Cli Path**.

### Package a `.vsix`

```bash
cd editors/vscode
npm run package
# installs as: cursor --install-extension rosegold-0.6.3.vsix
```

## Features

| Feature | How |
|---|---|
| Highlighting | TextMate grammar matching the lexer (`#` comments, `##` docs, `@export` / `@test` / `@ufcs` / `@node`, bitwise `&` `|` `^` `<<` `>>` `~`, f-strings, `signal`, `class` / `trait` / `extends` / `impl`) |
| Diagnostics | `rosegold check --json --stdin` (unsaved buffers included) |
| Go to Definition | F12 / Cmd-click via `rosegold def` (`import utils` → that file; `utils.move_line` → the fn) |
| Hover | Host/stdlib from `catalog.json`; user symbols from `rosegold hover` (`##` text when present) |
| Signature help | Catalog params while typing `(` / `,` (`strata.move`, `input.pressed`, `math.clamp`, …) |
| Completions | keywords, hooks, builtins, `strata.` / `input.` / `math.` / `str.` / `io.` / `checks.` / `option` / `result` members, crate types (`Vec2`, `Sprite`, …), locals |
| Outline | `fn` / `struct` / `class` / `trait` / `impl` / `enum` / `signal` / `@export var` |
| Snippets | `fn`, `on_update`, `on_exit`, `class`, `trait`, `nodeclass` / `@node`, `@export`, `@export_group`, `@ufcs`, `.emit`, `signal`, `match`, … |
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

Edit files here, then **Developer: Reload Window**. There is no hot reload. Keep `catalog.json` in sync with Script mode (`src/lib/rosegold-docs.ts` imports it).

## Changelog

**0.6.3** — F12 / Cmd-click on `import utils`, `from utils import move_line`, and dotted `import util.math`.
**0.6.2** — `@node class` snippet and completions; catalog covers crate `Node` / `Sprite` / …; `@node` highlights like `@ufcs`.
**0.6.1** — After `name.`, complete `emit`; `on_update` snippet matches the short hook; squiggles cover the whole token.
**0.6.0** — Catalog covers crate `option` / `result` / `Vec2`, `Some`/`None`/`Ok`/`Err`, `strata.find`, and WASM `io` as an in-memory VFS. Same `catalog.json` as Script mode.
