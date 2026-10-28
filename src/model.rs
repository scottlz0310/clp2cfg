//! Core data models for MCP server configs and client targets.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Normalized representation of an MCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedServerConfig {
    pub command: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// Extra vendor-specific or extended properties preserved as-is.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl NormalizedServerConfig {
    /// Create a basic server configuration.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// A collection of normalized MCP servers (name -> config).
pub type McpServersMap = BTreeMap<String, NormalizedServerConfig>;

/// Error types occurring during snippet parsing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("Empty input provided")]
    EmptyInput,

    #[error("No valid JSON structure found in input")]
    NoJsonFound,

    #[error("Failed to parse JSON/JSON5 snippet: {0}")]
    SyntaxError(String),

    #[error("Invalid MCP configuration format: {0}")]
    InvalidFormat(String),
}
