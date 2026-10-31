//! ソースの文字列走査。`use` と完全修飾パスから、依存先のモジュール・外部 crate・I/O の使用を抽出する。
//!
//! `syn` などの依存を増やさないため、字句レベルの近似である。マクロ展開や型推論は見えない
//! （例: `use` せずに `.exists()` を持つ独自型を呼ぶと誤検知しうる）。誤検知は検査側を直す。

use std::collections::BTreeSet;

/// ファイルの使用状況
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// `crate::` 配下への参照（`crate::` を除いたパス。例: `model::ClientTarget`）
    pub crate_paths: BTreeSet<String>,
    /// 同じ workspace の別 crate への参照。crate の短い名前を先頭につけたパス（例: `core::domain::ClientTarget`）
    pub member_paths: BTreeSet<String>,
    /// 外部 crate 名と `std::fs` などの std のサブモジュール（例: `egui`、`std::fs`）
    pub externals: BTreeSet<String>,
    /// ファイルシステムを触るメソッド呼び出し（例: `.is_file()`）
    pub io_calls: BTreeSet<String>,
}

/// workspace の crate（`use` に現れる名前 → 検査で使う短い名前）
const MEMBER_CRATES: [(&str, &str); 2] = [("clp2cfg_core", "core"), ("clp2cfg_cli", "cli")];

/// ファイルシステムの状態を読むメソッド。型を見ないため、名前だけで判定する。
const IO_CALLS: [&str; 4] = [".is_file(", ".is_dir(", ".exists(", ".metadata("];

/// `#[cfg(test)]` より前（本体部分）。テストは依存方向の検査対象にしない。
pub fn production_part(source: &str) -> &str {
    source
        .find("#[cfg(test)]")
        .map_or(source, |index| &source[..index])
}

/// 本体部分を走査して使用状況を返す。
pub fn analyze(source: &str) -> Usage {
    let masked = mask_comments_and_literals(production_part(source));
    let mut usage = Usage::default();

    let remainder = extract_use_statements(&masked, &mut usage);
    for path in path_sequences(&remainder) {
        record(&path, &mut usage);
    }
    for call in IO_CALLS {
        if remainder.contains(call) {
            usage.io_calls.insert(format!("{call})"));
        }
    }
    usage
}

