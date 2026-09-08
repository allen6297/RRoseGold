# Publishing RoseGold (`allen6297.rosegold-language`)

The extension id is **`allen6297.rosegold-language`**: `package.json` `publisher` + `name`. Display name is **RoseGold Language**.

- **VS Code search** = Visual Studio Marketplace (`vsce`)
- **Cursor search** = Open VSX (`ovsx`); Cursor mirrors Open VSX, often hours later
- Sideload still works: `install.ps1` / `install.sh`

Do not put PATs in files or git. Use env vars or `vsce login` / `ovsx login`. The name `rosegold` is reserved on the Marketplace if you deleted that listing; do not reuse it.

## 0. Package locally

From `vscode/` (Node on PATH, or `C:\Program Files\nodejs`):

```powershell
npm run package
```

That writes `rosegold-language-<version>.vsix` in this folder (from `package.json` `name`). Fix any `vsce` errors before publishing.

Helper (packages, then publishes only if tokens are in the environment):

```powershell
.\publish.ps1
```

## 1. Visual Studio Marketplace

Publisher **ID must be `allen6297`** (same as `package.json` `publisher`). The ID cannot be changed after create.

1. Open [https://marketplace.visualstudio.com/manage](https://marketplace.visualstudio.com/manage) and sign in with a Microsoft account.
2. **Create publisher**
   - **ID:** `allen6297`
   - **Name:** display name (e.g. `allen6297` or `RoseGold`)
3. If `allen6297` is taken, stop and pick another ID, then change `publisher` in `package.json` to match. Do not publish under someone else’s publisher.

### PAT (local `vsce login` / `vsce publish`)

Still the documented local path. Azure DevOps **global** PATs retire **1 December 2026**. After that, use an org-scoped PAT if Microsoft still allows it for Marketplace, or Entra ID / OIDC in CI (`vsce publish --azure-credential`). Do not build an Azure pipeline unless you want CI publish.

Create a PAT:

1. Azure DevOps org: [https://dev.azure.com](https://dev.azure.com) (create an org if you have none).
2. User settings → **Personal access tokens** → **New Token**.
3. Direct link pattern: `https://dev.azure.com/<org>/_usersSettings/tokens`
4. Token settings:
   - **Organization:** **All accessible organizations** (required for Marketplace today)
   - **Scopes:** Show all scopes → **Marketplace** → **Manage**
5. Copy the token. Do not commit it.

Login and publish from `vscode/`:

```powershell
$env:VSCE_PAT = "<paste once in this shell>"
npx --yes @vscode/vsce login allen6297
npx --yes @vscode/vsce publish
```

`vsce` also reads `VSCE_PAT` if set. `.\publish.ps1` uses that env var and never writes it to disk.

Live listing (after a successful publish):

`https://marketplace.visualstudio.com/items?itemName=allen6297.rosegold-language`

## 2. Open VSX (Cursor)

Namespace **must be `allen6297`** (matches GitHub user `allen6297`, which makes claiming/verifying the namespace easier).

1. Sign in at [https://open-vsx.org](https://open-vsx.org) with GitHub (`allen6297`).
2. Create a token: [https://open-vsx.org/user-settings/tokens](https://open-vsx.org/user-settings/tokens)
3. Create the namespace, then publish from `vscode/`:

```powershell
$env:OVSX_PAT = "<paste once in this shell>"
npx --yes ovsx create-namespace allen6297 -p $env:OVSX_PAT
npx --yes ovsx publish -p $env:OVSX_PAT
```

If the namespace already exists and you own it, skip `create-namespace`. Exclusive / verified ownership: [Managing Namespaces](https://github.com/EclipseFdn/open-vsx.org/wiki/Managing-Namespaces) (GitHub issue template on `EclipseFdn/open-vsx.org`).

Live listing:

`https://open-vsx.org/extension/allen6297/rosegold-language`

Cursor’s extension search can lag **hours** behind Open VSX. Sideload with `install.ps1` if you need it immediately.

## 3. What not to do

- Do not store tokens in `package.json`, scripts, `.env` committed to git, or CI logs.
- Do not guess a publisher ID that belongs to someone else.
- Do not bump `engines.vscode` to “latest”; keep `^1.85.0` so Cursor’s VS Code base can install it.
