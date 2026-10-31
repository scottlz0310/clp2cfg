// `tests/` 直下に置くと別のテスト crate として扱われるため、サブディレクトリに置いて指定する
#[path = "architecture_test/scanner.rs"]
mod scanner;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// エントリポイントのファイル（GUI 版とコンソール版）
const ENTRY_POINTS: [&str; 2] = ["src/main.rs", "src/bin/clp2cfg-cli.rs"];

#[test]
fn test_main_stays_thin() {
    for path in ENTRY_POINTS {
        let content =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        let line_count = content.lines().count();

        // エントリポイントは起動配線のみに留め、ロジックの混入を機械的に防ぐ（80行以下）
        assert!(
            line_count <= 80,
            "{path} is too fat ({line_count} lines > 80 lines). Move logic to modules or src/entry.rs!"
        );
    }
}

#[test]
fn test_main_has_no_business_logic() {
    for path in ENTRY_POINTS {
        let main_content =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        assert_no_business_logic(path, &main_content);
    }
}

fn assert_no_business_logic(path: &str, main_content: &str) {
    // 禁止キーワード: main.rs に直書きしてはならないコア業務ロジックや I/O
    let forbidden_patterns = [
        "std::fs::write",
        "std::fs::read",
        "parse_mcp_snippet",
        "merge_mcp_servers",
        "safe_atomic_write",
        "detect_client_targets",
        "apply_filters",
    ];

    for pattern in forbidden_patterns {
        assert!(
            !main_content.contains(pattern),
            "{path} contains forbidden business logic or I/O direct call: '{pattern}'. Move this logic to appropriate modules!"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// モジュール間の依存方向（ラチェット）
//
// 層の定義は #57 の「目標構造」。`src/**/*.rs` の本体部分（`#[cfg(test)]` より前）を字句レベルで
// 走査して検査する（走査の限界は `architecture_test/scanner.rs` の冒頭を参照）。
// 外部 crate の使用の検査は、Phase 2（#68）で crate の境界に置き換わる。crate 内の層の検査は残す。
//
// 現状の違反は `ALLOWLIST` に固定する。リストに無い違反は失敗し、違反が解消されたのにリストに
// 残っている行も失敗する（解消した PR が行を削除する。ラチェットが戻らない）。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    /// L0: ドメイン型
    Domain,
    /// L1: 純粋ロジック（入出力・OS・UI に触れない）
    Pure,
    /// L2: 副作用（ファイル・プロセス・OS）
    Platform,
    /// L3: ユースケース（副作用は port 経由）
    Apply,
    /// L4: コマンドライン frontend
    CliFrontend,
    /// L4: GUI frontend
    GuiFrontend,
    /// L5: 配線
    Entry,
}

/// ファイル（`src/` からの相対パス。拡張子なし、区切りは `::`）→ 層。
/// 現在はフラットな配置のため、1 ファイルに複数の責務が混在するものは、主たる層に置いて
/// 混在の分を `ALLOWLIST` に載せている。ファイルの分割・移動に合わせてこの表を更新する。
const FILE_LAYERS: &[(&str, Layer)] = &[
    ("domain", Layer::Domain),
    ("domain::change", Layer::Domain),
    ("domain::client", Layer::Domain),
    ("domain::server", Layer::Domain),
    ("snippet", Layer::Pure),
    ("snippet::parser", Layer::Pure),
    ("snippet::filter", Layer::Pure),
    ("snippet::pipeline", Layer::Pure),
    ("snippet::placeholder", Layer::Pure),
    ("merge", Layer::Pure),
    ("delegate", Layer::Pure),
    ("delegate::plan", Layer::Pure),
    ("discovery", Layer::Platform),
    ("delegate::exec", Layer::Platform),
    ("writer", Layer::Platform),
    ("undo", Layer::Platform),
    ("preview", Layer::Apply),
    ("args", Layer::CliFrontend),
    ("headless", Layer::CliFrontend),
    ("app", Layer::GuiFrontend),
    ("entry", Layer::Entry),
];

/// 検査の対象外（宣言だけのファイル。エントリポイントは `ENTRY_POINTS` で別に見る）
const SKIPPED_FILES: &[&str] = &["lib", "main"];

/// 画面・引数・クリップボード・画像（frontend の道具）
const FRONTEND_CRATES: &[&str] = &["egui", "eframe", "clap", "arboard", "image"];

fn forbidden_externals(layer: Layer) -> Vec<&'static str> {
    // `.is_file()` などは、型を見ないため名前で判定する（`scanner::IO_CALLS`）
    let io_calls = [".is_file()", ".is_dir()", ".exists()", ".metadata()"];
    let os = ["dirs", "std::fs", "std::process", "std::env"];
    match layer {
        Layer::Domain | Layer::Pure => [FRONTEND_CRATES, &["tempfile"], &os, &io_calls].concat(),
        Layer::Platform => FRONTEND_CRATES.to_vec(),
        Layer::Apply => [FRONTEND_CRATES, &os].concat(),
        Layer::CliFrontend => vec!["egui", "eframe", "image"],
        Layer::GuiFrontend => vec!["clap"],
        Layer::Entry => vec![],
    }
}

/// 上の層だけが下の層に依存できる。frontend は副作用の層（L2）を直接呼ばず、ユースケース（L3）を通す。
fn may_depend_on(from: Layer, to: Layer) -> bool {
    use Layer::*;
    from == to
        || matches!(
            (from, to),
            (Pure, Domain)
                | (Platform, Domain | Pure)
                | (Apply, Domain | Pure | Platform)
                | (CliFrontend | GuiFrontend, Domain | Pure | Apply)
                | (Entry, _)
        )
}

/// 層の規則では表せない、ファイル単位の禁止（from, to, 理由）
const FORBIDDEN_EDGES: &[(&str, &str, &str)] = &[(
    "headless",
    "args",
    "ヘッドレス実行は clap のパース結果ではなく、要求の型を受ける",
)];

struct Allowed {
    /// `<ファイル> -> crate::<モジュール>` または `<ファイル> -> <外部 crate / std / 呼び出し>`
    key: &'static str,
    reason: &'static str,
    /// 解消する Issue
    issue: &'static str,
}

/// 現状の違反。解消した PR が、その行を削除する。
const ALLOWLIST: &[Allowed] = &[
    Allowed {
        key: "preview -> std::fs",
        reason: "プレビューの生成が既存ファイルを直接読む（port 経由にする）",
        issue: "#64",
    },
    Allowed {
        key: "app -> crate::writer",
        reason: "GUI が原子書き込みを直接呼ぶ（ユースケースを通す）",
        issue: "#64",
    },
    Allowed {
        key: "app -> crate::delegate::exec",
        reason: "GUI が CLI コマンドの実行を直接呼ぶ（ユースケースを通す）",
        issue: "#64",
    },
    Allowed {
        key: "app -> crate::undo",
        reason: "GUI が取り消しの復元を直接呼ぶ（適用結果の型と復元処理の置き場を決める）",
        issue: "#64",
    },
    Allowed {
        key: "app -> crate::discovery",
        reason: "GUI がクライアント検出を直接呼ぶ（検出は entry が行い、frontend へ渡す）",
        issue: "#66",
    },
    Allowed {
        key: "headless -> crate::writer",
        reason: "ヘッドレス実行が原子書き込みを直接呼ぶ（ユースケースを通す）",
        issue: "#64",
    },
    Allowed {
        key: "headless -> crate::delegate::exec",
        reason: "ヘッドレス実行が CLI コマンドの実行を直接呼ぶ（ユースケースを通す）",
        issue: "#64",
    },
    Allowed {
        key: "headless -> crate::discovery",
        reason:
            "ヘッドレス実行がクライアント検出を直接呼ぶ（検出は entry が行い、frontend へ渡す）",
        issue: "#65",
    },
    Allowed {
        key: "headless -> crate::args",
        reason: "ヘッドレス実行が clap のパース結果（Args）を直接受ける",
        issue: "#65",
    },
];

/// `src/` の対象ファイルを、キー（`app`、`frontend::gui::state` など）→ ソースで返す。
fn collect_sources(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // `src/bin/` はエントリポイント（`ENTRY_POINTS`）として別に見る
                if path.strip_prefix(root).is_ok_and(|p| p != Path::new("bin")) {
                    walk(root, &path, out);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path.strip_prefix(root).unwrap().with_extension("");
                let mut segments: Vec<String> = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if segments.last().is_some_and(|s| s == "mod") {
                    segments.pop();
                }
                let key = segments.join("::");
                if !SKIPPED_FILES.contains(&key.as_str()) {
                    let source = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    out.insert(key, source);
                }
            }
        }
    }

    let mut sources = BTreeMap::new();
    walk(dir, dir, &mut sources);
    sources
}

