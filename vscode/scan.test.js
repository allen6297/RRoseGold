/** SE8 acceptance: node --test editors/vscode/scan.test.js */
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
}

class Slime extends Sprite {
    fn on_update(dt: Float) {
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
  assert.deepEqual(hit.items.map((i) => i.name), ["emit"]);
});

test("extends offers node bases via NODE_BASES", () => {
  assert.ok(scan.NODE_BASES.includes("Sprite"));
  const file = scan.scanSource(SRC);
  assert.ok(scan.allClasses([file]).includes("Player"));
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

test("super. on Sprite child lists node methods", () => {
  const file = scan.scanSource(SRC);
  const slime = file.classes.find((c) => c.name === "Slime");
  const pos = SRC.indexOf("fn on_update");
  const names = labels(scan.membersFor(file, [file], pos, "super"));
  assert.ok(names.includes("on_update"));
  assert.ok(names.includes("on_create"));
});

test("typed local p. offers Player members", () => {
  const file = scan.scanSource(SRC);
  const pos = SRC.length - 1;
  const names = labels(scan.membersFor(file, [file], pos, "p"));
  assert.ok(names.includes("current_health"));
  assert.ok(names.includes("take_damage"));
});

test("self. on Sprite subclass includes x/y/z", () => {
  const file = scan.scanSource(SRC);
  const slime = file.classes.find((c) => c.name === "Slime");
  const mem = scan.classMembers(slime, [file]);
  assert.ok(mem.fields.includes("x"));
  assert.ok(mem.fields.includes("name"));
  assert.ok(mem.methods.includes("on_create"));
});
