//! CLI: `check`, `run`, `test`, `fmt`, `hover`, `def`.

use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rosegold::{SymbolInfo, check_file, check_source_at, def_at, hover_at, sibling_modules};

fn usage() -> ! {
    eprintln!(
        "\
RoseGold — Strata scripting language

Usage:
  rosegold check [--json] [--stdin] <file>
  rosegold run   [--json] <file>
  rosegold test  [--json] <file>
  rosegold fmt   [--write] [--check] [--stdin] <file>
  rosegold hover [--json] [--stdin] <file> <line> <col>
  rosegold def   [--json] [--stdin] <file> <line> <col>

  check  parse and typecheck (no eval)
  run    compile and run (calls main if present)
  test   run @test functions
  fmt    pretty-print (stdout; --write in place; --check exit 1 if dirty)
  hover  symbol signature at 1-based line:col
  def    go-to-definition at 1-based line:col

  --stdin  read source from stdin; <file> is the path label / import root
"
    );
    std::process::exit(2);
}

struct Args {
    cmd: String,
    json: bool,
    stdin: bool,
    write: bool,
    check: bool,
    path: PathBuf,
    line: Option<u32>,
    col: Option<u32>,
}

fn parse_args(mut argv: impl Iterator<Item = String>) -> Args {
    let cmd = argv.next().unwrap_or_else(|| usage());
    if matches!(cmd.as_str(), "-h" | "--help" | "help") {
        usage();
    }
    let mut json = false;
    let mut stdin = false;
    let mut write = false;
    let mut check = false;
    let mut path: Option<PathBuf> = None;
    let mut nums: Vec<u32> = Vec::new();
    for a in argv {
        if a == "--json" {
            json = true;
        } else if a == "--stdin" {
            stdin = true;
        } else if a == "--write" {
            write = true;
        } else if a == "--check" {
            check = true;
        } else if a.starts_with('-') {
            eprintln!("unknown flag: {a}");
            usage();
        } else if path.is_none() {
            path = Some(PathBuf::from(a));
        } else if let Ok(n) = a.parse::<u32>() {
            nums.push(n);
        } else {
            eprintln!("unexpected argument: {a}");
            usage();
        }
    }
    let path = path.unwrap_or_else(|| usage());
    let line = nums.first().copied();
    let col = nums.get(1).copied();
    Args {
        cmd,
        json,
        stdin,
        write,
        check,
        path,
        line,
        col,
    }
}

fn read_source(path: &Path, stdin: bool) -> Result<String, String> {
    if stdin {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        return Ok(buf);
    }
    std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

fn emit_diagnostics(diags: &[rosegold::Diagnostic], json: bool) -> i32 {
    if json {
        match serde_json::to_string_pretty(diags) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("json encode failed: {e}");
                return 2;
            }
        }
    } else {
        for d in diags {
            eprintln!("{d}");
        }
    }
    if diags.is_empty() { 0 } else { 1 }
}

fn cmd_check(path: &Path, json: bool, stdin: bool) -> i32 {
    let diags = if stdin {
        match read_source(path, true) {
            Ok(src) => check_source_at(&src, path),
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        }
    } else {
        check_file(path)
    };
    emit_diagnostics(&diags, json)
}

fn cmd_run(path: &Path, json: bool) -> i32 {
    let result = rosegold::run_file(path);
    if json {
        let payload = serde_json::json!({
          "ok": result.ok,
          "stdout": result.stdout,
          "stderr": result.stderr,
          "message": result.message,
        });
        println!("{payload}");
    } else {
        print!("{}", result.stdout);
        if !result.stderr.is_empty() {
            eprint!("{}", result.stderr);
            if !result.stderr.ends_with('\n') {
                eprintln!();
            }
        }
        if !result.ok && result.stderr.is_empty() {
            eprintln!("{}", result.message);
        }
    }
    if result.ok { 0 } else { 1 }
}

fn cmd_test(path: &Path, json: bool) -> i32 {
    if let Err(e) = std::fs::read_to_string(path) {
        eprintln!("failed to read {}: {e}", path.display());
        return 2;
    }
    let result = rosegold::run_tests_file(path);
    if json {
        let payload = serde_json::json!({
          "ok": result.ok,
          "stdout": result.stdout,
          "stderr": result.stderr,
          "message": result.message,
        });
        println!("{payload}");
    } else {
        print!("{}", result.stdout);
        if !result.stderr.is_empty() {
            eprint!("{}", result.stderr);
            if !result.stderr.ends_with('\n') {
                eprintln!();
            }
        }
        if result.ok {
            if result.stdout.is_empty() {
                eprintln!("{}", result.message);
            }
        } else if result.stderr.is_empty() {
            eprintln!("{}", result.message);
        }
    }
    if result.ok { 0 } else { 1 }
}

