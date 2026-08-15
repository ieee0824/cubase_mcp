# Cubase MCP

CubaseをMCPクライアントから操作するIntegration Daemonです。MVPの6 Tool、stdio MCP transport、Bridge Protocol v1、Cubase MIDI Remoteスクリプト、MIDI/SysEx Bridgeを同梱しています。macOSでは外部のMIDIループバックドライバを使わず、実Cubaseの再生・停止・状態取得ができます。

Cubase 13.0.30 / macOSで、`get_status`、`get_transport`、再生、停止の実機往復を確認済みです。`record`は実際の録音を開始するため、実行時は録音先とRecord Enableの状態を確認してください。

## 実装済みTool

| MCP Tool | Bridge method | 状態 |
| --- | --- | --- |
| `cubase.get_status` | `system.get_status` | MVP |
| `cubase.play` | `transport.play` | MVP |
| `cubase.stop` | `transport.stop` | MVP |
| `cubase.record` | `transport.record` | MVP |
| `cubase.get_transport` | `transport.get` | MVP |
| `cubase.get_capabilities` | `capabilities.get` | MVP |

MCP側は`2025-06-18`、`2025-03-26`、`2024-11-05`の初期化を受け付けます。Tool結果は互換性のため、JSON文字列の`content`とオブジェクトの`structuredContent`の両方で返します。

## 構成

```text
MCP Client
    | newline-delimited JSON-RPC over stdio
    v
cubase_mcp
    | Bridge Protocol v1 in 7-bit-safe SysEx
    v
CubaseMCP MIDI Remote script
    | Steinberg MIDI Remote API
    v
Cubase
```

公式のCubase MIDI Remote APIがTransportとTempoを担当し、外部デーモンとのIPCにMIDI SysExを使います。参考: [Steinberg MIDI Remote API](https://steinbergmedia.github.io/midiremote_api_doc/)。TCP Bridgeとモックも開発・別adapter用に残しています。

## Cubaseで使う

1. リリースバイナリをビルドし、MIDI Remoteスクリプトをインストールします。

   ```bash
   cargo build --release
   ./target/release/cubase_mcp --install-midi-remote
   ```

   既存の内容が異なる場合は、同じ場所に`.bak`を作ってから更新します。

2. MCPクライアントに次のように登録します。

   ```json
   {
     "mcpServers": {
       "cubase": {
         "command": "/absolute/path/to/cubase_mcp/target/release/cubase_mcp",
         "args": ["--bridge", "midi", "--timeout-ms", "3000"]
       }
     }
   }
   ```

3. 初回はIntegration Daemonが起動している状態でCubaseを起動します。Cubaseを開いたままスクリプトをインストールまたは更新した場合は、MIDI RemoteのScripting Toolsから`Reload Scripts`を実行するか、Cubaseを一度再起動してください。

CubaseのMIDI Remote下ゾーンに`CubaseMCP - CubaseMCP`が自動表示されれば検出成功です。macOS/Linuxではデーモンが次の2つの仮想MIDIポートを作成します。

```text
Cubase MCP To Cubase
Cubase MCP From Cubase
```

利用可能なポートは`cubase_mcp --list-midi-ports`で確認できます。Windowsまたは既存のポートを使う場合は、`--midi-input <NAME> --midi-output <NAME>`をペアで指定してください。

## ビルドとテスト

Rust 2024 editionに対応するtoolchainが必要です。

```bash
cargo build --release
cargo test --all-targets
node tests/midi_remote_script.test.js
```

localhost TCPを使うテストは、sandbox環境ではローカルsocket作成権限が必要になる場合があります。

## モックでMCPを確認する

Bridgeをプロセス内で模擬する最短の起動方法です。

```bash
cargo run --bin cubase_mcp -- --bridge mock
```

stdioへは1行1 JSON-RPCメッセージを入力します。サンプルセッションも利用できます。

```bash
cargo run --quiet --bin cubase_mcp -- --bridge mock < examples/mcp-session.ndjson
```

`stdout`にはMCPメッセージだけが出力され、Bridgeリクエストログは`stderr`へJSON Lines形式で出力されます。

## TCP BridgeをE2E確認する

ターミナル1でBridgeシミュレーターを起動します。

```bash
cargo run --bin cubase_bridge_mock
```

ターミナル2でIntegration Daemonを起動します。

```bash
cargo run --bin cubase_mcp -- --bridge tcp --bridge-address 127.0.0.1:8765
```

一般的なTCP Bridge用MCPクライアント設定例:

```json
{
  "mcpServers": {
    "cubase": {
      "command": "/absolute/path/to/cubase_mcp",
      "args": [
        "--bridge",
        "tcp",
        "--bridge-address",
        "127.0.0.1:8765"
      ]
    }
  }
}
```

クライアント固有の設定キーは、使用するMCPクライアントのドキュメントに合わせてください。

## 設定

| CLI | 環境変数 | 既定値 |
| --- | --- | --- |
| `--bridge` | `CUBASE_MCP_BRIDGE_MODE` | `tcp` |
| `--bridge-address` | `CUBASE_MCP_BRIDGE_ADDRESS` | `127.0.0.1:8765` |
| `--midi-input` | なし | 仮想ポート |
| `--midi-output` | なし | 仮想ポート |
| `--timeout-ms` | `CUBASE_MCP_TIMEOUT_MS` | `2000` |

`--bridge`は`midi`、`tcp`、`mock`です。実Cubaseには`midi`を使います。TCPの接続先とシミュレーターのlisten先はloopback interfaceに制限されます。タイムアウトは1〜600,000 msの範囲で設定できます。

## 未接続時の動作

- `cubase.get_status`はBridgeへ接続できない場合もTool Call自体を成功させ、`connected: false`と未知フィールドの`null`を返します。
- その他のToolは`isError: true`と標準エラー`NOT_CONNECTED`を返します。
- Bridgeが不足フィールドを返した場合、推測で補完しません。Capabilityの省略値だけは安全側の`false`へ正規化します。
- Bridge由来の操作エラーはJSON-RPCエラーではなくTool Execution Errorとして返るため、MCPクライアント側のモデルが内容を確認できます。

## Bridge実装

論理プロトコル、MIDI/SysExとTCPのframing、methodごとの契約は[Bridge Protocol v1](docs/bridge-protocol-v1.md)を参照してください。機械可読なenvelope定義は[JSON Schema](schemas/bridge-protocol-v1.schema.json)にあります。

実機Control Bridgeは次の条件を満たす必要があります。

- Cubaseの公式またはサポート対象インターフェースだけを使用する
- 利用できない値を推測せず、`null`または標準エラーを返す
- 状態変更を受理しただけで成功とせず、Bridge側で実行可能だったことを確認する
- MIDI Remoteを使う場合、外部IPCをAudio Processing Threadへ持ち込まない
- TCP endpointをloopback以外へ既定公開しない

## 現在のPhase

- MVP: 実装済み（Integration Daemon、protocol、MIDI Remote Control Bridge、TCP、mock）
- Phase 2 Track API: 未実装
- Phase 3 Marker / Command API: 未実装
- Phase 4 VST3 / Audio Analysis: 未実装
- 実Cubase向けTransport Control Bridge: 実装・実機確認済み

MCPのframingとlifecycleについては[Model Context Protocol 2025-06-18 specification](https://modelcontextprotocol.io/specification/2025-06-18/)に従っています。
