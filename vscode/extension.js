const vscode = require("vscode");
const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const catalog = require("./catalog");
const scan = require("./scan");

/** @type {vscode.DiagnosticCollection} */
let diagnostics;
/** @type {NodeJS.Timeout | undefined} */
let diagTimer;
/** @type {vscode.StatusBarItem} */
let statusBar;
/** @type {vscode.TestController} */
let testController;

const KEYWORDS = [
  "fn",
  "var",
  "const",
  "struct",
  "class",
  "trait",
  "extends",
  "enum",
  "mod",
  "impl",
  "pub",
  "import",
  "from",
  "as",
  "if",
  "elif",
  "else",
  "while",
  "for",
  "in",
  "match",
  "return",
  "break",
  "continue",
  "pass",
  "self",
  "super",
  "signal",
];
const CONSTANTS = ["true", "false", "none"];
const TYPES = [
  "Int",
  "Float",
  "String",
  "Str",
  "Bool",
  "Void",
  "Array",
  "Map",
  "Option",
  "Result",
  "Vec2",
  "Vec3",
  "Node",
  "Empty",
  "Sprite",
  "Tilemap",
  "Camera",
  "Mesh",
  "Light",
];

const KIND = {
  function: vscode.CompletionItemKind.Function,
  hook: vscode.CompletionItemKind.Function,
  builtin: vscode.CompletionItemKind.Function,
  module: vscode.CompletionItemKind.Module,
  type: vscode.CompletionItemKind.TypeParameter,
};

/**
 * @param {string | undefined} startDir
 * @returns {string | null}
 */
