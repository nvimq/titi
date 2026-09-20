// The regexes below are compile-time literals; a bad pattern is a bug the unit
// tests catch, not a runtime condition worth threading through callers.
#![allow(clippy::expect_used)]

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Other,
}

impl Language {
    pub fn from_path(path: &str) -> Self {
        match Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
        {
            "rs" => Self::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Self::TypeScript,
            "py" => Self::Python,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub exports: Vec<String>,
    pub imports: Vec<String>,
}

pub fn parse(path: &str, source: &str, files: &std::collections::HashSet<String>) -> ParsedFile {
    match Language::from_path(path) {
        Language::Rust => parse_rust(path, source, files),
        Language::TypeScript => parse_typescript(path, source, files),
        Language::Python => parse_python(path, source, files),
        Language::Other => ParsedFile {
            exports: Vec::new(),
            imports: Vec::new(),
        },
    }
}

fn parse_rust(path: &str, source: &str, files: &std::collections::HashSet<String>) -> ParsedFile {
    static EXPORTS: OnceLock<Regex> = OnceLock::new();
    static USES: OnceLock<Regex> = OnceLock::new();
    static MODS: OnceLock<Regex> = OnceLock::new();
    let exports_re = EXPORTS.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*pub(?:\s*\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("exports regex")
    });
    let uses_re =
        USES.get_or_init(|| Regex::new(r"(?m)^\s*(?:pub\s+)?use\s+([^;{]+)").expect("use regex"));
    let mods_re = MODS.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;").expect("mod regex")
    });

    let mut exports: Vec<String> = exports_re
        .captures_iter(source)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
        .collect();
    exports.sort();
    exports.dedup();

    let mut imports = Vec::new();
    for cap in uses_re.captures_iter(source) {
        let raw = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if let Some(resolved) = resolve_rust_use(path, raw, files) {
            imports.push(resolved);
        }
    }
    for cap in mods_re.captures_iter(source) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Some(resolved) = resolve_child_module(path, name, files) {
            imports.push(resolved);
        }
    }
    imports.sort();
    imports.dedup();
    ParsedFile { exports, imports }
}

fn parse_typescript(
    path: &str,
    source: &str,
    files: &std::collections::HashSet<String>,
) -> ParsedFile {
    static EXPORTS: OnceLock<Regex> = OnceLock::new();
    static IMPORTS: OnceLock<Regex> = OnceLock::new();
    let exports_re = EXPORTS.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*export\s+(?:default\s+)?(?:async\s+)?(?:function|class|const|let|var|enum|type|interface)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("ts exports")
    });
    let imports_re = IMPORTS.get_or_init(|| {
        Regex::new(r#"(?m)(?:from|import)\s+['"](\.[^'"]+)['"]"#).expect("ts imports")
    });
    let mut exports: Vec<String> = exports_re
        .captures_iter(source)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
        .collect();
    exports.sort();
    exports.dedup();
    let mut imports = Vec::new();
    for cap in imports_re.captures_iter(source) {
        let spec = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Some(resolved) = resolve_relative(path, spec, files, &["ts", "tsx", "js", "jsx"]) {
            imports.push(resolved);
        }
    }
    imports.sort();
    imports.dedup();
    ParsedFile { exports, imports }
}

fn parse_python(path: &str, source: &str, files: &std::collections::HashSet<String>) -> ParsedFile {
    static EXPORTS: OnceLock<Regex> = OnceLock::new();
    static IMPORTS: OnceLock<Regex> = OnceLock::new();
    let exports_re = EXPORTS.get_or_init(|| {
        Regex::new(r"(?m)^(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)").expect("py exports")
    });
    let imports_re = IMPORTS.get_or_init(|| {
        Regex::new(r"(?m)^from\s+(\.+[A-Za-z0-9_\.]*)\s+import").expect("py imports")
    });
    let mut exports: Vec<String> = exports_re
        .captures_iter(source)
        .filter_map(|cap| cap.get(2).map(|m| m.as_str().to_owned()))
        .collect();
    exports.sort();
    exports.dedup();
    let mut imports = Vec::new();
    for cap in imports_re.captures_iter(source) {
        let spec = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Some(resolved) = resolve_python_relative(path, spec, files) {
            imports.push(resolved);
        }
    }
    imports.sort();
    imports.dedup();
    ParsedFile { exports, imports }
}

