# `rosegold test examples/tests.rg`
import checks;
import json;

@test
fn add() {
    checks.eq(2 + 2, 4);
}

@test
fn json_roundtrip() {
    var n = json.parse(json.stringify(7).unwrap()).unwrap();
    checks.eq(n, 7);
}
