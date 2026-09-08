/** Fail if catalog.json is missing public host / ui API keys.
 *  scan.js is outline-only; catalog.json is the docs source of truth.
 */
const fs = require("fs");
const path = require("path");

const vscodeDir = __dirname;
const repoRoot = path.resolve(vscodeDir, "..");
const catalog = require("./catalog.json");
const keys = new Set(catalog.map((e) => e.key));

const UI_REQUIRED = [
  "ui",
  "ui.theme",
  "ui.alert",
  "ui.window",
  "ui.column",
  "ui.row",
  "ui.scroll",
  "ui.text",
  "ui.field",
  "ui.button",
  "ui.checkbox",
  "ui.slider",
  "ui.select",
  "ui.progress",
  "ui.open",
  "ui.save",
  "ui.separator",
  "ui.spacer",
  "ui.quit",
  "ui.invalidate",
  "ui.run",
];

const HOST_MODULES = new Set([
  "io",
  "time",
  "process",
  "json",
  "path",
  "http",
  "regex",
  "Array",
]);

function publicHostApis(dispatchSrc) {
  const re = /\("([A-Za-z_][A-Za-z0-9_]*)", "([A-Za-z_][A-Za-z0-9_]*)"\)/g;
  const out = [];
  let m;
  while ((m = re.exec(dispatchSrc))) {
    if (!HOST_MODULES.has(m[1])) continue;
    out.push(`${m[1]}.${m[2]}`);
  }
  return unique(out);
}

function pubFns(src, moduleName) {
  const re = /pub fn ([A-Za-z_]\w*)/g;
  const out = [];
  let m;
  while ((m = re.exec(src))) {
    if (m[1].endsWith("_float")) continue;
    out.push(`${moduleName}.${m[1]}`);
  }
  return out;
}

function widgetMethods(uiSrc) {
  const block = uiSrc.match(/pub class Widget \{([\s\S]*?)\n    \}/);
  if (!block) return [];
  const re = /fn ([A-Za-z_]\w*)\(/g;
  const out = [];
  let m;
  while ((m = re.exec(block[1]))) {
    out.push(`Widget.${m[1]}`);
  }
  return out;
}

function unique(items) {
  return [...new Set(items)].sort();
}

function check() {
  const dispatch = fs.readFileSync(
    path.join(repoRoot, "src", "interpreter", "dispatch.rs"),
    "utf8",
  );
  const uiSrc = fs.readFileSync(path.join(repoRoot, "stdlib", "ui.rg"), "utf8");
  const strSrc = fs.readFileSync(path.join(repoRoot, "stdlib", "str.rg"), "utf8");
  const mathSrc = fs.readFileSync(path.join(repoRoot, "stdlib", "math.rg"), "utf8");
  const checksSrc = fs.readFileSync(
    path.join(repoRoot, "stdlib", "checks.rg"),
    "utf8",
  );

  const required = unique([
    ...publicHostApis(dispatch),
    ...pubFns(uiSrc, "ui"),
    ...widgetMethods(uiSrc),
    ...pubFns(strSrc, "str"),
    ...pubFns(mathSrc, "math"),
    ...pubFns(checksSrc, "checks"),
    ...UI_REQUIRED,
    "ui",
    "io",
    "time",
    "process",
    "json",
    "path",
    "http",
    "regex",
  ]);

  return {
    required,
    missing: required.filter((k) => !keys.has(k)),
    catalogSize: keys.size,
  };
}

if (require.main === module) {
  const result = check();
  if (result.missing.length) {
    console.error("catalog.json missing keys:\n  " + result.missing.join("\n  "));
    process.exit(1);
  }
  console.log(
    `catalog.json OK (${result.required.length} host/stdlib/ui keys checked, ${result.catalogSize} entries)`,
  );
}

module.exports = { check, UI_REQUIRED };