/// `crate::` 配下のパスを、最も長く一致するファイルのキーに解決する。
fn resolve_module<'a>(path: &str, keys: &BTreeSet<&'a str>) -> Option<&'a str> {
    let segments: Vec<&str> = path.split("::").collect();
    (1..=segments.len())
        .rev()
        .find_map(|n| keys.get(segments[..n].join("::").as_str()).copied())
}

/// 違反（キー → 説明）を数える。表とファイルの食い違いは `Err`。
fn find_violations(
    sources: &BTreeMap<String, String>,
    layers: &[(&str, Layer)],
    forbidden_edges: &[(&str, &str, &str)],
) -> Result<BTreeMap<String, String>, String> {
    let layer_of: BTreeMap<&str, Layer> = layers.iter().copied().collect();
    for key in sources.keys() {
        if !layer_of.contains_key(key.as_str()) {
            return Err(format!(
                "`{key}` が FILE_LAYERS に無い。層を決めて追加してください"
            ));
        }
    }
    for key in layer_of.keys() {
        if !sources.contains_key(*key) {
            return Err(format!(
                "FILE_LAYERS の `{key}` に対応するファイルが無い。移動・削除に合わせて表を更新してください"
            ));
        }
    }

    let keys: BTreeSet<&str> = layer_of.keys().copied().collect();
    let mut violations = BTreeMap::new();
    for (from, source) in sources {
        let from_layer = layer_of[from.as_str()];
        let usage = scanner::analyze(source);

        for path in &usage.crate_paths {
            let Some(to) = resolve_module(path, &keys) else {
                return Err(format!(
                    "`{from}` の `crate::{path}` が、FILE_LAYERS のどのファイルにも解決できない"
                ));
            };
            if to == from {
                continue;
            }
            let key = format!("{from} -> crate::{to}");
            let to_layer = layer_of[to];
            if !may_depend_on(from_layer, to_layer) {
                violations.insert(
                    key,
                    format!("{from_layer:?} は {to_layer:?} に依存できない"),
                );
            } else if let Some((_, _, why)) = forbidden_edges
                .iter()
                .find(|(f, t, _)| f == from && *t == to)
            {
                violations.insert(key, (*why).to_string());
            }
        }

        let forbidden = forbidden_externals(from_layer);
        for used in usage.externals.iter().chain(&usage.io_calls) {
            if forbidden.contains(&used.as_str()) {
                violations.insert(
                    format!("{from} -> {used}"),
                    format!("{from_layer:?} では `{used}` を使えない"),
                );
            }
        }
    }
    Ok(violations)
}

