/** Shared lookup helpers over catalog.json. Same keys as Script mode. */

const DOCS = require("./catalog.json");

const BY_KEY = new Map(DOCS.map((e) => [e.key, e]));

function lookup(key) {
  return BY_KEY.get(key);
}

function members(mod) {
  const prefix = `${mod}.`;
  return DOCS.filter((e) => e.key.startsWith(prefix) && e.kind === "function");
}

function hooks() {
  return DOCS.filter((e) => e.kind === "hook");
}

function builtins() {
  return DOCS.filter((e) => e.kind === "builtin");
}

function modules() {
  return DOCS.filter((e) => e.kind === "module");
}

function types() {
  return DOCS.filter((e) => e.kind === "type");
}

/** CodeMirror `${name}` / `${}` → VS Code snippet placeholders. */
function toSnippet(template) {
  if (!template) return "";
  let n = 1;
  return template
    .replace(/\$\{\}/g, "$0")
    .replace(/\$\{([A-Za-z_]\w*)\}/g, (_, name) => `\${${n++}:${name}}`);
}

function hoverMarkdown(entry) {
  const parts = [`\`\`\`rosegold\n${entry.signature}\n\`\`\``];
  if (entry.doc) parts.push(entry.doc);
  return parts.join("\n\n");
}

module.exports = {
  DOCS,
  lookup,
  members,
  hooks,
  builtins,
  modules,
  types,
  toSnippet,
  hoverMarkdown,
};
