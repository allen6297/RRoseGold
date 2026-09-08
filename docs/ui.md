# `import ui`

Write-once desktop UI on **egui**. Same script on Windows / macOS / Linux. Widgets do not look native. There is no CSS, HTML layout, or OS chrome.

Public API is [`stdlib/ui.rg`](../stdlib/ui.rg). The host (`__ui`) only runs the window, paints primitives, and applies a style blob. Theme maps and modifiers stay in `.rg`.

## Run

`ui.run()` must stay on the **main thread**. Do not `spawn` the UI. Click handlers run on that same thread.

Use the **workspace** binary. `rosegold` on PATH is often an older `cargo install`:

```bash
cargo run --offline -- run examples/ui_window.rg
```

`examples/ui.rg` is headless (tests). It does not call `ui.run()`. `__ui.pump()` rebuilds the widget tree without opening a window. File dialogs return `none` during `pump()` and in tests.

WASM: `__ui` errors with `ui is not supported on this target`.

## Immediate mode

The window body runs **every frame**. Inputs take the current value and return the next:

```rg
name = ui.field(name);
on = ui.checkbox("Loud", on);
vol = ui.slider(vol, 0.0, 1.0);
choice = ui.select(["a", "b", "c"], choice);
```

Locals you `var` in `main` persist. Do not recreate them inside the window body.

The window waits for input (idle). Call `ui.invalidate()` when a script needs a frame without a click or key. `ui.quit()` closes from a button (or close the chrome).

## Theme

Call `ui.theme({ … })` once before `run()`. Keys: `bg`, `text`, `accent` (hex `#rrggbb` or `#rgb`), `font_size` (Int or Float). Widgets inherit unless a modifier overrides.

## Containers

Trailing `{ }` is the last argument (a zero-param lambda). That is the egui parent for nested widgets.

| Call | Role |
|---|---|
| `ui.window(title) { … }` | Register a window. `run()` shows it. |
| `ui.column { … }` | Vertical stack |
| `ui.row { … }` | Horizontal stack |
| `ui.scroll { … }` | Vertical scroll. Pin size with `.width(n)` / `.height(n)` |

Omit `.width` on `scroll` to fill the parent. `.width(n)` and `.height(n)` set the viewport (pixels). Content taller than `.height` scrolls. Scroll position is kept across frames.

## Widgets

| Call | Returns | Notes |
|---|---|---|
| `ui.text(s)` | `Widget` | Label |
| `ui.field(s)` | `String` | Single-line edit |
| `ui.button(label) { … }` | `Widget` | Trailing closure is the click action |
| `ui.checkbox(label, on)` | `Bool` | |
| `ui.slider(value, min, max)` | `Float` | |
| `ui.select(options, current)` | `String` | `options` is `Array` of strings |
| `ui.progress(t)` | `Widget` | `t` clamped to `0..1` |
| `ui.separator()` | `Widget` | Line |
| `ui.spacer()` | `Widget` | Gap; `.height` is the space |
| `ui.alert(msg)` | — | Modal until OK or backdrop click |
| `ui.open()` | `Option` | Native open dialog. `none` if cancel / not in `run()` |
| `ui.save()` | `Option` | Native save dialog. Call from a **button**, not every frame |

## Modifiers

Chain on a `Widget` handle (including after a trailing `{ }`):

`.padding(n)` · `.color(hex)` · `.bg(hex)` · `.width(n)` · `.height(n)` · `.font_size(n)` · `.disabled(on)`

Custom widgets are ordinary functions:

```rg
fn primary_button(label: String, action: Fn): Widget {
    return ui.button(label, action).color("#c45c26").padding(10);
}
```

## Example

See [`examples/ui_window.rg`](../examples/ui_window.rg) and the short walkthrough in [tour.md](tour.md#ui).
