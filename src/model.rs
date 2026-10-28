//! Core data models for MCP server configs and client targets.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Normalized representation of an MCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedServerConfig {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// A collection of normalized MCP servers (name -> config).
pub type McpServersMap = BTreeMap<String, NormalizedServerConfig>;