/// コメントと文字列・文字リテラルの中身を空白に置き換える（改行は残す）。
fn mask_comments_and_literals(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let blank = |out: &mut String, c: char| out.push(if c == '\n' { '\n' } else { ' ' });

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, chars[i]);
                    i += 1;
                }
            }
        } else if c == 'r' && is_raw_string_start(&chars, i) {
            let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
            let terminator: Vec<char> = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            let body_start = i + 1 + hashes + 1;
            let end = (body_start..chars.len())
                .find(|&j| chars[j..].starts_with(&terminator))
                .map_or(chars.len(), |j| j + terminator.len());
            chars[i..end].iter().for_each(|&ch| blank(&mut out, ch));
            i = end;
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                let skip = if chars[i] == '\\' { 2 } else { 1 };
                for &ch in chars.iter().skip(i).take(skip) {
                    blank(&mut out, ch);
                }
                i += skip;
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' {
            // 文字リテラル（'a' / '\n'）か、ライフタイム（'static）か
            let literal_len = match (next, chars.get(i + 2)) {
                // エスケープされた文字（`'\''` の 2 つ目の `'` を含む）の次から閉じの `'` を探す
                (Some('\\'), _) => chars
                    .get(i + 3..)
                    .and_then(|rest| rest.iter().position(|&ch| ch == '\''))
                    .map(|p| p + 4),
                (Some(_), Some('\'')) => Some(3),
                _ => None,
            };
            match literal_len {
                Some(len) => {
                    out.push_str(&" ".repeat(len));
                    i += len;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `r"..."` / `r#"..."#` の開始か（識別子の途中の `r` ではない）。
fn is_raw_string_start(chars: &[char], i: usize) -> bool {
    let preceded_by_ident = i > 0 && is_ident_char(chars[i - 1]);
    let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
    !preceded_by_ident && chars.get(i + 1 + hashes) == Some(&'"')
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `use ...;` を取り出して使用状況に記録し、それらを空白にした残りの本文を返す。
fn extract_use_statements(masked: &str, usage: &mut Usage) -> String {
    let chars: Vec<char> = masked.chars().collect();
    let mut remainder = String::with_capacity(masked.len());
    let mut i = 0;
    while i < chars.len() {
        let is_use_keyword = chars[i..].starts_with(&['u', 's', 'e'])
            && (i == 0 || !is_ident_char(chars[i - 1]))
            && chars.get(i + 3).is_some_and(|c| c.is_whitespace());
        if !is_use_keyword {
            remainder.push(chars[i]);
            i += 1;
            continue;
        }
        let end = chars[i..]
            .iter()
            .position(|&c| c == ';')
            .map_or(chars.len(), |p| i + p + 1);
        let statement: String = chars[i + 3..end].iter().collect();
        for path in flatten_use_tree(&statement) {
            record(&path, usage);
        }
        chars[i..end]
            .iter()
            .for_each(|&c| remainder.push(if c == '\n' { '\n' } else { ' ' }));
        i = end;
    }
    remainder
}

/// `use a::{b, c::{d, e}}` を `a::b` / `a::c::d` / `a::c::e` に展開する。
fn flatten_use_tree(statement: &str) -> Vec<Vec<String>> {
    let tokens = tokenize_use(statement);
    let mut paths = Vec::new();
    let mut pos = 0;
    parse_use_tree(&tokens, &mut pos, &[], &mut paths);
    paths
}

fn tokenize_use(statement: &str) -> Vec<String> {
    let chars: Vec<char> = statement.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_ident_char(c) {
            let end = (i..chars.len())
                .find(|&j| !is_ident_char(chars[j]))
                .unwrap_or(chars.len());
            tokens.push(chars[i..end].iter().collect());
            i = end;
        } else if c == ':' && chars.get(i + 1) == Some(&':') {
            tokens.push("::".to_string());
            i += 2;
        } else {
            if matches!(c, '{' | '}' | ',' | '*') {
                tokens.push(c.to_string());
            }
            i += 1;
        }
    }
    tokens
}

fn parse_use_tree(
    tokens: &[String],
    pos: &mut usize,
    prefix: &[String],
    paths: &mut Vec<Vec<String>>,
) {
    let mut path = prefix.to_vec();
    loop {
        match tokens.get(*pos).map(String::as_str) {
            Some("{") => {
                *pos += 1;
                loop {
                    parse_use_tree(tokens, pos, &path, paths);
                    match tokens.get(*pos).map(String::as_str) {
                        Some(",") => *pos += 1,
                        Some("}") => {
                            *pos += 1;
                            break;
                        }
                        _ => break,
                    }
                }
                return;
            }
            Some("*") => {
                *pos += 1;
                paths.push(path);
                return;
            }
            Some(ident) if ident != "::" && ident != "," && ident != "}" => {
                path.push(ident.to_string());
                *pos += 1;
                if tokens.get(*pos).map(String::as_str) == Some("::") {
                    *pos += 1;
                    continue;
                }
                if tokens.get(*pos).map(String::as_str) == Some("as") {
                    *pos += 2; // `as alias`
                }
                // `use a::{self, b}` の `self` は `a` 自身
                if path.len() > 1 && path.last().is_some_and(|last| last == "self") {
                    path.pop();
                }
                paths.push(path);
                return;
            }
            // 末尾のカンマの直後など。何も足さない
            _ => return,
        }
    }
}

/// 本文から `a::b::c` 形式のパス（`::` を 1 つ以上含むもの）を取り出す。
fn path_sequences(body: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = body.chars().collect();
    let mut sequences = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !(is_ident_char(chars[i]) && (i == 0 || !is_ident_char(chars[i - 1]))) {
            i += 1;
            continue;
        }
        let mut segments = Vec::new();
        let mut j = i;
        loop {
            let end = (j..chars.len())
                .find(|&k| !is_ident_char(chars[k]))
                .unwrap_or(chars.len());
            segments.push(chars[j..end].iter().collect::<String>());
            j = end;
            let has_next = chars.get(j) == Some(&':')
                && chars.get(j + 1) == Some(&':')
                && chars.get(j + 2).is_some_and(|&c| is_ident_char(c));
            if !has_next {
                break;
            }
            j += 2;
        }
        if segments.len() > 1 {
            sequences.push(segments);
        }
        i = j.max(i + 1);
    }
    sequences
}

fn record(path: &[String], usage: &mut Usage) {
    match path.first().map(String::as_str) {
        None | Some("self" | "super") => {}
        Some("crate") => {
            if path.len() > 1 {
                usage.crate_paths.insert(path[1..].join("::"));
            }
        }
        Some(root @ ("std" | "core" | "alloc")) => {
            if let Some(second) = path.get(1) {
                usage.externals.insert(format!("{root}::{second}"));
            }
        }
        Some(root) => match MEMBER_CRATES.iter().find(|(name, _)| *name == root) {
            Some((_, short)) => {
                if path.len() > 1 {
                    usage
                        .member_paths
                        .insert(format!("{short}::{}", path[1..].join("::")));
                }
            }
            None => {
                usage.externals.insert(root.to_string());
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_use_forms_are_flattened() {
        let cases: [(&str, &[&str], &[&str]); 7] = [
            (
                "use crate::model::ClientTarget;",
                &["model::ClientTarget"],
                &[],
            ),
            (
                "use crate::model::{ClientTarget, McpServersMap};",
                &["model::ClientTarget", "model::McpServersMap"],
                &[],
            ),
            (
                "use crate::{\n    model::{A, B},\n    parser::parse,\n};",
                &["model::A", "model::B", "parser::parse"],
                &[],
            ),
            (
                "use std::{fs, io::Write, path::{Path, PathBuf}};",
                &[],
                &["std::fs", "std::io", "std::path"],
            ),
            ("use egui::Color32 as C;", &[], &["egui"]),
            ("pub use crate::model::*;", &["model"], &[]),
            ("use super::*;\nuse self::x::y;", &[], &[]),
        ];

        for (source, crate_paths, externals) in cases {
            let usage = analyze(source);
            assert_eq!(usage.crate_paths, set(crate_paths), "{source}");
            assert_eq!(usage.externals, set(externals), "{source}");
        }
    }

    #[test]
    fn test_fully_qualified_paths_and_io_calls_are_found() {
        let source = r#"
            fn f(p: &Path) {
                let _ = std::fs::read_to_string(p);
                let _ = arboard::Clipboard::new();
                let _ = crate::writer::backup_path_for(p);
                if p.is_file() {}
            }
        "#;

        let usage = analyze(source);

        assert_eq!(usage.crate_paths, set(&["writer::backup_path_for"]));
        assert_eq!(usage.externals, set(&["std::fs", "arboard"]));
        assert_eq!(usage.io_calls, set(&[".is_file()"]));
    }

    #[test]
    fn test_other_workspace_crates_are_not_externals() {
        let source = r#"
            use clp2cfg_core::apply::{plan_target, TargetIo};
            use clp2cfg_cli::args::Args;
            fn f() { let _ = clp2cfg_core::writer::is_file(p); egui::Ui; }
        "#;

        let usage = analyze(source);

        assert_eq!(
            usage.member_paths,
            set(&[
                "core::apply::plan_target",
                "core::apply::TargetIo",
                "cli::args::Args",
                "core::writer::is_file"
            ])
        );
        assert_eq!(usage.externals, set(&["egui"]));
        assert!(usage.crate_paths.is_empty());
    }
    #[test]
    fn test_comments_literals_and_tests_are_ignored() {
        let source = r##"
            // use egui::Ui; std::fs::read
            /* use clap::Parser; /* nested std::process */ */
            /// `crate::app` in a doc comment
            fn f() -> &'static str {
                let c = 'x';
                let e = '\'';
                let _ = "use eframe::App; std::env::var";
                let _ = r#"use dirs::home_dir; "quoted" .exists("#;
                "ok"
            }

            #[cfg(test)]
            mod tests {
                use egui::Ui;
                fn t() { std::fs::read("x").unwrap(); }
            }
        "##;

        assert_eq!(analyze(source), Usage::default());
    }

    #[test]
    fn test_word_boundaries_do_not_match_identifiers() {
        let source = "fn reuse() { let user = 1; let _ = misuse::go(); }";

        let usage = analyze(source);

        assert_eq!(usage.externals, set(&["misuse"]));
        assert!(usage.crate_paths.is_empty());
    }
}
