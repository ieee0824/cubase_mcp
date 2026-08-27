# Track Host API実機スパイク結果

この文書は[Issue #3](https://github.com/ieee0824/cubase_mcp/issues/3)の調査記録です。[Track API実機検証fixture](track-api-fixture.md) revision 1を使い、CubaseのMixer BankとDirectAccessでTrack列挙に必要な情報を安全かつ完全に取得できるかを比較します。ここでの観測結果を[Issue #4](https://github.com/ieee0824/cubase_mcp/issues/4)の方式・対応version決定と[Issue #5](https://github.com/ieee0824/cubase_mcp/issues/5)のDTO / ID / pagination契約へ入力します。

文書状態: `PENDING_RUNTIME`

このrevisionではread-onlyの静的preflightと、実機観測用Probe / collectorのoffline実装だけが完了しています。Cubaseを使うruntime runはまだ実施していません。したがって、観測欄の`PENDING`は成功、対応、非対応のいずれも意味しません。runtime表、比較、最終推奨、完了checklistが埋まるまでIssue #3をcloseしません。

## 結果class

この文書では次のclassだけを使用します。

| class | 意味 |
| --- | --- |
| `STATIC_CONFIRMED` | installed app bundleの型定義、OS情報、または公式version資料からread-onlyで確認した。runtime挙動の証拠ではない |
| `PENDING` | runtime未実施または証拠未採取。成功として数えない |
| `NOT_AVAILABLE` | 指定した正確なversionや安全な検証条件を現在の環境で利用できない |
| `OBSERVED` | 完全なcallback windowとsequence検査を通過したruntime観測。製品要件を満たす意味ではない |
| `UNSUPPORTED` | 対象の正確なruntime hostでfeature不存在を確認した。静的な型定義だけでは付けない |
| `INCONCLUSIVE_CALLBACK_TIMEOUT` | 観測期限内にcallbackが収束しない、ready snapshotを取得できない、または遅延callbackを検出した |
| `INCONCLUSIVE_RECONNECT_TIMEOUT` | reload / restart後に期限内でreadyにならなかった |
| `INCONCLUSIVE_SEQ_GAP` | 同一probe sessionの連番に欠落があり、完全性を証明できない |
| `INCONCLUSIVE_SEQ_ORDER` | 同一probe sessionでduplicateまたは逆順のrecordを検出した |
| `FAIL` | UI ground truthとの矛盾や、明示した安全・整合性条件への違反を完全な観測から確認した |

`PENDING`、`NOT_AVAILABLE`、`UNSUPPORTED`、`INCONCLUSIVE_*`を`OBSERVED`や成功へ読み替えません。

## 静的preflight

### 環境inventory

2026-08-26のinstalled app bundle確認と、2026-08-27のread-only OS確認で得た値です。runtime run開始時にはAbout画面等から再取得し、別のrun recordへ記録します。

| 項目 | 静的確認値 | 状態 | runtime状態 |
| --- | --- | --- | --- |
| OS | macOS 26.5.1 / build 25F80 / arm64 | `STATIC_CONFIRMED` | `PENDING` |
| Cubase 13 | 13.0.30.226 | `STATIC_CONFIRMED` | `PENDING` |
| Cubase 13 MIDI Remote API | v1.1。公式release indexではv1.1は13.0.0から | `STATIC_CONFIRMED` | `PENDING` |
| Cubase 13.0.50 | installされていない | `NOT_AVAILABLE` | `NOT_AVAILABLE` |
| Cubase 13.0.50 MIDI Remote API | v1.2の導入境界であることを公式release indexで確認 | `STATIC_CONFIRMED` | `NOT_AVAILABLE` |
| Cubase 15 | 15.0.30.287 | `STATIC_CONFIRMED` | `PENDING` |
| Cubase 15 MIDI Remote API | v1.3。公式release indexではv1.3は15.0.20から | `STATIC_CONFIRMED` | `PENDING` |

Cubase edition、runtimeが実際にloadしたAPI、run日時、About画面の完全な表示、repository commit、probe source / deployed SHA-256は各runで`PENDING`から置換します。13.0.50は未installなので、Cubase 15のrunを「13.0.50確認済み」として扱いません。

### Cubase 13.0.30 / API v1.1の静的surface

| API surface | 静的結果 | runtime状態 | 含意ではないもの |
| --- | --- | --- | --- |
| `makeMixerBankZone` / `makeMixerBankChannel` | 存在 | `PENDING` | bankの完全列挙や終端検出を保証しない |
| channel type include / exclude、window-zone filter、`setFollowVisibility` | 存在 | `PENDING` | 実際のscopeや順序を保証しない |
| Prev / Next / Shift / Reset action | 存在 | `PENDING` | offset、総数、終端を返さない |
| title callback | 存在 | `PENDING` | 初期値、Unicode、空slot時の挙動を保証しない |
| selected / mute / solo / volume / pan value | 存在 | `PENDING` | callback順、初期値、slot再割当の整合性を保証しない |
| Mixer Bank channelの`getUniqueIDString` | 型定義に存在しない | `PENDING` | runtime IDが別経路にも存在しないとは断定しない |
| `makeDirectAccess` | 型定義に存在しない | `PENDING` | runtimeでの最終classは実機確認まで付けない |
| Track type getter | 型定義に存在しない | `PENDING` | name、位置、routingからtypeを推測してよい意味ではない |
| bank総数、現在offset、終端の直接getter | 型定義に存在しない | `PENDING` | callbackから安全な終端を導出できるとは限らない |

共有Documentsにある汎用`.api/v1`型定義はCubase 15同梱型定義と一致していたため、Cubase 13の能力証拠には使用しません。repositoryへabsolute pathは記録しません。

### Cubase 15.0.30 / API v1.3の静的surface

| API surface | 静的結果 | runtime状態 | 含意ではないもの |
| --- | --- | --- | --- |
| Mixer Bank API | 存在 | `PENDING` | v1.1と同じcallback・終端挙動を保証しない |
| MixerBankChannel / SelectedTrackChannelのunique ID getter | 存在 | `PENDING` | rename、project切替、reload、restartをまたぐ寿命を保証しない |
| DirectAccess activate / update / deactivate | 存在 | `PENDING` | lifecycle中のready条件を保証しない |
| base object、child count、child object IDによるtree走査 | 存在 | `PENDING` | Project Track全体を重複・欠落なく返すことを保証しない |
| object unique name / unique ID / title | 存在 | `PENDING` | 各値の必須性、長さ、寿命を保証しない |
| mixer visibility / index / zone | 存在 | `PENDING` | Project順やhidden Trackのinclusionを保証しない |
| object change / removal callback | 存在 | `PENDING` | add / rename / deleteの順序や完全性を保証しない |
| `getObjectTypeName` | 存在 | `PENDING` | API v1.2でもtypeを取得できる証拠にはならない |

### API v1.2とv1.3を混同しないための規則

公式資料ではDirectAccess coreはAPI v1.2で導入され、`getObjectTypeName`はAPI v1.3で追加されています。したがって、Cubase 15.0.30 / API v1.3でtypeが取得できても、その結果をCubase 13.0.50 / API v1.2へ一般化しません。

- v1.2の能力として記載できるのは、v1.2公式資料で明示されたmethodまたは正確なv1.2 hostで実測した結果だけです。
- 現行の総合API Referenceにmethodが掲載されていることだけを、過去versionで利用できる証拠にしません。
- API v1.3固有methodは必ずfeature detectionし、API v1.2以下では呼びません。
- type getterを利用できないhostでは、Track名、object階層、index、routingからtypeを推測しません。
- 13.0.50のruntime欄はinstallして正確なbuildで実施するまで`NOT_AVAILABLE`のままです。

## 安全条件とdata handling

runtime runはfixtureの[安全条件](track-api-fixture.md#安全条件)と[Run終了時のcleanup](track-api-fixture.md#run終了時のcleanup)をすべて満たす場合だけ開始します。

1. 編集中の通常projectを保存して閉じ、Cubaseは対象versionを1 instanceだけ起動する。
2. versionごとに専用の空projectからfixtureを作り、新しいCubaseで保存した`.cpr`を古いCubaseへ使い回さない。
3. playback / record、Record Enable、Monitor、Automation Write、audio / MIDI / video import、event / part作成を行わない。
4. mutationはM1 copy、Input / Output / VCAはO1 copyだけで実施し、C1 baselineを上書きしない。
5. O1のbus inventoryはlocalだけに保持し、終了時に完全一致しなければ`RESTORE_FAILED`として通常projectを開かない。
6. repository commit、probe source、deployed probeのdigestを照合し、不一致ならrunを開始しない。
7. raw log、`.cpr`、autosave、audio、MIDI、SysEx、absolute path、raw device名、credential、raw host IDをcommitしない。
8. committed resultに記録するTrack名はfixtureの合成名だけとし、host IDはrun-local alias、byte length、必要な場合だけSHA-256 digestで表現する。

## Probeとcollector

### artifact確定状況

Probe / collectorのrepository内artifactとoffline commandは次で固定します。配備先、digest、commitはrun固有値なので実機runまで`PENDING_RUNTIME`です。未確定値を推測で補ってrunを開始しません。

| 項目 | 値 | 状態 |
| --- | --- | --- |
| probe source | `cubase/midi_remote/CubaseMCPTrackProbe/CubaseMCPTrackProbe/CubaseMCPTrackProbe_CubaseMCPTrackProbe.js` | 確定 |
| probe offline test | `node tests/track_probe_script.test.js` | 確定 |
| production MIDI Remote test | `node tests/midi_remote_script.test.js` | 確定 |
| collector source | `src/bin/cubase_track_probe_collector.rs` | 確定 |
| collector test | `cargo test --all-targets --locked` | 確定 |
| collector build | `cargo build --release --locked` | 確定 |
| collector binary | `target/release/cubase_track_probe_collector`（Windowsは`.exe`） | 確定 |
| collector output format | flushed JSON Lines、`record_format_version = 1` | 確定 |
| deployed probe location | repository外のlocal Cubase MIDI Remote script directory | `PENDING_RUNTIME` |
| source SHA-256 | run開始時にrepository sourceから取得 | `PENDING_RUNTIME` |
| deployed SHA-256 | run開始時に配備済みsourceから取得 | `PENDING_RUNTIME` |
| repository commit | run開始時の40桁commit SHA | `PENDING_RUNTIME` |

offline validationはrepository rootで次をすべて実行します。

```text
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
node tests/midi_remote_script.test.js
node tests/track_probe_script.test.js
```

macOS / Linuxでは、repository外の専用directoryを出力先としてcollectorを先に起動します。

```text
target/release/cubase_track_probe_collector \
  --run-id <RUN_ID> \
  --discovery-window-ms 1000 \
  --drain-timeout-ms 5000 \
  > <REPOSITORY_OUTSIDE>/<RUN_ID>.jsonl
```

Windows、または既存portを明示して使う場合は、localで確認した完全なport名を追加します。raw port名はlocal run recordにだけ残します。

```text
target/release/cubase_track_probe_collector \
  --run-id <RUN_ID> \
  --discovery-window-ms 1000 \
  --drain-timeout-ms 5000 \
  --midi-input "<FROM_CUBASE_PORT>" \
  --midi-output "<TO_CUBASE_PORT>" \
  > <REPOSITORY_OUTSIDE>/<RUN_ID>.jsonl
```

`--run-id`にはcredential、project名、個人名、顧客名、absolute pathを含めません。collectorは観測fileを自動作成せずstdoutへ出力するため、repository内へredirectしません。primary runでは上記の1000 ms discoveryと5000 ms graceful drainを固定し、変更したrunは値と理由を記録してprimary comparisonから分離します。

### JSONL v1 record契約

collectorが出力する全recordは最低限次の共通fieldを持ちます。

```text
record_format_version       # 1
record_type
run_id
timestamp_unix_ms
monotonic_timestamp_ms      # collector開始時の単一monotonic clock基準
```

`collector_started` / `collector_summary`の`session_id`はcollector process内のrequest ID namespaceを表します。Probe由来recordでは`source_instance_id`を`probe session ID`として扱い、同じIDの`source_seq`を検査します。別の`probe_session_id`を合成しません。payloadには取得できた範囲で次を保持します。

```text
source_instance_id
source_seq
checkpoint_id
message.type
message.event / message.id
message.data
bank config / slot index または DirectAccess tree位置
host_id_raw                # local raw JSONLのみ
host_id_byte_length
returned title / type / selected / mute / solo / visibility
capability result
```

APIが値を提供しないfieldは`null`またはfield不存在のままにし、直前値やUI ground truthで補いません。`host_id_alias`と必要な場合のSHA-256はraw JSONL検証後に作るcommitted summaryだけの値です。Probeとcollectorはcallbackの到着順を維持し、同一timestampのrecordを並べ替えません。

### command、checkpoint、integrity契約

stdinは1行1 JSONです。`collector.checkpoint.begin` / `collector.checkpoint.end`はcollector内だけで処理し、Cubaseへ送信しません。

```json
{"method":"collector.checkpoint.begin","params":{"checkpoint_id":"B0-reset","window_ms":5000}}
{"method":"collector.checkpoint.end","params":{"checkpoint_id":"B0-reset"}}
```

- checkpoint IDは同一run内で一意とし、同時に複数を開かない。
- `begin`をUI操作またはProbe commandより前に記録し、`end`は5000 msの完全なwindow、必要なfinal snapshot、関連response / chunkの完了後に送る。
- 5000 ms未満の`end`、対応しないID、開いたcheckpointを残したEOFはfatalとする。
- 初期load、reload、restartを含むProbe観測も専用checkpointで囲む。期待していないProbe recordがcheckpoint外または終了後に届いた場合はorphanとして記録し、そのsequenceを成功にしない。
- Probe commandはactive checkpoint内だけで許可し、MIDI送信前の`probe_command`（`phase: started`）と`probe_command_send_result`を別々に記録する。request、response / error、必要なsnapshot follow-upは同じcheckpoint IDへ結び付け、送信失敗を送信済みとして扱わない。
- Probe command送信とcheckpoint終了の直前にMIDI ingress barrierを通し、その時点までに開始したcallback、分割受信中のSysEx、受信済みqueueをProtocol trackerへ反映する。未完SysEx、未処理、queue overflow、barrier timeout、integrity failureがあればcommandを送信せずfail closedにする。
- 未完request、discovery window、snapshot follow-up、chunkがある間はcheckpointを終了しない。callback受信時刻が終了marker以前なら、collector側の処理がmarker後になっても同じcheckpointへ分類する。

discoveryだけは`target_instance_id: null`でbroadcastし、既定1000 msのbounded discovery windowを最後まで待ちます。windowはrequest登録時ではなく、`probe_command_send_result`が`sent: true`となった送信時点から開始し、未送信requestをtimeout済みとして扱いません。

```json
{"target_instance_id":null,"method":"probe.discover","params":{}}
```

0 instance、複数instance、初期化未完のinstanceが応答したrunではtargetを選ばずfatalにします。`probe.ready(true)`済みのexactly 1 instanceをwindow全体で確認した後のcommandだけが、その`source_instance_id`を明示して直列に送信されます。page deactivate / reactivate、reload、restartでmapping lifecycleが変わった後は選択を破棄し、再discoverします。

```json
{"target_instance_id":"<SOURCE_INSTANCE_ID>","method":"probe.capabilities.get","params":{}}
{"target_instance_id":"<SOURCE_INSTANCE_ID>","method":"probe.bank.reset","params":{"config_id":"MB_CORE_ALL"}}
{"target_instance_id":"<SOURCE_INSTANCE_ID>","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_ALL"}}
{"target_instance_id":"<SOURCE_INSTANCE_ID>","method":"probe.direct_access.snapshot","params":{}}
```

targeted requestは、同じrequest IDに対する同じ`source_instance_id`からのresponseまたはerrorがexactly once必要です。unmatched、duplicate、wrong-source、未応答のrequest、および成功response後に必要なsnapshot follow-upがない状態はfatalです。

`probe.bank.chunk` / `probe.direct_access.chunk`は`source_instance_id + snapshot_id`単位で、chunk count、0始まりの連続index、total item数、complete flagを検査します。fragment化されたhost IDもreference、fragment count / index、UTF-8 byte lengthを検査します。欠落、重複、逆順、不整合、未完snapshot / fragment / requestを残したEOFはrun fatalです。collectorはEOF後も設定されたdrain期限まで受信を続け、quiescentにならないrunを成功にしません。

`probe.overflow`はsource queue、outbound frame、host-ID fragment上限、snapshot queue、deactivation時の未送信破棄を含め、理由を問わずrun fatalです。raw logの`collector_summary`が`integrity_ok: true`かつ`exit_ok: true`でないrunを`OBSERVED`へ使いません。

### runtime run手順template

この手順はruntime artifactを作成しますが、本revisionではまだ実施していません。

1. 通常projectを保存して閉じ、対象Cubaseも終了した状態で、他のCubase instanceがないことを確認する。
2. offline validationをすべて実行し、repository commitとprobe source SHA-256をlocal run recordへ記録する。
3. 対象version用のprobe sourceを配備し、source / deployed SHA-256が完全一致することを確認する。配備先はlocal recordにだけ残す。
4. repository外を出力先にし、`--run-id <RUN_ID>`付きでcollectorを先に起動する。最初の`collector_started` recordを確認し、固有IDの初期化checkpointを開始する。
5. 初期化checkpoint内で対象Cubaseを1 instanceだけ起動してprobeを新規loadする。R1ではcollectorを動かしたまま、reload用checkpointを開始してから明示的にReload Scriptsする。
6. 新しい`source_instance_id`から`source_seq = 1`の`probe.loaded`を受信したことを確認する。これを新しいprobe sessionの開始markerとする。続く同じsourceの`probe.mapping_active`でpage activation境界を確認する。
7. capability recordと全初期snapshotのcomplete chunkを受信した後、同じsourceから`probe.ready`の`ready: true`かつ`initial_snapshots_complete: true`を確認する。順序や完全性が違う場合は開始しない。同じscript sessionのpage再activationは新しい`probe.loaded`ではなく`probe.mapping_active`から始まり、再度readyになるまで操作しない。
8. `probe.discover`を送信し、bounded discovery window全体でexactly 1 responderだけであることを確認する。その`source_instance_id`だけを後続requestのtargetにし、初期化checkpointを完全なwindow経過後に終了する。
9. fixtureの再現性checklistを完了し、E0、E1、C1、M1、必要な場合だけO1を順に観測する。
10. 各bank / mutation操作を固有checkpointで囲み、完全なcallback window、final snapshot、request / chunk / sequence integrityを確認してから次の操作へ進む。
11. R1 reloadとR2 restartは独立phaseとして実施し、各phaseの前後でsession境界とsnapshotを記録する。
12. active checkpointがなく、request / follow-up / snapshotがquiescentなことを確認してstdinをEOFにする。graceful drain後の`collector_summary`を確認する。
13. gap、duplicate、逆順、overflow、orphan、未完request / chunk / fragment、truncation、fatal diagnosticが1件でもあれば、そのrunを完全性の証拠にしない。
14. この文書にはredacted集計と最小限の合成名例だけを転記する。raw JSONLを移動・copy・stageしない。
15. cleanup後、`git status --short`で意図したsource / document以外のartifactがないことを確認する。

## Callback観測windowとsequence判定

### Callback観測window

fixtureに従い、bank操作とmutation操作は操作直前から記録し、操作開始から必ず`5000 ms`観測します。最初のquiet periodで早期終了しません。

次の条件をすべて満たすcheckpointだけを`OBSERVED`にできます。

- window最後の`1000 ms`にprobe callbackがない。
- UI ground truthを確認でき、access方式がsnapshot / ready状態を提供する場合はそれも取得できた。
- 直前checkpointのwindow終了後から次のUI操作前までにorphan callbackがない。
- 同一probe sessionのsequence検査にgap、duplicate、逆順がない。

期限までcallbackが続く、必要なsnapshotがない、またはwindow後にcallbackが届いた場合は`INCONCLUSIVE_CALLBACK_TIMEOUT`としてsequenceを停止します。遅延recordは`orphan_after_<checkpoint>`と記録し、次のUI操作を行いません。

collectorはcheckpoint終了時に上記`1000 ms` quiet periodを検証します。終了marker以前に受信され、終了処理後にqueueから取り出されたcallbackもreceive timestampで判定し、quiet period内ならintegrity failureとします。

### Sequence integrity

`source_seq`は同一`source_instance_id`、すなわち同一probe session内で厳密に単調増加し、隣接recordは1ずつ増える契約とします。新sessionの最初のrecordは必ず`source_seq = 1`の`probe.loaded`です。collector起動前に既に送信されたsessionへ途中参加したrunは、最初に見えた値を基準にせず無効とします。

| 条件 | 判定 | 対応 |
| --- | --- | --- |
| 新`source_instance_id`の最初が`source_seq == 1`かつ`probe.loaded` | session start | 初期化観測を継続できる |
| 新sessionの最初が1でない、または`probe.loaded`でない | `INCONCLUSIVE_SEQ_GAP` | pre-attach lossとして停止する |
| `next == previous + 1` | continuous | 観測を継続できる |
| `next > previous + 1` | `INCONCLUSIVE_SEQ_GAP` | 欠落範囲をlocal記録し、UI操作を止め、fresh runで再実施する |
| `next == previous` | `INCONCLUSIVE_SEQ_ORDER` | duplicateとして停止する |
| `next < previous` | `INCONCLUSIVE_SEQ_ORDER` | out-of-orderとして停止する |
| `source_instance_id`がreload / restartなしに変化 | `INCONCLUSIVE_SEQ_ORDER` | unexpected session changeとして停止する |
| reload / restart後に新IDの`probe.loaded` / seq 1へ切替 | session boundary | 新sessionを別系列として検査する |
| 同じsourceの`probe.ready(false)`後に`probe.mapping_active` | mapping boundary | 初期snapshotとreadyを再確認し、再discoverする |

reload / restartをまたぐsequence値を同じ系列として比較しません。ただし、旧sessionのrecordが新session開始後に到着した場合はorphanとしてphaseを`INCONCLUSIVE_SEQ_ORDER`にします。seq gap、`probe.overflow`、未完request / snapshot / host-ID fragment、deactivation discardがあるrunからbank完全性、callback不存在、ID寿命、終端を結論付けません。

## Runtime run matrix

| run | exact host | API | access | fixture cases | status |
| --- | --- | --- | --- | --- | --- |
| C13-MB | Cubase 13.0.30.226 | v1.1 | Mixer Bank | E0 / E1 / C1 / M1 / R1 / R2、O1は安全時のみ | `PENDING` |
| C13.0.50 | `NOT_AVAILABLE` | v1.2 | Mixer Bank / DirectAccess | `NOT_AVAILABLE` | `NOT_AVAILABLE` |
| C15-MB | Cubase 15.0.30.287 | v1.3 | Mixer Bank | E0 / E1 / C1 / M1 / R1 / R2、O1は安全時のみ | `PENDING` |
| C15-DA | Cubase 15.0.30.287 | v1.3 | DirectAccess | E0 / E1 / C1 / M1 / R1 / R2、O1は安全時のみ | `PENDING` |

各runでedition、About表示、実施日時 / timezone、repository commit、probe digest、MixConsole surface / visibility sync、filter、callback window、reconnect deadlineを追記します。

## Cubase 13.0.30 / API v1.1 Mixer Bank観測

### Host accessと初期化

| 項目 | 観測値 | result |
| --- | --- | --- |
| exact Cubase edition / About表示 | `PENDING` | `PENDING` |
| runtime API version evidence | `PENDING` | `PENDING` |
| `probe.loaded` seq 1 → `probe.mapping_active` → 初期snapshot → `probe.ready(true)`順 | `PENDING` | `PENDING` |
| probe activate / deactivate順 | `PENDING` | `PENDING` |
| 8 slot初期callbackの種類と順序 | `PENDING` | `PENDING` |
| title初期値の完全性 | `PENDING` | `PENDING` |
| selected / mute / solo初期値の完全性 | `PENDING` | `PENDING` |
| unique ID取得可否とsource | `PENDING` | `PENDING` |
| empty slot表現 | `PENDING` | `PENDING` |
| explicit bank終端signal | `PENDING` | `PENDING` |
| callback収束時間 | `PENDING` | `PENDING` |

### C1 bank sequence

各slotのtitle / ID alias / selected / mute / soloはlocal tableで保持し、この表には順序付きalias集計だけを転記します。

| config | checkpoint | ordered fixture labels / empty slots | callback count | seq integrity | result |
| --- | --- | --- | ---: | --- | --- |
| `MB_CORE_ALL` | B0-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B1-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B2-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B3-extra-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B4-prev | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B5-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B0-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B1-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B2-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B3-extra-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B4-prev | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B5-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

### Scopeと完全性

| case / candidate | expected UI ground truth | observed inclusion / order | missing / duplicate / unknown | result |
| --- | --- | --- | --- | --- |
| E0 Project inventory | 0 Project Track | `PENDING` | `PENDING` | `PENDING` |
| E1 | Audio Track 1本 | `PENDING` | `PENDING` | `PENDING` |
| C1 Project inventory | P01〜P20 | `PENDING` | `PENDING` | `PENDING` |
| C1 all core channels | P02〜P20 | `PENDING` | `PENDING` | `PENDING` |
| C1 visible core channels | P02〜P07、P09〜P20 | `PENDING` | `PENDING` | `PENDING` |
| O1 Input / Output / VCA | fixtureで安全に作成できた行 | `PENDING` | `PENDING` | `PENDING` |

Probeはcore比較用の`MB_CORE_ALL` / `MB_CORE_VISIBLE`に加え、fixtureのO1手順と同じ次のoptional configを実装しています。O1を安全に作成できたrunだけで使い、各configを独立したB0〜B5 sequenceとして観測します。

| config | channel type | window zone | followVisibility |
| --- | --- | --- | --- |
| `MB_OPTIONAL_MAIN` | VCA / Input / Outputだけをinclude | unlocked main。API v1.1ではleft / right excludeによるimplicit main | false |
| `MB_OPTIONAL_LEFT` | VCA / Input / Outputだけをinclude | left | false |
| `MB_OPTIONAL_RIGHT` | VCA / Input / Outputだけをinclude | right | false |

optional configの存在はInput / Output / VCAがruntimeで返る証拠ではありません。既存busもslotへ現れ得るためfixture entityだけに絞ったと推測せず、raw snapshot全体を記録します。

`B3-extra-next`が同一snapshotに見えることだけを終端証拠にしません。repeat bank、前値維持、callback欠落を区別できなければ完全列挙は`PENDING`または`INCONCLUSIVE_*`のままです。

## Cubase 15.0.30 / API v1.3 Mixer Bank観測

### Host accessと初期化

| 項目 | 観測値 | result |
| --- | --- | --- |
| exact Cubase edition / About表示 | `PENDING` | `PENDING` |
| runtime API version evidence | `PENDING` | `PENDING` |
| `probe.loaded` seq 1 → `probe.mapping_active` → 初期snapshot → `probe.ready(true)`順 | `PENDING` | `PENDING` |
| v1.1 runとのfilter / callback差分 | `PENDING` | `PENDING` |
| Mixer Bank unique ID初期値 | `PENDING` | `PENDING` |
| title / selected / mute / solo初期値 | `PENDING` | `PENDING` |
| empty slot表現 | `PENDING` | `PENDING` |
| explicit bank終端signal | `PENDING` | `PENDING` |
| callback収束時間 | `PENDING` | `PENDING` |

### C1 bank sequence

| config | checkpoint | ordered fixture labels / empty slots | callback count | seq integrity | result |
| --- | --- | --- | ---: | --- | --- |
| `MB_CORE_ALL` | B0-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B1-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B2-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B3-extra-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B4-prev | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | B5-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B0-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B1-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B2-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B3-extra-next | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B4-prev | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | B5-reset | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

### ScopeとID寿命

| 項目 | 観測値 | result |
| --- | --- | --- |
| E0 / E1の件数と順序 | `PENDING` | `PENDING` |
| C1のscope / hidden P08 | `PENDING` | `PENDING` |
| 同名P02 / P03のID分離 | `PENDING` | `PENDING` |
| Unicode / 長いtitle | `PENDING` | `PENDING` |
| bank移動後の同一Track ID | `PENDING` | `PENDING` |
| rename後のID | `PENDING` | `PENDING` |
| project切替後のID | `PENDING` | `PENDING` |
| reload / restart後のID | `PENDING` | `PENDING` |

## Cubase 15.0.30 / API v1.3 DirectAccess観測

### Lifecycleとtree

| 項目 | 観測値 | result |
| --- | --- | --- |
| feature detection結果 | `PENDING` | `PENDING` |
| activate → base object取得順 | `PENDING` | `PENDING` |
| update / object change callback順 | `PENDING` | `PENDING` |
| deactivate後のcallback有無 | `PENDING` | `PENDING` |
| E0 root / child count | `PENDING` | `PENDING` |
| E1 root / child count /順序 | `PENDING` | `PENDING` |
| C1 treeのscope、深さ、順序 | `PENDING` | `PENDING` |
| Folder / hidden / Group / FX inclusion | `PENDING` | `PENDING` |
| Input / Output / VCA inclusion | `PENDING` | `PENDING` |
| duplicate / cycle / invalid child ID | `PENDING` | `PENDING` |
| tree走査の収束時間 | `PENDING` | `PENDING` |

### Metadata

| 項目 | 観測値 | result |
| --- | --- | --- |
| object unique IDの空 / 重複 | `PENDING` | `PENDING` |
| 同名P02 / P03のID分離 | `PENDING` | `PENDING` |
| titleとUI ground truthの一致 | `PENDING` | `PENDING` |
| Unicode / 長いtitle | `PENDING` | `PENDING` |
| mixer index / zone / visibility | `PENDING` | `PENDING` |
| `getObjectTypeName`の返却値 | `PENDING` | `PENDING` |
| typeを取得できないnode | `PENDING` | `PENDING` |

`getObjectTypeName`の結果は、この正確なCubase 15.0.30 / API v1.3 runだけの観測として記録します。API v1.2のTrack DTOへ同じtype取得可否を転記しません。

### Change callback

| 操作 | object change | will-be-removed | tree / metadata再取得 | seq integrity | result |
| --- | --- | --- | --- | --- | --- |
| rename | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| add | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| delete | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| visibility | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| selection | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| mute / solo | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

## Mutation、project切替、reload、restart

各checkpointはfixtureのM1手順どおりに分離し、selection準備操作をrename / add / deleteと同じwindowへ混ぜません。

| checkpoint | 確認対象 | C13-MB | C15-MB | C15-DA |
| --- | --- | --- | --- | --- |
| S0 | C1初期snapshot、ID、state | `PENDING` | `PENDING` | `PENDING` |
| S1-select | P10単独selection | `PENDING` | `PENDING` | `PENDING` |
| S1-rename | title callback、ID寿命 | `PENDING` | `PENDING` | `PENDING` |
| S2-select-anchor | P12単独selection | `PENDING` | `PENDING` | `PENDING` |
| S2-add | add、自動selection、後続index | `PENDING` | `PENDING` | `PENDING` |
| S3-select-delete | P11単独selection | `PENDING` | `PENDING` | `PENDING` |
| S3-delete | removal、自動selection、後続index | `PENDING` | `PENDING` | `PENDING` |
| S4-show | hidden P08をshow、bank / tree差分 | `PENDING` | `PENDING` | `PENDING` |
| S5-select-anchor | P12比較元selection | `PENDING` | `PENDING` | `PENDING` |
| S5-select-change | P12 false / P13 trueの順 | `PENDING` | `PENDING` | `PENDING` |
| S6-mute | P04 muteと意図しないselection | `PENDING` | `PENDING` | `PENDING` |
| S7-solo | P03 soloと意図しないselection | `PENDING` | `PENDING` | `PENDING` |
| S8-project-only-hide | visibility同期off時のsurface分離 | `PENDING` | `PENDING` | `PENDING` |
| S8-restore | 同期とP08 hiddenの原状回復 | `PENDING` | `PENDING` | `PENDING` |
| S9-empty | E0へproject切替 | `PENDING` | `PENDING` | `PENDING` |
| S9-mutation | M1へ戻る | `PENDING` | `PENDING` | `PENDING` |
| S9-baseline | C1へ戻る | `PENDING` | `PENDING` | `PENDING` |

R1 / R2のreconnect deadlineは既定`30000 ms`です。deadlineを延長して成功したように見せず、変更したrunはprimary comparisonから分離します。

| phase | 確認対象 | C13-MB | C15-MB | C15-DA |
| --- | --- | --- | --- | --- |
| R1 pre-reload | session、ID alias、初期snapshot | `PENDING` | `PENDING` | `PENDING` |
| R1 reload | clean `probe.ready(false)` / 新IDの`probe.loaded` seq 1 / `probe.mapping_active` / 初期snapshot / `probe.ready(true)`順 | `PENDING` | `PENDING` | `PENDING` |
| R1 post-reload | 30秒以内のready、ID、初期callback | `PENDING` | `PENDING` | `PENDING` |
| R2 pre-restart | session、ID alias、初期snapshot | `PENDING` | `PENDING` | `PENDING` |
| R2 restart | 正常終了、新process、新session境界 | `PENDING` | `PENDING` | `PENDING` |
| R2 post-restart | 30秒以内のready、ID、初期callback | `PENDING` | `PENDING` | `PENDING` |

## Access方式比較

この表はruntime完了後にだけ更新します。静的なmethod存在だけで「可」にしません。

| 判断項目 | C13 v1.1 Mixer Bank | C15 v1.3 Mixer Bank | C15 v1.3 DirectAccess |
| --- | --- | --- | --- |
| boundedな0件 / 1件 / 複数bank列挙 | `PENDING` | `PENDING` | `PENDING` |
| 終端を重複・欠落なく判定 | `PENDING` | `PENDING` | `PENDING` |
| 同名を分離するopaque ID | `PENDING` | `PENDING` | `PENDING` |
| rename中のID一貫性 | `PENDING` | `PENDING` | `PENDING` |
| title / Unicode保持 | `PENDING` | `PENDING` | `PENDING` |
| typeを推測せず取得 | `PENDING` | `PENDING` | `PENDING`（v1.3だけ） |
| selected / mute / solo初期値 | `PENDING` | `PENDING` | `PENDING` |
| delayed / out-of-order callback耐性 | `PENDING` | `PENDING` | `PENDING` |
| mutation中snapshotの失効検出 | `PENDING` | `PENDING` | `PENDING` |
| hidden / zone / I/O scopeの明確さ | `PENDING` | `PENDING` | `PENDING` |
| reload / restart後の回復 | `PENDING` | `PENDING` | `PENDING` |
| API feature detectionによる安全なfallback | `PENDING` | `PENDING` | `PENDING` |

## 推奨方式の判定基準

最終推奨: `PENDING`

Issue #4へ方式を提案するには、候補が次のgateをすべて満たす必要があります。

1. **観測integrity**: callback windowとsequence検査を通過し、drop、duplicate、逆順、orphanがない。
2. **bounded completeness**: E0、E1、bank幅ちょうど、bank幅超過、最終partial pageで、有限回の操作から重複・欠落なく終端を判定できる。
3. **opaque identity**: 同名Trackを区別でき、name、index、type、routingからIDを合成しない。ID寿命をsession / rename / project / reload / restart単位で明記できる。
4. **truthful metadata**: titleとstateの初期値到着前をreadyにせず、取得不能なtypeやstateを推測しない。
5. **snapshot consistency**: add / rename / delete / visibility / state変更中に異なる時点のTrackを正常な単一snapshotへ混ぜない。変更検知またはgeneration失効を実装できる。
6. **scope determinism**: Project、MixConsole、visibility、window zone、Input / Output / VCAの包含規則を明示できる。
7. **lifecycle safety**: activate / deactivate / reload / restartでpending stateを破棄でき、unsupported hostでも既存Transport 6 Toolを壊さない。
8. **version evidence**: 正確なruntime hostで確認した能力だけを対応表へ載せる。Cubase 15 / API v1.3のtype結果をAPI v1.2へ一般化しない。

判定時は次のfail-closed規則を適用します。

- API v1.1 Mixer Bankで完全な終端またはopaque IDを保証できなければ、そのhostで`tracks.list`を成功扱いせずCapabilityを`false`とする案を選ぶ。
- DirectAccessがgateを満たす場合も、feature detectionとlifecycle guardを必須にする。
- API v1.2の最低対応versionを主張するには、v1.2公式契約だけで十分な項目と、正確なv1.2 runtime runを必要とする項目を分離する。現在13.0.50 runtimeは`NOT_AVAILABLE`である。
- どの候補も完全性を証明できなければ、推測によるfallbackを追加せず`tracks.list`を未対応のままにする。

## Issue #3完了checklist

- [ ] Cubase 13.0.30とCubase 15について、edition / version / build、MIDI Remote API、OS / build / architectureをruntime runごとに記録した
- [ ] repository commitとprobe source / deployed digestが一致している
- [ ] fixture E0 / E1 / C1 / M1、R1 / R2、必要に応じO1を再現した
- [ ] C13 Mixer BankとC15 Mixer Bank / DirectAccessの完全な観測tableがある
- [ ] callback windowとsequence gap判定を全checkpointへ適用した
- [ ] 同名、Unicode、長い名前、hidden、Folder、複数type、bank幅超過をUI ground truthと比較した
- [ ] add / rename / delete / selection / mute / solo / visibility / project切替を個別windowで観測した
- [ ] ID寿命、callback順、空slot、bank終端、tree scopeを観測値と未確認事項に分けた
- [ ] Mixer BankとDirectAccessの制約を比較した
- [ ] 完全性を保証できない項目を`PENDING`、`NOT_AVAILABLE`、`UNSUPPORTED`、`INCONCLUSIVE_*`から適切に分類した
- [ ] 推奨方式とfallback方針を上のgateから導出した
- [ ] 13.0.50未installと、C15 API v1.3 typeをv1.2へ一般化できないことを維持した
- [ ] raw log、raw ID、absolute path、device名、`.cpr`、audio / MIDI / SysEx、credentialをcommitしていない

## 公式資料

- [MIDI Remote API releases and compatibility](https://steinbergmedia.github.io/midiremote_api_doc/versions/)
- [MIDI Remote API v1.2 changes](https://steinbergmedia.github.io/midiremote_api_doc/new_in_v1.2/)
- [MIDI Remote API v1.3 changes](https://steinbergmedia.github.io/midiremote_api_doc/new_in_v1.3/)
- [DirectAccess advanced topic](https://steinbergmedia.github.io/midiremote_api_doc/advanced-topics/direct-access/)
- [MIDI Remote API Reference](https://steinbergmedia.github.io/midiremote_api_doc/codedoc_api_reference/)
- [Cubase 13 MIDI Remote Script Console](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/midi_remote/midi_remote_script_console_r.html)
- [Cubase 13 Track Controls](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/tracks_about/tracks_about_track_controls_r.html)
- [Cubase 13 Track Visibility](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/project_window/project_window_showinghiding_individual_tracks_c.html)
- [Cubase 13 Audio Connections](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/vst_connections/vst_connections_c.html)
