//! Crate-embedded public stdlib (`.rg`). Host APIs (`io`, `strata`, `input`, `time`, `ui`)
//! stay native. Trig / string search stay a thin primitive table (`__math`, `__str`).

use std::collections::HashMap;

/// Filename stem → source. Injected by every resolver ahead of user modules.
pub const SOURCES: &[(&str, &str)] = &[
    ("option", include_str!("../stdlib/option.rg")),
    ("result", include_str!("../stdlib/result.rg")),
    ("math", include_str!("../stdlib/math.rg")),
    ("str", include_str!("../stdlib/str.rg")),
    ("checks", include_str!("../stdlib/checks.rg")),
    ("vec", include_str!("../stdlib/vec.rg")),
    ("node", include_str!("../stdlib/node.rg")),
];

pub fn sources_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (name, src) in SOURCES {
        map.insert((*name).to_string(), (*src).to_string());
        map.insert(format!("{name}.rg"), (*src).to_string());
    }
    map
}

/// Host-only modules. Never loaded from `.rg`.
pub fn is_host_module(name: &str) -> bool {
    matches!(
        name,
        "io" | "strata" | "input" | "time" | "ui" | "__math" | "__str"
    )
}

/// Public stdlib that lives in `stdlib/*.rg`.
pub fn is_embedded_stdlib(name: &str) -> bool {
    canonical_module(name).is_some()
}

/// Map `Option` / `Result` aliases onto the file stem.
pub fn canonical_module(name: &str) -> Option<&'static str> {
    let stem = name.strip_suffix(".rg").unwrap_or(name);
    match stem {
        "option" | "Option" => Some("option"),
        "result" | "Result" => Some("result"),
        "math" => Some("math"),
        "str" => Some("str"),
        "checks" => Some("checks"),
        "vec" | "Vec2" | "Vec3" => Some("vec"),
        "node" | "Node" | "Empty" | "Sprite" | "Tilemap" | "Camera" | "Mesh" | "Light" => {
            Some("node")
        }
        _ => None,
    }
}

/// Scene kind for a builtin node class (`Sprite` → `sprite`). `Node` / `Empty` → `empty`.
pub fn node_kind(type_name: &str) -> Option<&'static str> {
    match type_name {
        "Node" | "Empty" => Some("empty"),
        "Sprite" => Some("sprite"),
        "Tilemap" => Some("tilemap"),
        "Camera" => Some("camera"),
        "Mesh" => Some("mesh"),
        "Light" => Some("light"),
        _ => None,
    }
}

/// `import strata.Sprite` / `from strata import Node` — types from `node.rg`, not host `strata.move`.
pub fn is_node_type_import(path: &[String]) -> bool {
    path.len() == 2 && path[0] == "strata" && node_kind(&path[1]).is_some()
}

/// Host primitives used only inside crate stdlib (`.rg` wrappers). Always in scope.
pub fn is_internal_host(name: &str) -> bool {
    matches!(name, "__math" | "__str")
}

/// Language constructors with qualified statics (`Array.first(xs)`). Always in scope.
pub fn is_language_module(name: &str) -> bool {
    matches!(name, "Array")
}

/// Filename stem and source for an embedded crate file (`math` / `math.rg`).
pub fn file_source(name: &str) -> Option<(&'static str, &'static str)> {
    let wanted = canonical_module(name).unwrap_or_else(|| name.strip_suffix(".rg").unwrap_or(name));
    SOURCES.iter().copied().find(|(n, _)| *n == wanted)
}

/// Sources for an embedded module, or empty if `name` is not crate stdlib.
pub fn resolve(name: &str) -> Vec<(String, String)> {
    if let Some(stem) = canonical_module(name) {
        for (n, src) in SOURCES {
            if *n == stem {
                return vec![(format!("{n}.rg"), (*src).to_string())];
            }
        }
    }
    for (n, src) in SOURCES {
        if crate::parser::source_has_mod(src, name) {
            return vec![(format!("{n}.rg"), (*src).to_string())];
        }
    }
    Vec::new()
}

/// Prefer crate stdlib; ignore project files with the same name.
pub fn merge(name: &str, rest: Vec<(String, String)>) -> Vec<(String, String)> {
    let std = resolve(name);
    if !std.is_empty() {
        return std;
    }
    rest
}
