//! Client target kinds, what differs between clients (as data), and where their configuration lives.

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
///
/// Everything that differs between clients lives in [`ClientTargetKind::spec`]: adding a client
/// means a variant here, an entry in `ALL`, and an arm in `spec` (plus its config location in the
/// detection code). Forgetting the `spec` arm is a compile error.
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

/// What differs between clients, as data.
#[derive(Debug, PartialEq, Eq)]
pub struct ClientSpec {
    /// Stable identifier used on the command line (`--target <name>`).
    pub cli_name: &'static str,
    /// Friendly display name.
    pub display_name: &'static str,
    /// How the definitions reach the client.
    pub apply: ApplySpec,
}

/// How a client receives the definitions.
#[derive(Debug, PartialEq, Eq)]
pub enum ApplySpec {
    File(FileSpec),
    Cli(CliSpec),
}

/// A client whose config file is merged directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec {
    /// The JSON key that holds the MCP servers.
    pub servers_key: &'static str,
    /// The existing file is JSONC (comments are not preserved on rewrite; the `.bak` keeps them).
    pub allows_jsonc: bool,
    /// Each server gets `"type": "stdio"` when it has no `type`.
    pub adds_stdio_type: bool,
}

/// A client reached through its official CLI (`<program> mcp add ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliSpec {
    /// Executable name looked up on `PATH`.
    pub program: &'static str,
    /// `mcp add` rejects an existing name, so a `remove` (failure ignored) runs first.
    /// Verified on real CLIs: `claude` and `copilot` reject; `codex` and `agy` upsert.
    pub remove_before_add: bool,
    /// Value of `--scope`, when the CLI needs one.
    pub scope: Option<&'static str>,
    /// Flag that passes one `KEY=VALUE` environment variable.
    pub env_flag: &'static str,
    /// The env flags go before the server name (`agy` requires it). Otherwise they go after the
    /// name: `claude`'s variadic `-e` would swallow a name that comes after it.
    pub flags_before_name: bool,
}

const MCP_SERVERS_FILE: FileSpec = FileSpec {
    servers_key: "mcpServers",
    allows_jsonc: false,
    adds_stdio_type: false,
};

const DEFAULT_CLI: CliSpec = CliSpec {
    program: "",
    remove_before_add: false,
    scope: None,
    env_flag: "--env",
    flags_before_name: false,
};

static CLAUDE: ClientSpec = ClientSpec {
    cli_name: "claude",
    display_name: "Claude Code / Desktop",
    apply: ApplySpec::File(FileSpec {
        adds_stdio_type: true,
        ..MCP_SERVERS_FILE
    }),
};

static CURSOR: ClientSpec = ClientSpec {
    cli_name: "cursor",
    display_name: "Cursor",
    apply: ApplySpec::File(MCP_SERVERS_FILE),
};

static VSCODE: ClientSpec = ClientSpec {
    cli_name: "vscode",
    display_name: "VS Code Copilot",
    apply: ApplySpec::File(FileSpec {
        servers_key: "github.copilot.chat.mcp",
        ..MCP_SERVERS_FILE
    }),
};

static PROJECT: ClientSpec = ClientSpec {
    cli_name: "project",
    display_name: "Project Local (.mcp.json)",
    apply: ApplySpec::File(MCP_SERVERS_FILE),
};

static WINDSURF: ClientSpec = ClientSpec {
    cli_name: "windsurf",
    display_name: "Windsurf",
    apply: ApplySpec::File(MCP_SERVERS_FILE),
};

static CLINE: ClientSpec = ClientSpec {
    cli_name: "cline",
    display_name: "Cline (VS Code)",
    apply: ApplySpec::File(MCP_SERVERS_FILE),
};

static ROO_CODE: ClientSpec = ClientSpec {
    cli_name: "roo-code",
    display_name: "Roo Code (VS Code)",
    apply: ApplySpec::File(MCP_SERVERS_FILE),
};

static ZED: ClientSpec = ClientSpec {
    cli_name: "zed",
    display_name: "Zed",
    apply: ApplySpec::File(FileSpec {
        servers_key: "context_servers",
        allows_jsonc: true,
        ..MCP_SERVERS_FILE
    }),
};

static CLAUDE_CLI: ClientSpec = ClientSpec {
    cli_name: "claude-cli",
    display_name: "Claude Code CLI",
    apply: ApplySpec::Cli(CliSpec {
        program: "claude",
        remove_before_add: true,
        scope: Some("user"),
        env_flag: "-e",
        ..DEFAULT_CLI
    }),
};

static CODEX_CLI: ClientSpec = ClientSpec {
    cli_name: "codex-cli",
    display_name: "Codex CLI",
    apply: ApplySpec::Cli(CliSpec {
        program: "codex",
        ..DEFAULT_CLI
    }),
};

