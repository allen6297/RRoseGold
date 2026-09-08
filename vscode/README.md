# RoseGold for VS Code / Cursor

Language support for [RoseGold](https://github.com/allen6297/RoseGold): highlighting, diagnostics, go to definition, hover, completions, format, run, and tests for `.rg` files.

Extension id: **`allen6297.rosegold-language`**.

## Install

Search for **RoseGold Language** in the Extensions view, or install by id `allen6297.rosegold-language`.

This extension talks to the `rosegold` CLI. Build or install it from the [language repo](https://github.com/allen6297/RoseGold) (`cargo build` or `cargo install --path .`). If the editor cannot find it, set **RoseGold › Cli Path** to the `rosegold` binary.

Sideload from a clone of the repo:

```powershell
.\vscode\install.ps1
```

Reload the window if prompted.

## What you get

- Diagnostics as you type (`rosegold check`)
- Go to definition, hover, signatures, completions
- Format Document
- Run File and Run Tests
- Test Explorer for `@test` functions
- Snippets for `fn`, `class`, `match`, and similar

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `rosegold.cliPath` | *(empty)* | Path to the `rosegold` binary. Empty = `target/{debug,release}/rosegold` in the workspace, then `PATH` |
| `rosegold.diagnosticsDelayMs` | `400` | How long to wait after you stop typing before re-checking |
