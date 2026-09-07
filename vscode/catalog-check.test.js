/** node --test vscode/catalog-check.test.js */
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { check, UI_REQUIRED } = require("./catalog-check");

test("catalog has public host and ui keys", () => {
  const result = check();
  assert.deepEqual(result.missing, [], `missing: ${result.missing.join(", ")}`);
  for (const key of UI_REQUIRED) {
    assert.ok(result.required.includes(key), key);
  }
});
