# Public string API. Search / case / trim stay native primitives (`__str.*`).

pub mod str {
    pub fn contains(s: String, sub: String): Bool {
        return __str.contains(s, sub);
    }

    pub fn starts_with(s: String, prefix: String): Bool {
        return __str.starts_with(s, prefix);
    }

    pub fn ends_with(s: String, suffix: String): Bool {
        return __str.ends_with(s, suffix);
    }

    pub fn length(s: String): Int {
        return __str.length(s);
    }

    pub fn is_empty(s: String): Bool {
        return __str.is_empty(s);
    }

    pub fn repeat(s: String, n: Int): String {
        return __str.repeat(s, n);
    }

    pub fn upper(s: String): String {
        return __str.upper(s);
    }

    pub fn lower(s: String): String {
        return __str.lower(s);
    }

    pub fn trim(s: String): String {
        return __str.trim(s);
    }

    pub fn split(s: String, sep: String): Array {
        return __str.split(s, sep);
    }

    pub fn slice(s: String, start: Int, end: Int): String {
        return __str.slice(s, start, end);
    }
}
