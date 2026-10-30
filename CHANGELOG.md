# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- CLI 委譲ターゲットの「実行予定コマンド」が右端で見切れて全文を確認できない問題を修正。ターゲット一覧の非常に長いパスが右ペインの幅を押し広げ、プレビューが見えている幅より広く折り返されていたため、一覧のパスを残り幅で省略表示（ホバーで全文）に変更し、コマンドは折り返し表示にした

### Changed

- ボトムバーに「適用対象: N 件（うち CLI 実行 M 件）: 名前…」を常時表示し、ターゲット一覧の高さを右ペインに応じて拡大（一覧の画面外にあるチェック済みの CLI ターゲットに気付かず実適用してしまうのを防ぐ）
- CLI 委譲ターゲットの実行失敗メッセージで、`-e` / `--env` の値を `***` に伏せるように（実行予定コマンドのプレビューは従来どおり実値を表示）
- クリップボード自動読み取り（起動時・フォーカス獲得時）の判定を洗練: 有効な MCP 設定としてパースできる内容のみ入力欄へ取り込み、前回判定済みと同一の内容では編集中の入力を上書きしない（手動の読み取りボタン・`Ctrl+V` は従来どおり内容を問わず取り込む）
- 「自動読み取り」トグルの設定を次回起動へ永続化（`eframe` の `persistence` 機能。ウィンドウ位置・サイズは保存しない）

### Removed

- 実在しない `~/.codex/config.json` への Codex ファイルマージターゲットを削除（Codex は `config.toml` を使うため、CLI 委譲ターゲットに一本化）

### Added

- 「↩ 直前の適用を取り消す」ボタンを追加。直前の適用（Dry-Run を除く）で書き込んだファイル系ターゲットを、既存ファイルは `.bak` から復元し、新規作成したファイルは削除して元に戻す。適用後に内容が変わっているファイルは他の変更を失わないよう取り消さず、CLI 委譲ターゲットは取り消せない旨を表示する（取り消しはアプリ起動中の直前の 1 回のみ）
- ファイル系ターゲットのマージプレビューで、サーバーごとに「新規追加 / 上書き / 変更なし」を判定して色分けバッジで表示し、見出しの下に件数を要約。ターゲット一覧には「⚠ 上書き N 件」を表示（既存の定義は丸ごと置き換わるため、既存側にだけある項目が消える場合も「上書き」とする）。CLI 委譲ターゲットは既存の登録を事前に確認できない旨を注記
- `pipx run パッケージ名 …` を `uvx パッケージ名 …` に書き換える最適化トグルを追加（既定 ON）。`--spec` 等のオプション・URL・パス・バージョン指定つきは書き換えず、`python -m` も対象外（モジュール名とパッケージ名が一致しないことがあるため）
- スニペット内の `env` に残ったプレースホルダー（`YOUR_API_KEY`、`<token>`、`PASTE_HERE`、`TODO`、`xxxx` 等の伏せ字、空文字）を検知して警告表示し、左ペインのミニ入力欄で実値を入力・置換できるように（入力値は既定でマスク表示、プレビューと適用に反映。未入力のまま適用した場合はステータスに件数を警告）
- 対応クライアントの拡充: Windsurf, Cline, Roo Code, Zed（AST 直接マージ）
- 公式 CLI 委譲アダプター（`TargetAction::CliCommand`）: Claude Code / Codex / GitHub Copilot / Antigravity CLI を `mcp add` で登録し（Claude Code / Copilot は同名上書き不可のため事前に `mcp remove`）、実行予定コマンドを差分欄にプレビュー表示
- キーボードショートカット操作のサポート（`Ctrl+V`: クリップボード再読み取り、`Ctrl+Enter`: 適用、`Esc`: ウィンドウ閉じる）および UI ヒント表示の追加
- README への実機 UI スクリーンショットおよびステータスバッジ（CI, Codecov, Release, Scoop, License）の追加 (`assets/screenshot.png`)
- コントリビューター向けガイドラインおよび PR テンプレートの追加 (`CONTRIBUTING.md`, `.github/PULL_REQUEST_TEMPLATE.md`)
- 公式 MCP モノグラムをモチーフにしたアプリアイコンの追加
  - 透過 PNG および Windows 用マルチサイズ ICO の同梱 (`assets/icon.png`, `assets/icon.ico`)
  - `eframe` ウィンドウアイコン設定 (`load_app_icon`)
  - `winres` による Windows PE リソースへの実行ファイルアイコン埋め込み (`build.rs`)
- README にインストール・入手方法および SmartScreen 対策ガイダンスを追加
  - Scoop / winget によるパッケージマネージャー経由の導入方法（`scottlz0310/scoop-bucket` 開通、`microsoft/winget-pkgs#464302` 提出中）
  - ソースコードからのビルド手順（`cargo install --git` / `cargo build --release` による SmartScreen 回避）
  - ポータブル ZIP 利用時の Windows Defender SmartScreen 解除手順（GUI / `Unblock-File`）および SHA-256 チェックサム照合方法
- winget 配布用マニフェスト（v0.1.0）の追加 (`dist/winget/`)

### Changed

- `eframe` / `egui` を 0.31.1 から 0.37.0 へ更新
  - `egui::TopBottomPanel` / `egui::SidePanel` の `egui::Panel` への統合、`eframe::App::update` の `ui` への置き換え、`Context::run` の `run_ui` への置き換えに追従
  - `Cargo.lock` に残っていた旧 `windows 0.58.0` を除去し、`wgpu-hal` と `gpu-allocator` の `windows` クレートを 0.62.2 に統一（Windows ビルドでの型不整合の解消）

### Fixed

