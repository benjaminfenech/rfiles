//! The UI's source language is English (`t!("New folder")`); other languages, Turkish among
//! them, live in `locales/<code>.json`. These tests read every crate's source and fail when
//!
//! - a string literal outside a test module looks Turkish: it has a Turkish letter, or a word
//!   that only Turkish texts in `locales/tr.json` use (catches "Dosya zaten var" too). Such a
//!   text reaches the UI untranslated; write it in English (and add its Turkish to tr.json).
//!   A line that means it (a Turkish font sample, Turkish casing rules) says `i18n-ok`.
//! - a `t!("…")` text isn't listed in `locales/_source.json`, so no translator sees it
//!   (`python scripts/i18n_check.py --add` lists it).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Words that Turkish texts happen to use and English code does too.
const ENGLISH: &[&str] = &["render", "var"];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if p.is_dir() {
            // build output, integration tests, and examples (benchmarks print what they like)
            if !["target", "tests", "examples"].contains(&name.as_str()) {
                rust_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// A plain string literal: its byte offset and its text as written (escapes not undone).
struct Lit<'a> {
    at: usize,
    raw: &'a str,
}

/// The plain "…" literals of a Rust file, outside comments; raw and byte strings are skipped.
fn literals(src: &str) -> Vec<Lit<'_>> {
    let b = src.as_bytes();
    let (mut i, mut out) = (0, vec![]);
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        let c = b[i];
        if src[i..].starts_with("//") {
            i = src[i..].find('\n').map_or(b.len(), |n| i + n);
        } else if src[i..].starts_with("/*") {
            let (mut depth, mut j) = (1, i + 2);
            while j < b.len() && depth > 0 {
                if src[j..].starts_with("/*") {
                    depth += 1;
                    j += 2;
                } else if src[j..].starts_with("*/") {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            i = j;
        } else if ident(c) {
            let start = i;
            while i < b.len() && ident(b[i]) {
                i += 1;
            }
            let word = &src[start..i];
            if (word == "r" || word == "br") && i < b.len() && (b[i] == b'"' || b[i] == b'#') {
                // raw string: r"…", r#"…"#
                let hashes = src[i..].bytes().take_while(|&c| c == b'#').count();
                let close = format!("\"{}", "#".repeat(hashes));
                let body = i + hashes + 1;
                i = src[body..].find(&close).map_or(b.len(), |n| body + n + close.len());
            } else if word == "b" && i < b.len() && b[i] == b'"' {
                i = skip_string(b, i).1;
            }
        } else if c == b'"' {
            let (body, end) = skip_string(b, i);
            out.push(Lit { at: i, raw: &src[body..end - 1] });
            i = end;
        } else if c == b'\'' {
            // a char literal ('a', '\n', '\u{2026}', 'ç') or a lifetime ('a)
            let rest = &src[i + 1..];
            let len = if rest.starts_with('\\') {
                rest.find('\'').map(|n| n + 1)
            } else {
                rest.chars().next().map(|ch| ch.len_utf8()).filter(|&n| rest[n..].starts_with('\''))
            };
            i += len.map_or(1, |n| n + 2);
        } else {
            i += 1;
        }
    }
    out
}

/// From an opening quote: (offset of the text, offset after the closing quote).
fn skip_string(b: &[u8], open: usize) -> (usize, usize) {
    let mut j = open + 1;
    while j < b.len() && b[j] != b'"' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    (open + 1, j + 1)
}

/// Byte ranges of `#[cfg(test)] mod … { … }` blocks (to their closing brace at column 0).
fn test_modules(src: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(n) = src[from..].find("\n#[cfg(test)]\n") {
        let start = from + n;
        let next = &src[start + 14..];
        let is_mod = ["mod ", "pub mod ", "pub(crate) mod "].iter().any(|p| next.starts_with(p));
        let end = src[start..].find("\n}\n").map_or(src.len(), |e| start + e + 2);
        if is_mod {
            out.push((start, end));
        }
        from = start + 1;
    }
    out
}

/// Words of a text, its `{placeholders}` left out.
fn words(text: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

fn locale(name: &str) -> serde_json::Value {
    let path = crates_dir().join("rf-app/locales").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn source_texts() -> HashSet<String> {
    locale("_source.json")
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e.get("key").is_none())
        .map(|e| e["en"].as_str().unwrap().to_string())
        .collect()
}

/// Words that Turkish texts use and no English text does (3 letters or more).
fn turkish_words() -> HashSet<String> {
    let english: HashSet<String> = source_texts().iter().flat_map(|t| words(t)).chain(ENGLISH.iter().map(|w| w.to_string())).collect();
    locale("tr.json")
        .as_object()
        .unwrap()
        .values()
        .flat_map(|v| words(v.as_str().unwrap()))
        .filter(|w| w.chars().count() >= 3 && !english.contains(w))
        .collect()
}

fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut it = raw.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = it.by_ref().skip(1).take_while(|&c| c != '}').collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Each non-test literal of every crate, with its file and line.
fn ui_literals() -> Vec<(String, usize, String, String)> {
    let mut files = vec![];
    rust_files(&crates_dir(), &mut files);
    files.sort();
    let mut out = vec![];
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let tests = test_modules(&src);
        let rel = f.strip_prefix(crates_dir()).unwrap().display().to_string();
        for lit in literals(&src) {
            if tests.iter().any(|&(a, b)| a < lit.at && lit.at < b) {
                continue;
            }
            let line_no = src[..lit.at].matches('\n').count();
            let line = src.lines().nth(line_no).unwrap_or_default().to_string();
            out.push((rel.clone(), line_no + 1, unescape(lit.raw), line));
        }
    }
    out
}

#[test]
fn no_turkish_literals_outside_the_translations() {
    let turkish = turkish_words();
    assert!(turkish.len() > 500, "tr.json gave only {} Turkish words", turkish.len());
    let mut bad = vec![];
    for (file, line, text, src_line) in ui_literals() {
        if src_line.contains("i18n-ok") {
            continue;
        }
        let letters = text.chars().any(|c| "çğıöşüÇĞİÖŞÜ".contains(c));
        // single tokens like "rfiles.desktop", "kio", "app-path=" are names, not words
        let token = !text.contains(' ') && (text.contains(|c| "_.:/\\=-".contains(c)) || text.chars().all(|c| !c.is_uppercase()));
        let hits: Vec<String> = if token { vec![] } else { words(&text).into_iter().filter(|w| turkish.contains(w)).collect() };
        if letters || !hits.is_empty() {
            bad.push(format!("{file}:{line}: {text:?} {}", hits.join(" ")));
        }
    }
    assert!(
        bad.is_empty(),
        "Turkish-looking strings outside the translation table; write them in English and add the \
         Turkish to locales/tr.json (or mark the line `i18n-ok`):\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_t_text_is_listed_for_translators() {
    let listed = source_texts();
    let mut missing = vec![];
    for (file, line, text, src_line) in ui_literals() {
        let quoted = format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t"));
        let call = format!("t!({quoted}");
        // t!("…"), not format!("…")
        let in_t = src_line.match_indices(&call).any(|(i, _)| !src_line[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
        if in_t && !src_line.trim_start().starts_with("//") && !listed.contains(&text) {
            missing.push(format!("{file}:{line}: {text:?}"));
        }
    }
    assert!(missing.is_empty(), "t!() texts missing from locales/_source.json (run `python scripts/i18n_check.py --add`):\n{}", missing.join("\n"));
}

#[test]
fn the_scanner_finds_what_it_should() {
    let src = "let a = t!(\"Yes\"); // \"yorum\"\nlet b = r#\"raw \"x\"#; let c = 'ç'; let d = b\"bytes\";\nfn f<'a>(x: &'a str) { g(\"Dosya \\\"zaten\\\" var\") }\n#[cfg(test)]\nmod tests {\n    const X: &str = \"test\";\n}\nconst Y: &str = \"after\";\n";
    let lits: Vec<&str> = literals(src).iter().map(|l| l.raw).collect();
    assert_eq!(lits, ["Yes", "Dosya \\\"zaten\\\" var", "test", "after"]);
    let tests = test_modules(src);
    let in_test: Vec<bool> = literals(src).iter().map(|l| tests.iter().any(|&(a, b)| a < l.at && l.at < b)).collect();
    assert_eq!(in_test, [false, false, true, false]);
    assert_eq!(words("Couldn't read {name}: {e}"), ["couldn", "t", "read"]);
    assert_eq!(unescape("a\\\"b\\u{2026}"), "a\"b…");
}