fn resolve_rust_use(
    from: &str,
    raw: &str,
    files: &std::collections::HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("crate::") {
        let segs: Vec<&str> = raw.trim_start_matches("crate::").split("::").collect();
        return resolve_from_dir(&crate_root(from, files), &segs, files);
    }
    if raw.starts_with("super::") {
        let mut depth = 0;
        let mut rest = raw;
        while let Some(stripped) = rest.strip_prefix("super::") {
            depth += 1;
            rest = stripped;
        }
        let mut dir = parent(from)?;
        for _ in 0..depth {
            dir = parent(dir)?;
        }
        let segs: Vec<&str> = rest.split("::").collect();
        return resolve_from_dir(dir, &segs, files);
    }
    if let Some(rest) = raw.strip_prefix("self::") {
        let segs: Vec<&str> = rest.split("::").collect();
        return resolve_from_dir(parent(from).unwrap_or(""), &segs, files);
    }
    None
}

fn resolve_child_module(
    from: &str,
    name: &str,
    files: &std::collections::HashSet<String>,
) -> Option<String> {
    let dir = if from.ends_with("/lib.rs")
        || from.ends_with("/main.rs")
        || from.ends_with("lib.rs")
        || from.ends_with("main.rs")
    {
        parent(from).unwrap_or("")
    } else if let Some(stem) = from.strip_suffix(".rs") {
        stem
    } else {
        parent(from).unwrap_or("")
    };
    resolve_from_dir(dir, &[name], files)
}

fn crate_root(from: &str, files: &std::collections::HashSet<String>) -> String {
    let mut dir = parent(from).unwrap_or("").to_owned();
    loop {
        if files.contains(&join(&dir, "lib.rs")) || files.contains(&join(&dir, "main.rs")) {
            return dir;
        }
        match parent(&dir) {
            Some(parent_dir) => dir = parent_dir.to_owned(),
            None => return parent(from).unwrap_or("").to_owned(),
        }
    }
}

fn resolve_from_dir(
    dir: &str,
    segs: &[&str],
    files: &std::collections::HashSet<String>,
) -> Option<String> {
    if segs.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = segs
        .iter()
        .copied()
        .filter(|seg| !seg.is_empty() && *seg != "*")
        .collect();
    while !parts.is_empty() {
        let rel = if dir.is_empty() {
            parts.join("/")
        } else {
            format!("{dir}/{}", parts.join("/"))
        };
        for candidate in [
            format!("{rel}.rs"),
            format!("{rel}/mod.rs"),
            format!("{rel}.ts"),
            format!("{rel}.tsx"),
            format!("{rel}.js"),
            format!("{rel}/index.ts"),
            format!("{rel}.py"),
            format!("{rel}/__init__.py"),
        ] {
            if files.contains(&candidate) {
                return Some(candidate);
            }
        }
        parts.pop();
    }
    None
}

fn resolve_relative(
    from: &str,
    spec: &str,
    files: &std::collections::HashSet<String>,
    exts: &[&str],
) -> Option<String> {
    let from_dir = parent(from).unwrap_or("");
    let joined = normalize_join(from_dir, spec)?;
    for ext in exts {
        let file = format!("{joined}.{ext}");
        if files.contains(&file) {
            return Some(file);
        }
        let index = format!("{joined}/index.{ext}");
        if files.contains(&index) {
            return Some(index);
        }
    }
    if files.contains(&joined) {
        return Some(joined);
    }
    None
}

fn resolve_python_relative(
    from: &str,
    spec: &str,
    files: &std::collections::HashSet<String>,
) -> Option<String> {
    let mut rest = spec;
    let mut dir = parent(from).unwrap_or("");
    while rest.starts_with('.') {
        rest = &rest[1..];
        if rest.starts_with('.') {
            dir = parent(dir).unwrap_or("");
        }
    }
    if rest.is_empty() {
        return None;
    }
    let segs: Vec<&str> = rest.split('.').collect();
    resolve_from_dir(dir, &segs, files)
}

fn parent(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(head, _)| head)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

fn normalize_join(dir: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}
