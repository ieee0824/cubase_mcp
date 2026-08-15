# Bridge Protocol v1

この文書は`cubase_mcp`とControl Bridge間の論理契約を定義します。JSONは基準表現です。別の物理Transportを使う場合も同じ情報と意味を保持してください。

## Envelope

Request:

```json
{
  "version": 1,
  "id": "req-123",
  "type": "request",
  "method": "transport.play",
  "params": {}
}
```

Response:

```json
{
  "version": 1,
  "id": "req-123",
  "type": "response",
  "result": {}
}
```

Error:

```json
{
  "version": 1,
  "id": "req-123",
  "type": "error",
  "error": {
    "code": "NOT_CONNECTED",
    "message": "Cubase is not connected"
  }
}
```

Event:

```json
{
  "version": 1,
  "type": "event",
  "event": "tempo.changed",
  "data": {
    "tempo": 132.0
  }
}
```

`id`は送信側セッション内で一意な非空文字列です。Response/ErrorはRequestと同じ`id`を返します。未知のversion、異なる`id`、不正なmessage typeは`PROTOCOL_ERROR`として接続を破棄します。

## Reference TCP transport

同梱実装は次のframingを使用します。

- localhost TCPのみ
- UTF-8 JSON
- 1メッセージ1行
- LF区切り（受信時はCRLFも許容）
- JSON内の生の改行は禁止
- 最大1 MiB/メッセージ
- 1 connectionにつき同期Requestは1件ずつ

EventはResponseの前に到着してもよく、Integration Daemonは対応する`id`のResponse/Errorまで読み続けます。1 Request中に1,024件を超えるEventが到着した場合は、不正または飢餓状態として接続を終了します。

TCPは論理仕様の必須要件ではありません。MIDI/SysEx、Unix Domain Socket、Named Pipe等のadapterも同じrequest correlationとerror semanticsを維持すれば使用できます。

## MIDI/SysEx transport

同梱のCubase MIDI Remote adapterは、Bridge ProtocolのUTF-8 JSONをMIDI transport envelopeで包み、7-bit safeなSysExへ格納します。フレームは次の構成です。

```text
F0 7D 43 4D 43 50 01 <payload> F7
```

- `7D`: educational/development用のnon-commercial SysEx ID
- `43 4D 43 50`: ASCII `CMCP`
- `01`: SysEx envelope version
- `payload`: MIDI transport envelope JSONの各byteを上位4 bit、下位4 bitの順に2 byteへ分解したもの
- JSONサイズ上限: 64 KiB

DaemonからCubaseへ送るenvelope:

```json
{
  "midi_transport_version": 1,
  "target_instance_id": "cubase-instance-id",
  "message": {
    "version": 1,
    "id": "req-123",
    "type": "request",
    "method": "transport.play",
    "params": {}
  }
}
```

CubaseからDaemonへ返すenvelope:

```json
{
  "midi_transport_version": 1,
  "source_instance_id": "cubase-instance-id",
  "message": {
    "version": 1,
    "id": "req-123",
    "type": "response",
    "result": {}
  }
}
```

Bridge Protocolのmessage自体はTCP transportと同一です。MIDI adapterは選択済みinstanceがない場合、読み取り専用の`system.discover`をbroadcastし、各MIDI Remoteが返す一時instance IDを確認してから通常Requestへ`target_instance_id`を設定します。connection eventが1件だけキャッシュされていても選択根拠にはせず、discoveryは最初の応答後も短い無通信時間では終了せず、設定された期間全体を使います。対象外のMIDI RemoteはRequestを実行しません。成功応答または接続通知から複数instanceが観測された場合は`BUSY`を返し、状態変更をbroadcastしません。inactive instanceからの`NOT_CONNECTED`等はinstance単位で記録し、他に成功候補がない場合だけ返します。

この変換により、UTF-8 JSON内の8 bit値はSysEx data byteに直接出現しません。Request/Responseの`id`照合、Eventの割り込み、タイムアウトはTCP transportと同じです。discovery期間と通常Requestの応答期間は独立しており、MIDIの設定timeoutは500 ms以上です。通常Requestの応答期限超過は`TIMEOUT`であり、そのinstance IDを候補から失効させて次回に再discoveryします。単発の期限超過だけでは最後に確認した接続hintを即座に切断扱いにはしません。受信callbackは64 frameのbounded queueへnon-blockingで書き込み、要求前のdrainも256 messageを上限とします。queue overflowを検出した場合は選択情報を破棄して`BUSY`を返し、新しいdiscoveryが完了するまで後続の状態変更を行いません。要求送信後のoverflowでは、その要求の実行結果を不明として扱います。デフォルトの仮想ポートは次の通りです。

```text
Integration Daemon -> Cubase: Cubase MCP To Cubase
Cubase -> Integration Daemon: Cubase MCP From Cubase
```

## MVP methods

### `system.get_status`

Params: `{}`

Result:

```json
{
  "connected": true,
  "project_open": true,
  "playing": false,
  "recording": false,
  "tempo": 120.0
}
```

`connected`は必須booleanです。他の値を取得できない場合は`null`または省略できます。`tempo`を返す場合は0より大きい数値です。

Integration Daemon自体がBridgeへ接続できない場合、MCPの`cubase.get_status`は次の正規化結果を返します。

```json
{
  "connected": false,
  "project_open": null,
  "playing": null,
  "recording": null,
  "tempo": null
}
```

### `transport.play`

Params: `{}`。再生状態を`true`にします。同じ要求の反復で追加作用が発生しないよう実装してください。

Result: `{}`

### `transport.stop`

Params: `{}`。再生と録音を停止します。

Result: `{}`

### `transport.record`

Params: `{}`。録音を開始します。単なるtoggleとして実装せず、既に録音中なら状態を維持してください。

Result: `{}`

### `transport.get`

Params: `{}`

Result:

```json
{
  "playing": true,
  "recording": false,
  "tempo": 128.0,
  "position": {
    "bars": 32,
    "beats": 1,
    "ticks": 0
  }
}
```

`playing`と`recording`は必須booleanです。`tempo`と`position`は取得できない場合に`null`または省略できます。position内の既知フィールドは整数です。

### `capabilities.get`

Params: `{}`

Result:

```json
{
  "transport": {
    "read": true,
    "write": true
  },
  "tracks": {
    "list": false,
    "select": false,
    "mute": false,
    "solo": false,
    "volume": false,
    "pan": false
  },
  "markers": false,
  "commands": false,
  "audio_analysis": false,
  "plugin_parameters": false
}
```

省略された既知CapabilityはIntegration Daemonが`false`へ正規化します。未実装機能を`true`にしてはいけません。

## Error codes

次のcodeを標準とします。

```text
NOT_CONNECTED
PROJECT_NOT_OPEN
NOT_SUPPORTED
INVALID_ARGUMENT
TRACK_NOT_FOUND
MARKER_NOT_FOUND
COMMAND_NOT_FOUND
COMMAND_NOT_ALLOWED
TIMEOUT
BUSY
PROTOCOL_ERROR
INTERNAL_ERROR
```

Bridgeのdomain errorは、MCPでは`isError: true`のTool結果へ変換されます。JSON-RPC envelopeやMCP lifecycle自体の不正だけがJSON-RPC errorになります。

## Logging and privacy

Integration Daemonは次の値だけを既定で`stderr`へ記録します。

```text
timestamp
request_id
method
duration_ms
result
error_code
bridge_connection_state
```

Tool引数、project内容、track名、audio dataは既定ログへ保存しません。
