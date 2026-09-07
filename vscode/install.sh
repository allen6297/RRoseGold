#!/usr/bin/env bash
# Package the RoseGold VSIX if needed and install into Cursor (else VS Code).
set -euo pipefail
cd "$(dirname "$0")"

force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
fi

latest_vsix() {
  ls -t allen6297-*.vsix 2>/dev/null | head -n1 || true
}

vsix="$(latest_vsix)"
need=0
if [[ "$force" -eq 1 || -z "$vsix" ]]; then
  need=1
elif [[ package.json -nt "$vsix" ]]; then
  need=1
fi

if [[ "$need" -eq 1 ]]; then
  echo "Packaging RoseGold VSIX..."
  npm run package
  vsix="$(latest_vsix)"
fi

if [[ -z "$vsix" ]]; then
  echo "Failed to produce a .vsix in $(pwd)" >&2
  exit 1
fi

if command -v cursor >/dev/null 2>&1; then
  cli=cursor
elif command -v code >/dev/null 2>&1; then
  cli=code
else
  echo "Neither 'cursor' nor 'code' is on PATH." >&2
  echo "Install manually:" >&2
  echo "  cursor --install-extension $(pwd)/$vsix" >&2
  echo "  code --install-extension $(pwd)/$vsix" >&2
  exit 1
fi

echo "Installing $vsix with $cli..."
"$cli" --install-extension "$vsix"
echo "Installed. Reload the window (Developer: Reload Window)."
