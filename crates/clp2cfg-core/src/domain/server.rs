//! MCP server definitions.

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Transport-specific endpoint configuration for an MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTransport {
    /// Local process launched via standard I/O (`command` / `args` / `env`).
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    /// Remote HTTP or SSE endpoint (`url` / optional `transport_type` / `headers`).
    Remote {
        /// Original `type` field value if present (e.g. `"http"`, `"sse"`, `"streamable-http"`).
        transport_type: Option<String>,
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// Normalized classification of a remote server's `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTransportKind {
    /// Streamable HTTP / generic HTTP (`None`, `"http"`, `"streamable-http"`, `"streamableHttp"`).
    Http { explicit: bool },
    /// Server-Sent Events (`"sse"`).
    Sse,
}

impl RemoteTransportKind {
    /// Classify a raw optional remote `type` string (case-insensitive), returning `None` if unsupported.
    pub fn parse(transport_type: Option<&str>) -> Option<Self> {
        let Some(raw) = transport_type else {
            return Some(Self::Http { explicit: false });
        };
        if raw.eq_ignore_ascii_case("http")
            || raw.eq_ignore_ascii_case("streamable-http")
            || raw.eq_ignore_ascii_case("streamablehttp")
        {
            Some(Self::Http { explicit: true })
        } else if raw.eq_ignore_ascii_case("sse") {
            Some(Self::Sse)
        } else {
            None
        }
    }
}

/// Normalized representation of an MCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedServerConfig {
    pub transport: ServerTransport,

    /// Extra vendor-specific or extended properties preserved as-is.
    pub extra: serde_json::Map<String, Value>,
}

impl NormalizedServerConfig {
    /// Create a basic stdio server configuration.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            transport: ServerTransport::Stdio {
                command: command.into(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            extra: serde_json::Map::new(),
        }
    }

    /// Create a basic remote server configuration.
    pub fn remote(url: impl Into<String>) -> Self {
        Self {
            transport: ServerTransport::Remote {
                transport_type: None,
                url: url.into(),
                headers: BTreeMap::new(),
            },
            extra: serde_json::Map::new(),
        }
    }

    /// Set the `type` field on a remote server configuration.
    pub fn with_transport_type(mut self, transport_type: impl Into<String>) -> Self {
        if let ServerTransport::Remote {
            transport_type: slot,
            ..
        } = &mut self.transport
        {
            *slot = Some(transport_type.into());
        }
        self
    }

    pub fn is_stdio(&self) -> bool {
        matches!(self.transport, ServerTransport::Stdio { .. })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.transport, ServerTransport::Remote { .. })
    }

    pub fn command(&self) -> Option<&str> {
        match &self.transport {
            ServerTransport::Stdio { command, .. } => Some(command.as_str()),
            ServerTransport::Remote { .. } => None,
        }
    }

    pub fn args(&self) -> &[String] {
        match &self.transport {
            ServerTransport::Stdio { args, .. } => args,
            ServerTransport::Remote { .. } => &[],
        }
    }

    pub fn url(&self) -> Option<&str> {
        match &self.transport {
            ServerTransport::Stdio { .. } => None,
            ServerTransport::Remote { url, .. } => Some(url.as_str()),
        }
    }

    pub fn transport_type(&self) -> Option<&str> {
        match &self.transport {
            ServerTransport::Stdio { .. } => None,
            ServerTransport::Remote { transport_type, .. } => transport_type.as_deref(),
        }
    }

    pub fn env(&self) -> Option<&BTreeMap<String, String>> {
        match &self.transport {
            ServerTransport::Stdio { env, .. } => Some(env),
            ServerTransport::Remote { .. } => None,
        }
    }

    pub fn env_mut(&mut self) -> Option<&mut BTreeMap<String, String>> {
        match &mut self.transport {
            ServerTransport::Stdio { env, .. } => Some(env),
            ServerTransport::Remote { .. } => None,
        }
    }

    pub fn headers(&self) -> Option<&BTreeMap<String, String>> {
        match &self.transport {
            ServerTransport::Stdio { .. } => None,
            ServerTransport::Remote { headers, .. } => Some(headers),
        }
    }

    pub fn headers_mut(&mut self) -> Option<&mut BTreeMap<String, String>> {
        match &mut self.transport {
            ServerTransport::Stdio { .. } => None,
            ServerTransport::Remote { headers, .. } => Some(headers),
        }
    }

    /// Key-value secret map for this server (`env` for stdio, `headers` for remote).
    pub fn secret_fields(&self) -> &BTreeMap<String, String> {
        match &self.transport {
            ServerTransport::Stdio { env, .. } => env,
            ServerTransport::Remote { headers, .. } => headers,
        }
    }

    /// Mutable key-value secret map for this server (`env` for stdio, `headers` for remote).
    pub fn secret_fields_mut(&mut self) -> &mut BTreeMap<String, String> {
        match &mut self.transport {
            ServerTransport::Stdio { env, .. } => env,
            ServerTransport::Remote { headers, .. } => headers,
        }
    }
}

