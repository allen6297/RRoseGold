//! [`ModuleResolver`]: resolve `import` from memory, sibling `.rg` files, or both.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type ResolverRef = Arc<Mutex<dyn ModuleResolver>>;

pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Option<String> {
        self.resolve_all(name)
            .into_iter()
            .next()
            .map(|(_, src)| src)
    }

    /// Every source that contributes to `name`: filename match and/or `mod name`.
    /// Duplicate map keys (`utils` / `utils.rg`) are collapsed. Order is stable.
    fn resolve_all(&self, name: &str) -> Vec<(String, String)>;
}

pub struct HashMapResolver {
    sources: HashMap<String, String>,
}

impl HashMapResolver {
    pub fn new(sources: HashMap<String, String>) -> Self {
        Self { sources }
    }
}

impl ModuleResolver for HashMapResolver {
    fn resolve_all(&self, name: &str) -> Vec<(String, String)> {
        crate::stdlib::merge(name, collect_named_sources(&self.sources, name))
    }
}

/// In-memory sources plus optional sibling-file lookup (Play host + CLI).
pub struct CombinedResolver {
    map: HashMapResolver,
    file: Option<FileModuleResolver>,
}

impl CombinedResolver {
    pub fn new(sources: HashMap<String, String>, file_base: Option<PathBuf>) -> Self {
        Self {
            map: HashMapResolver::new(sources),
            file: file_base.map(FileModuleResolver::new),
        }
    }
}

impl ModuleResolver for CombinedResolver {
    fn resolve_all(&self, name: &str) -> Vec<(String, String)> {
        if crate::stdlib::is_embedded_stdlib(name) {
            return crate::stdlib::resolve(name);
        }
        let mut out = self.map.resolve_all(name);
        let mut seen: HashSet<String> = out.iter().map(|(_, src)| src.clone()).collect();
        if let Some(file) = &self.file {
            for (key, src) in file.resolve_all(name) {
                if seen.insert(src.clone()) {
                    out.push((key, src));
                }
            }
        }
        out
    }
}

fn collect_named_sources(sources: &HashMap<String, String>, name: &str) -> Vec<(String, String)> {
    let stem = name.strip_suffix(".rg").unwrap_or(name);
    let mut keys: Vec<&String> = sources.keys().collect();
    keys.sort_by(|a, b| {
        b.ends_with(".rg")
            .cmp(&a.ends_with(".rg"))
            .then_with(|| a.cmp(b))
    });
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for key in keys {
        let Some(source) = sources.get(key) else {
            continue;
        };
        let key_stem = key.strip_suffix(".rg").unwrap_or(key.as_str());
        let filename_hit = key_stem.eq_ignore_ascii_case(stem);
        if !filename_hit && !crate::parser::source_has_mod(source, name) {
            continue;
        }
        // `utils` / `utils.rg` / `scripts.utils` are aliases of one file. Last
        // segment so a dotted relative path does not look like a second module.
        let identity = key_stem
            .rsplit(['.', '/', '\\'])
            .next()
            .unwrap_or(key_stem)
            .to_lowercase();
        if !seen.insert(identity) {
            continue;
        }
        out.push((key.clone(), source.clone()));
    }
    out
}

pub struct FileModuleResolver {
    base: PathBuf,
}

impl FileModuleResolver {
    pub fn new(base: impl AsRef<Path>) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
        }
    }
}

/// Paths the file resolver tries for `import name` (docs / error text).
pub fn module_lookup_hint(name: &str) -> String {
    let stem = name.strip_suffix(".rg").unwrap_or(name);
    let dotted = stem.replace('.', "/");
    let vendor =
        stem.eq_ignore_ascii_case("vendor") || stem.to_ascii_lowercase().starts_with("vendor.");
    if dotted == stem {
        if vendor {
            format!("{stem}.rg or {stem}/lib.rg")
        } else {
            format!("{stem}.rg, {stem}/lib.rg, vendor/{stem}.rg, or vendor/{stem}/lib.rg")
        }
    } else if vendor {
        format!("{stem}.rg, {dotted}.rg, or {dotted}/lib.rg")
    } else {
        format!(
            "{stem}.rg, {dotted}.rg, {dotted}/lib.rg, vendor/{dotted}.rg, or vendor/{dotted}/lib.rg"
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn local_import_candidates(base: &Path, name: &str) -> Vec<PathBuf> {
    let dotted = name.replace('.', std::path::MAIN_SEPARATOR_STR);
    vec![
        base.join(format!("{name}.rg")),
        base.join(format!("{dotted}.rg")),
        base.join(&dotted).join("lib.rg"),
        base.join(&dotted).join("main.rg"),
        base.join(name).join("lib.rg"),
        base.join(name).join("main.rg"),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn vendor_import_candidates(base: &Path, name: &str) -> Vec<PathBuf> {
    let lower = name.to_ascii_lowercase();
    if lower == "vendor" || lower.starts_with("vendor.") {
        return Vec::new();
    }
    let dotted = name.replace('.', std::path::MAIN_SEPARATOR_STR);
    let vendor = base.join("vendor");
    vec![
        vendor.join(format!("{name}.rg")),
        vendor.join(format!("{dotted}.rg")),
        vendor.join(&dotted).join("lib.rg"),
        vendor.join(&dotted).join("main.rg"),
        vendor.join(name).join("lib.rg"),
        vendor.join(name).join("main.rg"),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn push_module_file(
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<(String, String)>,
    fallback_name: &str,
    path: PathBuf,
    source: String,
) {
    let id = path.canonicalize().unwrap_or_else(|_| path.clone());
    if !seen.insert(id) {
        return;
    }
    let key = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(fallback_name)
        .to_string();
    out.push((key, source));
}

impl ModuleResolver for FileModuleResolver {
    fn resolve_all(&self, name: &str) -> Vec<(String, String)> {
        let std = crate::stdlib::resolve(name);
        if !std.is_empty() {
            return std;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (&self.base, name);
            return Vec::new();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let stem = name.strip_suffix(".rg").unwrap_or(name);
            let mut out = Vec::new();
            let mut seen = HashSet::new();

            for path in local_import_candidates(&self.base, name) {
                if path.exists() {
                    if let Ok(source) = std::fs::read_to_string(&path) {
                        push_module_file(&mut seen, &mut out, name, path, source);
                    }
                }
            }
            if let Ok(entries) = std::fs::read_dir(&self.base) {
                let mut paths: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rg"))
                    .collect();
                paths.sort();
                for path in paths {
                    let Ok(source) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let file_stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
                    if file_stem.eq_ignore_ascii_case(stem)
                        || crate::parser::source_has_mod(&source, name)
                    {
                        push_module_file(&mut seen, &mut out, name, path, source);
                    }
                }
            }
            if out.is_empty() {
                for path in vendor_import_candidates(&self.base, name) {
                    if path.exists() {
                        if let Ok(source) = std::fs::read_to_string(&path) {
                            push_module_file(&mut seen, &mut out, name, path, source);
                        }
                    }
                }
            }
            out
        }
    }
}