static COPILOT_CLI: ClientSpec = ClientSpec {
    cli_name: "copilot-cli",
    display_name: "GitHub Copilot CLI",
    apply: ApplySpec::Cli(CliSpec {
        program: "copilot",
        remove_before_add: true,
        ..DEFAULT_CLI
    }),
};

static ANTIGRAVITY_CLI: ClientSpec = ClientSpec {
    cli_name: "antigravity-cli",
    display_name: "Antigravity CLI",
    apply: ApplySpec::Cli(CliSpec {
        program: "agy",
        flags_before_name: true,
        ..DEFAULT_CLI
    }),
};

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

    /// What differs for this client. The only exhaustive `match` over the kinds.
    pub fn spec(&self) -> &'static ClientSpec {
        match self {
            Self::Claude => &CLAUDE,
            Self::Cursor => &CURSOR,
            Self::VsCode => &VSCODE,
            Self::Project => &PROJECT,
            Self::Windsurf => &WINDSURF,
            Self::Cline => &CLINE,
            Self::RooCode => &ROO_CODE,
            Self::Zed => &ZED,
            Self::ClaudeCli => &CLAUDE_CLI,
            Self::CodexCli => &CODEX_CLI,
            Self::CopilotCli => &COPILOT_CLI,
            Self::AntigravityCli => &ANTIGRAVITY_CLI,
        }
    }

    /// The file-merge settings, for file targets.
    pub fn file_spec(&self) -> Option<&'static FileSpec> {
        match &self.spec().apply {
            ApplySpec::File(file) => Some(file),
            ApplySpec::Cli(_) => None,
        }
    }

    /// The CLI-delegation settings, for CLI targets.
    pub fn cli_spec(&self) -> Option<&'static CliSpec> {
        match &self.spec().apply {
            ApplySpec::Cli(cli) => Some(cli),
            ApplySpec::File(_) => None,
        }
    }

    /// Stable identifier used on the command line (`--target <name>`).
    pub fn cli_name(&self) -> &'static str {
        self.spec().cli_name
    }

    /// Inverse of [`cli_name`](Self::cli_name).
    pub fn from_cli_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.cli_name() == name)
    }

    /// Friendly display name of the client target.
    pub fn display_name(&self) -> &'static str {
        self.spec().display_name
    }

    /// How this target is applied (file merge vs. official CLI delegation).
    pub fn action(&self) -> TargetAction {
        match self.spec().apply {
            ApplySpec::File(_) => TargetAction::FileMerge,
            ApplySpec::Cli(_) => TargetAction::CliCommand,
        }
    }

    /// Executable name of the official CLI for CLI-delegation targets.
    pub fn cli_program(&self) -> Option<&'static str> {
        self.cli_spec().map(|cli| cli.program)
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
}

impl ClientTarget {
    /// `exists` is whether the config file is already there (the caller checks; this stays pure).
    pub fn new(kind: ClientTargetKind, path: PathBuf, exists: bool) -> Self {
        Self {
            name: kind.display_name().to_string(),
            kind,
            exists,
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

    #[test]
    fn test_client_target_new_takes_existence_from_the_caller() {
        // 存在しないパスでも、渡された `exists` をそのまま使う（ファイルシステムを見ない）
        let path = PathBuf::from("/definitely/not/a/real/path.json");

        let present = ClientTarget::new(ClientTargetKind::Cursor, path.clone(), true);
        let missing = ClientTarget::new(ClientTargetKind::Cursor, path.clone(), false);

        assert!(present.exists);
        assert!(!missing.exists);
        assert_eq!(present.name, "Cursor");
        assert_eq!(present.path, path);
    }

    #[test]
    fn test_spec_matches_how_each_client_is_applied() {
        for kind in ClientTargetKind::ALL {
            match &kind.spec().apply {
                ApplySpec::File(file) => {
                    assert!(!file.servers_key.is_empty(), "{kind:?}");
                    assert_eq!(kind.action(), TargetAction::FileMerge, "{kind:?}");
                    assert_eq!(kind.file_spec(), Some(file), "{kind:?}");
                    assert_eq!(kind.cli_spec(), None, "{kind:?}");
                    assert_eq!(kind.cli_program(), None, "{kind:?}");
                }
                ApplySpec::Cli(cli) => {
                    assert!(!cli.program.is_empty(), "{kind:?}");
                    assert!(cli.env_flag.starts_with('-'), "{kind:?}");
                    assert_eq!(kind.action(), TargetAction::CliCommand, "{kind:?}");
                    assert_eq!(kind.cli_spec(), Some(cli), "{kind:?}");
                    assert_eq!(kind.file_spec(), None, "{kind:?}");
                    assert_eq!(kind.cli_program(), Some(cli.program), "{kind:?}");
                }
            }
        }
    }
}
