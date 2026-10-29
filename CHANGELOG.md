# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