function findRustCli(startDir) {
  if (!startDir) {
    return null;
  }
  let dir = startDir;
  for (let i = 0; i < 12; i++) {
    const candidates = [
      path.join(dir, "target", "release", "rosegold"),
      path.join(dir, "target", "debug", "rosegold"),
      path.join(dir, "crates", "rosegold", "target", "release", "rosegold"),
      path.join(dir, "crates", "rosegold", "target", "debug", "rosegold"),
    ];
    if (process.platform === "win32") {
      candidates.push(
        path.join(dir, "target", "release", "rosegold.exe"),
        path.join(dir, "target", "debug", "rosegold.exe")
      );
    }
    for (const cand of candidates) {
      if (fs.existsSync(cand)) {
        return cand;
      }
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }
  return null;
}

/**
 * @param {string} [hintPath]
 * @returns {{cmd: string, prefix: string[]}}
 */
function cliInvocation(hintPath) {
  const cfg = vscode.workspace.getConfiguration("rosegold");
  const raw = (cfg.get("cliPath", "") || "").trim();
  if (raw && raw !== "rosegold") {
    const parts = raw.split(/\s+/).filter(Boolean);
    return { cmd: parts[0], prefix: parts.slice(1) };
  }
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const fromHint =
    hintPath && path.isAbsolute(hintPath) ? path.dirname(hintPath) : undefined;
  const found = findRustCli(folder) || findRustCli(fromHint);
  if (found) {
    return { cmd: found, prefix: [] };
  }
  return { cmd: "rosegold", prefix: [] };
}

/**
 * @param {string[]} args
 * @param {string} [stdin]
 * @param {string} [hintPath]
 * @returns {Promise<{code: number, stdout: string, stderr: string}>}
 */
function runCli(args, stdin, hintPath) {
  const { cmd, prefix } = cliInvocation(hintPath);
  return new Promise((resolve) => {
    const child = spawn(cmd, [...prefix, ...args], {
      cwd:
        vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ||
        (hintPath ? path.dirname(hintPath) : undefined),
      env: process.env,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => {
      stdout += d.toString();
    });
    child.stderr.on("data", (d) => {
      stderr += d.toString();
    });
    child.on("error", (err) => {
      resolve({
        code: 1,
        stdout: "",
        stderr: `${err.message} (cmd=${cmd}). Build with: cargo build -p rosegold`,
      });
    });
    child.on("close", (code) => {
      resolve({ code: code ?? 1, stdout, stderr });
    });
    if (stdin != null) {
      child.stdin.write(stdin);
    }
    child.stdin.end();
  });
}

async function parseJson(args, stdin, hintPath) {
  const result = await runCli(args, stdin, hintPath);
  const text = result.stdout.trim();
  if (!text) {
    return { ok: false, error: result.stderr.trim() || "empty CLI output" };
  }
  try {
    return JSON.parse(text);
  } catch {
    return { ok: false, error: result.stderr.trim() || "invalid JSON" };
  }
}

/** @param {vscode.TextDocument} doc */
function filePathOf(doc) {
  return doc.uri.scheme === "file" ? doc.uri.fsPath : doc.uri.toString();
}

/** @param {any} r */
function rangeFrom(r) {
  return new vscode.Range(
    r.start.line,
    r.start.character,
    r.end.line,
    r.end.character
  );
}

/** @param {any} loc @param {vscode.Uri} fallback */
function locationFrom(loc, fallback) {
  if (!loc || !loc.range) {
    return null;
  }
  let uri = fallback;
  if (loc.path) {
    if (path.isAbsolute(loc.path) && fs.existsSync(loc.path)) {
      uri = vscode.Uri.file(loc.path);
    } else {
      const fromDoc = path.dirname(fallback.fsPath);
      const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
      const candidates = [
        path.resolve(fromDoc, loc.path),
        folder ? path.resolve(folder, loc.path) : null,
      ].filter(Boolean);
      const hit = candidates.find((p) => p && fs.existsSync(p));
      if (hit) uri = vscode.Uri.file(hit);
    }
  }
  return new vscode.Location(uri, rangeFrom(loc.range));
}

/**
 * Identifier (and optional `mod.`) at the cursor. Range is the use site.
 * @param {vscode.TextDocument} document
 * @param {vscode.Position} position
 */
function identAt(document, position) {
  const line = document.lineAt(position.line).text;
  let i = position.character;
  if (i > 0 && i === line.length) i -= 1;
  if (i < 0 || i >= line.length || !/[A-Za-z0-9_]/.test(line[i])) {
    if (i > 0 && /[A-Za-z0-9_]/.test(line[i - 1])) i -= 1;
    else return null;
  }
  let start = i;
  let end = i + 1;
  while (start > 0 && /[A-Za-z0-9_]/.test(line[start - 1])) start -= 1;
  while (end < line.length && /[A-Za-z0-9_]/.test(line[end])) end += 1;
  const text = line.slice(start, end);
  if (!/^[A-Za-z_]\w*$/.test(text)) return null;
  let mod = null;
  let rangeStart = start;
  if (start > 1 && line[start - 1] === ".") {
    let ms = start - 1;
    while (ms > 0 && /[A-Za-z0-9_]/.test(line[ms - 1])) ms -= 1;
    const m = line.slice(ms, start - 1);
    if (/^[A-Za-z_]\w*$/.test(m)) {
      mod = m;
      rangeStart = ms;
    }
  }
  return {
    text,
    mod,
    range: new vscode.Range(position.line, rangeStart, position.line, end),
  };
}

function catalogHit(tok) {
  if (!tok) return null;
  if (tok.mod) {
    const entry = catalog.lookup(`${tok.mod}.${tok.text}`);
    if (entry) return entry;
  }
  return catalog.lookup(tok.text) || null;
}

function hoverFromEntry(entry, range) {
  const md = new vscode.MarkdownString(catalog.hoverMarkdown(entry), true);
  return new vscode.Hover(md, range);
}

function skipStringBack(text, i) {
  const q = text[i];
  if (q !== '"' && q !== "'") return i;
  i -= 1;
  while (i >= 0) {
    if (text[i] === q && (i === 0 || text[i - 1] !== "\\")) return i - 1;
    i -= 1;
  }
  return i;
}

/**
 * Catalog call site at `position` (signature help). Same walk as Script mode.
 * @param {vscode.TextDocument} document
 * @param {vscode.Position} position
 */
function callSiteAt(document, position) {
  const offset = document.offsetAt(position);
  const start = Math.max(0, offset - 400);
  const slice = document.getText(
    new vscode.Range(document.positionAt(start), position)
  );
  let depth = 0;
  let argIndex = 0;
  for (let i = slice.length - 1; i >= 0; i -= 1) {
    const ch = slice[i];
    if (ch === '"' || ch === "'") {
      i = skipStringBack(slice, i);
      continue;
    }
    if (ch === ")" || ch === "}" || ch === "]") {
      depth += 1;
      continue;
    }
    if (ch === "(" || ch === "{" || ch === "[") {
      if (depth > 0) {
        depth -= 1;
        continue;
      }
      if (ch !== "(") return null;
      let end = i;
      while (end > 0 && /\s/.test(slice[end - 1])) end -= 1;
      let nameStart = end;
      while (nameStart > 0 && /[A-Za-z0-9_]/.test(slice[nameStart - 1])) {
        nameStart -= 1;
      }
      const name = slice.slice(nameStart, end);
      if (!/^[A-Za-z_]\w*$/.test(name)) return null;
      let key = name;
      if (nameStart > 0 && slice[nameStart - 1] === ".") {
        let modEnd = nameStart - 1;
        let modStart = modEnd;
        while (modStart > 0 && /[A-Za-z0-9_]/.test(slice[modStart - 1])) {
          modStart -= 1;
        }
        const mod = slice.slice(modStart, modEnd);
        if (/^[A-Za-z_]\w*$/.test(mod)) key = `${mod}.${name}`;
      }
      const entry = catalog.lookup(key);
      if (!entry || !entry.params || !entry.params.length) return null;
      return { entry, argIndex };
    }
    if (ch === "," && depth === 0) argIndex += 1;
  }
  return null;
}

function completionFromEntry(entry, { member } = {}) {
  const item = new vscode.CompletionItem(
    entry.label,
    KIND[entry.kind] || vscode.CompletionItemKind.Function
  );
  item.detail = entry.detail;
  item.documentation = new vscode.MarkdownString(
    catalog.hoverMarkdown(entry),
    true
  );
  if (entry.boost) item.sortText = String(100 - entry.boost).padStart(3, "0");
  const tpl = member
    ? entry.template
    : entry.kind === "hook" || entry.kind === "builtin"
      ? entry.template
      : entry.key.includes(".")
        ? `${entry.key.split(".")[0]}.${entry.template}`
        : entry.template;
  if (tpl) item.insertText = new vscode.SnippetString(catalog.toSnippet(tpl));
  return item;
}

function memberCompletions(mod) {
  return catalog
    .members(mod)
    .filter((e) => e.template)
    .map((e) => completionFromEntry(e, { member: true }));
}

function nameItem(label, kind, detail) {
  const item = new vscode.CompletionItem(label, kind);
  item.detail = detail;
  return item;
}

function emitAfterDot() {
  const item = new vscode.CompletionItem(
    "emit",
    vscode.CompletionItemKind.Function
  );
  item.detail = "signal";
  item.documentation = new vscode.MarkdownString(
    "```rosegold\nsignal.emit(args…)\n```\n\nFire this signal. Inspector connections run the target method.",
    true
  );
  item.insertText = new vscode.SnippetString("emit(${0})");
  return item;
}

function scanOpenFiles(document) {
  const seen = new Set();
  const files = [];
  const push = (doc) => {
    if (seen.has(doc.uri.toString())) return;
    seen.add(doc.uri.toString());
    files.push(scan.scanSource(doc.getText()));
  };
  push(document);
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === "rosegold") push(doc);
  }
  return files;
}

