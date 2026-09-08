/** node --test vscode/scan.test.js */
const { test } = require("node:test");
const assert = require("node:assert/strict");
const scan = require("./scan");

const SRC = `
trait Damageable {
    signal died();
    fn take_damage(damage: Float): Float;
}

class Player impl Damageable {
    var current_health: Float = 10.0;
    fn take_damage(damage: Float): Float {
        return 0.0;
    }
    fn shout() {
        pass;
    }
}

class Slime extends Player {
    fn tick() {
        pass;
    }
}

class Nested {
    var hp: Float = 1.0;
    impl Damageable {
        fn take_damage(damage: Float): Float {
            return 0.0;
        }
    }
}

var p = Player { current_health: 3.0 };
`;

function labels(hit) {
  return hit.kind === "list" ? hit.items.map((i) => i.name) : [];
}

test("word complete: take_damage from class + trait", () => {
  const file = scan.scanSource(SRC);
  const player = file.classes.find((c) => c.name === "Player");
  const mem = scan.classMembers(player, [file]);
  assert.ok(mem.methods.includes("take_damage"));
  assert.ok(mem.fields.includes("current_health"));
  assert.ok(mem.signals.includes("died"));
});

test("self. lists fields, methods, trait signals", () => {
  const file = scan.scanSource(SRC);
  const pos = SRC.indexOf("return 0.0");
  const names = labels(scan.membersFor(file, [file], pos, "self"));
  assert.ok(names.includes("current_health"));
  assert.ok(names.includes("take_damage"));
  assert.ok(names.includes("died"));
});

test("died. offers emit", () => {
  const file = scan.scanSource(SRC);
  const pos = SRC.indexOf("return 0.0");
  const hit = scan.membersFor(file, [file], pos, "died");
  assert.equal(hit.kind, "list");
  assert.deepEqual(hit.items.map((i) => i.name), ["emit", "connect"]);
});

test("extends offers local classes", () => {
  const file = scan.scanSource(SRC);
  assert.ok(scan.allClasses([file]).includes("Player"));
  assert.ok(scan.allClasses([file]).includes("Slime"));
});

test("impl lists traits", () => {
  const file = scan.scanSource(SRC);
  assert.deepEqual(scan.allTraits([file]), ["Damageable"]);
});

test("nested impl Trait pulls trait members", () => {
  const file = scan.scanSource(SRC);
  const nested = file.classes.find((c) => c.name === "Nested");
  assert.ok(nested.impls.includes("Damageable"));
  const mem = scan.classMembers(nested, [file]);
  assert.ok(mem.methods.includes("take_damage"));
  assert.ok(mem.signals.includes("died"));
});

test("super. on subclass lists parent methods", () => {
  const file = scan.scanSource(SRC);
  const slime = file.classes.find((c) => c.name === "Slime");
  const pos = SRC.indexOf("fn tick");
  const names = labels(scan.membersFor(file, [file], pos, "super"));
  assert.ok(names.includes("shout"));
  assert.ok(names.includes("take_damage"));
});

test("typed local p. offers Player members", () => {
  const file = scan.scanSource(SRC);
  const pos = SRC.length - 1;
  const names = labels(scan.membersFor(file, [file], pos, "p"));
  assert.ok(names.includes("current_health"));
  assert.ok(names.includes("take_damage"));
});

test("self. on subclass inherits parent fields", () => {
  const file = scan.scanSource(SRC);
  const slime = file.classes.find((c) => c.name === "Slime");
  const mem = scan.classMembers(slime, [file]);
  assert.ok(mem.fields.includes("current_health"));
  assert.ok(mem.methods.includes("shout"));
});

test("import binds last segment and alias, not from-import items", () => {
  const src =
    "import ui;\nimport util.helpers as h;\nfrom str import contains;\n";
  assert.deepEqual(scan.importedModuleBinds(src).sort(), ["h", "ui"]);
});

test("module names color on import lines and before dots", () => {
  const src =
    "import ui;\nimport util.helpers as h;\nfrom str import contains;\nfn main(): Int {\n    ui.theme({});\n    h.n();\n    return ui.run();\n}\n";
  const names = scan.moduleNameRanges(src).map((r) => src.slice(r.from, r.to));
  assert.ok(names.includes("ui"));
  assert.ok(names.includes("util"));
  assert.ok(names.includes("helpers"));
  assert.ok(names.includes("h"));
  assert.ok(names.includes("str"));
  assert.ok(!names.includes("contains"));
  assert.ok(names.filter((n) => n === "ui").length >= 3);
});
