# clp2cfg (Clipboard to Config)

Web サイトやドキュメントからコピーした任意の MCP (Model Context Protocol) サーバー設定 JSON スニペットを自動解釈（正規化パース）し、ローカル環境の各 AI クライアントの設定ファイルへ安全に差分マージ（Upsert）する軽量デスクトップ GUI ツールです。

## 主な特徴

- **多様なスニペットの自動解釈**: 完全な設定 JSON、サーバー定義のマップ、単一サーバー定義、JSONC（コメント・末尾カンマ）、Markdown 内のコードブロックを自動判定して正規化
- **対象クライアント設定の自動検出と AST マージ**:
  - Claude Code / Claude Desktop (`~/.claude.json`)
  - Cursor (`~/.cursor/mcp.json`)
  - Codex CLI (`~/.codex/config.json`)
  - VS Code Copilot (`settings.json`)
  - プロジェクト単位設定 (`.mcp.json`)
- **既存設定の温存**: 未知のフィールドや既存ツール設定を破壊せず、MCP 設定のみを安全に注入
- **安全なアトミック書き込み**: 設定反映前に `.bak` バックアップを作成し、一時ファイルからのアトミック置換を実施
- **コマンドフィルタ**: `npx` から `bunx` への自動最適化トグル対応
- **高速・軽量**: Rust + `eframe` (`egui`) による単一バイナリ動作

## 開発環境

- **言語・ツールチェーン**:
  - Rust (最新 stable)
  - Bun (Biome / Lefthook 実行用)

## 開発コマンド

```bash
# 依存関係のインストール（Lefthook フック同期）
bun install

# コード品質チェック（Biome: Markdown, YAML, JSON）
bun run check

# Rust コード検証
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## ライセンス

本プロジェクトは以下のデュアルライセンスの下で公開されています：

- [MIT License](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)
