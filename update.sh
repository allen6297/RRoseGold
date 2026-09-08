#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
version="26.0.1"
cd "$root"

# WSL does not see Windows `cargo` as `cargo` — it's `cargo.exe` under the Windows home.
if grep -qi microsoft /proc/version 2>/dev/null; then
  win_profile="$(cmd.exe /c "echo %USERPROFILE%" 2>/dev/null | tr -d '\r')"
  if [[ -n "$win_profile" ]] && command -v wslpath >/dev/null 2>&1; then
    win_home="$(wslpath "$win_profile")"
    export PATH="$win_home/.cargo/bin:$win_home/AppData/Local/Programs/cursor/resources/app/bin:/mnt/c/Program Files/nodejs:$PATH"
  fi
fi

resolve() {
  local name="$1"
  if command -v "$name" >/dev/null 2>&1; then
    command -v "$name"
    return 0
  fi
  if command -v "${name}.exe" >/dev/null 2>&1; then
    command -v "${name}.exe"
    return 0
  fi
  if command -v "${name}.cmd" >/dev/null 2>&1; then
    command -v "${name}.cmd"
    return 0
  fi
  return 1
}

need() {
  local name="$1"
  local found
  if found="$(resolve "$name")"; then
    echo "$found"
    return 0
  fi
  echo "missing $name" >&2
  exit 1
}

CARGO="$(need cargo)"
EDITOR_CLI="$(resolve cursor || resolve code || true)"

echo "Updating to version $version"

"$CARGO" test
"$CARGO" build
"$CARGO" install --path . --force

# `sed -i` tries to chmod a tempfile; that fails on /mnt/c (Windows drives).
replace_version() {
  local file="$1"
  local pattern="$2"
  local tmp
  tmp="$(mktemp)"
  sed "$pattern" "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
}

pkg="$root/vscode/package.json"
if [[ ! -f "$pkg" ]]; then
  echo "missing $pkg" >&2
  exit 1
fi

replace_version "$pkg" "s/\"version\": \"[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\"/\"version\": \"$version\"/"

lock="$root/vscode/package-lock.json"
if [[ -f "$lock" ]]; then
  # Only the lockfile's own "name"/"version" pair, not dependency versions.
  replace_version "$lock" "0,/\"version\": \"[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\"/s//\"version\": \"$version\"/"
fi

win_node="/mnt/c/Program Files/nodejs/node.exe"
win_npx_cli="/mnt/c/Program Files/nodejs/node_modules/npm/bin/npx-cli.js"
if [[ -f "$win_node" && -f "$win_npx_cli" ]]; then
  node_win="$(wslpath -w "$win_node" | tr -d '\r')"
  npx_win="$(wslpath -w "$win_npx_cli" | tr -d '\r')"
  vscode_win="$(wslpath -w "$root/vscode" | tr -d '\r')"
  powershell.exe -NoProfile -Command "Set-Location -LiteralPath '$vscode_win'; \$env:PATH = 'C:\Program Files\nodejs;' + \$env:PATH; & '$node_win' '$npx_win' --yes @vscode/vsce package"
elif NPM="$(resolve npm)"; then
  (
    cd "$root/vscode"
    "$NPM" run package
  )
else
  echo "skipping vsix: npm is not installed (Windows or WSL)."
  echo "install Node from https://nodejs.org then re-run, or in WSL: sudo apt install nodejs npm"
  echo "until then, in Cursor: Developer: Install Extension from Location… → vscode/"
  exit 0
fi

vsix="$root/vscode/allen6297-$version.vsix"
if [[ ! -f "$vsix" ]]; then
  echo "missing $vsix" >&2
  exit 1
fi

vsix_win="$(wslpath -w "$vsix" | tr -d '\r')"
cursor_cmd="/mnt/c/Users/monki/AppData/Local/Programs/cursor/resources/app/bin/cursor.cmd"
if [[ -n "${win_home:-}" && -f "$win_home/AppData/Local/Programs/cursor/resources/app/bin/cursor.cmd" ]]; then
  cursor_cmd="$win_home/AppData/Local/Programs/cursor/resources/app/bin/cursor.cmd"
fi
if [[ -f "$cursor_cmd" ]]; then
  cursor_win="$(wslpath -w "$cursor_cmd" | tr -d '\r')"
  powershell.exe -NoProfile -Command "& '$cursor_win' --install-extension '$vsix_win'"
elif [[ -n "${EDITOR_CLI}" ]]; then
  "$EDITOR_CLI" --install-extension "$vsix_win"
else
  echo "built $vsix_win"
  echo "install with: cursor --install-extension \"$vsix_win\""
fi