function afterDotCompletions(mod, document, position) {
  const files = scanOpenFiles(document);
  const file = files[0];
  const offset = document.offsetAt(position);
  const hit = scan.membersFor(file, files, offset, mod);
  if (hit.kind === "list") {
    return hit.items.map((item) => {
      if (item.role === "emit") return emitAfterDot();
      const kind =
        item.role === "field"
          ? vscode.CompletionItemKind.Field
          : item.role === "signal"
            ? vscode.CompletionItemKind.Event
            : vscode.CompletionItemKind.Method;
      return nameItem(item.name, kind, item.role === "super" ? "super" : item.role);
    });
  }
  const items = memberCompletions(mod);
  if (items.length) return items;
  return [emitAfterDot()];
}

function localCompletions(document) {
  const items = [];
  const seen = new Set();
  const file = scan.scanSource(document.getText());
  const kindOf = {
    fn: vscode.CompletionItemKind.Function,
    var: vscode.CompletionItemKind.Variable,
    const: vscode.CompletionItemKind.Constant,
    struct: vscode.CompletionItemKind.Struct,
    enum: vscode.CompletionItemKind.Enum,
    class: vscode.CompletionItemKind.Class,
    trait: vscode.CompletionItemKind.Interface,
    mod: vscode.CompletionItemKind.Module,
    signal: vscode.CompletionItemKind.Event,
  };
  for (const sym of file.symbols) {
    if (seen.has(sym.name)) continue;
    seen.add(sym.name);
    const item = new vscode.CompletionItem(
      sym.name,
      kindOf[sym.kind] || vscode.CompletionItemKind.Variable,
    );
    item.detail = sym.kind === "fn" ? "in file" : sym.kind;
    items.push(item);
  }
  return items;
}

