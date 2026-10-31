//! GUI 版（`clp2cfg`）とコンソール版（`clp2cfg-cli`）で共通の起動引数。

use crate::domain::ClientTargetKind;
use clap::builder::PossibleValuesParser;
use clap::{ArgGroup, Parser};

/// MCP サーバー設定のスニペットを、AI クライアントの設定ファイルへマージします。
///
/// `--apply` / `--dry-run` がなければ GUI を開きます（INPUT があれば流し込み済みで開きます）。
/// どちらかを付けるとウィンドウを出さずにヘッドレスで実行します。
#[derive(Debug, Default, Parser, PartialEq, Eq)]
#[command(name = "clp2cfg", version, about, long_about)]
#[command(group(ArgGroup::new("mode").args(["apply", "dry_run"])))]
pub struct Args {
    /// マージするスニペット。ファイルパス、JSON 文字列、または標準入力を表す `-`。
    /// ヘッドレスで INPUT を省略するとクリップボードを読みます。
    #[arg(value_name = "INPUT")]
    pub input: Option<String>,

    /// ヘッドレスで適用する（ウィンドウなし）。対象のクライアントへマージして書き込みます。
    #[arg(long)]
    pub apply: bool,

    /// ヘッドレスでシミュレーションする。検証して変更内容を表示するだけで、何も書き込みません。
    #[arg(long)]
    pub dry_run: bool,

    /// 適用先のクライアント（繰り返し指定、またはカンマ区切り）。省略時は既存の設定ファイルすべて。
    /// 公式 CLI へ委譲するクライアントは、ここで名前を指定したときだけ実行します。
    #[arg(
        long = "target",
        value_name = "NAME",
        value_delimiter = ',',
        requires = "mode",
        value_parser = PossibleValuesParser::new(ClientTargetKind::ALL.map(|kind| kind.cli_name()))
    )]
    pub targets: Vec<String>,

    /// `--dry-run` の出力で、env の値を `***` に伏せずそのまま表示する。
    #[arg(long, requires = "mode", conflicts_with = "apply")]
    pub show_secrets: bool,

    /// env の値がプレースホルダー（`YOUR_API_KEY`、`<token>` など）のままでも適用する。
    #[arg(long, requires = "mode")]
    pub allow_placeholders: bool,

    /// `npx` を `bunx` に置き換えず、そのまま残す。
    #[arg(long)]
    pub no_npx_to_bunx: bool,

    /// `pipx run` を `uvx` に置き換えず、そのまま残す。
    #[arg(long)]
    pub no_pipx_to_uvx: bool,
}

/// What a headless run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessAction {
    Apply,
    DryRun,
}

/// Whether the arguments ask for the GUI or a headless run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Gui,
    Headless(HeadlessAction),
}

impl Args {
    pub fn mode(&self) -> Mode {
        match (self.apply, self.dry_run) {
            (true, _) => Mode::Headless(HeadlessAction::Apply),
            (_, true) => Mode::Headless(HeadlessAction::DryRun),
            _ => Mode::Gui,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::error::ErrorKind> {
        Args::try_parse_from(std::iter::once("clp2cfg").chain(args.iter().copied()))
            .map_err(|e| e.kind())
    }

    #[test]
    fn test_parse_valid_arguments() {
        struct Case {
            name: &'static str,
            args: &'static [&'static str],
            mode: Mode,
            input: Option<&'static str>,
            targets: &'static [&'static str],
        }

        let cases = [
            Case {
                name: "no arguments opens the GUI",
                args: &[],
                mode: Mode::Gui,
                input: None,
                targets: &[],
            },
            Case {
                name: "input only opens the GUI with the input",
                args: &["{\"a\":1}"],
                mode: Mode::Gui,
                input: Some("{\"a\":1}"),
                targets: &[],
            },
            Case {
                name: "--apply from the clipboard",
                args: &["--apply"],
                mode: Mode::Headless(HeadlessAction::Apply),
                input: None,
                targets: &[],
            },
            Case {
                name: "--dry-run with a file input",
                args: &["--dry-run", "snippet.json"],
                mode: Mode::Headless(HeadlessAction::DryRun),
                input: Some("snippet.json"),
                targets: &[],
            },
            Case {
                name: "stdin marker",
                args: &["--apply", "-"],
                mode: Mode::Headless(HeadlessAction::Apply),
                input: Some("-"),
                targets: &[],
            },
            Case {
                name: "repeated and comma separated targets",
                args: &[
                    "--apply",
                    "--target",
                    "claude,cursor",
                    "--target",
                    "codex-cli",
                ],
                mode: Mode::Headless(HeadlessAction::Apply),
                input: None,
                targets: &["claude", "cursor", "codex-cli"],
            },
        ];

        for case in cases {
            let args = parse(case.args).unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
            assert_eq!(args.mode(), case.mode, "{}", case.name);
            assert_eq!(args.input.as_deref(), case.input, "{}", case.name);
            assert_eq!(args.targets, case.targets, "{}", case.name);
        }
    }

    #[test]
    fn test_parse_rejects_invalid_combinations() {
        struct Case {
            name: &'static str,
            args: &'static [&'static str],
            kind: clap::error::ErrorKind,
        }

        let cases = [
            Case {
                name: "--apply and --dry-run are mutually exclusive",
                args: &["--apply", "--dry-run"],
                kind: clap::error::ErrorKind::ArgumentConflict,
            },
            Case {
                name: "--target needs a headless mode",
                args: &["--target", "claude"],
                kind: clap::error::ErrorKind::MissingRequiredArgument,
            },
            Case {
                name: "--allow-placeholders needs a headless mode",
                args: &["--allow-placeholders"],
                kind: clap::error::ErrorKind::MissingRequiredArgument,
            },
            Case {
                name: "--show-secrets only makes sense with --dry-run",
                args: &["--apply", "--show-secrets"],
                kind: clap::error::ErrorKind::ArgumentConflict,
            },
            Case {
                name: "--show-secrets needs a headless mode",
                args: &["--show-secrets"],
                kind: clap::error::ErrorKind::MissingRequiredArgument,
            },
            Case {
                name: "unknown target name",
                args: &["--apply", "--target", "notepad"],
                kind: clap::error::ErrorKind::InvalidValue,
            },
            Case {
                name: "unknown option",
                args: &["--frobnicate"],
                kind: clap::error::ErrorKind::UnknownArgument,
            },
        ];

        for case in cases {
            assert_eq!(parse(case.args).unwrap_err(), case.kind, "{}", case.name);
        }
    }

    #[test]
    fn test_filter_flags_work_in_gui_mode_too() {
        let args = parse(&["--no-npx-to-bunx", "--no-pipx-to-uvx"]).unwrap();
        assert_eq!(args.mode(), Mode::Gui);
        assert!(args.no_npx_to_bunx && args.no_pipx_to_uvx);
    }

    #[test]
    fn test_target_names_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for kind in ClientTargetKind::ALL {
            assert!(
                seen.insert(kind.cli_name()),
                "duplicate {}",
                kind.cli_name()
            );
            assert_eq!(ClientTargetKind::from_cli_name(kind.cli_name()), Some(kind));
        }
        assert_eq!(ClientTargetKind::from_cli_name("nope"), None);

        // 検出されるターゲットはすべて名前を持つ（ALL の登録漏れ防止）
        for target in crate::discovery::detect_client_targets(None) {
            assert!(ClientTargetKind::ALL.contains(&target.kind));
        }
    }
}
