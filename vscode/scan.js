/** Regex/brace scan of a .rg buffer. Keep in lockstep with src/lib/rosegold-scan.ts. */

const IDENT = "[A-Za-z_]\\w*";

const NODE_BASES = ["Node", "Empty", "Sprite", "Tilemap", "Camera", "Mesh", "Light"];
const NODE_FIELDS = ["name", "x", "y", "z"];
const NODE_METHODS = ["on_create", "on_update", "on_destroy", "on_enter", "on_exit"];

function maskNoise(src) {
  let out = "";
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === "#") {
      while (i < src.length && src[i] !== "\n") {
        out += " ";
        i += 1;
      }
      continue;
    }
    if (c === '"' || c === "'") {
      const q = c;
      out += " ";
      i += 1;
      while (i < src.length && src[i] !== q && src[i] !== "\n") {
        if (src[i] === "\\" && i + 1 < src.length) {
          out += "  ";
          i += 2;
          continue;
        }
        out += " ";
        i += 1;
      }
      if (i < src.length) {
        out += " ";
        i += 1;
      }
      continue;
    }
    out += c;
    i += 1;
  }
  return out;
}

function findBody(masked, from) {
  const start = masked.indexOf("{", from);
  if (start < 0) return null;
  let depth = 0;
  for (let i = start; i < masked.length; i += 1) {
    if (masked[i] === "{") depth += 1;
    else if (masked[i] === "}") {
      depth -= 1;
      if (depth === 0) return { start, end: i };
    }
  }
  return { start, end: masked.length };
}

function prevNonemptyLine(src, index) {
  let i = index;
  while (i > 0 && src[i - 1] !== "\n") i -= 1;
  let end = i;
  while (end > 0 && src[end - 1] === "\n") end -= 1;
  let start = end;
  while (start > 0 && src[start - 1] !== "\n") start -= 1;
  return src.slice(start, end).trim();
}

function membersIn(masked, start, end, re) {
  const slice = masked.slice(start, end);
  const out = [];
  re.lastIndex = 0;
  let m;
  while ((m = re.exec(slice))) {
    const name = m[1];
    const from = start + m.index + m[0].length - name.length;
    out.push({ name, from });
  }
  return out;
}

function parseImpls(header) {
  const impl = header.match(/\bimpl\s+([A-Za-z_][\w\s,]*)/);
  if (!impl) return [];
  return impl[1]
    .split(",")
    .map((s) => s.trim())
    .filter((n) => /^[A-Za-z_]\w*$/.test(n));
}

function parseExtends(header) {
  const m = header.match(/\bextends\s+([A-Za-z_]\w*)/);
  return m ? m[1] : null;
}

function parseNestedImpls(masked, start, end) {
  const slice = masked.slice(start, end);
  const re =
    /(?:^|[^A-Za-z0-9_])impl\s+([A-Za-z_]\w*)(\s+for\s+[A-Za-z_]\w*)?\s*\{/g;
  const out = [];
  let m;
  while ((m = re.exec(slice))) {
    if (m[2]) continue;
    out.push(m[1]);
  }
  return out;
}

function scanTypedLocals(masked) {
  const out = [];
  const seen = new Set();
  const push = (name, typeName) => {
    if (seen.has(name)) return;
    seen.add(name);
    out.push({ name, typeName });
  };
  const annotated =
    /(?:^|[^A-Za-z0-9_])(?:pub\s+)?(?:var|const)\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_]\w*)/g;
  let m;
  while ((m = annotated.exec(masked))) {
    push(m[1], m[2]);
  }
  const inferred =
    /(?:^|[^A-Za-z0-9_])(?:pub\s+)?(?:var|const)\s+([A-Za-z_]\w*)\s*=\s*([A-Za-z_]\w*)\s*\{/g;
  while ((m = inferred.exec(masked))) {
    push(m[1], m[2]);
  }
  return out;
}

function insideAny(pos, ranges) {
  return ranges.some((r) => pos > r.bodyFrom && pos < r.bodyTo);
}