function extraSnippets() {
  const extra = [
    {
      label: "@export",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "inspector property",
      insert: "@export var ${1:name}: ${2:Float} = ${3:0.0};",
    },
    {
      label: "@export_group",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "inspector group",
      insert: '@export_group("${1:Name}")',
    },
    {
      label: "@test",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "test function",
      insert: "@test\nfn ${1:test_name}() {\n\t$0\n}",
    },
    {
      label: "@ufcs",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "call as a method",
      insert: "@ufcs\nfn ${1:name}(${2:n}: ${3:Float}): ${4:Float} {\n\t$0\n}",
    },
    {
      label: "@node",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "scene node class",
      insert:
        "@node\nclass ${1:MyNode} extends ${2:Sprite} {\n\tfn on_create() {\n\t\t$0\n\t}\n\tfn on_update(dt: Float) {\n\t\tpass;\n\t}\n\tfn on_destroy() {\n\t\tpass;\n\t}\n}",
    },
    {
      label: ".emit",
      kind: vscode.CompletionItemKind.Function,
      detail: "fire signal",
      insert: "${1:name}.emit(${0});",
    },
    {
      label: "signal",
      kind: vscode.CompletionItemKind.Keyword,
      detail: "declare signal",
      insert: "signal ${1:name}(${2:args});",
    },
  ];
  for (const key of [
    "strata.move",
    "strata.after",
    "strata.find",
    "input.pressed",
    "input.held",
  ]) {
    const e = catalog.lookup(key);
    if (!e || !e.template) continue;
    extra.push({
      label: key,
      kind: vscode.CompletionItemKind.Function,
      detail: e.detail,
      insert: catalog.toSnippet(`${key.slice(0, key.indexOf("."))}.${e.template}`),
    });
  }
  return extra.map((s) => {
    const item = new vscode.CompletionItem(s.label, s.kind);
    item.detail = s.detail;
    item.insertText = new vscode.SnippetString(s.insert);
    return item;
  });
}

