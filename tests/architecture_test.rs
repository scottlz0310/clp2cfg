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
