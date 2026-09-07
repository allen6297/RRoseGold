//! `rosegold vendor`: clone into `vendor/<name>/`, pin SHAs, restore, remove.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One pin in `vendor.lock`: folder name, clone URL, git SHA, optional version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorPin {
    pub name: String,
    pub url: String,
    pub sha: String,
    pub version: Option<String>,
}

/// Optional `rg.toml` in a library repo (`name`, `version`, `files`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RgToml {
    pub name: Option<String>,
    pub version: Option<String>,
    pub files: Vec<String>,
}

/// Clone `url` into `root/vendor/<name>/` and upsert `root/vendor.lock`.
pub fn vendor_git(url: &str, root: &Path) -> Result<VendorPin, String> {
    vendor_git_with(url, root, "git")
}

/// Same as [`vendor_git`], with an explicit `git` program (tests).
pub fn vendor_git_with(url: &str, root: &Path, git: &str) -> Result<VendorPin, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("git URL is empty".into());
    }
    ensure_git(git)?;
    let url_name = package_name_from_url(url)?;
    let vendor_dir = root.join("vendor");
    fs::create_dir_all(&vendor_dir).map_err(|e| format!("cannot create vendor/: {e}"))?;

    let staging = vendor_dir.join(format!(".rg-staging-{url_name}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("cannot clear {}: {e}", staging.display()))?;
    }

    let sha = match clone_fresh(git, url, &staging) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    let result = place_clone(root, git, url, &url_name, &sha, &vendor_dir, &staging);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Restore every pin in `root/vendor.lock` into `root/vendor/<name>/`.
pub fn vendor_from_lock(root: &Path) -> Result<Vec<VendorPin>, String> {
    vendor_from_lock_with(root, "git")
}

/// Same as [`vendor_from_lock`], with an explicit `git` program (tests).
pub fn vendor_from_lock_with(root: &Path, git: &str) -> Result<Vec<VendorPin>, String> {
    let path = lockfile_path(root);
    if !path.is_file() {
        return Err("no vendor.lock; run rosegold vendor <git-url> first".into());
    }
    ensure_git(git)?;
    let pins = read_lockfile(root)?;
    if pins.is_empty() {
        return Err("vendor.lock has no packages".into());
    }
    let vendor_dir = root.join("vendor");
    fs::create_dir_all(&vendor_dir).map_err(|e| format!("cannot create vendor/: {e}"))?;
    for pin in &pins {
        restore_pin(git, pin, &vendor_dir)?;
    }
    Ok(pins)
}

/// Drop `vendor/<name>/` and its lock line.
pub fn vendor_remove(root: &Path, name: &str) -> Result<VendorPin, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("package name is empty".into());
    }
    if name.contains(['/', '\\', ':', '<', '>', '|', '*', '?']) {
        return Err(format!("invalid package name '{name}'"));
    }
    let path = lockfile_path(root);
    if !path.is_file() {
        return Err(format!(
            "vendor.lock has no package '{name}' (no vendor.lock)"
        ));
    }
    let mut pins = read_lockfile(root)?;
    let idx = pins
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| format!("vendor.lock has no package '{name}'"))?;
    let pin = pins.remove(idx);
    let dest = root.join("vendor").join(name);
    if dest.exists() {
        if dest.is_dir() {
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("failed to remove vendor/{name}: {e}"))?;
        } else {
            fs::remove_file(&dest).map_err(|e| format!("failed to remove vendor/{name}: {e}"))?;
        }
    }
    write_lockfile(root, &pins)?;
    Ok(pin)
}

/// Last path segment of a git URL or local path (`httpclient.git` → `httpclient`).
pub fn package_name_from_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Err("git URL is empty".into());
    }
    let pathish = ssh_path(trimmed).unwrap_or(trimmed);
    let basename = pathish
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(pathish);
    let name = basename
        .strip_suffix(".git")
        .unwrap_or(basename)
        .trim()
        .to_string();
    validate_package_name(&name, url)
}

pub fn lockfile_path(root: &Path) -> PathBuf {
    root.join("vendor.lock")
}

