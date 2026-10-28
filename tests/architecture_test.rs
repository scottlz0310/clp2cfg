#[test]
fn test_main_stays_thin() {
    let main_content = std::fs::read_to_string("src/main.rs").expect("failed to read src/main.rs");
    let line_count = main_content.lines().count();

    // main.rs はエントリポイントと起動配線のみに留め、ロジックの混入を機械的に防ぐ（80行以下）
    assert!(
        line_count <= 80,
        "src/main.rs is too fat ({} lines > 80 lines). Move logic to modules or src/app.rs!",
        line_count
    );
}

#[test]
fn test_main_has_no_business_logic() {
    let main_content = std::fs::read_to_string("src/main.rs").expect("failed to read src/main.rs");

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
            "src/main.rs contains forbidden business logic or I/O direct call: '{}'. Move this logic to appropriate modules!",
            pattern
        );
    }
}
