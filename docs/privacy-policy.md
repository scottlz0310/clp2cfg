# プライバシーポリシー / Privacy Policy

最終更新日 / Last updated: 2026-11-17

- [日本語](#日本語)
- [English](#english)

---

## 日本語

`clp2cfg`（GUI 版 `clp2cfg` およびコンソール版 `clp2cfg-cli`、以下「本アプリ」）は、MCP（Model Context Protocol）サーバー設定のスニペットを解釈し、ローカル環境の AI クライアント設定へ反映するツールです。開発者は個人（GitHub: [scottlz0310](https://github.com/scottlz0310)）です。

本アプリは、あなたが入力した設定スニペット、環境変数、API キー、ローカルの設定ファイルの内容を、開発者や第三者へ送信しません。このポリシーは、本アプリが扱う情報と、その取り扱いを説明します。

### 開発者が収集する情報

**開発者は、あなたの個人情報を収集しません。** 本アプリは、開発者が運営するサーバーや外部の解析基盤へ通信しません。アカウント、広告、独自のクラッシュレポート送信、利用状況の解析サービスも、本アプリには含まれません。また、設定スニペットに含まれる外部 URL やコマンド先へ、本アプリ自身が通信することはありません。

### クリップボードと入力データの扱い

本アプリは、以下の操作時にクリップボードのテキストを読み取ります。

- GUI 版で「自動読み取り」が有効なときの起動時およびウィンドウフォーカス獲得時
- GUI 版での「読み取り」ボタン押下または貼り付け操作時
- コンソール版（`clp2cfg-cli`）またはヘッドレス実行で入力ソースを省略したとき

クリップボード、ファイル、標準入力、または引数から受け取った設定スニペットには、API キーや認証トークン等の環境変数・HTTP ヘッダーが含まれる場合があります。本アプリはこれらを**すべて端末内のメモリ上でのみ処理**し、外部へ送信したり、入力履歴として永続保存したりしません。

`--dry-run` の出力および CLI 委譲エラー（実行コマンド表記、および失敗時の標準エラー出力等のエラーメッセージ）では、本アプリが渡した環境変数や HTTP ヘッダーの実値（`KEY=VALUE` / `Header: Value` 形式、および 4 文字以上の値）を `***` としてマスク表示します（また、GUI のプレースホルダー入力欄では既定で伏せ字表示されます）。なお、設定ファイル・外部 CLI へ実際に適用する際や、GUI のプレビュー表示・「値を表示」切替時には実値を使用します。また、1〜3 文字の短い値が環境変数等のペア形式ではなく単体でエラー文に現れた場合は、一般的なエラー文字列との誤置換を避けるためマスク対象外となります。

### ローカルの設定ファイルと外部 CLI へのアクセス

本アプリは、あなたが選択または指定したローカルの AI クライアント（Claude Code / Desktop、Cursor、VS Code Copilot、Project Local `.mcp.json`、Windsurf、Cline、Roo Code、Zed）の設定ファイルを読み取り、マージ結果を書き込みます。既存ファイルを上書きする際は、同じディレクトリにバックアップファイル（`.bak`）を作成し、直前の適用の取り消し時に復元（新規作成したファイルは削除）できるようにします。

CLI 委譲対象（Claude Code CLI、Codex CLI、GitHub Copilot CLI、Antigravity CLI）を選択して適用を実行した場合は、ローカル環境にインストールされている各公式 CLI（`claude`、`codex`、`copilot`、`agy`）をサブプロセスとして実行し、サーバー設定を渡します。これらの外部 CLI 自体が行う通信やデータ処理には、各ツールおよび提供元のプライバシーポリシーが適用されます。

### あなたの端末に保存される情報

GUI 版の本アプリは、あなたの Windows ユーザーまたは OS のローカルアプリデータ領域（`eframe` の永続化ストレージ、Windows では通常 `%APPDATA%\clp2cfg` 配下）に、次の設定のみを保存します。

- クリップボード「自動読み取り」トグルの状態（`auto_read_clipboard`）

ウィンドウの位置・サイズや入力欄のスニペット本文は保存されません。また、診断用のログファイルも生成しません。保存された情報は端末の外へ送信されることはなく、アプリデータ領域（Microsoft Store 版がある場合はアンインストール時）を削除することで消去できます。

### Windows と Microsoft Store が収集する診断情報

本アプリは、クラッシュ情報を収集したり送信したりしません。ただし、あなたの Windows の診断データの設定によっては、本アプリが異常終了したときなどの情報を、Windows（Windows エラー報告）が Microsoft へ送ることがあります。今後 Microsoft Store から配布された版を提供する場合、開発者が Microsoft Partner Center で集計された形として参照できる場合があります。

これは Windows と Microsoft Store の機能であり、本アプリが独自に送信するものではありません。送信の有無は Windows の「設定」→「プライバシーとセキュリティ」→「診断とフィードバック」で決まります。開発者はこの情報を不具合の調査にのみ使用し、個人の識別には使いません。Microsoft によるデータの取り扱いには、[Microsoft のプライバシーに関する声明](https://privacy.microsoft.com/privacystatement)が適用されます。

### 子どものプライバシー

本アプリは個人情報を収集しないため、年齢を問わずお使いいただけます。

### このポリシーの変更

扱う情報やデータの取り扱いを変更するときは、変更を反映したこのポリシーを、本アプリの更新より前に公開します。変更の履歴は、このファイルの [Git の履歴](https://github.com/scottlz0310/clp2cfg/commits/main/docs/privacy-policy.md)で確認できます。

### 連絡先

質問や指摘は、[GitHub の Issue](https://github.com/scottlz0310/clp2cfg/issues) へお寄せください。脆弱性は、公開の Issue ではなく、[Security Advisories](https://github.com/scottlz0310/clp2cfg/security/advisories/new) から非公開で報告してください。

---

## English

`clp2cfg` (both the GUI application `clp2cfg` and the console application `clp2cfg-cli`, hereinafter "the app") is a tool that parses MCP (Model Context Protocol) server configuration snippets and merges them into local AI client configurations. It is developed by an individual (GitHub: [scottlz0310](https://github.com/scottlz0310)).

The app does not send your configuration snippets, environment variables, API keys, or local configuration files to the developer or any third party. This policy explains what information the app handles and how.

### Information the developer collects

**The developer does not collect your personal information.** The app does not communicate with any server operated by the developer or any external analytics platform. It has no accounts, no advertising, no built-in crash-report upload, and no usage analytics service. The app also does not connect on its own to any external URL or command target listed in your configuration snippets.

### Clipboard and input data handling

The app reads text from your clipboard in the following situations:

- In the GUI version, on startup and when the window gains focus while "Auto-read" is enabled
- In the GUI version, when you click the read button or paste into the app
- In the console version (`clp2cfg-cli`) or headless mode, when no input source argument is specified

Configuration snippets received from the clipboard, files, standard input, or command-line arguments may contain sensitive values such as API keys, tokens, environment variables, or HTTP headers. The app processes all of this data **exclusively in memory on your device** and never transmits it externally or persists it as input history.

In `--dry-run` output and CLI delegation errors (both the command notation and error messages such as `stderr` on failure), environment variable and HTTP header values passed by the app (`KEY=VALUE` / `Header: Value` pairs and standalone values of 4 or more characters) are masked as `***` (and placeholder input fields in the GUI are masked by default). Actual values are used when applying configurations to target files or external CLIs, in GUI preview panes, and when you toggle value visibility in the GUI. Short values of 1 to 3 characters appearing standalone without a `KEY=VALUE` or `Header: Value` pair are not masked to avoid corrupting ordinary error text.

### Access to local configuration files and external CLIs

The app reads and writes local configuration files of the AI clients you select or specify (Claude Code / Desktop, Cursor, VS Code Copilot, Project Local `.mcp.json`, Windsurf, Cline, Roo Code, and Zed). Before overwriting an existing file, the app creates a backup file (`.bak`) in the same directory so that the last change can be reverted (newly created files are removed upon undo).

When you apply changes to CLI delegation targets (Claude Code CLI, Codex CLI, GitHub Copilot CLI, or Antigravity CLI), the app invokes the locally installed official CLI executable (`claude`, `codex`, `copilot`, or `agy`) as a subprocess to pass the server configuration. Any network communication or data processing performed by those external CLIs is governed by their respective providers' privacy policies.

### Information stored on your device

The GUI version stores only the following setting in your OS user profile's local application data directory (`eframe` persistence storage, typically under `%APPDATA%\clp2cfg` on Windows):

- State of the clipboard auto-read toggle (`auto_read_clipboard`)

Window position, window size, and input snippet text are not persisted, and no diagnostic log files are created. Stored settings never leave your device and can be removed by deleting the application data directory (or by uninstalling the app if distributed via the Microsoft Store).

### Diagnostic information collected by Windows and the Microsoft Store

The app does not collect or send crash information. However, depending on your Windows diagnostic data settings, Windows (Windows Error Reporting) may send diagnostic information to Microsoft, such as when the app stops unexpectedly. If a version distributed through the Microsoft Store is provided, the developer may be able to view this information in Microsoft Partner Center in aggregated form.

This is a feature of Windows and the Microsoft Store, not something the app sends on its own. Whether it is sent depends on "Diagnostics & feedback" under "Privacy & security" in Windows Settings. The developer uses this information solely to investigate issues in the app and never to identify individuals. Microsoft's handling of the data is governed by the [Microsoft Privacy Statement](https://privacy.microsoft.com/privacystatement).

### Children's privacy

The app does not collect personal information and can be used regardless of age.

### Changes to this policy

Before changing what information is handled or how it is processed, the updated policy will be published ahead of the corresponding app update. The change history is available in the [Git history](https://github.com/scottlz0310/clp2cfg/commits/main/docs/privacy-policy.md) of this file.

### Contact

Please send questions or feedback through [GitHub Issues](https://github.com/scottlz0310/clp2cfg/issues). Please report vulnerabilities privately through [Security Advisories](https://github.com/scottlz0310/clp2cfg/security/advisories/new), not through a public issue.