impl Serialize for NormalizedServerConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match &self.transport {
            ServerTransport::Stdio { command, args, env } => {
                map.serialize_entry("command", command)?;
                if !args.is_empty() {
                    map.serialize_entry("args", args)?;
                }
                if !env.is_empty() {
                    map.serialize_entry("env", env)?;
                }
            }
            ServerTransport::Remote {
                transport_type,
                url,
                headers,
            } => {
                if let Some(t) = transport_type {
                    map.serialize_entry("type", t)?;
                }
                map.serialize_entry("url", url)?;
                if !headers.is_empty() {
                    map.serialize_entry("headers", headers)?;
                }
            }
        }
        for (k, v) in &self.extra {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for NormalizedServerConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(map) = value else {
            return Err(de::Error::custom("server config must be an object"));
        };
        parse_server_map(map).map_err(de::Error::custom)
    }
}

/// Validates and normalizes a JSON object into [`NormalizedServerConfig`].
///
/// Error messages deliberately omit user-supplied values so secrets (e.g. API keys in `env` or
/// `headers`) never leak into previews, logs, or stderr.
fn parse_server_map(
    mut map: serde_json::Map<String, Value>,
) -> Result<NormalizedServerConfig, String> {
    let has_command = map.contains_key("command");
    let has_url = map.contains_key("url");
    let has_server_url = map.contains_key("serverUrl");

    if has_command && (has_url || has_server_url) {
        return Err(
            "conflicting fields: server cannot specify both `command` and `url`".to_string(),
        );
    }

    if !has_command && !has_url && !has_server_url {
        return Err("missing field `command` or `url`".to_string());
    }

    if has_command {
        let command = match map.remove("command") {
            Some(Value::String(s)) => s,
            _ => {
                return Err(
                    "invalid field type (command must be a string, args an array of strings, env a map of strings)"
                        .to_string(),
                )
            }
        };
        let args = match map.remove("args") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s),
                    _ => Err(
                        "invalid field type (command must be a string, args an array of strings, env a map of strings)"
                            .to_string(),
                    ),
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => {
                return Err(
                    "invalid field type (command must be a string, args an array of strings, env a map of strings)"
                        .to_string(),
                )
            }
        };
        let env = parse_string_map(
            map.remove("env"),
            "invalid field type (command must be a string, args an array of strings, env a map of strings)",
        )?;

        return Ok(NormalizedServerConfig {
            transport: ServerTransport::Stdio { command, args, env },
            extra: map,
        });
    }

    let url_val = map.remove("url");
    let server_url_val = map.remove("serverUrl");
    let url = match (url_val, server_url_val) {
        (Some(Value::String(u)), None) | (None, Some(Value::String(u))) => u,
        (Some(Value::String(u)), Some(Value::String(su))) => {
            if u != su {
                return Err(
                    "conflicting fields: server cannot specify different `url` and `serverUrl`"
                        .to_string(),
                );
            }
            u
        }
        _ => return Err(
            "invalid field type (url must be a string, type a string, headers a map of strings)"
                .to_string(),
        ),
    };
    if url.trim().is_empty() {
        return Err("missing field `url`".to_string());
    }

    let transport_type = match map.remove("type") {
        None => None,
        Some(Value::String(t)) => {
            if t.eq_ignore_ascii_case("stdio") {
                return Err(
                    "conflicting fields: remote server with `url` cannot have `type` \"stdio\""
                        .to_string(),
                );
            }
            Some(t)
        }
        Some(_) => return Err(
            "invalid field type (url must be a string, type a string, headers a map of strings)"
                .to_string(),
        ),
    };

    let headers = parse_string_map(
        map.remove("headers"),
        "invalid field type (url must be a string, type a string, headers a map of strings)",
    )?;

    Ok(NormalizedServerConfig {
        transport: ServerTransport::Remote {
            transport_type,
            url,
            headers,
        },
        extra: map,
    })
}

