# clp2cfg への貢献

貢献ありがとうございます。変更は小さく、目的を一つに絞ってください。新しい AI クライアントの追加など、既存の課題に関係する作業は、その課題で仕様を確認してから始めてください。

## 開発環境

- Rust stable（バージョンは `rust-toolchain.toml` を参照）
- Bun（バージョンは `.bun-version` を参照）

依存関係をインストールし、Lefthook の Git フックを設定します。

```sh
bun install
```

## 変更の確認

変更に関係するチェックを実行してから PR を開いてください。

```sh
# Markdown、JSON、YAML などの lint と format
bun run check

# Rust の format、lint、テスト
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI でも `bun run check`、Rust の format と Clippy、`cargo llvm-cov` による Rust のテストとカバレッジを確認します。Lefthook はコミット前に変更されたファイルの Biome、Rust ファイルの format と Clippy を実行し、push 前に `cargo test` を実行します。

## アーキテクチャのガードレール

- `src/main.rs` は起動処理と配線だけに保ち、80 行を超えないようにしてください。`tests/architecture_test.rs` が行数と主要ロジックの混入を確認します。
- パース、フィルター、クライアント検出、設定のマージ、ファイル書き込みは、それぞれの既存モジュール（`src/parser.rs`、`src/filter.rs`、`src/merger.rs`、`src/writer.rs` など）に置いてください。
- 設定ファイルの保存には `src/writer.rs` の安全な書き込み処理を使ってください。既存ファイルのバックアップとアトミックな置き換えを維持します。
- マージ時は対象クライアントの MCP 設定だけを更新し、他の設定や未知のフィールドを保ってください。

## AI クライアントを追加する

クライアントごとの設定形式や保存先を確認し、必要な変更を関連するモジュールに分けてください。

1. `src/model.rs` の `ClientTargetKind` に種類を追加し、`display_name` に表示名を定義します。
2. `src/merger.rs` の `detect_client_targets` に、OS ごとの保存先と検出方法を追加します。ユーザーのホームや設定ディレクトリは既存の `dirs` 利用方法に合わせてください。
3. 同じファイルの `merge_mcp_servers` に、そのクライアントが要求する設定キーや形式を追加します。既存の設定を残し、同名サーバーは既存の upsert 動作に合わせて更新します。
4. 関連するテストに、設定形式、既存設定の保持、同名サーバーの更新を確認するケースを追加します。保存先の検出を変更した場合は、対応するパスの動作も確認してください。
5. 新しい入力形式が既存の形式で扱えない場合に限り、`src/parser.rs` を更新します。

## Pull request

PR は `main` を対象にし、変更の目的、関連する issue、実行したチェックを記載してください。設定形式や検出パスの変更では、対象クライアントと OS も明記してください。設定例には実際のトークンや秘密情報を含めないでください。