- リリースワークフロー (`.github/workflows/release.yml`) の Build ジョブが `mkdir dist` の `File exists` で失敗する問題を修正
  - リポジトリに `dist/winget/` が追加されたため、成果物の出力先を `release-assets/` に変更（`attach` ジョブの受け取り先も同名に統一し、将来の `assets/` 追加との衝突も回避）

## [0.1.0] - 2026-10-28

### Fixed

- GUI 上の日本語フォント文字化け（豆腐・□□□表示）の解消
  - OS 標準の日本語/CJKフォント（Windows: Meiryo, Yu Gothic, MS Gothic / Linux: Noto Sans CJK / macOS: PingFang 等）を自動探索し egui フォントフォールバックに登録 (`setup_system_fonts`)
- Windows リリースビルド時の不要なコンソールウィンドウのポップアップを防止 (`windows_subsystem = "windows"`)

### Added

- `LICENSE-MIT` および `LICENSE-APACHE` をリポジトリルートに配置（`MIT OR Apache-2.0` デュアルライセンス全文の同梱）
- `release-automate` によるリリース自動化ワークフローの導入
  - リリース準備ワークフロー (`.github/workflows/prepare-release.yml`): `reusable-prepare-release.yml`（`bump_strategy: rust`）によるバージョン更新と PR 生成
  - リリース・配布ワークフロー (`.github/workflows/release.yml`): draft Release 作成、Windows (`x86_64-pc-windows-msvc`) および Linux (`x86_64-unknown-linux-gnu`) バイナリビルド、ZIP アーカイブおよび SHA256 チェックサム生成、asset 添付、Release 公開
  - リリース asset 添付スクリプト (`scripts/attach-release-assets.sh`)
- egui / eframe による 2-Pane GUI アプリケーションの実装 (`src/app.rs`)
  - OS クリップボードからの自動読み取り・ウィンドウフォーカス獲得時同期・手動読み取り・クリア機能
  - スニペット入力エリアとリアルタイム自動パース
  - `npx → bunx` 置換フィルタの GUI トグル
  - パース結果プレビュー（認識サーバー一覧、コマンド、引数、環境変数）
  - ターゲットクライアント一覧表示（既存/新規バッジ、選択チェックボックス）
  - 選択ターゲットごとのマージ差分プレビュー表示
  - 検証用 Dry-Run シミュレーション機能
  - 選択ターゲットへのアトミック適用 (Apply) 機能
  - アプリケーション状態遷移・パース・適用・エラーハンドリング・ヘッドレスUI描画のユニットテスト
- コードビハインド・main.rs 肥大化防止のアーキテクチャラチェット制約テスト (`tests/architecture_test.rs`)
  - `src/main.rs` の行数制限（80行以下）検査
  - `src/main.rs` へのコア業務ロジックや直接ファイル I/O 直書き禁止キーワード検査
- `src/main.rs` の薄いエントリポイント化（ロガー初期化、ウィンドウサイズ設定、`eframe::run_native` による `Clp2cfgApp` 起動配線のみ）
- Target AST Merger と Safe Atomic Writer の実装
  - 各 AI クライアント（Claude, Cursor, Codex, VS Code, プロジェクトローカル）設定パスの自動検出 (`detect_client_targets`)
  - 既存設定 JSON の未知プロパティや既存構造を温存する差分マージ (`merge_mcp_servers`)
  - Claude ターゲットに対する `type: "stdio"` 自動付与
  - 同一ディレクトリ内の一時ファイル置換による安全なアトミック書き込み (`safe_atomic_write`)
  - 既存設定変更前の `.bak` 自動バックアップ作成機能 (`create_backup_if_exists`)
  - ターゲット検出・マージ・バックアップ・アトミック書き込みのユニットテスト（17件すべて通過）
- Codecov によるテストカバレッジ計測の導入
  - カバレッジポリシー設定 (`codecov.yml`)
  - CI での `cargo llvm-cov` による LCOV レポート生成と Codecov アップロード
  - `rust-toolchain.toml` に `llvm-tools-preview` を追加
- コアデータモデルと Loose JSON Parser の実装
  - `NormalizedServerConfig` への `extra` 拡張フィールド（未知プロパティ温存）対応
  - Markdown コードブロック除去および JSON 候補抽出 (`extract_json_candidate`)
  - 多様な MCP スニペット構造（`mcpServers` ラップ、`mcp` ラップ、単一サーバー定義、マップ形式）の正規化パース (`parse_mcp_snippet`)
  - JSON5（コメント、末尾カンマ、クォート省略）のフォールバックパース対応
  - 名前なし単一サーバー定義におけるサーバー名の自動推定 (`infer_server_name`)
  - パーサーの各種入力パターン・エラーケースを網羅するユニットテスト
- プロジェクト初期基盤の構築
  - Rust クレート構成 (`Cargo.toml`, `src/lib.rs`, `src/main.rs`)
  - コアモジュール構成の定義 (`model`, `parser`, `filter`, `merger`, `writer`)
  - Biome による Markdown / YAML / JSON lint & formatting 設定 (`biome.json`, `package.json`)
  - Lefthook による Git hooks 自動同期 (`lefthook.yml`)
  - GitHub Actions CI ワークフロー (`.github/workflows/ci.yml`)
  - LF 改行コード正規化設定 (`.gitattributes`)
  - `apply_filters` のテーブル駆動ユニットテスト

### Changed

- `Cargo.toml` の `[dev-dependencies]` から重複していた `tempfile` を削除

[Unreleased]: https://github.com/scottlz0310/clp2cfg/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/scottlz0310/clp2cfg/releases/tag/v0.1.0
