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
- パース、フィルター、クライアント検出、設定のマージ、ファイル書き込みは、それぞれの既存モジュール（`src/snippet/parser.rs`、`src/snippet/filter.rs`、`src/merge.rs`、`src/discovery.rs`、`src/writer.rs` など）に置いてください。
- スニペットから「フィルタ済みのサーバー定義 + プレースホルダー」への変換は、`src/snippet/pipeline.rs` の `prepare` を通してください。GUI と headless が同じ経路を使うため、パース・フィルター・プレースホルダーの呼び出しを画面側に直接書かないでください。
- 設定ファイルの保存には `src/writer.rs` の安全な書き込み処理を使ってください。既存ファイルのバックアップとアトミックな置き換えを維持します。
- クライアントへの適用（計画・書き込み・CLI の実行・取り消し）は、`src/apply/` を通してください。GUI と headless が同じ経路を使います。ファイルとサブプロセスは `TargetIo`（実体は `SystemIo`）経由でだけ触るため、画面側から `writer` や `delegate::exec` を直接呼ばないでください。テストでは `src/apply/fake.rs` のインメモリ fake を使えます。読み取れない既存ファイルを空として扱わず、エラーにする点も維持してください。
- headless（`src/headless/`）は `args`（clap）を `use` しません。`HeadlessRequest` を受け取り、入力（クリップボード・標準入力）は `InputEnv`、クライアント検出は `TargetCatalog`、ファイルとサブプロセスは `TargetIo` で受けます。`Args` からの変換は `src/args.rs`、OS の実体の配線は `src/entry.rs` に置いてください。
- GUI の状態と更新（`src/app/state.rs`・`action.rs`・`ports.rs`）は `egui` / `eframe` / `arboard` / ファイル操作を使いません。ユーザーの操作は `UiAction`、ファイルは `TargetIo`、クリップボードは `ClipboardSource` を通し、テストはフェイクで egui なしに書きます。描画（`mod.rs`）は状態を読んで `UiAction` を渡すだけにしてください。OS の実体（`ArboardClipboard`、フォント、アイコン）は `clipboard.rs` / `system.rs` に置きます。
- マージ時は対象クライアントの MCP 設定だけを更新し、他の設定や未知のフィールドを保ってください。

### モジュールの依存方向

依存は上の層から下の層への一方向だけにします（設計の背景と移行計画は #57）。`tests/architecture_test.rs` が `src/**/*.rs` の本体部分（`#[cfg(test)]` より前）を走査して検査します。

| 層              | 置くもの                                                 | 依存してよい先 | 使えないもの                     |
| --------------- | -------------------------------------------------------- | -------------- | -------------------------------- |
| L0 ドメイン     | サーバー定義、クライアント定義                           | なし           | ファイル・OS・UI の操作          |
| L1 純粋ロジック | パース、フィルター、マージ、コマンド生成                 | L0             | ファイル・OS・UI の操作          |
| L2 副作用       | ファイルの書き込み・復元、クライアント検出、プロセス実行 | L0, L1         | UI                               |
| L3 ユースケース | プレビュー、適用（副作用は port 経由）                   | L0-L2          | UI、ファイル・OS の直接操作      |
| L4 GUI の状態   | `app/state.rs` など（状態と `UiAction` による更新）      | L0, L1, L3     | UI・OS、他の frontend、L2        |
| L4 frontend     | コマンドライン、GUI の描画と OS 実装                     | L0, L1, L3     | 他の frontend、L2 の直接呼び出し |
| L5 配線         | `entry.rs`                                               | すべて         | ロジック                         |

- ファイルを追加・移動・削除したら、`FILE_LAYERS`（ファイル → 層の表）を同じ変更で更新してください。表に無いファイルがあるとテストが失敗します。
- 層の規則に反する依存は、`ALLOWLIST` に「理由」と「解消する Issue」つきで固定しています。リストに無い違反を増やさず、解消した変更でその行を削除してください（違反が消えたのに行が残っていてもテストが失敗します）。
- 検査は文字列走査のため、誤検知した場合は `tests/architecture_test/scanner.rs` を直してください。許可リストで逃げないでください。
- 外部 crate の使用の検査は、workspace 分割（#68）で crate の境界に置き換わる予定です。

## AI クライアントを追加する

クライアントごとの設定形式や保存先を確認し、必要な変更を関連するモジュールに分けてください。

クライアントごとの違いは、`src/domain/client.rs` の `ClientSpec`（静的なデータ表）に集めています。ファイルを直接マージするクライアントは `FileSpec`（設定キー、既存の JSONC を許すか、`"type": "stdio"` を補うか）、公式 CLI へ委譲するクライアントは `CliSpec`（実行ファイル名、追加前の削除、`--scope`、環境変数のフラグ、フラグを名前の前に置くか）で表します。

1. `src/domain/client.rs` の `ClientTargetKind` に種類を追加し、`ALL` にも加えます。同じファイルに `ClientSpec` の定義（`--target` の名前、表示名、`FileSpec` または `CliSpec`）を書き、`spec()` の `match` に加えます（加え忘れるとコンパイルエラーになります）。
2. ファイルを直接マージするクライアントは、`src/discovery.rs` の `config_path` に、OS ごとの保存先を追加します（`match` が網羅されているため、追加し忘れるとコンパイルエラーになります）。ユーザーのホームや設定ディレクトリは `HostDirs` から受け取ります。公式 CLI へ委譲するクライアントは、PATH 上の実行ファイルの検出が自動で行われるため、追加は要りません。
3. 既存の設定を残し、同名サーバーは既存の upsert 動作に合わせて更新することを確認します。`src/merge.rs` のマージと `src/delegate/plan.rs` のコマンド生成は `ClientSpec` を読むため、通常はコードの変更は要りません。新しい形式の差が `FileSpec` / `CliSpec` で表せない場合に限り、項目を足してください。
4. テストを更新します。`src/domain/client.rs` の `test_every_client_keeps_its_identity` に 1 行足し、`src/merge.rs` の `test_file_targets_keep_their_servers_key_and_quirks`（ファイル種）または `src/delegate/plan.rs` のコマンド列のテスト（CLI 種）に、そのクライアントのケースを加えます。保存先は `src/discovery.rs` の `test_config_path_for_every_client` に 1 行足します。
5. 新しい入力形式が既存の形式で扱えない場合に限り、`src/snippet/parser.rs` を更新します。

## Pull request

PR は `main` を対象にし、変更の目的、関連する issue、実行したチェックを記載してください。設定形式や検出パスの変更では、対象クライアントと OS も明記してください。設定例には実際のトークンや秘密情報を含めないでください。
