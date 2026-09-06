# 既存Input / Outputの読み取り専用Probe

状態: `PENDING_RUNTIME`。これは[Issue #3](https://github.com/ieee0824/cubase_mcp/issues/3)の補助調査であり、本番の`cubase.get_tracks`ではありません。primary Track Probeの結果、44 checkpoint、入力ガード校正、正式受け入れ条件を変更・代替しません。

## 調査範囲と限界

`io-existing-v1`は既存のInputとOutputを、それぞれ8 slotの独立したMixer Bankへ投影します。選択した種類以外の8種類は明示除外し、visibilityには追従せず、利用可能な左右・中央zoneのincludeを要求します。要求したfilterは、実際に返ったchannel typeやzoneの証明ではありません。

- 公開操作はdiscovery、capability取得、snapshot、専用bankのReset / Next / Prevだけです。再生、録音、選択、Mute / Solo、bus作成・削除・rename、routing変更、ホスト値へのbinding / 書込みは提供しません。
- 名前とhost IDはメモリ内で`title-N` / `host-N`へ置換します。各tableは256件、元文字列は4096 UTF-16 code unitまでです。別名はactivation内だけ有効で、同名は同じtitle aliasになります。title aliasはchannel IDではなく、host aliasも永続IDではありません。
- `title_state: unobserved`はcallback未受信、`empty`は空文字列を実際に受信したことを表します。未通知を空slotとみなさず、空slot・同じpage・callback停止から列挙終端を推測しません。
- host ID getterの不存在、未採取、空文字、取得エラーを区別します。snapshotは常に`metadata_only: true`、`complete: false`です。8 slotの分割転送完了は、全busの取得完了ではありません。
- 旧callback参照はactivation / generationで拒否します。ただしホストが新しいhandlerへ古い通知を配送した場合、APIから通知元generationを識別できません。
- static確認ではCubase 13.0.30 / API 1.1には明示的な中央zone includeとbank channelのID getterがなく、15.0.30 / API 1.3にはあります。これは実機での挙動確認とは別です。
- 生の名前を送信しないため、このprofileだけでUI上の各busとの名前一致や同一性は検証できません。UI inventoryとの比較ではこの制約を残し、aliasから名前・type・routingを復元しません。

Inputが存在しなければ「検証対象なし」と記録します。API非対応とは結論付けず、Control RoomのExternal Inputを代用しません。今回のためにbusやroutingを作り直しません。

## 配備前の準備

本手順を読むこと、offline testを実行することではCubaseは起動・変更されません。実機操作へ進む際は、その時点の準備確認を得て、通常projectと保存状態を保護します。

1. 全Cubase instanceを正常終了し、対象製品の既存`MIDI Remote/Driver Scripts/Local`を一つだけ特定します。
2. repositoryの`cubase/midi_remote/CubaseMCPIOProbe/CubaseMCPIOProbe/`から、次の3 fileを同じ相対構造で新規配備します。既存pathがあれば上書きせず比較して停止します。`--install-track-probe`は別のprimary script用なので使いません。

   ```text
   Local/
     CubaseMCPIOProbe/
       CubaseMCPIOProbe/
         CubaseMCPIOProbe_CubaseMCPIOProbe.js
         io-profile.js
         wire.js
   ```

3. repository / 配備先の3 fileのSHA-256を個別に照合します。相対`require`で読み込むため1 fileだけの配備は不可です。対象Cubaseの完全build、OS、配備先、commitとdigestの確認記録はローカルに保持します。
4. collectorを単独でビルドします。過去の校正に固定したrelease guardを再ビルドする必要はありません。

   ```sh
   cargo build --locked --bin cubase_track_probe_collector
   ```

## 最小の実機観測

他のProbe / daemonを停止し、既存busを確認できる安全なprojectを使用します。collectorはCubase起動前に開始し、stdoutを新しいローカルJSONLへ保存します。run IDやpathへ秘密情報を含めず、既存記録を上書きしません。

```text
target/debug/cubase_track_probe_collector --run-id io-observation-1 --profile io-existing-v1
```

macOS/Linuxの専用仮想portは`Cubase MCP IO Probe To Cubase`と`Cubase MCP IO Probe From Cubase`です。Windowsでは既存の対応loopback portを用意し、`--midi-input "Cubase MCP IO Probe From Cubase" --midi-output "Cubase MCP IO Probe To Cubase"`を追加します。明示指定のport名はprofileによって書き換えません。

以下は順序の説明であり、一括投入するバッチではありません。各待機条件を実際の記録で確認します。

1. `collector_started`の`probe_profile: io-existing-v1`を確認し、Cubaseのmappingがactivateする**前**にcheckpointとaction markerを作ります。

   ```json
   {"method":"collector.checkpoint.begin","params":{"checkpoint_id":"IO_INITIAL","window_ms":5000}}
   {"method":"collector.action","params":{"checkpoint_id":"IO_INITIAL"}}
   ```

2. 準備済みのCubase / projectを開き、単一sourceの`probe.loaded` → `probe.mapping_active` → `probe.capabilities` → `probe.ready(true)`を待ちます。間にfeedbackが挟まる場合があります。readyは要求受付の準備完了です。`initial_snapshots_complete: true`も、on-request方式で未処理の自動snapshotがない意味であり、初期titleの通知完了を保証しません。
3. discoveryを送り、応答だけでなくdiscovery window終了とexactly-one選択を確認します。その後capabilityを取得します。

   ```json
   {"target_instance_id":null,"method":"probe.discover","params":{}}
   {"target_instance_id":"@selected","method":"probe.capabilities.get","params":{}}
   ```

4. markerから5000 ms以上観測した後、次のsnapshotを一つずつ要求します。各成功responseと4 chunk / 8 slotの完了を待ってから次へ進みます。

   ```json
   {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"IO_INPUT_ALL"}}
   {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"IO_OUTPUT_ALL"}}
   ```

5. 最後の受信から追加1000 ms message-freeであることを確認してcheckpointを閉じます。

   ```json
   {"method":"collector.checkpoint.end","params":{"checkpoint_id":"IO_INITIAL"}}
   ```

6. bank移動を調べるときは新しいcheckpoint / markerを作り、`probe.bank.reset`、`probe.bank.next`、`probe.bank.prev`を同じ`config_id`引数で一操作ずつ実施します。それぞれresponseと後続4 chunkを待ち、5000 ms観測後に明示snapshot、追加1000 ms quietの順で閉じます。有限の事前に決めたpage数だけ観測し、反復pageを終端の証拠にしません。
7. collectorへEOFを送り、graceful drain完了と`exit_ok: true` / `integrity_ok: true`のsummaryを確認します。採取終了前にmappingをdeactivateしません。timeout、欠落、overflow、別source、再activation、未完了要求は成功へ読み替えず保存して停止します。

生の名前はscriptから出力しませんが、raw collector記録にはsource / session IDやoperatorが指定した値が含まれます。rawやUI画像をrepositoryへcommitしません。UIのbus数、種類、zone、実際に何が観測されたかは独立のローカル記録へ残し、公開時はsanitizedな結論だけを使用します。

## 構造検証

manifestの例です。`<...>`を実際に確認したdigestへ置き換えます。hostは`13.0.30` / `1.1`または`15.0.30` / `1.3`の組です。完全buildの照合記録は別途保持します。

```json
{
  "version": 1,
  "profile": "io-existing-v1",
  "expected_collector_sha256": "<実行したcollectorのSHA-256>",
  "host": {"cubase_version": "15.0.30", "api_version": "1.3"},
  "expected_probe_files": {
    "CubaseMCPIOProbe_CubaseMCPIOProbe.js": "<確認したSHA-256>",
    "io-profile.js": "<確認したSHA-256>",
    "wire.js": "<確認したSHA-256>"
  },
  "ui_review": "pending"
}
```

```sh
node scripts/audit-io-probe.js <RAW_JSONL> <MANIFEST_JSON>
```

auditorはrawの上限16 MiB / 20,000 records、source連番、lifecycle、要求/送信/応答の対応、chunk、checkpoint、summary整合性と両configのsnapshotを検査します。出力は固定のmetadataと別名だけで、失敗時も入力内容やfilesystem errorを転記しません。

合格は`status: structurally_valid`、`runtime_acceptance: pending_ui_review`、`complete: false`です。manifestのhostとProbe digestは宣言であり、実際のloadやUI inventoryをauditorが認証するわけではありません。構造検証の成功だけで実機対応・完全列挙・Issue #3完了とは扱いません。

## Offline tests

```sh
node tests/io_profile.test.js
node tests/io_probe_script.test.js
node tests/io_probe_audit.test.js
cargo test --locked --bin cubase_track_probe_collector
```

Rust suiteにもNode.jsが必要です。`actual_io_driver_frames_pass_collector_and_io_auditor_offline`は、旧/新APIの模擬hostで実JS driverが生成したbyte列を実collectorの受信・要求処理・checkpoint・summaryに通し、その記録をI/O auditorで検証します。テスト専用のpipeを使い、MIDI portを作らず、raw記録の時刻や成功値を書き換えません。

これらは模擬hostとソフトウェア接続の検証です。実際のCubase callback、MIDI port、配備、UIとの一致の証拠にはなりません。以後の優先はcore Track列挙範囲とID寿命の実測、方式 / DTOの決定、本番`cubase.get_tracks`の実装です。補助Probeの一般化やbus編集機能は追加しません。
