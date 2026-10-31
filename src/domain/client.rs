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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// 全クライアントの識別子・表示名・適用方法・CLI 名を固定する。
    /// クライアントを足したら、ここにも 1 行足す（`ALL` の足し忘れもここで気付く）。
    #[test]
    fn test_every_client_keeps_its_identity() {
        use ClientTargetKind::*;
        // (種類, --target の名前, 表示名, 適用方法, CLI の実行ファイル)
        let expected = [
            (
                Claude,
                "claude",
                "Claude Code / Desktop",
                TargetAction::FileMerge,
                None,
            ),
            (Cursor, "cursor", "Cursor", TargetAction::FileMerge, None),
            (
                VsCode,
                "vscode",
                "VS Code Copilot",
                TargetAction::FileMerge,
                None,
            ),
            (
                Project,
                "project",
                "Project Local (.mcp.json)",
                TargetAction::FileMerge,
                None,
            ),
            (
                Windsurf,
                "windsurf",
                "Windsurf",
                TargetAction::FileMerge,
                None,
            ),
            (
                Cline,
                "cline",
                "Cline (VS Code)",
                TargetAction::FileMerge,
                None,
            ),
            (
                RooCode,
                "roo-code",
                "Roo Code (VS Code)",
                TargetAction::FileMerge,
                None,
            ),
            (Zed, "zed", "Zed", TargetAction::FileMerge, None),
            (
                ClaudeCli,
                "claude-cli",
                "Claude Code CLI",
                TargetAction::CliCommand,
                Some("claude"),
            ),
            (
                CodexCli,
                "codex-cli",
                "Codex CLI",
                TargetAction::CliCommand,
                Some("codex"),
            ),
            (
                CopilotCli,
                "copilot-cli",
                "GitHub Copilot CLI",
                TargetAction::CliCommand,
                Some("copilot"),
            ),
            (
                AntigravityCli,
                "antigravity-cli",
                "Antigravity CLI",
                TargetAction::CliCommand,
                Some("agy"),
            ),
        ];

        assert_eq!(
            expected.map(|(kind, ..)| kind),
            ClientTargetKind::ALL,
            "ALL は表示順に全種類を持つ"
        );
        for (kind, cli_name, display_name, action, program) in expected {
            assert_eq!(kind.cli_name(), cli_name);
            assert_eq!(kind.display_name(), display_name);
            assert_eq!(kind.action(), action, "{kind:?}");
            assert_eq!(kind.cli_program(), program, "{kind:?}");
            assert_eq!(ClientTargetKind::from_cli_name(cli_name), Some(kind));
        }
        assert_eq!(ClientTargetKind::from_cli_name("unknown"), None);
    }

    #[test]
    fn test_cli_names_and_display_names_are_unique() {
        let cli_names: BTreeSet<_> = ClientTargetKind::ALL.iter().map(|k| k.cli_name()).collect();
        let display_names: BTreeSet<_> = ClientTargetKind::ALL
            .iter()
            .map(|k| k.display_name())
            .collect();

        assert_eq!(cli_names.len(), ClientTargetKind::ALL.len());
        assert_eq!(display_names.len(), ClientTargetKind::ALL.len());
    }
}
