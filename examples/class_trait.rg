# Port of a typical RoseGold-PY class example, plus single inheritance.
# cargo run -- run examples/class_trait.rg

import math;

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

class Vec3 extends Point impl Named {
    var z: Float = 0.0;

    fn length(self): Float {
        var xy = super.length();
        return math.sqrt(xy * xy + self.z * self.z);
    }

    fn label(self): String {
        return "vec3";
    }
}

fn main(): Int {
    var p = Point { x: 3.0, y: 4.0 };
    print(p.length());
    print(p.label());
    var crate_v = Vec2 { x: 6.0, y: 8.0 };
    print(crate_v.length());
    var q = Vec3 { x: 3.0, y: 4.0, z: 12.0 };
    print(q.length());
    print(q.label());
    return 0;
}
