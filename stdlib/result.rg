# Public Result type. Ok/Err constructors stay language-native.

pub mod result {
    pub enum Result<T, E> {
        Ok(T),
        Err(E),
    }

    impl Result {
        fn is_ok(self): Bool {
            match self {
                Ok(_) { return true; }
                Err(_) { return false; }
            }
        }

        fn is_err(self): Bool {
            match self {
                Ok(_) { return false; }
                Err(_) { return true; }
            }
        }

        fn unwrap_or(self, fallback: Int) {
            match self {
                Ok(v) { return v; }
                Err(_) { return fallback; }
            }
        }
    }
}
