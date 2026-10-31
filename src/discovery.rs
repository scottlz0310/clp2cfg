//! Finds the AI clients on this machine: config file locations (per OS) and official CLIs on `PATH`.
//! The environment and the file system are read here; the rest of the app receives `ClientTarget`s.

use crate::domain::{ApplySpec, ClientTarget, ClientTargetKind};
use std::path::{Path, PathBuf};

/// Where the host keeps what clients write to (injectable for tests).
#[derive(Default)]
struct HostDirs {
    home: Option<PathBuf>,
    /// `%APPDATA%` on Windows.
    data_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    /// The directory of the project-local target (the current directory unless given).
    project_dir: Option<PathBuf>,
}

impl HostDirs {
    fn detect(project_dir: Option<&Path>) -> Self {
        Self {
            home: dirs::home_dir(),
            data_dir: dirs::data_dir(),
            config_dir: dirs::config_dir(),
            project_dir: project_dir
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok()),
        }
    }
}

/// Detect standard client configuration targets on the host system, in [`ClientTargetKind::ALL`] order.
pub fn detect_client_targets(project_dir: Option<&Path>) -> Vec<ClientTarget> {
    detect_with(&HostDirs::detect(project_dir), find_in_path)
}

fn detect_with(
    dirs: &HostDirs,
    find_program: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<ClientTarget> {
    ClientTargetKind::ALL
        .into_iter()
        .filter_map(|kind| match &kind.spec().apply {
            ApplySpec::File(_) => config_path(kind, dirs).map(|path| file_target(kind, path)),
            ApplySpec::Cli(cli) => Some(ClientTarget::new_cli(kind, find_program(cli.program))),
        })
        .collect()
}

/// A file target; it exists (and is selected by default) when the config file is already there.
pub(crate) fn file_target(kind: ClientTargetKind, path: PathBuf) -> ClientTarget {
    let exists = path.is_file();
    ClientTarget::new(kind, path, exists)
}

/// Where a file target's configuration lives, or `None` when the host does not have the
/// directory it lives under (or the kind is applied through its CLI).
/// The only exhaustive `match` over the kinds besides `ClientTargetKind::spec`.
fn config_path(kind: ClientTargetKind, dirs: &HostDirs) -> Option<PathBuf> {
    let home = dirs.home.as_deref();
    match kind {
        ClientTargetKind::Claude => home.map(|h| h.join(".claude.json")),
        ClientTargetKind::Cursor => home.map(|h| h.join(".cursor").join("mcp.json")),
        ClientTargetKind::VsCode => vscode_user_dir(dirs).map(|d| d.join("settings.json")),
        ClientTargetKind::Project => dirs.project_dir.as_deref().map(|dir| {
            let plain_mcp = dir.join("mcp.json");
            if plain_mcp.is_file() {
                plain_mcp
            } else {
                dir.join(".mcp.json")
            }
        }),
        ClientTargetKind::Windsurf => {
            home.map(|h| h.join(".codeium").join("windsurf").join("mcp_config.json"))
        }
        // VS Code extensions keep their settings in globalStorage
        ClientTargetKind::Cline => vscode_user_dir(dirs).map(|d| {
            d.join("globalStorage")
                .join("saoudrizwan.claude-dev")
                .join("settings")
                .join("cline_mcp_settings.json")
        }),
        ClientTargetKind::RooCode => vscode_user_dir(dirs).map(|d| {
            d.join("globalStorage")
                .join("rooveterinaryinc.roo-cline")
                .join("settings")
                .join("mcp_settings.json")
        }),
        ClientTargetKind::Zed => {
            let zed_dir = if cfg!(target_os = "windows") {
                dirs.data_dir.as_deref().map(|d| d.join("Zed"))
            } else {
                home.map(|h| h.join(".config").join("zed"))
            };
            zed_dir.map(|d| d.join("settings.json"))
        }
        ClientTargetKind::ClaudeCli
        | ClientTargetKind::CodexCli
        | ClientTargetKind::CopilotCli
        | ClientTargetKind::AntigravityCli => None,
    }
}

/// VS Code `User` directory (holds `settings.json` and `globalStorage`).
fn vscode_user_dir(dirs: &HostDirs) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        dirs.data_dir
            .as_deref()
            .map(|d| d.join("Code").join("User"))
    } else if cfg!(target_os = "macos") {
        dirs.home.as_deref().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
        })
    } else {
        dirs.config_dir
            .as_deref()
            .map(|c| c.join("Code").join("User"))
    }
}

/// Locate an executable on `PATH`.
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    find_in_dirs(program, std::env::split_paths(&paths))
}

