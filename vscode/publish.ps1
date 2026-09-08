# Package the RoseGold VSIX, then publish to VS Marketplace and/or Open VSX
# if VSCE_PAT / OVSX_PAT are set in this shell. Never writes tokens to disk.
#
# Usage (from vscode/ or repo root):
#   $env:VSCE_PAT = "..."   # optional
#   $env:OVSX_PAT = "..."   # optional
#   .\publish.ps1
param(
    [switch]$PackageOnly
)

$ErrorActionPreference = "Stop"

$nodeDir = "C:\Program Files\nodejs"
if (Test-Path $nodeDir) {
    $env:PATH = "$nodeDir;" + $env:PATH
}

$here = $PSScriptRoot
Set-Location $here

if (-not (Get-Command npx -ErrorAction SilentlyContinue)) {
    throw "npx not found. Install Node from https://nodejs.org (or add C:\Program Files\nodejs to PATH)."
}

$pkg = Get-Content (Join-Path $here "package.json") -Raw | ConvertFrom-Json
$id = "$($pkg.publisher).$($pkg.name)"
Write-Host "Extension id: $id (version $($pkg.version))"

if ($pkg.publisher -ne "allen6297") {
    throw "package.json publisher is '$($pkg.publisher)'; expected allen6297. See PUBLISHING.md."
}

Write-Host "Packaging VSIX..."
npm run package
if ($LASTEXITCODE -ne 0) {
    throw "vsce package failed with exit $LASTEXITCODE"
}

$vsix = Get-ChildItem -File -Filter "rosegold-language-*.vsix" |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $vsix) {
    $vsix = Get-ChildItem -File -Filter "*.vsix" |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}
if (-not $vsix) {
    throw "No .vsix produced in $here"
}
Write-Host "Packaged $($vsix.Name)"

if ($PackageOnly) {
    Write-Host "Package-only. Marketplace: https://marketplace.visualstudio.com/manage"
    Write-Host "Open VSX namespace: npx ovsx create-namespace allen6297 -p `$env:OVSX_PAT"
    exit 0
}

$didPublish = $false

if ($env:VSCE_PAT) {
    Write-Host "Publishing to Visual Studio Marketplace (VSCE_PAT is set)..."
    npx --yes @vscode/vsce publish --no-dependencies
    if ($LASTEXITCODE -ne 0) {
        throw "vsce publish failed with exit $LASTEXITCODE"
    }
    $didPublish = $true
    Write-Host "Marketplace: https://marketplace.visualstudio.com/items?itemName=$id"
} else {
    Write-Host "VSCE_PAT not set. Skipping Marketplace."
    Write-Host "  Create publisher allen6297 at https://marketplace.visualstudio.com/manage"
    Write-Host "  PAT scope: Marketplace (Manage), Organization: All accessible organizations"
    Write-Host "  Then: `$env:VSCE_PAT = '<token>'; npx @vscode/vsce publish"
}

if ($env:OVSX_PAT) {
    Write-Host "Publishing to Open VSX (OVSX_PAT is set)..."
    npx --yes ovsx publish $vsix.Name -p $env:OVSX_PAT
    if ($LASTEXITCODE -ne 0) {
        throw "ovsx publish failed with exit $LASTEXITCODE"
    }
    $didPublish = $true
    Write-Host "Open VSX: https://open-vsx.org/extension/$($pkg.publisher)/$($pkg.name)"
    Write-Host "Cursor search can lag hours behind Open VSX."
} else {
    Write-Host "OVSX_PAT not set. Skipping Open VSX."
    Write-Host "  Sign in at https://open-vsx.org (GitHub allen6297)"
    Write-Host "  Token: https://open-vsx.org/user-settings/tokens"
    Write-Host "  Then: npx ovsx create-namespace allen6297 -p `$env:OVSX_PAT"
    Write-Host "        npx ovsx publish -p `$env:OVSX_PAT"
}

if (-not $didPublish) {
    Write-Host "No tokens in env. Sideload: .\install.ps1   Docs: PUBLISHING.md"
    exit 2
}