function scanSource(src) {
  const masked = maskNoise(src);
  const classes = [];
  const traits = [];

  const classRe = new RegExp(`(?:^|[^A-Za-z0-9_])(?:pub\\s+)?class\\s+(${IDENT})`, "g");
  let cm;
  while ((cm = classRe.exec(masked))) {
    const name = cm[1];
    const nameFrom = cm.index + cm[0].length - name.length;
    const body = findBody(masked, nameFrom);
    if (!body) continue;
    const header = masked.slice(nameFrom + name.length, body.start);
    const extendsName = parseExtends(header);
    const methods = membersIn(
      masked,
      body.start,
      body.end,
      /(?:^|[^A-Za-z0-9_])(?:pub\s+)?fn\s+([A-Za-z_]\w*)/g,
    );
    const methodBodies = methods
      .map((m) => findBody(masked, m.from))
      .filter(Boolean)
      .map((b) => ({ bodyFrom: b.start, bodyTo: b.end }));
    const fields = membersIn(
      masked,
      body.start,
      body.end,
      /(?:^|[^A-Za-z0-9_])(?:@export(?:_group\s*\([^)]*\))?\s+)?(?:pub\s+)?var\s+([A-Za-z_]\w*)/g,
    ).filter((f) => !insideAny(f.from, methodBodies));
    classes.push({
      name,
      from: nameFrom,
      bodyFrom: body.start,
      bodyTo: body.end,
      extendsName,
      impls: unique([
        ...parseImpls(header),
        ...parseNestedImpls(masked, body.start, body.end),
      ]),
      isNode:
        prevNonemptyLine(masked, nameFrom) === "@node" ||
        (extendsName != null && NODE_BASES.includes(extendsName)),
      fields,
      methods,
      signals: membersIn(
        masked,
        body.start,
        body.end,
        /(?:^|[^A-Za-z0-9_])(?:pub\s+)?signal\s+([A-Za-z_]\w*)/g,
      ),
    });
  }

  const traitRe = new RegExp(`(?:^|[^A-Za-z0-9_])(?:pub\\s+)?trait\\s+(${IDENT})`, "g");
  let tm;
  while ((tm = traitRe.exec(masked))) {
    const name = tm[1];
    const nameFrom = tm.index + tm[0].length - name.length;
    const body = findBody(masked, nameFrom);
    if (!body) continue;
    traits.push({
      name,
      from: nameFrom,
      bodyFrom: body.start,
      bodyTo: body.end,
      methods: membersIn(
        masked,
        body.start,
        body.end,
        /(?:^|[^A-Za-z0-9_])(?:pub\s+)?fn\s+([A-Za-z_]\w*)/g,
      ),
      signals: membersIn(
        masked,
        body.start,
        body.end,
        /(?:^|[^A-Za-z0-9_])(?:pub\s+)?signal\s+([A-Za-z_]\w*)/g,
      ),
    });
  }

  const nested = [...classes, ...traits];
  const top = (kind, re) => {
    const out = [];
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(masked))) {
      const name = m[1];
      const from = m.index + m[0].length - name.length;
      if (insideAny(from, nested)) continue;
      out.push({ name, kind, from, to: from + name.length });
    }
    return out;
  };

  const symbols = [
    ...top("fn", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?fn\s+([A-Za-z_]\w*)/g),
    ...top(
      "var",
      /(?:^|[^A-Za-z0-9_])(?:@export(?:_group\s*\([^)]*\))?\s+)?(?:pub\s+)?var\s+([A-Za-z_]\w*)/g,
    ),
    ...top("const", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?const\s+([A-Za-z_]\w*)/g),
    ...top("struct", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?struct\s+([A-Za-z_]\w*)/g),
    ...top("enum", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?enum\s+([A-Za-z_]\w*)/g),
    ...top("mod", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?mod\s+([A-Za-z_]\w*)/g),
    ...top("signal", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?signal\s+([A-Za-z_]\w*)/g),
    ...classes.map((c) => ({
      name: c.name,
      kind: "class",
      from: c.from,
      to: c.from + c.name.length,
    })),
    ...traits.map((t) => ({
      name: t.name,
      kind: "trait",
      from: t.from,
      to: t.from + t.name.length,
    })),
  ];
  symbols.sort((a, b) => a.from - b.from);

  const signals = [
    ...top("signal", /(?:^|[^A-Za-z0-9_])(?:pub\s+)?signal\s+([A-Za-z_]\w*)/g).map((s) => ({
      name: s.name,
      from: s.from,
    })),
    ...classes.flatMap((c) => c.signals),
    ...traits.flatMap((t) => t.signals),
  ];

  return { symbols, classes, traits, signals, typedLocals: scanTypedLocals(masked) };
}

function classAt(file, pos) {
  let hit = null;
  for (const c of file.classes) {
    if (pos >= c.from && pos <= c.bodyTo) hit = c;
  }
  return hit;
}

