# Public math API. Trig / pow / sqrt / conversions are native primitives
# (`__math.*`). The rest is RoseGold. Bodies must not call sibling fns —
# they run in the caller's VM, not this module.

pub mod math {
    pub fn sin(n: Float): Float {
        return __math.sin(n);
    }

    pub fn cos(n: Float): Float {
        return __math.cos(n);
    }

    pub fn atan2(y: Float, x: Float): Float {
        return __math.atan2(y, x);
    }

    pub fn sqrt(n: Float): Float {
        return __math.sqrt(n);
    }

    pub fn pow(a: Float, b: Float): Float {
        return __math.pow(a, b);
    }

    pub fn to_int(n: Float): Int {
        return __math.to_int(n);
    }

    pub fn to_float(n: Int): Float {
        return __math.to_float(n);
    }

    pub fn abs(n: Float): Float {
        if n < 0 {
            return -n;
        }
        return n;
    }

    pub fn abs_float(n: Float): Float {
        if n < 0 {
            return -n;
        }
        return n;
    }

    pub fn sign(n: Float): Int {
        if n > 0 {
            return 1;
        }
        if n < 0 {
            return -1;
        }
        return 0;
    }

    pub fn min(a: Float, b: Float): Float {
        if a < b {
            return a;
        }
        return b;
    }

    pub fn max(a: Float, b: Float): Float {
        if a > b {
            return a;
        }
        return b;
    }

    pub fn min_float(a: Float, b: Float): Float {
        if a < b {
            return a;
        }
        return b;
    }

    pub fn max_float(a: Float, b: Float): Float {
        if a > b {
            return a;
        }
        return b;
    }

    pub fn clamp(v: Float, lo: Float, hi: Float): Float {
        if v < lo {
            return lo;
        }
        if v > hi {
            return hi;
        }
        return v;
    }

    pub fn clamp_float(v: Float, lo: Float, hi: Float): Float {
        if v < lo {
            return lo;
        }
        if v > hi {
            return hi;
        }
        return v;
    }

    pub fn lerp(a: Float, b: Float, t: Float): Float {
        return a + (b - a) * t;
    }

    pub fn lerp_float(a: Float, b: Float, t: Float): Float {
        return a + (b - a) * t;
    }

    pub fn move_toward(current: Float, target: Float, delta: Float): Float {
        var step = delta;
        if step < 0 {
            step = -step;
        }
        var d = target - current;
        if d < 0 {
            d = -d;
        }
        if d <= step {
            return target;
        }
        if target > current {
            return current + step;
        }
        return current - step;
    }

    pub fn gcd(a: Int, b: Int): Int {
        var x = a;
        var y = b;
        if x < 0 {
            x = -x;
        }
        if y < 0 {
            y = -y;
        }
        while y != 0 {
            var t = y;
            y = x % y;
            x = t;
        }
        return x;
    }
}