/// 問題点の一覧（空なら合格）。
fn check(
    sources: &BTreeMap<String, String>,
    layers: &[(&str, Layer)],
    forbidden_edges: &[(&str, &str, &str)],
    allowlist: &[Allowed],
) -> Vec<String> {
    let violations = match find_violations(sources, layers, forbidden_edges) {
        Ok(violations) => violations,
        Err(message) => return vec![message],
    };

    let allowed: BTreeSet<&str> = allowlist.iter().map(|a| a.key).collect();
    let mut problems = Vec::new();
    for (key, why) in &violations {
        if !allowed.contains(key.as_str()) {
            problems.push(format!(
                "許可リストに無い違反: `{key}`（{why}）。依存を直してください。ALLOWLIST への追加で逃げないでください"
            ));
        }
    }
    for entry in allowlist {
        if !violations.contains_key(entry.key) {
            problems.push(format!(
                "許可リストにあるが違反が消えている: `{}`（解消する Issue: {}）。ALLOWLIST から削除してください",
                entry.key, entry.issue
            ));
        }
    }
    problems
}

#[test]
fn test_dependencies_follow_the_layers() {
    let sources = collect_sources(Path::new("src"));

    let problems = check(&sources, FILE_LAYERS, FORBIDDEN_EDGES, ALLOWLIST);

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn test_allowlist_entries_are_documented_and_unique() {
    let mut seen = BTreeSet::new();
    for entry in ALLOWLIST {
        assert!(seen.insert(entry.key), "重複: {}", entry.key);
        assert!(!entry.reason.trim().is_empty(), "理由が空: {}", entry.key);
        assert!(
            entry.issue.starts_with('#'),
            "解消する Issue（#番号）が必要: {}",
            entry.key
        );
    }
}

#[cfg(test)]
mod ratchet_tests {
    use super::*;

    const LAYERS: &[(&str, Layer)] = &[
        ("model", Layer::Domain),
        ("parser", Layer::Pure),
        ("writer", Layer::Platform),
        ("app", Layer::GuiFrontend),
    ];

    /// 4 ファイルすべて空のソースに、指定したファイルだけ本体を差し替えて検査する
    fn problems_with(overrides: &[(&str, &str)], allowlist: &[Allowed]) -> Vec<String> {
        let sources = LAYERS
            .iter()
            .map(|(key, _)| {
                let body = overrides
                    .iter()
                    .find(|(k, _)| k == key)
                    .map_or("", |(_, body)| *body);
                (key.to_string(), body.to_string())
            })
            .collect();
        check(&sources, LAYERS, &[], allowlist)
    }

    #[test]
    fn test_downward_dependencies_pass() {
        let found = problems_with(
            &[
                ("parser", "use crate::model::{A, B};"),
                ("writer", "use crate::model::A; use std::fs;"),
                (
                    "app",
                    "use crate::parser::parse; use egui::Ui; use crate::model::A;",
                ),
            ],
            &[],
        );

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn test_violations_are_reported_when_not_allowlisted() {
        // (ファイル, 本体, 報告されるキー)
        let cases = [
            (
                "model",
                "use crate::parser::parse;",
                "model -> crate::parser",
            ),
            (
                "parser",
                "use crate::writer::write;",
                "parser -> crate::writer",
            ),
            (
                "app",
                "fn f() { crate::writer::write(); }",
                "app -> crate::writer",
            ),
            ("parser", "use egui::Ui;", "parser -> egui"),
            ("parser", "use std::{fs, io};", "parser -> std::fs"),
            (
                "parser",
                "fn f(p: &Path) { p.exists(); }",
                "parser -> .exists()",
            ),
            (
                "model",
                "fn f() { std::env::var(\"A\"); }",
                "model -> std::env",
            ),
            ("app", "use clap::Parser;", "app -> clap"),
        ];

        for (file, body, key) in cases {
            let found = problems_with(&[(file, body)], &[]);

            assert_eq!(found.len(), 1, "{key}: {found:?}");
            assert!(
                found[0].contains("許可リストに無い違反") && found[0].contains(key),
                "{key}: {found:?}"
            );
        }
    }

    #[test]
    fn test_allowlisted_violation_passes_and_stale_entry_fails() {
        let allowlist = [Allowed {
            key: "parser -> egui",
            reason: "テスト",
            issue: "#1",
        }];

        let with_violation = problems_with(&[("parser", "use egui::Ui;")], &allowlist);
        let stale = problems_with(&[], &allowlist);

        assert!(with_violation.is_empty(), "{with_violation:?}");
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert!(stale[0].contains("違反が消えている") && stale[0].contains("#1"));
    }

    #[test]
    fn test_file_and_table_mismatches_fail() {
        let sources = |keys: &[&str]| -> BTreeMap<String, String> {
            keys.iter()
                .map(|k| (k.to_string(), String::new()))
                .collect()
        };

        let missing_in_table = check(
            &sources(&["model", "parser", "writer", "app", "extra"]),
            LAYERS,
            &[],
            &[],
        );
        let missing_file = check(&sources(&["model", "parser", "writer"]), LAYERS, &[], &[]);
        let unresolved = problems_with(&[("parser", "use crate::nowhere::X;")], &[]);

        assert!(missing_in_table[0].contains("FILE_LAYERS に無い"));
        assert!(missing_file[0].contains("対応するファイルが無い"));
        assert!(unresolved[0].contains("解決できない"));
    }

    #[test]
    fn test_forbidden_edge_is_reported_within_the_same_layer() {
        let layers = [("a", Layer::CliFrontend), ("b", Layer::CliFrontend)];
        let sources = BTreeMap::from([
            ("a".to_string(), "use crate::b::X;".to_string()),
            ("b".to_string(), String::new()),
        ]);

        let found = check(&sources, &layers, &[("a", "b", "理由")], &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("a -> crate::b") && found[0].contains("理由"));
    }
}