function provideCompletions(document, position) {
  const line = document.lineAt(position.line).text.slice(0, position.character);
  const member = line.match(/([A-Za-z_]\w*)\.\w*$/);
  if (member) {
    return afterDotCompletions(member[1], document, position);
  }
  if (/@\w*$/.test(line)) {
    const start = line.lastIndexOf("@");
    const range = new vscode.Range(
      position.line,
      start,
      position.line,
      position.character,
    );
    return extraSnippets()
      .filter((i) => i.label.startsWith("@"))
      .map((i) => {
        i.range = range;
        return i;
      });
  }

  const files = scanOpenFiles(document);
  const file = files[0];
  const enclosing = scan.classAt(file, document.offsetAt(position));

  const extendsM = line.match(/\bextends\s+([A-Za-z_]*)$/);
  if (extendsM) {
    const local = scan.allClasses(files).filter((n) => n !== enclosing?.name);
    return [
      ...scan.NODE_BASES.map((n) => {
        const e = catalog.lookup(n);
        return e ? completionFromEntry(e) : nameItem(n, vscode.CompletionItemKind.Class, "node");
      }),
      ...local.map((n) => nameItem(n, vscode.CompletionItemKind.Class, "class")),
    ];
  }
  const implFor = line.match(/\bimpl\s+[A-Za-z_]\w*\s+for\s+([A-Za-z_]*)$/);
  if (implFor) {
    return [
      ...scan.NODE_BASES.map((n) => nameItem(n, vscode.CompletionItemKind.Class, "node")),
      ...scan.allClasses(files).map((n) => nameItem(n, vscode.CompletionItemKind.Class, "class")),
    ];
  }
  const implM = line.match(/\bimpl\s+([A-Za-z_]*)$/);
  if (implM) {
    return scan
      .allTraits(files)
      .map((n) => nameItem(n, vscode.CompletionItemKind.Interface, "trait"));
  }

  const items = [];
  for (const kw of KEYWORDS) {
    items.push(new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword));
  }
  for (const c of CONSTANTS) {
    items.push(new vscode.CompletionItem(c, vscode.CompletionItemKind.Constant));
  }
  const catalogTypeKeys = new Set(catalog.types().map((e) => e.key));
  for (const t of TYPES) {
    if (catalogTypeKeys.has(t)) continue;
    items.push(new vscode.CompletionItem(t, vscode.CompletionItemKind.TypeParameter));
  }
  for (const e of catalog.types()) {
    items.push(completionFromEntry(e));
  }
  if (enclosing?.isNode) {
    for (const [label, insert] of [
      ["on_create", "fn on_create() {\n\t$0\n}"],
      ["on_update", "fn on_update(dt: Float) {\n\t$0\n}"],
      ["on_destroy", "fn on_destroy() {\n\t$0\n}"],
      ["on_enter", "fn on_enter(other: Str) {\n\t$0\n}"],
      ["on_exit", "fn on_exit(other: Str) {\n\t$0\n}"],
    ]) {
      const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.Function);
      item.detail = "@node hook";
      item.insertText = new vscode.SnippetString(insert);
      items.push(item);
    }
  } else {
    for (const e of catalog.hooks()) {
      items.push(completionFromEntry(e));
    }
  }
  for (const e of catalog.builtins()) {
    items.push(completionFromEntry(e));
  }
  for (const e of catalog.modules()) {
    items.push(completionFromEntry(e));
  }
  items.push(...extraSnippets());
  items.push(...localCompletions(document));
  if (enclosing) {
    const mem = scan.classMembers(enclosing, files);
    for (const n of mem.fields) {
      items.push(nameItem(n, vscode.CompletionItemKind.Field, "field"));
    }
    for (const n of mem.methods) {
      items.push(nameItem(n, vscode.CompletionItemKind.Method, "method"));
    }
    for (const n of mem.signals) {
      items.push(nameItem(n, vscode.CompletionItemKind.Event, "signal"));
    }
  }
  return items;
}

/** Underline the token at a 0-based line/col (CLI reports start only). */
function diagnosticRange(doc, line0, col0) {
  const maxLine = Math.max(0, doc.lineCount - 1);
  const line = doc.lineAt(Math.min(Math.max(line0, 0), maxLine));
  const text = line.text;
  let start = Math.min(Math.max(col0, 0), text.length);
  let end = start;
  if (start < text.length && /[A-Za-z0-9_]/.test(text[start])) {
    while (end < text.length && /[A-Za-z0-9_]/.test(text[end])) end += 1;
  } else if (start < text.length) {
    end = start + 1;
  } else if (text.length) {
    start = text.length - 1;
    end = text.length;
  } else {
    end = start + 1;
  }
  return new vscode.Range(line.lineNumber, start, line.lineNumber, end);
}

