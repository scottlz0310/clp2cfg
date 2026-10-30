//! Detection and filling of unfilled placeholder values (e.g. `YOUR_API_KEY`) in server `env`.

use crate::model::McpServersMap;
use std::collections::BTreeMap;

/// An `env` entry of a server, addressed by server name and variable name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EnvRef {
    pub server: String,
    pub key: String,
}

/// User-entered real values keyed by the placeholder entry they replace.
pub type EnvOverrides = BTreeMap<EnvRef, String>;

/// Phrases that mark a value as "to be filled in" (compared after separator normalization).
const MARKERS: &[&str] = &[
    "paste_here",
    "changeme",
    "change_me",
    "replace_me",
    "replace_with",
    "insert_here",
    "insert_your",
    "enter_your",
    "put_your",
    "fill_in",
];

/// Bare words that mean "not decided yet" when they start a value (`TODO`, `TODO: set key`).
const LEADING_WORDS: &[&str] = &["todo", "tbd", "fixme", "placeholder"];

/// Whether `value` looks like an unfilled placeholder rather than a real value.
pub fn is_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }

    // `-` と空白を `_` に揃え、`your-api-key` / `YOUR API KEY` / `YOUR_API_KEY` を同一に扱う
    let normalized = trimmed.to_ascii_lowercase().replace(['-', ' '], "_");

    normalized.starts_with("your_")
        || has_angle_placeholder(&normalized)
        || MARKERS.iter().any(|m| normalized.contains(m))
        || LEADING_WORDS
            .iter()
            .any(|w| starts_with_word(&normalized, w))
        || is_mask(&normalized)
}

/// `<token>` / `Bearer <your-key>` のように、英字で始まる `<...>` を含む。
fn has_angle_placeholder(value: &str) -> bool {
    value.match_indices('<').any(|(start, _)| {
        value[start + 1..]
            .split_once('>')
            .is_some_and(|(inner, _)| inner.starts_with(|c: char| c.is_ascii_alphabetic()))
    })
}

/// `word` で始まり、直後が英数字でない（`todo_later` は対象、`todolist` は対象外）。
fn starts_with_word(value: &str, word: &str) -> bool {
    value
        .strip_prefix(word)
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric()))
}

/// `xxxxxxxx` / `********` のような伏せ字のみ。
fn is_mask(value: &str) -> bool {
    value.len() >= 3 && value.chars().all(|c| c == 'x' || c == '*')
}

/// All `env` entries whose current value looks like an unfilled placeholder.
pub fn find_placeholders(servers: &McpServersMap) -> Vec<EnvRef> {
    servers
        .iter()
        .flat_map(|(server, config)| {
            config
                .env
                .iter()
                .filter(|(_, value)| is_placeholder(value))
                .map(move |(key, _)| EnvRef {
                    server: server.clone(),
                    key: key.clone(),
                })
        })
        .collect()
}

/// Replace the placeholder entries in `refs` with the user-entered values in `overrides`.
///
/// Only entries in `refs` are touched, so a value that is no longer a placeholder in the
/// snippet is never overwritten by a stale entry.
pub fn fill_placeholders(servers: &mut McpServersMap, refs: &[EnvRef], overrides: &EnvOverrides) {
    for env_ref in refs {
        let Some(value) = overrides.get(env_ref) else {
            continue;
        };
        if let Some(slot) = servers
            .get_mut(&env_ref.server)
            .and_then(|config| config.env.get_mut(&env_ref.key))
        {
            slot.clone_from(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NormalizedServerConfig;

    #[test]
    fn test_is_placeholder() {
        let cases = [
            // 空・空白のみ
            ("", true),
            ("   ", true),
            // YOUR_*
            ("YOUR_API_KEY", true),
            ("your-api-key", true),
            ("Your API Key", true),
            ("your_token_here", true),
            // <...>
            ("<token>", true),
            ("<YOUR_API_KEY>", true),
            ("Bearer <your-key>", true),
            // マーカー語
            ("PASTE_HERE", true),
            ("paste here", true),
            ("changeme", true),
            ("CHANGE_ME", true),
            ("replace-with-your-key", true),
            // 先頭語
            ("TODO", true),
            ("todo: set key", true),
            ("TBD", true),
            // 伏せ字
            ("xxxxxxxx", true),
            ("********", true),
            // 実値らしいもの
            ("sk-ant-api03-abcdef", false),
            ("ghp_1234567890abcdefghijkl", false),
            ("/home/user/project", false),
            ("https://example.com/mcp", false),
            ("true", false),
            ("yourself", false),
            ("todolist", false),
            ("a<b", false),
            ("1 < 2 > 0", false),
            ("${GITHUB_TOKEN}", false),
        ];

        for (value, expected) in cases {
            assert_eq!(is_placeholder(value), expected, "value={value:?}");
        }
    }

    fn servers(entries: &[(&str, &[(&str, &str)])]) -> McpServersMap {
        entries
            .iter()
            .map(|(name, env)| {
                let mut config = NormalizedServerConfig::new("npx");
                config.env = env
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                (name.to_string(), config)
            })
            .collect()
    }

    fn env_ref(server: &str, key: &str) -> EnvRef {
        EnvRef {
            server: server.to_string(),
            key: key.to_string(),
        }
    }

    #[test]
    fn test_find_placeholders_reports_only_placeholder_entries() {
        let map = servers(&[
            ("a", &[("API_KEY", "YOUR_API_KEY"), ("REGION", "us-east-1")]),
            ("b", &[("TOKEN", ""), ("MODE", "prod")]),
            ("c", &[]),
        ]);

        assert_eq!(
            find_placeholders(&map),
            vec![env_ref("a", "API_KEY"), env_ref("b", "TOKEN")]
        );
    }

    #[test]
    fn test_fill_placeholders() {
        let refs = [env_ref("a", "API_KEY"), env_ref("a", "OTHER")];
        let base = servers(&[(
            "a",
            &[
                ("API_KEY", "YOUR_API_KEY"),
                ("OTHER", "<other>"),
                ("REAL", "keep"),
            ],
        )]);

        // (server, key, value)
        type Entry = (&'static str, &'static str, &'static str);
        // (overrides, 期待する [API_KEY, OTHER, REAL])
        let cases: [(&[Entry], [&str; 3]); 4] = [
            (&[], ["YOUR_API_KEY", "<other>", "keep"]),
            (
                &[("a", "API_KEY", "real-key")],
                ["real-key", "<other>", "keep"],
            ),
            (
                &[("a", "API_KEY", "k1"), ("a", "OTHER", "k2")],
                ["k1", "k2", "keep"],
            ),
            // refs に含まれない（既にプレースホルダーでない）項目は上書きしない
            (
                &[("a", "REAL", "clobber"), ("zzz", "API_KEY", "ghost")],
                ["YOUR_API_KEY", "<other>", "keep"],
            ),
        ];

        for (entries, expected) in cases {
            let overrides: EnvOverrides = entries
                .iter()
                .map(|(s, k, v)| (env_ref(s, k), v.to_string()))
                .collect();
            let mut map = base.clone();
            fill_placeholders(&mut map, &refs, &overrides);

            let env = &map["a"].env;
            let actual = [&env["API_KEY"], &env["OTHER"], &env["REAL"]];
            assert_eq!(actual, expected, "entries={entries:?}");
        }
    }
}