fn parse_string_map(
    value: Option<Value>,
    type_error: &str,
) -> Result<BTreeMap<String, String>, String> {
    match value {
        None => Ok(BTreeMap::new()),
        Some(Value::Object(obj)) => obj
            .into_iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((k, s)),
                _ => Err(type_error.to_string()),
            })
            .collect(),
        Some(_) => Err(type_error.to_string()),
    }
}

/// A collection of normalized MCP servers (name -> config).
pub type McpServersMap = BTreeMap<String, NormalizedServerConfig>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stdio_round_trip_preserves_extra() {
        let mut cfg = NormalizedServerConfig::new("npx");
        if let ServerTransport::Stdio { args, env, .. } = &mut cfg.transport {
            *args = vec!["-y".into(), "pkg".into()];
            *env = BTreeMap::from([("TOKEN".into(), "val".into())]);
        }
        cfg.extra.insert("type".into(), json!("stdio"));

        let value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(
            value,
            json!({
                "command": "npx",
                "args": ["-y", "pkg"],
                "env": { "TOKEN": "val" },
                "type": "stdio",
            })
        );
        let decoded: NormalizedServerConfig = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, cfg);
    }

    #[test]
    fn test_remote_round_trip_and_server_url_normalization() {
        let input = json!({
            "type": "http",
            "url": "https://mcp.cloudflare.com/mcp",
            "headers": {
                "Authorization": "Bearer secret"
            },
            "disabled": false
        });
        let decoded: NormalizedServerConfig = serde_json::from_value(input.clone()).unwrap();
        assert!(decoded.is_remote());
        assert_eq!(decoded.transport_type(), Some("http"));
        assert_eq!(decoded.url(), Some("https://mcp.cloudflare.com/mcp"));
        assert_eq!(
            decoded
                .headers()
                .unwrap()
                .get("Authorization")
                .map(String::as_str),
            Some("Bearer secret")
        );
        assert_eq!(serde_json::to_value(&decoded).unwrap(), input);

        // serverUrl is normalized to url
        let windsurf = json!({
            "serverUrl": "https://example.com/mcp"
        });
        let decoded_ws: NormalizedServerConfig = serde_json::from_value(windsurf).unwrap();
        assert_eq!(decoded_ws.url(), Some("https://example.com/mcp"));
        assert_eq!(decoded_ws.transport_type(), None);
        assert_eq!(
            serde_json::to_value(&decoded_ws).unwrap(),
            json!({ "url": "https://example.com/mcp" })
        );
    }

    #[test]
    fn test_deserialize_rejects_conflicts_and_leaks_no_secrets() {
        let cases = [
            (
                json!({"command": "node", "url": "https://secret.example/mcp"}),
                "secret.example",
                "conflicting fields",
            ),
            (
                json!({"url": "https://a.example", "serverUrl": "https://b.example"}),
                "a.example",
                "conflicting fields",
            ),
            (
                json!({"url": "https://secret.example", "type": "stdio"}),
                "secret.example",
                "conflicting fields",
            ),
            (json!({"url": "   "}), "   ", "missing field `url`"),
            (
                json!({"url": "https://example.com", "headers": {"Authorization": 123456}}),
                "123456",
                "invalid field type",
            ),
            (
                json!("secret-not-an-object"),
                "secret-not-an-object",
                "must be an object",
            ),
        ];

        for (val, secret, hint) in cases {
            let err = serde_json::from_value::<NormalizedServerConfig>(val)
                .unwrap_err()
                .to_string();
            assert!(!err.contains(secret), "leaked secret in: {err}");
            assert!(err.contains(hint), "missing hint {hint:?} in: {err}");
        }
    }
}