pub fn read_lockfile(root: &Path) -> Result<Vec<VendorPin>, String> {
    let path = lockfile_path(root);
    let text = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("failed to read {}: {e}", path.display())),
    };
    parse_lockfile(&text)
}

pub fn write_lockfile(root: &Path, pins: &[VendorPin]) -> Result<(), String> {
    let path = lockfile_path(root);
    fs::write(&path, format_lockfile(pins))
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

pub fn parse_lockfile(text: &str) -> Result<Vec<VendorPin>, String> {
    let mut pins = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 3 && parts.len() != 4 {
            return Err(format!(
                "vendor.lock line {}: expected `name url sha` or `name url sha version`, got {line}",
                i + 1
            ));
        }
        pins.push(VendorPin {
            name: parts[0].to_string(),
            url: parts[1].to_string(),
            sha: parts[2].to_string(),
            version: parts.get(3).map(|s| (*s).to_string()),
        });
    }
    Ok(pins)
}

pub fn format_lockfile(pins: &[VendorPin]) -> String {
    let mut pins: Vec<&VendorPin> = pins.iter().collect();
    pins.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = String::from("# RoseGold vendor.lock — name url sha [version]\n");
    for pin in pins {
        out.push_str(&pin.name);
        out.push(' ');
        out.push_str(&pin.url);
        out.push(' ');
        out.push_str(&pin.sha);
        if let Some(v) = pin.version.as_deref().filter(|s| !s.is_empty()) {
            out.push(' ');
            out.push_str(v);
        }
        out.push('\n');
    }
    out
}

pub fn parse_rg_toml(text: &str) -> Result<RgToml, String> {
    let mut name = None;
    let mut version = None;
    let mut files = Vec::new();
    let logical = merge_toml_continuations(text);
    for (i, raw) in logical.iter().enumerate() {
        let line = strip_toml_comment(raw);
        if line.is_empty() {
            continue;
        }
        let (key, value) = split_toml_kv(line).ok_or_else(|| {
            format!("rg.toml line {}: expected `key = value`, got {line}", i + 1)
        })?;
        match key {
            "name" => {
                let n = parse_toml_string(value)?;
                if n.is_empty() {
                    return Err("rg.toml name is empty".into());
                }
                validate_package_name(&n, "rg.toml")?;
                name = Some(n);
            }
            "version" => {
                let v = parse_toml_string(value)?;
                if !v.is_empty() {
                    validate_version(&v)?;
                    version = Some(v);
                }
            }
            "files" => files = parse_toml_string_array(value)?,
            _ => {}
        }
    }
    Ok(RgToml {
        name,
        version,
        files,
    })
}