fn find_in_dirs(program: &str, dirs: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    for dir in dirs {
        for ext in exts {
            let candidate = if ext.is_empty() {
                dir.join(program)
            } else {
                dir.join(format!("{program}.{ext}"))
            };
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dirs(project_dir: &Path) -> HostDirs {
        HostDirs {
            home: Some(PathBuf::from("/h")),
            data_dir: Some(PathBuf::from("/d")),
            config_dir: Some(PathBuf::from("/c")),
            project_dir: Some(project_dir.to_path_buf()),
        }
    }

    #[test]
    fn test_config_path_for_every_client() {
        use ClientTargetKind::*;
        let project = tempfile::tempdir().unwrap();
        let dirs = fake_dirs(project.path());
        let home = PathBuf::from("/h");
        let vscode_user = if cfg!(target_os = "windows") {
            PathBuf::from("/d/Code/User")
        } else if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Code/User")
        } else {
            PathBuf::from("/c/Code/User")
        };
        let zed = if cfg!(target_os = "windows") {
            PathBuf::from("/d/Zed")
        } else {
            home.join(".config/zed")
        };
        let storage = vscode_user.join("globalStorage");

        let expected = [
            (Claude, Some(home.join(".claude.json"))),
            (Cursor, Some(home.join(".cursor/mcp.json"))),
            (VsCode, Some(vscode_user.join("settings.json"))),
            (Project, Some(project.path().join(".mcp.json"))),
            (
                Windsurf,
                Some(home.join(".codeium/windsurf/mcp_config.json")),
            ),
            (
                Cline,
                Some(storage.join("saoudrizwan.claude-dev/settings/cline_mcp_settings.json")),
            ),
            (
                RooCode,
                Some(storage.join("rooveterinaryinc.roo-cline/settings/mcp_settings.json")),
            ),
            (Zed, Some(zed.join("settings.json"))),
            (ClaudeCli, None),
            (CodexCli, None),
            (CopilotCli, None),
            (AntigravityCli, None),
        ];

        assert_eq!(
            expected.clone().map(|(kind, _)| kind),
            ClientTargetKind::ALL
        );
        for (kind, path) in expected {
            assert_eq!(config_path(kind, &dirs), path, "{kind:?}");
        }
    }

    #[test]
    fn test_config_path_needs_the_host_directories() {
        use ClientTargetKind::*;
        let project = tempfile::tempdir().unwrap();
        let only_project = HostDirs {
            project_dir: Some(project.path().to_path_buf()),
            ..HostDirs::default()
        };

        for kind in ClientTargetKind::ALL {
            assert_eq!(config_path(kind, &HostDirs::default()), None, "{kind:?}");
            assert_eq!(
                config_path(kind, &only_project).is_some(),
                kind == Project,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn test_project_target_prefers_an_existing_plain_mcp_json() {
        let project = tempfile::tempdir().unwrap();
        let dirs = fake_dirs(project.path());
        let dot_mcp = project.path().join(".mcp.json");
        let plain_mcp = project.path().join("mcp.json");

        assert_eq!(
            config_path(ClientTargetKind::Project, &dirs),
            Some(dot_mcp.clone())
        );

        std::fs::write(&dot_mcp, "{}").unwrap();
        assert_eq!(config_path(ClientTargetKind::Project, &dirs), Some(dot_mcp));

        std::fs::write(&plain_mcp, "{}").unwrap();
        assert_eq!(
            config_path(ClientTargetKind::Project, &dirs),
            Some(plain_mcp)
        );
    }

    #[test]
    fn test_detect_with_follows_all_and_resolves_cli_programs() {
        let project = tempfile::tempdir().unwrap();
        let find = |program: &str| (program == "claude").then(|| PathBuf::from("/bin/claude"));

        let targets = detect_with(&fake_dirs(project.path()), find);

        assert_eq!(
            targets.iter().map(|t| t.kind).collect::<Vec<_>>(),
            ClientTargetKind::ALL
        );
        let by_kind = |kind| targets.iter().find(|t| t.kind == kind).unwrap();
        let claude = by_kind(ClientTargetKind::ClaudeCli);
        assert_eq!(claude.path, PathBuf::from("/bin/claude"));
        assert!(claude.exists && claude.selected);
        let codex = by_kind(ClientTargetKind::CodexCli);
        assert_eq!(codex.path, PathBuf::from("codex"), "未検出は裸の名前");
        assert!(!codex.exists && !codex.selected);
    }

    #[test]
    fn test_detect_with_skips_file_targets_without_host_directories() {
        let targets = detect_with(&HostDirs::default(), |_| None);

        assert_eq!(
            targets.iter().map(|t| t.kind).collect::<Vec<_>>(),
            [
                ClientTargetKind::ClaudeCli,
                ClientTargetKind::CodexCli,
                ClientTargetKind::CopilotCli,
                ClientTargetKind::AntigravityCli,
            ]
        );
    }

    #[test]
    fn test_file_target_exists_and_is_selected_only_when_the_file_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");

        let missing = file_target(ClientTargetKind::Cursor, path.clone());
        std::fs::write(&path, "{}").unwrap();
        let present = file_target(ClientTargetKind::Cursor, path);

        assert!(!missing.exists && !missing.selected);
        assert!(present.exists && present.selected);
    }

    #[test]
    fn test_detect_targets_include_new_clients() {
        let kinds: Vec<_> = detect_client_targets(None).iter().map(|t| t.kind).collect();
        for k in [
            ClientTargetKind::Windsurf,
            ClientTargetKind::Cline,
            ClientTargetKind::RooCode,
            ClientTargetKind::Zed,
            ClientTargetKind::ClaudeCli,
            ClientTargetKind::AntigravityCli,
        ] {
            assert!(kinds.contains(&k), "{k:?} missing");
        }
    }

    /// 検出結果の並び順と、種類ごとの保存先（ホームや設定ディレクトリからの相対パス）を固定する
    #[test]
    fn test_detect_client_targets_order_and_paths() {
        let home = dirs::home_dir().expect("home dir");
        let data_dir = dirs::data_dir().expect("data dir");
        let vscode_user = if cfg!(target_os = "windows") {
            data_dir.join("Code").join("User")
        } else if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
        } else {
            dirs::config_dir()
                .expect("config dir")
                .join("Code")
                .join("User")
        };
        let storage = vscode_user.join("globalStorage");
        let zed = if cfg!(target_os = "windows") {
            data_dir.join("Zed")
        } else {
            home.join(".config").join("zed")
        };

        let project = tempfile::tempdir().unwrap();
        let dot_mcp = project.path().join(".mcp.json");
        let plain_mcp = project.path().join("mcp.json");

        let targets = detect_client_targets(Some(project.path()));
        assert_eq!(
            targets.iter().map(|t| t.kind).collect::<Vec<_>>(),
            ClientTargetKind::ALL,
            "ALL と同じ並び順"
        );
        let path_of = |kind| {
            targets
                .iter()
                .find(|t| t.kind == kind)
                .map(|t| t.path.clone())
                .unwrap()
        };
        assert_eq!(path_of(ClientTargetKind::Claude), home.join(".claude.json"));
        assert_eq!(
            path_of(ClientTargetKind::Cursor),
            home.join(".cursor").join("mcp.json")
        );
        assert_eq!(
            path_of(ClientTargetKind::VsCode),
            vscode_user.join("settings.json")
        );
        assert_eq!(
            path_of(ClientTargetKind::Project),
            dot_mcp,
            "既定は .mcp.json"
        );
        assert_eq!(
            path_of(ClientTargetKind::Windsurf),
            home.join(".codeium")
                .join("windsurf")
                .join("mcp_config.json")
        );
        assert_eq!(
            path_of(ClientTargetKind::Cline),
            storage
                .join("saoudrizwan.claude-dev")
                .join("settings")
                .join("cline_mcp_settings.json")
        );
        assert_eq!(
            path_of(ClientTargetKind::RooCode),
            storage
                .join("rooveterinaryinc.roo-cline")
                .join("settings")
                .join("mcp_settings.json")
        );
        assert_eq!(path_of(ClientTargetKind::Zed), zed.join("settings.json"));

        // CLI 種は PATH 上の実行ファイル（見つからなければ裸の名前）。名前と存在判定の整合だけを見る
        for target in targets.iter().filter(|t| t.kind.cli_program().is_some()) {
            let program = target.kind.cli_program().unwrap();
            assert_eq!(target.exists, target.selected, "{:?}", target.kind);
            assert_eq!(target.exists, find_in_path(program).is_some());
            if !target.exists {
                assert_eq!(target.path, std::path::PathBuf::from(program));
            }
        }

        // `mcp.json` があればそちらを使う
        std::fs::write(&plain_mcp, "{}").unwrap();
        let targets = detect_client_targets(Some(project.path()));
        let project_target = targets
            .iter()
            .find(|t| t.kind == ClientTargetKind::Project)
            .unwrap();
        assert_eq!(project_target.path, plain_mcp);
        assert!(project_target.exists && project_target.selected);
    }

    #[test]
    fn test_detect_targets_non_empty() {
        let targets = detect_client_targets(None);
        assert!(!targets.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_find_in_dirs_requires_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mytool");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        assert!(find_in_dirs("mytool", [dir.path().to_path_buf()].into_iter()).is_none());
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_in_dirs("mytool", [dir.path().to_path_buf()].into_iter()),
            Some(exe)
        );
    }
}
