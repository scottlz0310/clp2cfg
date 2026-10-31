//! Client target kinds and where their configuration lives.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How a client target receives the MCP server definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TargetAction {
    /// Merge into a config file directly (atomic write + `.bak`).
    FileMerge,
    /// Delegate to the client's official CLI (`<cli> mcp add ...`).
    CliCommand,
}

/// Target AI client kind for MCP server configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientTargetKind {
    // --- File merge targets ---
    Claude,
    Cursor,
    VsCode,
    Project,
    Windsurf,
    Cline,
    RooCode,
    Zed,
    // --- CLI delegation targets ---
    ClaudeCli,
    CodexCli,
    CopilotCli,
    AntigravityCli,
}

impl ClientTargetKind {
    /// Every target kind, in display order.
    pub const ALL: [Self; 12] = [
        Self::Claude,
        Self::Cursor,
        Self::VsCode,
        Self::Project,
        Self::Windsurf,
        Self::Cline,
        Self::RooCode,
        Self::Zed,
        Self::ClaudeCli,
        Self::CodexCli,
        Self::CopilotCli,
        Self::AntigravityCli,
    ];

    /// Stable identifier used on the command line (`--target <name>`).
    pub fn cli_name(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::VsCode => "vscode",
            Self::Project => "project",
            Self::Windsurf => "windsurf",
            Self::Cline => "cline",
            Self::RooCode => "roo-code",
            Self::Zed => "zed",
            Self::ClaudeCli => "claude-cli",
            Self::CodexCli => "codex-cli",
            Self::CopilotCli => "copilot-cli",
            Self::AntigravityCli => "antigravity-cli",
        }
    }

    /// Inverse of [`cli_name`](Self::cli_name).
    pub fn from_cli_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.cli_name() == name)
    }

    /// Friendly display name of the client target.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code / Desktop",
            Self::Cursor => "Cursor",
            Self::VsCode => "VS Code Copilot",
            Self::Project => "Project Local (.mcp.json)",
            Self::Windsurf => "Windsurf",
            Self::Cline => "Cline (VS Code)",
            Self::RooCode => "Roo Code (VS Code)",
            Self::Zed => "Zed",
            Self::ClaudeCli => "Claude Code CLI",
            Self::CodexCli => "Codex CLI",
            Self::CopilotCli => "GitHub Copilot CLI",
            Self::AntigravityCli => "Antigravity CLI",
        }
    }

    /// How this target is applied (file merge vs. official CLI delegation).
    pub fn action(&self) -> TargetAction {
        match self {
            Self::ClaudeCli | Self::CodexCli | Self::CopilotCli | Self::AntigravityCli => {
                TargetAction::CliCommand
            }
            _ => TargetAction::FileMerge,
        }
    }

    /// Executable name of the official CLI for CLI-delegation targets.
    pub fn cli_program(&self) -> Option<&'static str> {
        match self {
            Self::ClaudeCli => Some("claude"),
            Self::CodexCli => Some("codex"),
            Self::CopilotCli => Some("copilot"),
            Self::AntigravityCli => Some("agy"),
            _ => None,
        }
    }
}

/// Information about a target client configuration file (or CLI executable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTarget {
    pub kind: ClientTargetKind,
    pub name: String,
    /// Config file path, or the CLI executable (resolved path or bare name).
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

    /// Create a CLI delegation target; `resolved` is the executable found on PATH, if any.
    pub fn new_cli(kind: ClientTargetKind, resolved: Option<PathBuf>) -> Self {
        let exists = resolved.is_some();
        let path = resolved.unwrap_or_else(|| PathBuf::from(kind.cli_program().unwrap_or("")));
        Self {
            name: kind.display_name().to_string(),
            kind,
            path,
            exists,
            selected: exists,
        }
    }
}