pub fn read_rg_toml(dir: &Path) -> Result<Option<RgToml>, String> {
    let path = dir.join("rg.toml");
    match fs::read_to_string(&path) {
        Ok(text) => parse_rg_toml(&text).map(Some),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

fn place_clone(
    root: &Path,
    git: &str,
    url: &str,
    url_name: &str,
    sha: &str,
    vendor_dir: &Path,
    staging: &Path,
) -> Result<VendorPin, String> {
    let meta = read_rg_toml(staging)?;
    let name = meta
        .as_ref()
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| url_name.to_string());
    let version = meta.as_ref().and_then(|m| m.version.clone());
    if !has_library_entry(staging, &name) {
        return Err(format!(
            "vendor/{name} has no lib.rg or {name}.rg; not a RoseGold library"
        ));
    }
    if let Some(meta) = &meta {
        check_declared_files(staging, &name, &meta.files)?;
    }

    let dest = vendor_dir.join(&name);
    let mut pins = read_lockfile(root)?;
    if dest.exists() {
        if dest.join(".git").exists() {
            let old_sha = run_git(git, &["rev-parse", "HEAD"], Some(&dest))?;
            let old_meta = read_rg_toml(&dest)?;
            let old_version = old_meta
                .as_ref()
                .and_then(|m| m.version.clone())
                .or_else(|| {
                    pins.iter()
                        .find(|p| p.name == name)
                        .and_then(|p| p.version.clone())
                });
            let old_url = run_git(git, &["remote", "get-url", "origin"], Some(&dest))
                .unwrap_or_else(|_| url.to_string());
            if should_keep_previous(&old_version, &version, &old_url, url) {
                snapshot_previous(
                    vendor_dir,
                    &dest,
                    &name,
                    &old_url,
                    &old_sha,
                    old_version.as_deref(),
                    &mut pins,
                )?;
            } else {
                fs::remove_dir_all(&dest)
                    .map_err(|e| format!("cannot replace vendor/{name}: {e}"))?;
            }
        } else if dest.is_dir() {
            return Err(format!(
                "vendor/{name} exists but is not a git repository; remove it to vendor {url}"
            ));
        } else {
            return Err(format!(
                "vendor/{name} exists and is not a directory; remove it and retry"
            ));
        }
    }

    fs::rename(staging, &dest).map_err(|e| format!("cannot move clone to vendor/{name}: {e}"))?;

    let pin = VendorPin {
        name: name.clone(),
        url: url.to_string(),
        sha: sha.to_string(),
        version,
    };
    upsert_pin(&mut pins, pin.clone());
    write_lockfile(root, &pins)?;
    Ok(pin)
}

fn should_keep_previous(
    old_version: &Option<String>,
    new_version: &Option<String>,
    old_url: &str,
    new_url: &str,
) -> bool {
    match (old_version.as_deref(), new_version.as_deref()) {
        (Some(a), Some(b)) if a != b => true,
        _ => !urls_match(old_url, new_url),
    }
}

fn snapshot_previous(
    vendor_dir: &Path,
    dest: &Path,
    name: &str,
    old_url: &str,
    old_sha: &str,
    old_version: Option<&str>,
    pins: &mut Vec<VendorPin>,
) -> Result<(), String> {
    let suffix = match old_version {
        Some(v) => v.to_string(),
        None => short_sha(old_sha),
    };
    let extra_name = format!("{name}-{suffix}");
    validate_package_name(&extra_name, "vendor collision")?;
    let extra = vendor_dir.join(&extra_name);
    if extra.exists() {
        return Err(format!(
            "vendor/{extra_name} already exists; remove it before keeping the previous {name}"
        ));
    }
    fs::rename(dest, &extra).map_err(|e| format!("cannot keep previous vendor/{name}: {e}"))?;
    pins.retain(|p| p.name != extra_name);
    pins.push(VendorPin {
        name: extra_name,
        url: old_url.to_string(),
        sha: old_sha.to_string(),
        version: old_version.map(str::to_string),
    });
    Ok(())
}

fn restore_pin(git: &str, pin: &VendorPin, vendor_dir: &Path) -> Result<(), String> {
    let dest = vendor_dir.join(&pin.name);
    if dest.join(".git").exists() {
        let head = run_git(git, &["rev-parse", "HEAD"], Some(&dest))?;
        if head != pin.sha {
            if run_git(git, &["checkout", "--detach", &pin.sha], Some(&dest)).is_err() {
                run_git(git, &["fetch", "origin"], Some(&dest))
                    .map_err(|e| format!("failed to fetch vendor/{}: {e}", pin.name))?;
                run_git(git, &["checkout", "--detach", &pin.sha], Some(&dest)).map_err(|e| {
                    format!(
                        "failed to checkout {} for vendor/{}: {e}",
                        pin.sha, pin.name
                    )
                })?;
            }
        }
    } else if dest.exists() {
        return Err(format!(
            "vendor/{} exists but is not a git repository; remove it to restore from vendor.lock",
            pin.name
        ));
    } else {
        clone_fresh(git, &pin.url, &dest)?;
        let head = run_git(git, &["rev-parse", "HEAD"], Some(&dest))?;
        if head != pin.sha {
            run_git(git, &["checkout", "--detach", &pin.sha], Some(&dest)).map_err(|e| {
                format!(
                    "failed to checkout {} for vendor/{}: {e}",
                    pin.sha, pin.name
                )
            })?;
        }
    }
    if !has_library_entry(&dest, &pin.name) {
        return Err(format!(
            "vendor/{} has no lib.rg or {}.rg; not a RoseGold library",
            pin.name, pin.name
        ));
    }
    if let Some(meta) = read_rg_toml(&dest)? {
        check_declared_files(&dest, &pin.name, &meta.files)?;
    }
    Ok(())
}

fn upsert_pin(pins: &mut Vec<VendorPin>, pin: VendorPin) {
    if let Some(existing) = pins.iter_mut().find(|p| p.name == pin.name) {
        *existing = pin;
    } else {
        pins.push(pin);
    }
}

fn validate_package_name(name: &str, from: &str) -> Result<String, String> {
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
        return Err(format!("cannot derive a package name from {from}"));
    }
    if name.contains(['/', '\\', ':', '<', '>', '|', '*', '?']) {
        return Err(format!("invalid package name '{name}' from {from}"));
    }
    Ok(name.to_string())
}

