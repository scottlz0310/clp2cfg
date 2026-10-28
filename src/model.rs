//! Core data models for MCP server configs and client targets.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
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

/// Target AI client kind for MCP server configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientTargetKind {
    Claude,
    Cursor,
    Codex,
    VsCode,
    Project,
}

impl ClientTargetKind {
    /// Friendly display name of the client target.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code / Desktop",
            Self::Cursor => "Cursor",
            Self::Codex => "Codex CLI",
            Self::VsCode => "VS Code Copilot",
            Self::Project => "Project Local (.mcp.json)",
        }
    }
}

/// Information about a target client configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTarget {
    pub kind: ClientTargetKind,
    pub name: String,
    pub path: PathBuf,
    pub exists: bool,
    pub selected: bool,
}

impl ClientTarget {
    pub fn new(kind: ClientTargetKind, path: PathBuf) -> Self {
        let exists = path.is_file();
        Self {
            name: kind.display_name().to_string(),
            kind,
            exists,
            selected: exists, // Default to true if the file already exists
            path,
        }
    }
}

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

/// Error types occurring during target AST merge.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("Failed to parse existing configuration JSON: {0}")]
    InvalidExistingJson(String),

    #[error("Configuration root must be a JSON object")]
    RootNotObject,

    #[error("Failed to serialize merged configuration: {0}")]
    SerializationFailed(String),
}

/// Error types occurring during safe atomic writing.
#[derive(Debug, Error)]
pub enum WriteError {
    #[error("I/O error during writing: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to create backup at {path}: {source}")]
    BackupFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to persist temporary file to {path}: {source}")]
    PersistFailed {
        path: PathBuf,
        source: tempfile::PersistError,
    },
}
