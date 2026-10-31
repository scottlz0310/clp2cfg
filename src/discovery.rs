//! Finds the AI clients on this machine: config file locations (per OS) and official CLIs on `PATH`.
//! The environment and the file system are read here; the rest of the app receives `ClientTarget`s.

use crate::domain::{ClientTarget, ClientTargetKind};
use std::path::{Path, PathBuf};

/// Detect standard client configuration targets on the host system.
pub fn detect_client_targets(project_dir: Option<&Path>) -> Vec<ClientTarget> {
    let mut targets = Vec::new();
    let home = dirs::home_dir();

    // 1. Claude Code / Desktop
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Claude,
            h.join(".claude.json"),
        ));
    }

    // 2. Cursor
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Cursor,
            h.join(".cursor").join("mcp.json"),
        ));
    }

    // 3. VS Code Copilot settings.json
    let vscode_user = vscode_user_dir(home.as_deref());
    if let Some(ref dir) = vscode_user {
        targets.push(ClientTarget::new(
            ClientTargetKind::VsCode,
            dir.join("settings.json"),
        ));
    }

    // 4. Project Local (.mcp.json or mcp.json)
    let cwd = std::env::current_dir().ok();
    let base_dir = project_dir.or(cwd.as_deref());

    if let Some(dir) = base_dir {
        let dot_mcp = dir.join(".mcp.json");
        let plain_mcp = dir.join("mcp.json");
        let project_mcp_path = if plain_mcp.is_file() {
            plain_mcp
        } else {
            dot_mcp
        };
        targets.push(ClientTarget::new(
            ClientTargetKind::Project,
            project_mcp_path,
        ));
    }

    // 6. Windsurf
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Windsurf,
            h.join(".codeium").join("windsurf").join("mcp_config.json"),
        ));
    }

    // 7. Cline / 8. Roo Code (VS Code extensions, stored in globalStorage)
    if let Some(ref dir) = vscode_user {
        let storage = dir.join("globalStorage");
        targets.push(ClientTarget::new(
            ClientTargetKind::Cline,
            storage
                .join("saoudrizwan.claude-dev")
                .join("settings")
                .join("cline_mcp_settings.json"),
        ));
        targets.push(ClientTarget::new(
            ClientTargetKind::RooCode,
            storage
                .join("rooveterinaryinc.roo-cline")
                .join("settings")
                .join("mcp_settings.json"),
        ));
    }

    // 9. Zed
    let zed_dir = if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("Zed"))
    } else {
        home.as_ref().map(|h| h.join(".config").join("zed"))
    };
    if let Some(dir) = zed_dir {
        targets.push(ClientTarget::new(
            ClientTargetKind::Zed,
            dir.join("settings.json"),
        ));
    }

    // 10. CLI delegation targets (detected on PATH)
    for kind in [
        ClientTargetKind::ClaudeCli,
        ClientTargetKind::CodexCli,
        ClientTargetKind::CopilotCli,
        ClientTargetKind::AntigravityCli,
    ] {
        let resolved = kind.cli_program().and_then(find_in_path);
        targets.push(ClientTarget::new_cli(kind, resolved));
    }

    targets
}

/// VS Code `User` directory (holds `settings.json` and `globalStorage`).
fn vscode_user_dir(home: Option<&Path>) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("Code").join("User"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
        })
    } else {
        dirs::config_dir().map(|c| c.join("Code").join("User"))
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