/** @param {vscode.TextDocument} doc */
async function refreshDiagnostics(doc) {
  if (doc.languageId !== "rosegold") {
    return;
  }
  const filePath = filePathOf(doc);
  const payload = await parseJson(
    ["check", "--json", "--stdin", filePath],
    doc.getText(),
    filePath
  );
  if (!Array.isArray(payload)) {
    if (statusBar) {
      statusBar.text = "$(error) RoseGold";
      statusBar.tooltip =
        (payload && payload.error) ||
        "check failed — cargo build -p rosegold, or set RoseGold: Cli Path";
    }
    return;
  }
  const items = payload.map((d) => {
    const line = Math.max(0, (d.line || 1) - 1);
    const col = Math.max(0, (d.col || 1) - 1);
    const sev =
      String(d.severity || "").toLowerCase() === "warning"
        ? vscode.DiagnosticSeverity.Warning
        : vscode.DiagnosticSeverity.Error;
    const diag = new vscode.Diagnostic(
      diagnosticRange(doc, line, col),
      d.message || "error",
      sev
    );
    diag.source = "rosegold";
    return diag;
  });
  diagnostics.set(doc.uri, items);
  if (statusBar) {
    const errs = items.filter(
      (i) => i.severity === vscode.DiagnosticSeverity.Error
    ).length;
    const warns = items.length - errs;
    if (errs) {
      statusBar.text = `$(error) RoseGold ${errs}`;
      statusBar.tooltip = `${errs} error(s), ${warns} warning(s)`;
    } else if (warns) {
      statusBar.text = `$(warning) RoseGold ${warns}`;
      statusBar.tooltip = `${warns} warning(s)`;
    } else {
      statusBar.text = "$(check) RoseGold";
      statusBar.tooltip = "No problems";
    }
  }
}

/** @param {vscode.TextDocument} doc */
function scheduleDiagnostics(doc) {
  if (doc.languageId !== "rosegold") {
    return;
  }
  clearTimeout(diagTimer);
  const delay = vscode.workspace
    .getConfiguration("rosegold")
    .get("diagnosticsDelayMs", 400);
  diagTimer = setTimeout(() => refreshDiagnostics(doc), delay);
}

function sendToTerminal(shellCmd) {
  const terminal =
    vscode.window.terminals.find((t) => t.name === "RoseGold") ||
    vscode.window.createTerminal("RoseGold");
  terminal.show();
  terminal.sendText(shellCmd);
}

function quote(p) {
  return JSON.stringify(p);
}

/** @param {string} source */
function scanTests(source) {
  const items = [];
  const re = /^[ \t]*@test[ \t]*\n[ \t]*(?:pub[ \t]+)?fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm;
  let m;
  while ((m = re.exec(source))) {
    const line = source.slice(0, m.index).split("\n").length - 1;
    items.push({
      name: m[1],
      range: new vscode.Range(line, 0, line, m[0].length),
    });
  }
  return items;
}

