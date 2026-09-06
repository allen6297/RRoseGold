# Public Option type. Constructors and match stay language-native;
# methods here are the documented API.

pub mod option {
    pub enum Option<T> {
        Some(T),
        None,
    }

    impl Option {
        fn is_some(self): Bool {
            match self {
                Some(_) { return true; }
                None { return false; }
            }
        }

        fn is_none(self): Bool {
            match self {
                Some(_) { return false; }
                None { return true; }
            }
        }

        fn unwrap_or(self, fallback: Int) {
            match self {
                Some(v) { return v; }
                None { return fallback; }
            }
        }
    }
}
