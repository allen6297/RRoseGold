# Language tour. `rosegold run examples/tour.rg`
# Prose: docs/tour.md

import math;
import json;
import checks;

signal ping();

fn add(a: Int, b: Int): Int {
    return a + b;
}

fn on_ping() {
    print("pong");
}

fn score_of(scores: Map<String, Int>, name: String): Result<Int, String> {
    if scores.has(name) {
        return Result.Ok(scores[name]);
    }
    return Result.Err(f"unknown: {name}");
}

trait Named {
    fn label(self): String;
}

class Point impl Named {
    var x: Float = 0.0;
    var y: Float = 0.0;

    fn length(self): Float {
        return math.sqrt(self.x * self.x + self.y * self.y);
    }

    fn label(self): String {
        return "point";
    }
}

fn main(): Int {
    var n: Int = 3;
    print(f"n={n}");
    print(add(2, 40));

    var sum: Int = 0;
    for i in 0..5 {
        sum = sum + i;
    }
    print(sum);

    var scores: Map<String, Int> = {"ada": 10, "grace": 12};
    scores["linus"] = 9;
    checks.eq(scores.len(), 3);
    match score_of(scores, "grace") {
        Ok(v) { print(v); }
        Err(_) { print("miss"); }
    }

    var p = Point { x: 3.0, y: 4.0 };
    print(p.length());
    print(p.label());

    var blob = json.stringify({"hits": 3, "ok": true}).unwrap();
    var parsed = json.parse(blob).unwrap();
    print(parsed["hits"]);

    print(json.parse("{").is_err());
    ping.connect(on_ping);
    ping.emit();
    return 0;
}