fn validate_version(version: &str) -> Result<(), String> {
    if version.contains(['/', '\\', ':', '<', '>', '|', '*', '?', ' ']) {
        return Err(format!("invalid version '{version}' in rg.toml"));
    }
    Ok(())
}

fn short_sha(sha: &str) -> String {
    let n = sha.len().min(7);
    sha[..n].to_string()
}

fn check_declared_files(dest: &Path, name: &str, files: &[String]) -> Result<(), String> {
    let mut missing = Vec::new();
    for file in files {
        let rel = file.trim();
        if rel.is_empty() {
            continue;
        }
        let path = Path::new(rel);
        if path.is_absolute() || rel.split(['/', '\\']).any(|p| p == "..") {
            return Err(format!("rg.toml files entry '{rel}' is not a project-relative path"));
        }
        if !dest.join(path).is_file() {
            missing.push(rel.to_string());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "vendor/{name} rg.toml lists missing files: {}",
            missing.join(", ")
        ))
    }
}

fn merge_toml_continuations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0i32;
    for raw in text.lines() {
        if buf.is_empty() {
            buf.push_str(raw);
        } else {
            buf.push(' ');
            buf.push_str(raw.trim());
        }
        for c in raw.chars() {
            match c {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            out.push(std::mem::take(&mut buf));
            depth = 0;
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_string = None;
    for (i, c) in line.char_indices() {
        match (c, in_string) {
            ('"' | '\'', None) => in_string = Some(c),
            (q, Some(open)) if q == open => in_string = None,
            ('#', None) => return line[..i].trim(),
            _ => {}
        }
    }
    line.trim()
}

fn split_toml_kv(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    let value = line[eq + 1..].trim();
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

fn parse_toml_string(value: &str) -> Result<String, String> {
    let v = value.trim();
    if let Some(s) = unquote(v) {
        return Ok(s);
    }
    if v.starts_with('[') {
        return Err(format!("expected a string, got {v}"));
    }
    if v.split_whitespace().count() == 1 {
        return Ok(v.to_string());
    }
    Err(format!("expected a quoted string, got {v}"))
}

fn parse_toml_string_array(value: &str) -> Result<Vec<String>, String> {
    let v = value.trim();
    let inner = v
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| format!("expected an array, got {v}"))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(parse_toml_string(part)?);
    }
    Ok(out)
}

fn unquote(value: &str) -> Option<String> {
    let v = value.trim();
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            if v.len() >= 2 {
                return Some(inner.to_string());
            }
        }
    }
    None
}

fn ssh_path(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("git@")?;
    let (_host, path) = rest.split_once(':')?;
    if path.contains('/') || path.ends_with(".git") {
        Some(path)
    } else {
        None
    }
}

fn ensure_git(git: &str) -> Result<(), String> {
    match Command::new(git).arg("--version").output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr.trim();
            if msg.is_empty() {
                Err(format!("{git} --version failed"))
            } else {
                Err(msg.to_string())
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Err(format!(
            "{git} not found; install git to use rosegold vendor"
        )),
        Err(e) => Err(format!("failed to run {git}: {e}")),
    }
}

fn clone_fresh(git: &str, url: &str, dest: &Path) -> Result<String, String> {
    let dest_s = dest.to_string_lossy();
    run_git(git, &["clone", "--", url, dest_s.as_ref()], None)
        .map_err(|e| format!("failed to clone {url}: {e}"))?;
    run_git(git, &["rev-parse", "HEAD"], Some(dest))
}

