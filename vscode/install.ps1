# Package the RoseGold VSIX if needed and install into Cursor (else VS Code).
# Usage (from vscode/ or repo root):
#   .\install.ps1
#   powershell -ExecutionPolicy Bypass -File .\vscode\install.ps1
param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
Set-Location $here

function Latest-Vsix {
    $hit = Get-ChildItem -File -Filter "rosegold-language-*.vsix" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($hit) { return $hit }
    Get-ChildItem -File -Filter "*.vsix" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

$pkg = Get-Item (Join-Path $here "package.json")
$vsix = Latest-Vsix
$needPackage = $Force -or -not $vsix -or ($vsix.LastWriteTime -lt $pkg.LastWriteTime)

if ($needPackage) {
    Write-Host "Packaging RoseGold VSIX..."
    npm run package
    $vsix = Latest-Vsix
}

if (-not $vsix) {
    throw "Failed to produce a .vsix in $here"
}

$cli = $null
if (Get-Command cursor -ErrorAction SilentlyContinue) {
    $cli = "cursor"
} elseif (Get-Command code -ErrorAction SilentlyContinue) {
    $cli = "code"
} else {
    throw @"
Neither 'cursor' nor 'code' is on PATH.
Install manually:
  cursor --install-extension `"$($vsix.FullName)`"
  code --install-extension `"$($vsix.FullName)`"
"@
}

Write-Host "Installing $($vsix.Name) with $cli..."
& $cli --install-extension $vsix.FullName
if ($LASTEXITCODE -ne 0) {
    throw "$cli --install-extension failed with exit $LASTEXITCODE"
}
Write-Host "Installed. Reload the window (Developer: Reload Window)."
