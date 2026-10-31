// `tests/` 直下に置くと別のテスト crate として扱われるため、サブディレクトリに置いて指定する
#[path = "architecture_test/scanner.rs"]
mod scanner;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// workspace のルート（この crate は `crates/clp2cfg`）
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// エントリポイントと配線のファイル（GUI 版とコンソール版。workspace のルートからの相対パス）。
/// `entry.rs` はカバレッジの対象外（`codecov.yml`）なので、薄さをここで行数で守る。
const ENTRY_POINTS: [&str; 4] = [
    "crates/clp2cfg/src/main.rs",
    "crates/clp2cfg-cli/src/main.rs",
    "crates/clp2cfg/src/entry.rs",
    "crates/clp2cfg-cli/src/entry.rs",
];

fn read_entry_point(path: &str) -> String {
    let full = workspace_root().join(path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", full.display()))
}

#[test]
fn test_main_stays_thin() {
    for path in ENTRY_POINTS {
        let line_count = read_entry_point(path).lines().count();

        // エントリポイントは起動配線のみに留め、ロジックの混入を機械的に防ぐ（80行以下）
        assert!(
            line_count <= 80,
            "{path} is too fat ({line_count} lines > 80 lines). Move logic to modules or entry.rs!"
        );
    }
}

#[test]
fn test_main_has_no_business_logic() {
    for path in ENTRY_POINTS {
        assert_no_business_logic(path, &read_entry_point(path));
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
// 層の定義は #57 の「目標構造」。各 crate の `src/**/*.rs` の本体部分（`#[cfg(test)]` より前）を字句レベルで
// 走査して検査する（走査の限界は `architecture_test/scanner.rs` の冒頭を参照）。
// 外部 crate（egui / clap / arboard / image）の使用は、crate の依存（Cargo.toml）でコンパイラが強制する。ここは crate の中の層と、crate をまたぐ参照を見る。
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
    /// L4: GUI の状態と更新（画面・OS に触れない。クリップボードとファイルは port 経由）
    GuiState,
    /// L4: GUI frontend（egui への接続と OS の実装）
    GuiFrontend,
    /// L5: 配線
    Entry,
}

/// ファイル（`src/` からの相対パス。拡張子なし、区切りは `::`）→ 層。
/// 現在はフラットな配置のため、1 ファイルに複数の責務が混在するものは、主たる層に置いて
/// 混在の分を `ALLOWLIST` に載せている。ファイルの分割・移動に合わせてこの表を更新する。
const FILE_LAYERS: &[(&str, Layer)] = &[
    ("core::domain", Layer::Domain),
    ("core::domain::change", Layer::Domain),
    ("core::domain::client", Layer::Domain),
    ("core::domain::server", Layer::Domain),
    ("core::snippet", Layer::Pure),
    ("core::snippet::parser", Layer::Pure),
    ("core::snippet::filter", Layer::Pure),
    ("core::snippet::pipeline", Layer::Pure),
    ("core::snippet::placeholder", Layer::Pure),
    ("core::merge", Layer::Pure),
    ("core::delegate", Layer::Pure),
    ("core::delegate::plan", Layer::Pure),
    ("core::discovery", Layer::Platform),
    ("core::delegate::exec", Layer::Platform),
    ("core::writer", Layer::Platform),
    ("core::apply", Layer::Apply),
    ("core::apply::error", Layer::Apply),
    ("core::apply::fake", Layer::Apply),
    ("core::apply::io", Layer::Apply),
    ("core::apply::plan", Layer::Apply),
    ("core::apply::run", Layer::Apply),
    ("core::apply::undo", Layer::Apply),
    ("cli::args", Layer::CliFrontend),
    ("cli::headless", Layer::CliFrontend),
    ("cli::headless::input", Layer::CliFrontend),
    ("cli::headless::report", Layer::CliFrontend),
    ("cli::headless::request", Layer::CliFrontend),
    ("cli::headless::select", Layer::CliFrontend),
    ("gui::app", Layer::GuiFrontend),
    ("gui::app::action", Layer::GuiState),
    ("gui::app::clipboard", Layer::GuiFrontend),
    ("gui::app::ports", Layer::GuiState),
    ("gui::app::state", Layer::GuiState),
    ("gui::app::system", Layer::GuiFrontend),
    ("gui::app::view", Layer::GuiFrontend),
    ("gui::app::view::bottom_bar", Layer::GuiFrontend),
    ("gui::app::view::input_pane", Layer::GuiFrontend),
    ("gui::app::view::preview_pane", Layer::GuiFrontend),
    ("gui::app::view::shortcuts", Layer::GuiFrontend),
    ("gui::app::view::target_pane", Layer::GuiFrontend),
    ("gui::app::view::top_bar", Layer::GuiFrontend),
    ("cli::entry", Layer::Entry),
    ("gui::entry", Layer::Entry),
];

/// 層の検査の対象外（宣言だけのファイル。`main.rs` は `ENTRY_POINTS` で別に見る）
const SKIPPED_FILES: &[&str] = &["lib", "main"];

/// GUI の道具。`gui` crate の中で、状態（`GuiState`）と描画（`GuiFrontend`）を分けるために見る
const GUI_TOOLS: &[&str] = &["egui", "eframe", "arboard", "image"];

/// 層ごとに使えない外部 crate・std・呼び出し。
///
/// `egui` / `clap` / `arboard` / `image` を `core` が使えないこと、`cli` が `egui` を使えないことは、
/// crate の依存（`Cargo.toml`）でコンパイラが強制するので、ここでは見ない。
/// 同じ crate の中の区別（`core` の純粋ロジックと副作用、`gui` の状態と描画）だけを見る。
fn forbidden_externals(layer: Layer) -> Vec<&'static str> {
    // `.is_file()` などは、型を見ないため名前で判定する（`scanner::IO_CALLS`）
    let io_calls = [".is_file()", ".is_dir()", ".exists()", ".metadata()"];
    let os = ["dirs", "std::fs", "std::process", "std::env"];
    match layer {
        Layer::Domain | Layer::Pure => [&["tempfile"][..], &os, &io_calls].concat(),
        Layer::Apply => os.to_vec(),
        Layer::GuiState => [GUI_TOOLS, &os].concat(),
        Layer::Platform | Layer::CliFrontend | Layer::GuiFrontend | Layer::Entry => vec![],
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
                | (CliFrontend | GuiState, Domain | Pure | Apply)
                | (GuiFrontend, Domain | Pure | Apply | GuiState)
                | (Entry, _)
        )
}

/// 層の規則では表せない、ファイル単位の禁止（from, to, 理由）。from は配下のファイル（`from::*`）にも効く
const FORBIDDEN_EDGES: &[(&str, &str, &str)] = &[(
    "cli::headless",
    "cli::args",
    "ヘッドレス実行は clap のパース結果ではなく、要求の型を受ける",
)];

struct Allowed {
    /// `<ファイル> -> <別のファイル>`（例: `gui::app -> core::writer`）または `<ファイル> -> <外部 crate / std / 呼び出し>`
    key: &'static str,
    reason: &'static str,
    /// 解消する Issue
    issue: &'static str,
}

/// 現状の違反。解消した PR が、その行を削除する。
const ALLOWLIST: &[Allowed] = &[];

/// workspace の crate（検査で使う短い名前, `crates/` 配下のディレクトリ名）
const MEMBER_CRATES: [(&str, &str); 3] = [
    ("core", "clp2cfg-core"),
    ("cli", "clp2cfg-cli"),
    ("gui", "clp2cfg"),
];

/// 各 crate の `src/` の対象ファイルを、キー（`core::domain::client`、`gui::app::state` など）→ ソースで返す。
fn collect_sources(crates_dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, short: &str, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, short, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path.strip_prefix(root).unwrap().with_extension("");
                let mut segments: Vec<String> = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if segments.last().is_some_and(|s| s == "mod") {
                    segments.pop();
                }
                let file = segments.join("::");
                // エントリポイント（`main.rs`）は `ENTRY_POINTS` で別に見る
                if !SKIPPED_FILES.contains(&file.as_str()) {
                    let source = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    out.insert(format!("{short}::{file}"), source);
                }
            }
        }
    }

    let mut sources = BTreeMap::new();
    for (short, directory) in MEMBER_CRATES {
        let src = crates_dir.join(directory).join("src");
        walk(&src, &src, short, &mut sources);
    }
    sources
}