fn urls_match(a: &str, b: &str) -> bool {
    normalize_git_url(a) == normalize_git_url(b)
}

fn normalize_git_url(url: &str) -> String {
    let s = url
        .trim()
        .trim_end_matches(['/', '\\'])
        .strip_suffix(".git")
        .unwrap_or(url.trim().trim_end_matches(['/', '\\']));
    let s = s.strip_prefix("file://").unwrap_or(s);
    let s = s.replace('\\', "/");
    s.to_ascii_lowercase()
}

fn has_library_entry(dest: &Path, name: &str) -> bool {
    dest.join("lib.rg").is_file() || dest.join(format!("{name}.rg")).is_file()
}

fn run_git(git: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new(git);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            format!("{git} not found; install git to use rosegold vendor")
        } else {
            format!("failed to run {git}: {e}")
        }
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err(format!("{git} {} failed", args.join(" ")));
        }
        return Err(msg.to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_https_and_ssh() {
        assert_eq!(
            package_name_from_url("https://github.com/you/httpclient.git").unwrap(),
            "httpclient"
        );
        assert_eq!(
            package_name_from_url("https://github.com/you/httpclient").unwrap(),
            "httpclient"
        );
        assert_eq!(
            package_name_from_url("git@github.com:you/httpclient.git").unwrap(),
            "httpclient"
        );
        assert_eq!(
            package_name_from_url("file:///tmp/httpclient").unwrap(),
            "httpclient"
        );
    }

    #[test]
    fn name_from_windows_path() {
        assert_eq!(
            package_name_from_url(r"C:\libs\httpclient").unwrap(),
            "httpclient"
        );
        assert_eq!(
            package_name_from_url(r"C:\libs\httpclient.git").unwrap(),
            "httpclient"
        );
    }

    #[test]
    fn name_rejects_empty() {
        assert!(package_name_from_url("").is_err());
        assert!(package_name_from_url("   ").is_err());
    }

    #[test]
    fn lockfile_roundtrip() {
        let text = format_lockfile(&[
            VendorPin {
                name: "regexish".into(),
                url: "https://example.com/regexish.git".into(),
                sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                version: None,
            },
            VendorPin {
                name: "httpclient".into(),
                url: "https://example.com/httpclient.git".into(),
                sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                version: Some("0.2".into()),
            },
        ]);
        assert!(text.starts_with("# RoseGold vendor.lock"));
        let pins = parse_lockfile(&text).unwrap();
        assert_eq!(pins[0].name, "httpclient");
        assert_eq!(pins[0].version.as_deref(), Some("0.2"));
        assert_eq!(pins[1].name, "regexish");
        assert_eq!(pins[1].version, None);
        assert_eq!(pins[0].sha.len(), 40);
    }

    #[test]
    fn lockfile_skips_comments() {
        let pins =
            parse_lockfile("# comment\n\nhttpclient https://example.com/httpclient.git abcdef0\n")
                .unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].name, "httpclient");
        assert_eq!(pins[0].version, None);
    }

    #[test]
    fn lockfile_rejects_bad_line() {
        let err = parse_lockfile("httpclient only-two-fields\n").unwrap_err();
        assert!(err.contains("expected `name url sha`"), "{err}");
    }

    #[test]
    fn rg_toml_reads_name_version_files() {
        let meta = parse_rg_toml(
            r#"
# library
name = "httpclient"
version = "0.1"
files = ["lib.rg", "parse.rg"]
"#,
        )
        .unwrap();
        assert_eq!(meta.name.as_deref(), Some("httpclient"));
        assert_eq!(meta.version.as_deref(), Some("0.1"));
        assert_eq!(meta.files, ["lib.rg", "parse.rg"]);
    }

    #[test]
    fn rg_toml_multiline_files() {
        let meta = parse_rg_toml(
            r#"
name = "httpclient"
files = [
  "lib.rg",
  "parse.rg",
]
"#,
        )
        .unwrap();
        assert_eq!(meta.files, ["lib.rg", "parse.rg"]);
    }

    #[test]
    fn rg_toml_missing_is_empty() {
        let meta = parse_rg_toml("").unwrap();
        assert_eq!(meta, RgToml::default());
    }
}
