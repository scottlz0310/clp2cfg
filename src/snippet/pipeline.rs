//! Snippet text → filtered server definitions and their placeholders.
//! The one path both frontends (GUI and headless) take, so they cannot drift apart.

use super::filter::{apply_filters, FilterOptions};
use super::parser::{parse_mcp_snippet, ParseError};
use super::placeholder::{fill_placeholders, find_placeholders, EnvOverrides, EnvRef};
use crate::domain::McpServersMap;

/// A snippet ready to be previewed or applied.
#[derive(Debug, PartialEq, Eq)]
pub struct Prepared {
    /// Filtered, with the entered placeholder values filled in.
    pub servers: McpServersMap,
    /// Every placeholder the snippet contains, including those already filled in
    /// (so the input field stays visible once a value is entered).
    pub placeholders: Vec<EnvRef>,
}

/// Parse `text`, apply the command filters, find the placeholders, then fill in `overrides`.
pub fn prepare(
    text: &str,
    filters: &FilterOptions,
    overrides: &EnvOverrides,
) -> Result<Prepared, ParseError> {
    let mut servers: McpServersMap = parse_mcp_snippet(text)?
        .into_iter()
        .map(|(name, config)| (name, apply_filters(config, filters)))
        .collect();

    let placeholders = find_placeholders(&servers);
    fill_placeholders(&mut servers, &placeholders, overrides);

    Ok(Prepared {
        servers,
        placeholders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ON: FilterOptions = FilterOptions {
        replace_npx_with_bunx: true,
        replace_pipx_with_uvx: true,
    };
    const ALL_OFF: FilterOptions = FilterOptions {
        replace_npx_with_bunx: false,
        replace_pipx_with_uvx: false,
    };

    fn prepared(text: &str, filters: &FilterOptions) -> Prepared {
        prepare(text, filters, &EnvOverrides::new()).unwrap()
    }

    fn commands(prepared: &Prepared) -> Vec<(&str, &str)> {
        prepared
            .servers
            .iter()
            .map(|(name, config)| (name.as_str(), config.command.as_str()))
            .collect()
    }

    #[test]
    fn test_every_input_form_is_filtered_by_the_options() {
        // (入力の形式, テキスト, フィルタ有効時の結果, フィルタ無効時の結果)
        let cases = [
            (
                "mcpServers の完全形式",
                r#"{"mcpServers":{"fetch":{"command":"npx","args":["-y","pkg"]}}}"#,
                vec![("fetch", "bunx")],
                vec![("fetch", "npx")],
            ),
            (
                "サーバー名をキーにしたマップ",
                r#"{"fetch":{"command":"npx","args":["-y","pkg"]}}"#,
                vec![("fetch", "bunx")],
                vec![("fetch", "npx")],
            ),
            (
                "名前のない単一定義（コマンドから名前を補う）",
                r#"{"command":"npx","args":["-y","@scope/server-fetch"]}"#,
                vec![("server-fetch", "bunx")],
                vec![("server-fetch", "npx")],
            ),
            (
                "コメントと末尾カンマを含む JSONC",
                "{\n // c\n \"mcpServers\": { \"a\": { \"command\": \"npx\", }, },\n}",
                vec![("a", "bunx")],
                vec![("a", "npx")],
            ),
            (
                "Markdown のコードブロック",
                "説明\n```json\n{\"mcpServers\":{\"a\":{\"command\":\"npx\"}}}\n```\n",
                vec![("a", "bunx")],
                vec![("a", "npx")],
            ),
            (
                "pipx run → uvx",
                r#"{"mcpServers":{"py":{"command":"pipx","args":["run","tool"]}}}"#,
                vec![("py", "uvx")],
                vec![("py", "pipx")],
            ),
        ];

        for (label, text, on, off) in cases {
            assert_eq!(commands(&prepared(text, &ALL_ON)), on, "{label}: ON");
            assert_eq!(commands(&prepared(text, &ALL_OFF)), off, "{label}: OFF");
        }
    }

    #[test]
    fn test_each_filter_option_is_independent() {
        let text =
            r#"{"a":{"command":"npx","args":["x"]},"b":{"command":"pipx","args":["run","y"]}}"#;
        let only_npx = FilterOptions {
            replace_npx_with_bunx: true,
            replace_pipx_with_uvx: false,
        };
        let only_pipx = FilterOptions {
            replace_npx_with_bunx: false,
            replace_pipx_with_uvx: true,
        };

        assert_eq!(
            commands(&prepared(text, &only_npx)),
            [("a", "bunx"), ("b", "pipx")]
        );
        assert_eq!(
            commands(&prepared(text, &only_pipx)),
            [("a", "npx"), ("b", "uvx")]
        );
    }

    #[test]
    fn test_placeholders_are_found_and_filled_from_overrides() {
        let text = r#"{"srv":{"command":"node","env":{"API_KEY":"YOUR_API_KEY","MODE":"prod"}}}"#;
        let api_key = EnvRef {
            server: "srv".to_string(),
            key: "API_KEY".to_string(),
        };
        let env_of = |p: &Prepared, key: &str| p.servers["srv"].env[key].clone();

        // 実値が無ければ、プレースホルダーのまま報告される（実値のある MODE は対象外）
        let unfilled = prepared(text, &ALL_ON);
        assert_eq!(unfilled.placeholders, std::slice::from_ref(&api_key));
        assert_eq!(env_of(&unfilled, "API_KEY"), "YOUR_API_KEY");

        // 実値を渡すと埋め込まれるが、入力欄を残すため placeholders には残る
        let overrides = EnvOverrides::from([(api_key.clone(), "secret".to_string())]);
        let filled = prepare(text, &ALL_ON, &overrides).unwrap();
        assert_eq!(filled.placeholders, [api_key]);
        assert_eq!(env_of(&filled, "API_KEY"), "secret");
        assert_eq!(env_of(&filled, "MODE"), "prod");
    }

    #[test]
    fn test_an_override_for_a_value_that_is_no_longer_a_placeholder_is_ignored() {
        let text = r#"{"srv":{"command":"node","env":{"API_KEY":"real-value"}}}"#;
        let stale = EnvOverrides::from([(
            EnvRef {
                server: "srv".to_string(),
                key: "API_KEY".to_string(),
            },
            "stale".to_string(),
        )]);

        let result = prepare(text, &ALL_ON, &stale).unwrap();

        assert!(result.placeholders.is_empty());
        assert_eq!(result.servers["srv"].env["API_KEY"], "real-value");
    }

    #[test]
    fn test_parse_errors_pass_through_unchanged() {
        for text in ["", "not json at all", r#"{"mcpServers": 1}"#] {
            let expected = parse_mcp_snippet(text).unwrap_err();

            let actual = prepare(text, &ALL_ON, &EnvOverrides::new()).unwrap_err();

            assert_eq!(actual, expected, "{text:?}");
        }
    }
}