/** @param {vscode.TextDocument} doc */
function documentSymbols(doc) {
  const text = doc.getText();
  const symbols = [];
  const patterns = [
    { re: /^[ \t]*(?:pub[ \t]+)?fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Function },
    { re: /^[ \t]*(?:pub[ \t]+)?struct[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Struct },
    { re: /^[ \t]*(?:pub[ \t]+)?class[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Class },
    { re: /^[ \t]*(?:pub[ \t]+)?trait[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Interface },
    { re: /^[ \t]*(?:pub[ \t]+)?impl[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Class },
    { re: /^[ \t]*(?:pub[ \t]+)?enum[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Enum },
    { re: /^[ \t]*(?:pub[ \t]+)?signal[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Event },
    { re: /^[ \t]*(?:pub[ \t]+)?mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Namespace },
    { re: /^[ \t]*(?:pub[ \t]+)?const[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Constant },
    { re: /^[ \t]*@export(?:_group)?[ \t]+var[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Property },
    { re: /^[ \t]*var[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, kind: vscode.SymbolKind.Variable },
  ];
  for (const { re, kind } of patterns) {
    let m;
    while ((m = re.exec(text))) {
      const pos = doc.positionAt(m.index);
      const nameRange = new vscode.Range(
        pos.line,
        m[0].indexOf(m[1]),
        pos.line,
        m[0].indexOf(m[1]) + m[1].length
      );
      const lineText = doc.lineAt(pos.line).text;
      const full = new vscode.Range(pos.line, 0, pos.line, lineText.length);
      let detail = "";
      if (kind === vscode.SymbolKind.Class && pos.line > 0) {
        const prev = doc.lineAt(pos.line - 1).text.trim();
        if (prev === "@node") detail = "@node";
      }
      symbols.push(new vscode.DocumentSymbol(m[1], detail, kind, full, nameRange));
    }
  }
  return symbols;
}

/**
 * @param {vscode.ExtensionContext} context
 */
async function setupTestExplorer(context) {
  testController = vscode.tests.createTestController("rosegoldTests", "RoseGold");
  context.subscriptions.push(testController);

  const runProfile = testController.createRunProfile(
    "Run",
    vscode.TestRunProfileKind.Run,
    async (request, token) => {
      const run = testController.createTestRun(request);
      const queue = request.include
        ? [...request.include]
        : [...testController.items].map(([, item]) => item);
      for (const item of queue) {
        if (token.isCancellationRequested) break;
        run.started(item);
        const file = item.uri?.fsPath;
        if (!file) {
          run.skipped(item);
          continue;
        }
        const result = await runCli(["test", file], undefined, file);
        if (result.code === 0) {
          run.passed(item);
        } else {
          run.failed(
            item,
            new vscode.TestMessage(result.stdout || result.stderr || "failed")
          );
        }
      }
      run.end();
    },
    true
  );
  context.subscriptions.push(runProfile);

  async function refreshTests() {
    testController.items.replace([]);
    const files = await vscode.workspace.findFiles("**/*.rg", "**/target/**");
    for (const uri of files) {
      let text = "";
      try {
        text = fs.readFileSync(uri.fsPath, "utf8");
      } catch {
        continue;
      }
      const tests = scanTests(text);
      if (!tests.length) continue;
      const parent = testController.createTestItem(
        uri.fsPath,
        vscode.workspace.asRelativePath(uri),
        uri
      );
      testController.items.add(parent);
      for (const t of tests) {
        const child = testController.createTestItem(
          `${uri.fsPath}::${t.name}`,
          t.name,
          uri
        );
        child.range = t.range;
        parent.children.add(child);
      }
    }
  }

  testController.refreshHandler = refreshTests;
  await refreshTests();
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.languageId === "rosegold") refreshTests();
    })
  );
}

function activate(context) {
  diagnostics = vscode.languages.createDiagnosticCollection("rosegold");
  context.subscriptions.push(diagnostics);

  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  statusBar.text = "$(loading~spin) RoseGold";
  statusBar.tooltip = "RoseGold language status";
  statusBar.command = "rosegold.showStatus";
  statusBar.show();
  context.subscriptions.push(statusBar);

  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((e) => scheduleDiagnostics(e.document)),
    vscode.workspace.onDidOpenTextDocument((doc) => refreshDiagnostics(doc)),
    vscode.workspace.onDidSaveTextDocument((doc) => refreshDiagnostics(doc)),
    vscode.workspace.onDidCloseTextDocument((doc) => diagnostics.delete(doc.uri))
  );
  for (const doc of vscode.workspace.textDocuments) {
    scheduleDiagnostics(doc);
  }

  context.subscriptions.push(
    vscode.languages.registerDefinitionProvider("rosegold", {
      async provideDefinition(document, position) {
        const filePath = filePathOf(document);
        const payload = await parseJson(
          [
            "def",
            "--json",
            "--stdin",
            filePath,
            String(position.line + 1),
            String(position.character + 1),
          ],
          document.getText(),
          filePath
        );
        return locationFrom(payload?.definition, document.uri);
      },
    })
  );

  context.subscriptions.push(
    vscode.languages.registerHoverProvider("rosegold", {
      async provideHover(document, position) {
        const tok = identAt(document, position);
        const entry = catalogHit(tok);
        if (entry) return hoverFromEntry(entry, tok.range);
        if (!tok) return null;
        const filePath = filePathOf(document);
        const payload = await parseJson(
          [
            "hover",
            "--json",
            "--stdin",
            filePath,
            String(position.line + 1),
            String(position.character + 1),
          ],
          document.getText(),
          filePath
        );
        const info = payload?.hover;
        if (!info || !info.contents) return null;
        return new vscode.Hover(
          new vscode.MarkdownString(info.contents, true),
          tok.range
        );
      },
    })
  );

  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider("rosegold", {
      async provideDocumentFormattingEdits(document) {
        const filePath = filePathOf(document);
        const result = await runCli(
          ["fmt", "--stdin", filePath],
          document.getText(),
          filePath
        );
        if (result.code !== 0) {
          return [];
        }
        const full = new vscode.Range(
          document.positionAt(0),
          document.positionAt(document.getText().length)
        );
        return [vscode.TextEdit.replace(full, result.stdout)];
      },
    })
  );

  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      "rosegold",
      {
        provideCompletionItems(document, position) {
          return provideCompletions(document, position);
        },
      },
      ".",
      "@"
    )
  );

  context.subscriptions.push(
    vscode.languages.registerSignatureHelpProvider(
      "rosegold",
      {
        provideSignatureHelp(document, position) {
          const site = callSiteAt(document, position);
          if (!site) return null;
          const { entry, argIndex } = site;
          const sig = new vscode.SignatureInformation(
            entry.signature,
            new vscode.MarkdownString(entry.doc, true)
          );
          sig.parameters = (entry.params || []).map(
            (p) => new vscode.ParameterInformation(p)
          );
          sig.activeParameter = Math.min(argIndex, Math.max(0, sig.parameters.length - 1));
          const help = new vscode.SignatureHelp();
          help.signatures = [sig];
          help.activeSignature = 0;
          help.activeParameter = sig.activeParameter;
          return help;
        },
      },
      "(",
      ","
    )
  );

  context.subscriptions.push(
    vscode.languages.registerDocumentSymbolProvider("rosegold", {
      provideDocumentSymbols(document) {
        return documentSymbols(document);
      },
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("rosegold.runFile", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "rosegold") {
        vscode.window.showWarningMessage("Open a .rg file to run.");
        return;
      }
      await editor.document.save();
      const file = editor.document.uri.fsPath;
      const { cmd, prefix } = cliInvocation(file);
      sendToTerminal([quote(cmd), ...prefix, "run", quote(file)].join(" "));
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("rosegold.testFile", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "rosegold") {
        vscode.window.showWarningMessage("Open a .rg file to run @test functions.");
        return;
      }
      await editor.document.save();
      const file = editor.document.uri.fsPath;
      const { cmd, prefix } = cliInvocation(file);
      sendToTerminal([quote(cmd), ...prefix, "test", quote(file)].join(" "));
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("rosegold.testFolder", async (uri) => {
      let target = uri?.fsPath;
      if (!target) target = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
      if (!target) {
        vscode.window.showWarningMessage("No folder to test.");
        return;
      }
      const files = fs.existsSync(target) && fs.statSync(target).isDirectory()
        ? fs.readdirSync(target).filter((f) => f.endsWith(".rg")).map((f) => path.join(target, f))
        : [target];
      const { cmd, prefix } = cliInvocation(target);
      const cmds = files
        .map((f) => [quote(cmd), ...prefix, "test", quote(f)].join(" "))
        .join(" && ");
      sendToTerminal(cmds || [quote(cmd), ...prefix, "test", quote(target)].join(" "));
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("rosegold.showStatus", async () => {
      const { cmd, prefix } = cliInvocation(
        vscode.workspace.workspaceFolders?.[0]?.uri.fsPath
      );
      const inv = [cmd, ...prefix].join(" ");
      vscode.window.showInformationMessage(`RoseGold CLI: ${inv}`);
    })
  );

  setupTestExplorer(context).catch(() => {});
}

function deactivate() {
  clearTimeout(diagTimer);
}

module.exports = { activate, deactivate };