function isNodeBase(name) {
  return NODE_BASES.includes(name);
}

function findClass(files, name) {
  for (const file of files) {
    const hit = file.classes.find((c) => c.name === name);
    if (hit) return hit;
  }
  return undefined;
}

function findTrait(files, name) {
  for (const file of files) {
    const hit = file.traits.find((t) => t.name === name);
    if (hit) return hit;
  }
  return undefined;
}

function unique(items) {
  const seen = new Set();
  const out = [];
  for (const n of items) {
    if (seen.has(n)) continue;
    seen.add(n);
    out.push(n);
  }
  return out;
}

function classMembers(cls, files, seen = new Set()) {
  if (seen.has(cls.name)) return { fields: [], methods: [], signals: [] };
  seen.add(cls.name);
  const fields = cls.fields.map((f) => f.name);
  const methods = cls.methods.map((m) => m.name);
  const signals = cls.signals.map((s) => s.name);
  for (const traitName of cls.impls) {
    const trait = findTrait(files, traitName);
    if (!trait) continue;
    for (const m of trait.methods) methods.push(m.name);
    for (const s of trait.signals) signals.push(s.name);
  }
  if (cls.extendsName) {
    if (isNodeBase(cls.extendsName)) {
      fields.push(...NODE_FIELDS);
      methods.push(...NODE_METHODS);
    } else {
      const parent = findClass(files, cls.extendsName);
      if (parent) {
        const up = classMembers(parent, files, seen);
        fields.push(...up.fields);
        methods.push(...up.methods);
        signals.push(...up.signals);
      }
    }
  } else if (cls.isNode) {
    fields.push(...NODE_FIELDS);
    methods.push(...NODE_METHODS);
  }
  return {
    fields: unique(fields),
    methods: unique(methods),
    signals: unique(signals),
  };
}

function parentMethods(cls, files) {
  if (!cls.extendsName) return cls.isNode ? [...NODE_METHODS] : [];
  if (isNodeBase(cls.extendsName)) return [...NODE_METHODS];
  const parent = findClass(files, cls.extendsName);
  if (!parent) return [];
  return classMembers(parent, files).methods;
}

function allTraits(files) {
  return unique(files.flatMap((f) => f.traits.map((t) => t.name)));
}

function allClasses(files) {
  return unique(files.flatMap((f) => f.classes.map((c) => c.name)));
}

function isSignalName(file, files, name) {
  if (file.signals.some((s) => s.name === name)) return true;
  for (const f of files) {
    if (f.signals.some((s) => s.name === name)) return true;
    for (const t of f.traits) {
      if (t.signals.some((s) => s.name === name)) return true;
    }
  }
  return false;
}

function typeOfLocal(file, name) {
  const hit = file.typedLocals.find((t) => t.name === name);
  return hit ? hit.typeName : undefined;
}

function itemsFromMembers(mem) {
  return [
    ...mem.fields.map((name) => ({ name, role: "field" })),
    ...mem.methods.map((name) => ({ name, role: "method" })),
    ...mem.signals.map((name) => ({ name, role: "signal" })),
  ];
}

function membersFor(file, files, pos, receiver) {
  const enclosing = classAt(file, pos);
  if (receiver === "self") {
    if (!enclosing) return { kind: "list", items: [] };
    return { kind: "list", items: itemsFromMembers(classMembers(enclosing, files)) };
  }
  if (receiver === "super") {
    if (!enclosing) return { kind: "list", items: [] };
    return {
      kind: "list",
      items: parentMethods(enclosing, files).map((name) => ({
        name,
        role: "super",
      })),
    };
  }
  const fromClass = enclosing
    ? classMembers(enclosing, files).signals.includes(receiver)
    : false;
  if (fromClass || isSignalName(file, files, receiver)) {
    return { kind: "list", items: [{ name: "emit", role: "emit" }] };
  }
  const typeName = typeOfLocal(file, receiver);
  if (typeName) {
    const cls = findClass(files, typeName);
    if (cls) {
      return { kind: "list", items: itemsFromMembers(classMembers(cls, files)) };
    }
  }
  return { kind: "catalog" };
}

module.exports = {
  NODE_BASES,
  NODE_FIELDS,
  NODE_METHODS,
  scanSource,
  classAt,
  classMembers,
  parentMethods,
  allTraits,
  allClasses,
  isSignalName,
  typeOfLocal,
  membersFor,
};
