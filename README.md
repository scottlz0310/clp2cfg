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

## インストール・入手方法

### 1. パッケージマネージャー（推奨）

Windows ではパッケージマネージャー経由でインストールすることで、環境変数 PATH の自動設定や、ブラウザダウンロード特有の **SmartScreen 警告（Mark-of-the-Web）を完全に回避** できます。

#### Scoop

個人バケットより即座にインストール・利用可能です：

```powershell
# バケットを追加してインストール
scoop bucket add scottlz0310 https://github.com/scottlz0310/scoop-bucket
scoop install clp2cfg
```

#### winget (Windows Package Manager)

現在 Microsoft 公式リポジトリへ登録申請中です（[microsoft/winget-pkgs#464302](https://github.com/microsoft/winget-pkgs/pull/464302)）。審査・自動パイプライン完了後に以下のコマンドで利用可能になります：

```powershell
winget install scottlz0310.clp2cfg
```

> [!NOTE]
> winget の審査・マージ完了までの間は、Scoop または以下のソースコードビルド / ポータブル ZIP をご利用ください。

### 2. ソースコードからビルド（SmartScreen 警告なし）

Rust (Cargo) 開発環境がある場合、ローカルでビルド・インストールすることで改ざんの心配なく安全に利用できます。ローカルでコンパイルされたバイナリには Mark-of-the-Web (Zone.Identifier) が付与されないため、**SmartScreen 警告は一切発生しません**。

```powershell
# 1 コマンドで GitHub から直接インストール
cargo install --git https://github.com/scottlz0310/clp2cfg

# またはソースコードをクローンしてビルド
git clone https://github.com/scottlz0310/clp2cfg.git
cd clp2cfg
cargo build --release
# target/release/clp2cfg.exe が生成されます
```

### 3. ポータブル ZIP のダウンロード（単体実行）

[GitHub Releases](https://github.com/scottlz0310/clp2cfg/releases) から最新の ZIP アーカイブ（Windows 用: `clp2cfg-x86_64-pc-windows-msvc.zip`、Linux 用: `clp2cfg-x86_64-unknown-linux-gnu.zip`）をダウンロードし、任意のフォルダに展開してそのまま実行できます。

#### Windows Defender SmartScreen 警告について

ブラウザ経由でダウンロードした未署名の実行可能ファイルには、Windows によって Mark-of-the-Web（外部ダウンロード識別情報）が付与されるため、初回起動時に青い画面で **「Windows によって PC が保護されました」** と表示される場合があります。

以下のいずれかの方法で解除して実行できます：

- **GUI で解除**:
  「詳細情報」をクリック ➜ 表示される **「実行」** ボタンを押す。
- **PowerShell で解除（推奨）**:
  展開した `clp2cfg.exe` に対し以下のコマンドを実行すると、MotW 属性が解除されて警告が出なくなります：

  ```powershell
  Unblock-File .\clp2cfg.exe
  ```
- **完全性の検証（チェックサム照合）**:
  ダウンロードしたバイナリが改ざんされていないか確認するには、Release に添付されている `SHA256SUMS.txt` と以下のハッシュ値を照合してください：

  ```powershell
  Get-FileHash .\clp2cfg.exe -Algorithm SHA256
  ```

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