fn resolve_symbol_file(info: &SymbolInfo, from: &Path) -> PathBuf {
    let raw = &info.file;
    let p = Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let dir = from.parent().unwrap_or(Path::new("."));
    let name = if raw.ends_with(".rg") {
        raw.clone()
    } else {
        format!("{raw}.rg")
    };
    let nested = raw.replace('.', std::path::MAIN_SEPARATOR_STR);
    let nested = if nested.ends_with(".rg") {
        nested
    } else {
        format!("{nested}.rg")
    };
    let candidates = [
        dir.join(&name),
        dir.join(&nested),
        dir.join(raw.replace('.', "/")).join("lib.rg"),
    ];
    for cand in candidates {
        if cand.exists() {
            return cand.canonicalize().unwrap_or(cand);
        }
    }
    if p.exists() {
        return p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    }
    if let Some(stdlib) = stdlib_rg_path(raw, from) {
        return stdlib;
    }
    from.to_path_buf()
}

fn stdlib_rg_path(name: &str, from: &Path) -> Option<PathBuf> {
    let stem = rosegold::stdlib::canonical_module(name)?;
    let file = format!("{stem}.rg");
    let mut dir = from.parent().unwrap_or(from).to_path_buf();
    for _ in 0..14 {
        let cand = dir.join("crates/rosegold/stdlib").join(&file);
        if cand.exists() {
            return Some(cand.canonicalize().unwrap_or(cand));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn symbol_payload(info: Option<SymbolInfo>, from: &Path) -> serde_json::Value {
    let Some(info) = info else {
        return serde_json::json!({ "hover": null, "definition": null });
    };
    let path = resolve_symbol_file(&info, from);
    let line0 = info.line.saturating_sub(1);
    let col0 = info.col.saturating_sub(1);
    let end = col0 + info.name.chars().count() as u32;
    let range = serde_json::json!({
      "start": { "line": line0, "character": col0 },
      "end": { "line": line0, "character": end }
    });
    let mut contents = format!("```rosegold\n{}\n```\n", info.signature);
    if let Some(doc) = info.doc.as_deref().filter(|s| !s.is_empty()) {
        contents.push('\n');
        contents.push_str(doc);
        contents.push('\n');
    }
    serde_json::json!({
      "hover": {
        "contents": contents,
        "kind": info.kind,
        "range": range
      },
      "definition": {
        "path": path.to_string_lossy(),
        "range": range
      }
    })
}

fn cmd_navigate(kind: &str, path: &Path, json: bool, stdin: bool, line: u32, col: u32) -> i32 {
    let source = match read_source(path, stdin) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("script.rg");
    let dir = path.parent().unwrap_or(Path::new("."));
    let modules = sibling_modules(dir, Some(label));
    let info = if kind == "hover" {
        hover_at(&source, label, line, col, modules)
    } else {
        def_at(&source, label, line, col, modules)
    };
    let payload = symbol_payload(info, path);
    if json {
        println!("{payload}");
    } else if kind == "hover" {
        if let Some(h) = payload.get("hover") {
            if h.is_null() {
                eprintln!("(no symbol)");
            } else if let Some(s) = h.get("contents").and_then(|c| c.as_str()) {
                print!("{s}");
            }
        }
    } else if let Some(d) = payload.get("definition") {
        if d.is_null() {
            eprintln!("(no definition)");
        } else {
            println!(
                "{}:{}:{}",
                d["path"].as_str().unwrap_or(""),
                d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1,
                d["range"]["start"]["character"].as_u64().unwrap_or(0) + 1
            );
        }
    }
    0
}

fn cmd_fmt(path: &Path, stdin: bool, write: bool, check: bool) -> i32 {
    let source = match read_source(path, stdin) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let formatted = match rosegold::format_source(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if check {
        if formatted == source {
            return 0;
        }
        eprintln!("would reformat {}", path.display());
        return 1;
    }
    if write && !stdin {
        if let Err(e) = std::fs::write(path, &formatted) {
            eprintln!("failed to write {}: {e}", path.display());
            return 2;
        }
        return 0;
    }
    print!("{formatted}");
    0
}

fn main() -> ExitCode {
    let mut argv = env::args();
    let _bin = argv.next();
    let args = parse_args(argv);
    if !args.stdin && !args.path.exists() {
        eprintln!("file not found: {}", args.path.display());
        return ExitCode::from(2);
    }
    let code = match args.cmd.as_str() {
        "check" => cmd_check(&args.path, args.json, args.stdin),
        "run" => cmd_run(&args.path, args.json),
        "test" => cmd_test(&args.path, args.json),
        "fmt" => cmd_fmt(&args.path, args.stdin, args.write, args.check),
        "hover" | "def" => {
            let (Some(line), Some(col)) = (args.line, args.col) else {
                eprintln!("{} requires <file> <line> <col> (1-based)", args.cmd);
                usage();
            };
            cmd_navigate(&args.cmd, &args.path, args.json, args.stdin, line, col)
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
        }
    };
    ExitCode::from(code as u8)
}