/// パス（crate の短い名前から始まる）を、最も長く一致するファイルのキーに解決する。
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

        // `crate::` は自分の crate、`clp2cfg_core::` などは別の crate。どちらも crate の短い名前つきのパスにそろえる
        let own_crate = from.split("::").next().unwrap_or_default();
        let paths = usage
            .crate_paths
            .iter()
            .map(|path| format!("{own_crate}::{path}"))
            .chain(usage.member_paths.iter().cloned());
        for path in paths {
            let Some(to) = resolve_module(&path, &keys) else {
                return Err(format!(
                    "`{from}` の `{path}` が、FILE_LAYERS のどのファイルにも解決できない"
                ));
            };
            if to == from {
                continue;
            }
            let key = format!("{from} -> {to}");
            let to_layer = layer_of[to];
            if !may_depend_on(from_layer, to_layer) {
                violations.insert(
                    key,
                    format!("{from_layer:?} は {to_layer:?} に依存できない"),
                );
            } else if let Some((_, _, why)) = forbidden_edges
                .iter()
                .find(|(f, t, _)| (from == *f || from.starts_with(&format!("{f}::"))) && *t == to)
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
    let sources = collect_sources(&workspace_root().join("crates"));

    let problems = check(&sources, FILE_LAYERS, FORBIDDEN_EDGES, ALLOWLIST);

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// view（`app::view` 配下）は状態を読むだけで、更新は `UiAction` として返す。
/// 字句の検査なので、状態を可変で受け取る形と `update` の呼び出しだけを見る。
#[test]
fn test_views_only_read_the_state() {
    let sources = collect_sources(&workspace_root().join("crates"));
    let views: Vec<_> = sources
        .iter()
        .filter(|(key, _)| key.starts_with("gui::app::view"))
        .collect();
    assert!(!views.is_empty(), "gui::app::view のファイルが見つからない");

    for (key, source) in views {
        let body = scanner::production_part(source);
        for forbidden in ["&mut AppState", ".update("] {
            assert!(
                !body.contains(forbidden),
                "`{key}` が `{forbidden}` を使っている。view は `&AppState` を読むだけにして、変更は UiAction で返してください"
            );
        }
    }
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
        ("core::model", Layer::Domain),
        ("core::parser", Layer::Pure),
        ("core::writer", Layer::Platform),
        ("cli::args", Layer::CliFrontend),
        ("gui::state", Layer::GuiState),
        ("gui::app", Layer::GuiFrontend),
    ];

    /// すべて空のソースに、指定したファイルだけ本体を差し替えて検査する
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
                ("core::parser", "use crate::model::{A, B};"),
                ("core::writer", "use crate::model::A; use std::fs;"),
                (
                    "gui::app",
                    "use crate::state::S; use egui::Ui; use clp2cfg_core::model::A;",
                ),
                ("cli::args", "use clp2cfg_core::parser::parse;"),
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
                "core::model",
                "use crate::parser::parse;",
                "core::model -> core::parser",
            ),
            (
                "core::parser",
                "use crate::writer::write;",
                "core::parser -> core::writer",
            ),
            (
                "gui::app",
                "fn f() { clp2cfg_core::writer::write(); }",
                "gui::app -> core::writer",
            ),
            (
                "cli::args",
                "use clp2cfg_core::writer::write;",
                "cli::args -> core::writer",
            ),
            (
                "gui::state",
                "use crate::app::Clp2cfgApp;",
                "gui::state -> gui::app",
            ),
            ("gui::state", "use egui::Ui;", "gui::state -> egui"),
            (
                "gui::state",
                "use arboard::Clipboard;",
                "gui::state -> arboard",
            ),
            (
                "core::parser",
                "use std::{fs, io};",
                "core::parser -> std::fs",
            ),
            (
                "core::parser",
                "fn f(p: &Path) { p.exists(); }",
                "core::parser -> .exists()",
            ),
            (
                "core::model",
                "fn f() { std::env::var(\"A\"); }",
                "core::model -> std::env",
            ),
            (
                "core::parser",
                "use tempfile::tempdir;",
                "core::parser -> tempfile",
            ),
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
    fn test_external_crates_are_left_to_the_compiler() {
        // egui / clap などを core や cli が使えないことは、crate の依存でコンパイラが強制する
        let found = problems_with(
            &[
                ("core::parser", "use egui::Ui; use clap::Parser;"),
                ("cli::args", "use egui::Ui;"),
            ],
            &[],
        );

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn test_allowlisted_violation_passes_and_stale_entry_fails() {
        let allowlist = [Allowed {
            key: "gui::state -> egui",
            reason: "テスト",
            issue: "#1",
        }];

        let with_violation = problems_with(&[("gui::state", "use egui::Ui;")], &allowlist);
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
        let all = [
            "core::model",
            "core::parser",
            "core::writer",
            "cli::args",
            "gui::state",
            "gui::app",
        ];

        let missing_in_table = check(
            &sources(&[&all[..], &["core::extra"]].concat()),
            LAYERS,
            &[],
            &[],
        );
        let missing_file = check(&sources(&all[..5]), LAYERS, &[], &[]);
        let unresolved = problems_with(&[("core::parser", "use crate::nowhere::X;")], &[]);

        assert!(missing_in_table[0].contains("FILE_LAYERS に無い"));
        assert!(missing_file[0].contains("対応するファイルが無い"));
        assert!(unresolved[0].contains("解決できない"));
    }

    #[test]
    fn test_forbidden_edge_is_reported_within_the_same_layer() {
        let layers = [
            ("cli::a", Layer::CliFrontend),
            ("cli::b", Layer::CliFrontend),
        ];
        let sources = BTreeMap::from([
            ("cli::a".to_string(), "use crate::b::X;".to_string()),
            ("cli::b".to_string(), String::new()),
        ]);

        let found = check(&sources, &layers, &[("cli::a", "cli::b", "理由")], &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("cli::a -> cli::b") && found[0].contains("理由"));
    }
}
