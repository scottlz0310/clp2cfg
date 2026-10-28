# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
