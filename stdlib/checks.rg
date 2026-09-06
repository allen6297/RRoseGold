# Assertion helpers for `@test`. Pure RoseGold on top of `assert`.

pub mod checks {
    pub fn that(value: Bool) {
        assert(value);
    }

    pub fn truthy(value: Bool) {
        assert(value);
    }

    pub fn falsey(value: Bool) {
        assert(!value);
    }

    pub fn eq(a: Int, b: Int) {
        assert(a == b);
    }

    pub fn neq(a: Int, b: Int) {
        assert(a != b);
    }

    pub fn eq_bool(a: Bool, b: Bool) {
        assert(a == b);
    }

    pub fn eq_float(a: Float, b: Float) {
        assert(a == b);
    }

    pub fn eq_string(a: String, b: String) {
        assert(a == b);
    }
}
