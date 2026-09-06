## 2D vector. Construction is `Vec2 { x, y }` (class field defaults apply).
## `length` uses `__math.sqrt` so callers do not need `import math`.
class Vec2 {
    var x: Float = 0.0;
    var y: Float = 0.0;

    fn length(self): Float {
        return __math.sqrt(self.x * self.x + self.y * self.y);
    }

    fn add(self, other: Vec2): Vec2 {
        return Vec2 { x: self.x + other.x, y: self.y + other.y };
    }
}

## 3D vector. Construction is `Vec3 { x, y, z }` (class field defaults apply).
## `length` uses `__math.sqrt` so callers do not need `import math`.
pub class Vec3 extends Vec2 {
    var z: Float = 0.0;

    fn length(self): Float {
        return __math.sqrt(self.x * self.x + self.y * self.y + self.z * self.z);
    }

    fn add(self, other: Vec3): Vec3 {
        return Vec3 { x: self.x + other.x, y: self.y + other.y, z: self.z + other.z };
    }
}