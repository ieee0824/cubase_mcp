# Track Host API実機スパイク結果

この文書は[Issue #3](https://github.com/ieee0824/cubase_mcp/issues/3)の調査記録です。[Track API実機検証fixture](track-api-fixture.md) revision 2を使い、CubaseのMixer BankとDirectAccessでそれぞれ明示したbounded scopeのTrack情報を安全かつ欠落なく取得できるかを比較します。DirectAccessの観測scopeは完全なhost graphではなく、`mix_console_root_children_v1`で固定したbase objectとその直下childだけです。ここでの観測結果を[Issue #4](https://github.com/ieee0824/cubase_mcp/issues/4)の方式・対応version決定と[Issue #5](https://github.com/ieee0824/cubase_mcp/issues/5)のDTO / ID / pagination契約へ入力します。revision 1のfixtureまたはrun artifactをrevision 2の証拠として再利用しません。

文書状態: `PENDING_RUNTIME`

このrevisionではread-onlyの静的preflightと、実機観測用Probe / collector / fail-closed auditorのoffline実装が完了しています。2026-08-28のCubase 15 formal attemptはE1で旧full-graph traversalがrepeat edgeを`cycle_detected` / `truncated`として停止したためinvalidであり、runtime結果へ採用しません。この失敗を新しいroot-child projectionで遡及的に成功へ変更せず、fresh collector、fresh run ID、修正版Probeで再実施します。したがって、観測欄の`PENDING`は成功、対応、非対応のいずれも意味しません。runtime表、比較、最終推奨、完了checklistが埋まるまでIssue #3をcloseしません。

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

Cubase edition、runtimeが実際にloadしたAPI、run日時、About画面の完全な表示、repository commit、probe source / installer embedded / deployedとcollector binaryのSHA-256は各runで`PENDING`から置換します。13.0.50は未installなので、Cubase 15のrunを「13.0.50確認済み」として扱いません。

Probeの`host_version`は`mDefaults.mAppVersion.getVersionString()`由来で、build番号を省略した`13.0.30` / `15.0.30`を返し得ます。これはruntime profileの補助照合にだけ使い、auditorは対応する3要素semantic versionまたはprofile固定のbuild込み完全値のどちらかだけを受理します。正確なCubase buildはmanifestのAbout / app bundle証拠で別に固定し、Probe値で補完・推測しません。別patchまたは別build文字列をprefix一致で受理しません。

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
| base object、child count、child object IDによるroot-child取得 | 存在 | `PENDING` | base object直下より深いhost graph、Project Track全体、Folderを欠落なく返すことを保証しない |
| object unique name getter | 存在 | `NOT_INVOKED_PRIVACY` | ambient host文字列をraw化しないためruntime値・例外・寿命は意図的に未観測 |
| object unique ID / title | 存在 | `PENDING_FIXTURE_SCOPE` | allowlist済みfixture node以外のraw値は収集せず、各値の必須性、長さ、寿命を一般化しない |
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
4. mutationはM1 copyでだけ実施し、C1 baselineを上書きしない。Input / Output / VCAを変更するO1は通常runと別に明示的な実施許可を得た場合だけO1 copyで実施する。
5. audit v1 / primary ProbeではO1を実施せず、`skipped / not_separately_authorized`とする。将来の別profileで実施する場合もbus inventoryはlocalだけに保持し、終了時に完全一致しなければ`RESTORE_FAILED`として通常projectを開かない。
6. repository commit、probe source、installer embedded source、deployed probeのdigestを照合し、3つが一致しなければrunを開始しない。実行するcollector release binaryのSHA-256も別に固定する。
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
| auditor source | `src/bin/cubase_track_probe_audit.rs` | 確定 |
| auditor test | `cargo test --bin cubase_track_probe_audit --locked` | 確定 |
| auditor binary | `target/release/cubase_track_probe_audit`（Windowsは`.exe`） | 確定 |
| auditor interface | `--manifest <FILE> --jsonl <FILE>`、sanitized JSON stdout | 確定 |
| probe installer | `target/release/cubase_mcp --install-track-probe --midi-remote-root <LOCAL_ROOT>`（Windowsは`.exe`） | 確定 |
| collector output format | flushed JSON Lines、`record_format_version = 1` | 確定 |
| deployed probe location | repository外のlocal Cubase MIDI Remote script directory | `PENDING_RUNTIME` |
| source SHA-256 | run開始時にrepository sourceから取得 | `PENDING_RUNTIME` |
| installer embedded SHA-256 | current HEADからbuildしたinstaller JSON reportから取得 | `PENDING_RUNTIME` |
| deployed SHA-256 | run開始時に配備済みsourceから取得 | `PENDING_RUNTIME` |
| collector binary SHA-256 | run開始時に実行するrelease binaryから取得 | `PENDING_RUNTIME` |
| repository commit | run開始時の40桁commit SHA | `PENDING_RUNTIME` |

Track Probeはcollectorへ渡す前にsource-side data minimizationを行います。revision 2の固定fixture allowlistへ一致するtitleと、それに属するopaque host IDだけをraw frameへ許可し、それ以外のbank / DirectAccess nodeはtitleとhost IDを`null`へして明示的redaction flag / 件数だけを送ります。P09は固定80文字のprefixかつ予約marker `CMCP_09_LONG_`全体を含む場合だけfixture扱いします。DirectAccessのobject unique name getterはambient文字列を取得しないよう意図的に呼び出さず、fixed allowlist外のtype名とhost exception textも送信前にredactまたは固定code化します。`mix_console_root_children_v1`内のroot-child位置、再訪辺、安全なboolean / numeric metadataはscope差を観測するため保持します。depth-1 observationではchild countをbounded metadataとして取得しますが、そのchild object ID getterは呼びません。runtime capabilityの固定`data_minimization`契約とartifact digestが一致しないrunを開始しません。

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
  > <REPOSITORY_OUTSIDE>/CMCP_TrackProbe_<RUN_ID>.jsonl
```

Windows、または既存portを明示して使う場合は、localで確認した完全なport名を追加します。raw port名はlocal run recordにだけ残します。

```text
target/release/cubase_track_probe_collector \
  --run-id <RUN_ID> \
  --discovery-window-ms 1000 \
  --drain-timeout-ms 5000 \
  --midi-input "<FROM_CUBASE_PORT>" \
  --midi-output "<TO_CUBASE_PORT>" \
  > <REPOSITORY_OUTSIDE>/CMCP_TrackProbe_<RUN_ID>.jsonl
```

`--run-id`にはcredential、project名、個人名、顧客名、absolute pathを含めません。collectorは観測fileを自動作成せずstdoutへ出力するため、repository内へredirectしません。primary runでは上記の1000 ms discoveryと5000 ms graceful drainを固定し、変更したrunは値と理由を記録してprimary comparisonから分離します。

collector終了後、raw JSONLとaudit manifestを次のauditorへ同時に渡します。最初の出力先はrepository外とし、exit statusが成功でsanitized JSONを得られた場合だけ内容を確認します。raw JSONLやmanifestのpath、raw host ID、port / device名をauditor stdoutへ含めません。

```text
target/release/cubase_track_probe_audit \
  --manifest <REPOSITORY_OUTSIDE>/CMCP_TrackProbe_<RUN_ID>.manifest.json \
  --jsonl <REPOSITORY_OUTSIDE>/CMCP_TrackProbe_<RUN_ID>.jsonl \
  > <REPOSITORY_OUTSIDE>/CMCP_TrackProbe_<RUN_ID>.audit.json
```

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
bank config / slot index または DirectAccess root-child位置・再訪辺
host_id_raw                # local raw JSONLのみ
host_id_byte_length
returned title / type / selected / mute / solo / visibility
capability result
```

APIが値を提供しないfieldは`null`またはfield不存在のままにし、直前値やUI ground truthで補いません。`host_id_alias`と必要な場合のSHA-256はraw JSONL検証後に作るcommitted summaryだけの値です。Probeとcollectorはcallbackの到着順を維持し、同一timestampのrecordを並べ替えません。

### audit manifest v1 / UI annotation sidecar

raw JSONLとは別に、fixtureの[runtime evidenceの3 artifact](track-api-fixture.md#runtime-evidenceの3-artifact)で定義する`audit_manifest_version = 1`のmanifestをrepository外へ作成します。manifestは`fixture_revision = 2`、raw JSONLと同じ`run_id`、raw開始時刻とmillisecond単位で一致する`run_started_at`、`c13_mixer_bank`または`c15_combined` profile、exact host / OS / API、repository commit、一致するprobe source / installer embedded / deployedの3 digest、実行したcollector binary digest、fixture acceptance、5000 ms window、30000 ms reconnect deadline、O1 status、必須checkpointごとのUI annotationを保持します。raw host ID、port / device名、absolute pathをmanifestへ転記しません。

raw JSONLの`collector_summary.exit_ok`は通信streamのintegrityだけを表し、必要なfixture case、UI確認、final snapshot、環境・digest一致を証明しません。auditorはprofileから必須coverageを導出し、raw JSONLとmanifestの双方をfail closedで検証します。どちらか一方しかないrun、manifestとraw JSONLの`run_id`が違うrun、fixture revision 1のrun、自動初期snapshotしかないrunを`OBSERVED`へ使いません。

両profileの必須checkpoint IDはfixtureで定義したexactly 44個です。各checkpointはbegin後・実操作直前のraw `collector_action` markerをexactly 1件必要とします。cutを使う21 checkpointではcollectorが成功cut responseと、同request ID / epochかつ`boundary_source = probe.observation.cut_response`のaction markerをatomic output pairとして隣接出力し、operatorは別のmarkerを送らず、その自動marker直後にUI操作します。このmarkerを5000 ms windowの監査anchorとします。E8 / C1 bank navigationではReset / Next / Prev成功response、`INIT` / `R1` / `R2`では明示的action markerがanchorです。final snapshotは対応anchorから5000 ms以上後でなければなりません。navigation checkpointはmarkerから1000 ms以内にID suffixどおりの操作を送り、同じconfigの明示的`probe.bank.snapshot`を必要とします。それ以外の`INIT` / `E0` / `E1` / `E8` / `C1`、S0〜S9、R1 / R2はfinal `MB_CORE_ALL`と`MB_CORE_VISIBLE` bank snapshotを両方必要とします。C15-COMBINED、およびINITのruntime capabilityがDirectAccessをsupported / activeと報告したC13 runでは、各checkpointで`probe.direct_access.snapshot`を先に完了させ、その後に必要なbank snapshotを完了させます。C13で継続可能なunsupported分岐は`supported: false, active: false`かつ固定のunavailable / incomplete reason、DirectAccess eventなしだけです。`supported: true, active: false`、activation error、またはその他の組合せはMB-only成功へ読み替えずrun invalidとします。`INIT`はaction marker後にCubaseを起動し、初期snapshot完了後にreadyとなったことを検証してからdiscoverとcapabilitiesを完了します。cold launchはR1 / R2用の30秒deadlineへ含めません。R1 / R2はaction markerから30000 ms以内に新しいreadyと再discoverを完了し、ready / discoveryから10000 ms以内にfinal snapshotを完了させます。

### command、checkpoint、integrity契約

stdinは1行1 JSONです。`collector.checkpoint.begin` / `collector.action` / `collector.checkpoint.end`はcollector内だけで処理し、Cubaseへ送信しません。

```json
{"method":"collector.checkpoint.begin","params":{"checkpoint_id":"E8-MB_CORE_ALL-B0-reset","window_ms":5000}}
{"method":"collector.action","params":{"checkpoint_id":"E8-MB_CORE_ALL-B0-reset"}}
{"method":"collector.checkpoint.end","params":{"checkpoint_id":"E8-MB_CORE_ALL-B0-reset"}}
```

- checkpoint IDは同一run内で一意とし、同時に複数を開かない。
- UI操作は対象Cubase instanceへの排他的な入力として扱う。操作ごとにfreshなactive application / window / project / dialog / targetを取得し、semantic操作はapplication / window / element identity、座標clickはapplication / current window、fresh screenshotから解決した明示座標、single / doubleのclick countをexactly 1 UI callへ束縛する。現在pointerの移動とclickを別callにせず、current window外の座標は使用しない。pointer movementだけではtarget-bound callの対象は変わらないが、guardはautomation callの対象、座標、click count、UI outcomeを検査しない。したがって、fresh pre-state、exactly 1 callのoperator tool trace、action-specificなfresh post-stateとProbe差分を必須とし、focus / window / targetの予期しない変化、対象外への作用またはpostcondition不一致をguard成功から救済しない。同じcheckpoint内で再clickしない。
- macOS primary runでは、Cubase停止中にcollectorを起動して`collector_started`を確認した後、最初のcheckpointより前に同一processの`cubase_input_guard`を起動し、`version = 3`、`source = hid_system_state`、`coverage = action_windows`、`privacy = counts_and_held_state_boolean`、`policy = consequential_input_only`を確認する。collector起動のOS承認はguard startup前に完了させ、startup後はcollectorを再起動しない。各操作のfresh pre-stateより前に従来どおり`{"command":"arm","action_id":"..."}`を送り、post-state確認直後に同じIDで`check`する。正常に完了したarm / check sample間の`mouse_moved`はinformationalであり、単独ではinterferenceにしない。left / right / other mouse down / up / drag、key down / up、flags changed、scroll wheel、tablet pointer / proximityの15 deltaはいずれも1件以上でfail closedとする。`HID_SYSTEM_STATE` counterはeventのactorや物理 / synthetic由来を認証しない。各sampleではper-type counterとheld-stateの読取り全体を`kCGAnyInputEventType` aggregate counterで前後から括り、その短い区間のaggregate変化はmove-onlyでも`INPUT_DURING_SAMPLE`として拒否する。held key / mouse buttonと`2000 ms` timeoutもfail closedにする。arm前のidle入力は次のbaselineへ繰り越さず、guard error / 終了 / ID不一致 / finish失敗 / cancelled / latchではrunを停止する。
- run前に実際のUI callごとのordered action inventoryをrepository外へ固定し、menu open / item選択 / dialog入力 / 確定 / window raise / scroll / click / keyをそれぞれ別IDでarm/checkする。run前に決まるAPI、操作、click count、precondition、postconditionをinventoryへ固定し、dynamicなsemantic identityまたは座標はfresh pre-stateから解決して同じIDのoperator tool traceへ記録する。guard JSONLのarmed / result ID列とinventory、全recordのv3 policy、全resultの15 consequential deltaが0であることを機械照合し、欠落・追加・逆順を拒否する。sidecarはautomation APIの直接callをinterceptしないため、inventory、exactly 1 callのtool trace、fresh pre / post-stateなしに全UI callの正しさを主張しない。formal run前には`open`、`press_key`、`set_value`、semantic click、single / double coordinate clickのexact contextで15 deltaが0となるnegative control、physical move-onlyが許可されるcontrol、physical click / key / scroll / dragが拒否されるpositive controlをfixture文書のmatrixどおりscratch surfaceで完了する。
- `begin`後、実操作直前に同じIDのaction markerをexactly 1回記録する。`E0` / `E1` / `E8` / `C1`とS0〜S9のexactly 21 checkpointでは、最初のtargeted commandとして`probe.observation.cut`を`@selected`へ送る。成功responseはcollectorがraw responseと同request ID / epochのaction markerをatomic output pairとして隣接出力するため、operatorは`collector.action`を重ねず、自動marker直後にUI操作する。`INIT` / `R1` / `R2`と、Reset / Next / Prevが新しいbank generationを作るE8 / C1 navigationではcutを送らず、明示的`collector.action`を使う。navigation commandはmarkerから1000 ms以内に送る。cut checkpointと`INIT` / `R1` / `R2`はaction marker、navigationは操作成功responseを観測anchorとし、対応anchorから5000 ms以上を経過させた後、checkpoint内で利用中の各projectionへ明示的final snapshot commandを送る。DirectAccessがactiveならDirectAccessを最初に完了し、同期`update()`が生むfeedbackもその完了前に収め、その後に必要なbank projectionを送る。最初のfinal snapshot完了後は、後続する同じsetの明示的snapshot request / response / chunk以外のprobe messageやhost callbackを許さない。set最後のresponse / 全chunk完了後から`end`までもprobe messageもhost callbackも1件もない状態で1000 ms以上quietを満たし、UI annotationを完了してからだけ`end`を送る。set途中または最後のsnapshot後に予期しないmessageが届いたrunは、quiet timerだけを再開して成功扱いにしない。
- 5000 ms未満の`end`、対応しないID、開いたcheckpointを残したEOFはfatalとする。
- 初期load、reload、restartを含むProbe観測も専用checkpointで囲む。期待していないProbe recordがcheckpoint外または終了後に届いた場合はorphanとして記録し、そのsequenceを成功にしない。
- Probe commandはactive checkpoint内だけで許可し、`probe_command`（`phase: started`）と`probe_command_send_result`を別々に記録する。collector自体は診断用に明示的なinstance IDも扱い、その経路ではcommand recordをMIDI送信前に出力するが、audit v1 runではdiscover以外の全commandをcollector-localの`@selected`経路に固定し、両evidence recordの`evidence_emission: after_midi_send_attempt`で逸脱を拒否する。`@selected`は送信barrier内で実IDへ解決するため、stdout I/Oでbarrierを塞がないよう、実ID入りcommand recordとsend-resultを送信試行直後に連続出力し、send完了時のmonotonic timestampを付ける。この経路では高速なresponseと、それに隣接するcut用auto action markerのpairが両command evidence recordより先にJSONLへ現れ得るので、auditorはfile順やevidence emit時刻を送信順と推測せずrequest ID、send-completed timestamp、response receive timestampで対応付ける。どちらの経路もrequest、response / error、必要なsnapshot follow-upを同じcheckpoint IDへ結び付け、送信失敗を送信済みとして扱わず、送信後のevidence出力失敗は結果不明のfatal runとする。
- Probe command送信とcheckpoint終了の直前にMIDI ingress barrierを通し、その時点までに開始したcallback、分割受信中のSysEx、受信済みqueueをProtocol trackerへ反映する。未完SysEx、未処理、queue overflow、barrier timeout、integrity failureがあればcommandを送信せずfail closedにする。
- 未完request、discovery window、snapshot follow-up、chunkがある間はcheckpointを終了しない。自動初期snapshot、操作直後のcallback、直前checkpointのsnapshotはfinal snapshotとして数えない。callback受信時刻が終了marker以前なら、collector側の処理がmarker後になっても同じcheckpointへ分類する。

discoveryだけは`target_instance_id: null`でbroadcastし、既定1000 msのbounded discovery windowを最後まで待ちます。windowはrequest登録時ではなく、`probe_command_send_result`が`sent: true`となった送信時点から開始し、未送信requestをtimeout済みとして扱いません。

```json
{"target_instance_id":null,"method":"probe.discover","params":{}}
```

0 instance、複数instance、初期化未完のinstanceが応答したrunではtargetを選ばずfatalにします。`probe.ready(true)`済みのexactly 1 instanceをwindow全体で確認した後のcommandだけが、collector-localの`@selected`を送信barrierでそのsourceへ解決して直列に送信されます。page deactivate / reactivate、reload、restartでmapping lifecycleが変わった後は選択を破棄し、再discoverします。operator-facing commandやmanifestへraw source IDをcopyしません。

E0 / E1 / E8 / C1やS9のproject切替を含む通常checkpointでも、hostが同じscript sourceをpage deactivate / reactivateする場合があります。その場合は操作を追加せず、同じcheckpoint内でaction後の`probe.ready(false)` → 同sourceの`probe.mapping_active` → capability → 全初期snapshot → `probe.ready(true)`をexactly 1 sequenceとして完了させ、再discoverしてからfinal snapshotへ進みます。旧activationで既に受信したbounded feedbackは`probe.ready(false)`より前に全件送信し、64件の1 idle batchを超えても最大queue件数まで有限回でdrainします。初期化中のDirectAccess change callbackもfeedbackとして保存しますが、`page_activate` snapshotが同じ初期状態を覆うため、ready前に別の自動change snapshotをscheduleしません。page境界で無効になるcoalescedな非command DirectAccess change snapshotだけはcancelし、明示command、`page_activate`、bank snapshot、未知reason、または未drain feedbackを残したままinactiveへ移行しません。新しい`probe.loaded`はINIT / R1 / R2以外では許可しません。lifecycleが無いrunへこのsequenceを捏造せず、partial、複数回、action前のsequence流用、再discoverなしをauditorが拒否します。

以下は独立したcommand例であり、1 checkpoint内の連続sequenceではありません。`probe.capabilities.get`は`INIT`だけ、`probe.observation.cut`は上記21 checkpointだけ、bank操作は対応navigation checkpointだけで使います。final snapshotはcheckpoint契約に従って必要なprojectionを順に送ります。

```json
{"target_instance_id":"@selected","method":"probe.capabilities.get","params":{}}
```

```json
{"target_instance_id":"@selected","method":"probe.observation.cut","params":{}}
```

```json
{"target_instance_id":"@selected","method":"probe.bank.reset","params":{"config_id":"MB_CORE_ALL"}}
{"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_ALL"}}
{"target_instance_id":"@selected","method":"probe.direct_access.snapshot","params":{}}
```

`probe.observation.cut`はProbe内のbounded epochを進めます。Mixer Bank fieldとDirectAccess callbackはcallback発生時のepochを保持し、cut前にqueue済みの値がcut後にflushされても旧epochのままです。auditorはcut responseと一致しないMixer Bank fieldを`stale`として値を共有reportで`null`にし、DirectAccess callback / live snapshotもrecordのepochと固定statusを検証します。上限到達時はwrapせずreload-requiredのfatal runとします。

targeted requestは、同じrequest IDに対する同じ`source_instance_id`からのresponseまたはerrorがexactly once必要です。unmatched、duplicate、wrong-source、未応答のrequest、および成功response後に必要なsnapshot follow-upがない状態はfatalです。

`probe.bank.chunk` / `probe.direct_access.chunk`は`source_instance_id + snapshot_id`単位で、chunk count、0始まりの連続index、total item数、complete flagを検査します。fragment化されたhost IDもreference、fragment count / index、UTF-8 byte lengthを検査します。欠落、重複、逆順、不整合、未完snapshot / fragment / requestを残したEOFはrun fatalです。collectorはEOF後も設定されたdrain期限まで受信を続け、quiescentにならないrunを成功にしません。

DirectAccess snapshotのcontractは`projection = "mix_console_root_children_v1"`です。これはbase objectをroot observationとして1件取得し、そのrootが宣言した直下child indexだけを0から昇順に1回ずつ取得する**depth-1 projection**です。root childのchild listは展開せず、完全なhost graph走査として扱いません。capabilityのDirectAccess欄は同じ`projection`、`scope_depth = 1`、`authoritative_for_track_enumeration = false`を返し、traversal limitは`direct_access_nodes = 256`、`direct_access_depth = 1`、`direct_access_children = 128`でなければなりません。`direct_access_nodes`はfail-closedな共通semantic-record ceilingであり、このprojectionの形状上の有効な最大件数は後述の129です。この固定値とsnapshot metadataが一致しないrunは開始または監査を続行しません。

各snapshot chunkは`projection`に加えて`scope_depth = 1`、`scope_complete`、`host_graph_complete`、`root_child_count`、`authoritative_for_track_enumeration`を同一値で持ちます。valid snapshotでは`scope_complete = true`、`host_graph_complete = false`、`authoritative_for_track_enumeration = false`です。`scope_complete`はrootと宣言された全root-child coordinateと、そのobservationに必要なbounded metadataを取得した意味に限り、depth 1より下のhost graphやProject Track全体の完全性を意味しません。root observationだけが`parent_id = null / depth = 0 / child_index = null / children_expanded = true`で、root-child observationは`parent_id = base_object_id / depth = 1 / children_expanded = false`です。depth-1 observationの`child_count`は子孫数のbounded metadataとして保持しますが、child object IDを列挙していないため、`children_expanded = false`をleafやdescendant completenessの証拠へ読み替えません。

初回観測したroot-child objectだけを`record_kind = "observation"`として出力します。同じroot child IDが後のindexに再出現した場合やchild IDがroot自身だった場合も辺を捨てず、`record_kind = "object_reference"`としてそのroot-child coordinateを保持します。referenceのexact fieldは`record_kind`、`observation_epoch`、`observation_epoch_status`、targetの`object_id`、source rootの`parent_id`、`child_index`、`depth = 1`、`target_observation_index`、`reference_kind`です。root自身を指す辺だけが`ancestor_cycle`、先行する別root-child observationを指す重複辺は`shared_reference`です。`target_observation_index`はwire item indexではなく0始まりのobservation ordinalで、必ずreferenceより前のobservationに一致させます。reference itemには`children_expanded`、title、unique name、opaque host ID、自由形式type / error文字列を含めません。

`root_child_count`は0以上128以下で、host-ID fragmentを除くsemantic recordは常にroot 1件とroot child coordinateごとのobservationまたはreference 1件、すなわち`observation_items + reference_items == root_child_count + 1 <= 129`です。`reference_items == cycle_count + shared_reference_count`、`cycle_count == ancestor_cycle record数`、`shared_reference_count == shared_reference record数`、`total_items == observation_items + reference_items + host_id_fragment record数`も必須です。host-ID fragmentはsemantic itemではなく、reference itemへhost-ID fieldを追加しません。

auditorは`base_object_id`と一致するroot observationがexactly 1件であること、semantic record列がrootの後にchild index昇順でexactly `0..root_child_count-1`を覆うこと、各coordinateがobservationまたはreferenceのどちらか1件だけであることを確認します。root-child observationのIDは初出、referenceのsourceはroot、targetは先行observation、target ordinalとobject IDは一致しなければなりません。root self-reference / duplicate childの分類、全count式、`children_expanded`、scope metadataの固定値も検証します。欠落・重複・逆順coordinate、forward target、depth 1以外、未知projection、`scope_complete != true`、`host_graph_complete != false`、`authoritative_for_track_enumeration != false`はfatalです。

base / root-child enumeration getterの失敗、invalid child ID、root child count 129以上、semantic-record上限、またはobservationに必須のchild-count取得失敗等でroot-child projection contractを満たせなかった場合は`truncated = true`かつ`scope_complete = false`にします。それ以外のbounded metadata getter failureは`metadata_error_count`と固定codeで別に表し、それだけでscope truncationへ変換しません。depth-1 observationのchild object IDを意図的に取得しないことはtruncationではなく`children_expanded = false` / `host_graph_complete = false`で表します。旧full-graph runのrepeat edge欠落をこの新contractで補完または再解釈せず、`truncated = true`は理由を問わず従来どおりrun fatalです。

`probe.overflow`はsource queue、outbound frame、host-ID fragment上限、snapshot queue、deactivation時の未drain feedbackまたは必須work破棄を含め、理由を問わずrun fatalです。唯一、same-script reactivationまたはrun内restart境界で、旧mappingのcallbackを全件送信済みかつ次の`page_activate` snapshotに置換されるcoalescedな非command DirectAccess change snapshotのcancelは未送信証拠の破棄として扱いません。明示command snapshot、`page_activate`、bank snapshot、未知reasonはこの例外へ含めません。raw logの`collector_summary`が`integrity_ok: true`かつ`exit_ok: true`でないrunを`OBSERVED`へ使いません。

Cubaseは新しく現れたMIDI outputを検出するため、Universal Non-Realtime broadcast Identity Requestのexact 6 byte `F0 7E 7F 06 01 F7`を専用virtual portへ反復送信する場合があります。collectorはSysEx reassembly後、このexact frameだけをProbe transport外の標準検出trafficとしてingress / quiet-period計数前に無視します。device ID、Sub-ID、長さ、終端を含む1 byteでも異なるframeやその他のforeign SysExは従来どおりfatalです。この検出trafficをProbe message、callback不存在、source sequence、checkpoint activityの証拠へ数えません。

### runtime run手順template

この手順はruntime artifactを作成しますが、本revisionではまだ実施していません。

1. 通常projectを保存して閉じ、対象Cubaseも終了した状態で、他のCubase instanceがないことを確認する。
2. offline validationをすべて実行し、repository commit、probe source SHA-256、実行するcollector release binary SHA-256をlocal run recordへ記録する。fixtureで定義した、対象versionで新規作成済みのINIT bootstrapについても、0 Project Track、正式E0とは別file、run前SHA-256を確認する。対象versionを起動するexactなapplication名またはbundle pathとbootstrapのabsolute pathをlocalで確定し、absolute pathをraw JSONL、manifest、run ID、共有report、repositoryへ転記しない。
3. Cubaseを終了したまま、対象Cubase製品の既存`MIDI Remote/Driver Scripts/Local`だけを`--midi-remote-root`へ指定し、`target/release/cubase_mcp --install-track-probe --midi-remote-root <LOCAL_ROOT>`でprobeを配備する。Cubase rootが0件・複数候補、root形状不正、別Steinberg製品、Cubase process検出、process確認失敗ではscriptを作成せず停止する。installerは既存pathを上書きも削除もせず、異なるprobeが存在すれば常に拒否する。必要なら既存fileを手動で退避・確認・削除してから再実行する。新規pathは`create_new`で確保し、symlinkを拒否して書込み後digestを検証する。作成後の書込み・検証・durability確認に失敗したpathは競合file保護のため自動削除せず、手動確認する。JSON reportのabsolute pathはlocalにだけ残し、repository source SHA-256、`embedded_source_sha256`、`deployed_sha256`の三者が完全一致しなければrunを開始しない。最初と作成直前のprocess checkの間にもCubaseを起動しない。
4. repository外を出力先にし、Cubaseを停止したまま`--run-id <RUN_ID>`付きでcollectorを先に起動する。最初の`collector_started` recordとCubase未起動を確認した後、macOS primary runでは入力guardを起動して`version = 3`、`source = hid_system_state`、`coverage = action_windows`、`privacy = counts_and_held_state_boolean`、`policy = consequential_input_only`の`ready`を確認し、以後のresponse / errorでもversion、coverage、policyの一致を要求する。collector起動に必要なOS承認はguard startup前に完了させ、startup後はcollectorを再起動しない。guardの最初の`arm`後にCubase未起動のfresh pre-stateを再確認してから、exact ID `INIT`の初期化checkpointを開始する。revision 2 primary C13 / C15 runではfixtureで定義した`CMCP_TrackFixture_Bootstrap_Empty.cpr`を例外なく使う。`collector.action`を記録した直後に、step 2で確定したexactな製品とbootstrap documentを同じOS launch操作へ指定する。macOSでは`open -a "<EXACT_CUBASE_APPLICATION_OR_BUNDLE_PATH>" "<ABSOLUTE_BOOTSTRAP_PROJECT_PATH>"`のように両pathをquoteする。bootstrapは0 Project Trackかつmedia、event、part、automation、plug-in、user preset、新しいrouting設定、通常project由来の設定を持たず、実機inputをmonitorしないINIT専用projectであり、fixture caseや独立checkpointではない。bootstrap用annotationを増やさず、既存`INIT` annotationを使うため、manifest v1 schemaとexactly 44 checkpoint IDは変更しない。
5. 初期化checkpoint内で対象Cubaseを1 instanceだけ起動してprobeを新規loadする。CubaseだけをHubへ先に起動した後のbootstrap open、別projectを経由したopen、またはprimary INIT中の後付け救済は禁止する。bootstrapが同じlaunch操作で開かなかった場合はrunを停止し、graceful drain後にfresh run ID、fresh collector process、fresh JSONLでINITから再実施する。INIT終了まで別projectを開かず、Project windowのexact basename、0 Project Track、他projectなし、dirty / modified表示なしをUIで確認する。この単一launchを行った場合だけ既存INIT annotationの`action_confirmed`、すべてのUI条件とbootstrap未変更を確認した場合だけ`ui_ground_truth_confirmed`を`true`にする。bootstrapのevidenceやannotationをE0へ流用しない。R1ではcollectorを動かしたままreload用checkpointを開始し、action marker直後に明示的にReload Scriptsする。R2もmarker直後に正常終了操作を開始する。
6. 新しい`source_instance_id`からexactly 1件の`source_seq = 1`の`probe.loaded`を受信したことを確認する。これを新しいprobe sessionの開始markerとする。続く同じsourceのexactly 1件の`probe.mapping_active`でpage activation境界を確認する。
7. exactly 1件の`probe.capabilities` record、その後に`reason = page_activate`の`MB_CORE_ALL` / `MB_CORE_VISIBLE`初期snapshot set、runtime capabilityがDirectAccessをsupported / activeとした場合はその初期snapshotもcompleteになった後、同じsourceからexactly 1件の`probe.ready`の`ready: true`かつ`initial_snapshots_complete: true`を確認する。INIT内の追加の新しい`probe.loaded`、`probe.ready(false)`、2回目のmapping / capability / page-activate initial snapshot set / ready、partial activation、別sourceのactivationが1件でもあればrun invalidとし、後続sequenceで成功へ戻さずfresh runで再実施する。`probe.loaded` → `probe.mapping_active` → capability → 全初期snapshot完了 → readyの順序や完全性が違う場合も開始しない。同じscript sessionの通常checkpointでのpage reactivationは新しい`probe.loaded`ではなく`probe.mapping_active`から始まり、再度同じ初期化sequenceが完了するまで操作しない。
8. `probe.discover`を送信し、bounded discovery window全体でexactly 1 responderだけであることを確認する。後続requestにはoperator側で`@selected`を使い、raw source IDを表示・copyしない。`INIT` action markerから5000 ms以上経過後、C15またはC13でDirectAccessがsupported / activeならDirectAccessを最初に、続けて`MB_CORE_ALL`、`MB_CORE_VISIBLE`の明示的final snapshotを取得し、最後のchunk後にmessageが1件もない1000 ms quietを満たしてから初期化checkpointを終了する。
9. fixtureの再現性checklistを完了し、E0、E1、E8、C1、M1を順に観測する。exact ID `E0`を開始して`probe.observation.cut`の成功responseと隣接する自動action markerを確認した直後に、bootstrapとは別fileの正式な`CMCP_TrackFixture_Empty.cpr`を開く。Project windowでbasename `CMCP_TrackFixture_Empty`と0 Project Trackを目視確認できた場合だけE0の`action_confirmed` / `ui_ground_truth_confirmed`を`true`にする。bootstrapのsnapshotはE0 final snapshotの代用にしない。case open / snapshotはexact ID `E0`、`E1`、`E8`、`C1`、E8 navigationは`E8-<CONFIG>-<STEP>`、C1 navigationは`C1-<CONFIG>-<STEP>`、mutation / project切替はS0〜S9を使う。project切替等で同sourceのreactivationが発生した場合はaction後の`ready(false)` → mapping → capability → 全初期snapshot → readyをexactly 1回完了し、再discoverを同じcheckpoint内で行ってからfinal snapshotへ進む。通常checkpointの新しい`probe.loaded`、partial / 複数reactivation、action前sequenceの流用、再discover省略はrun invalidとする。O1はaudit v1 / primary Probeに含めず`skipped / not_separately_authorized`固定とする。
10. 各bank / mutation操作を固有checkpointで囲む。E0 / E1 / E8 / C1とS0〜S9はobservation cutの成功responseと隣接する自動action markerを確認し、別のmarkerを送らず、その直後に操作する。E8 / C1 navigationは明示的marker直後に操作する。cut checkpointは自動marker、navigationは操作成功responseを観測anchorとして5000 ms以上待った後に明示的final snapshotを要求する。Cubase 15またはC13でDirectAccessがsupported / activeならDirectAccessを最初に取得し、navigationはその後に操作config、その他は`MB_CORE_ALL`と`MB_CORE_VISIBLE`の両方を取得する。最後のresponse / chunk後に新しいmessageがないこと、追加1000 ms quiet、UI annotation、sequence integrityを確認してから次の操作へ進む。
11. R1 reloadとR2 restartは独立phaseとして実施する。R1のpre-stateは`S9-baseline`、R2のpre-stateはR1の監査済みfinal snapshotを参照し、R1/R2内に追加のpre-action snapshot commandを送らない。
12. active checkpointがなく、request / follow-up / snapshotがquiescentなことを確認してstdinをEOFにする。graceful drain後の`collector_summary`を確認する。
13. gap、duplicate、逆順、overflow、orphan、未完request / chunk / fragment、truncation、fatal diagnosticが1件でもあれば、そのrunを完全性の証拠にしない。
14. raw JSONLとaudit manifest v1をfail-closed auditorへ同時に入力し、必須coverageとredaction検査を通過させる。この文書にはredacted audit report v2（`audit_report_version = 2`）の集計と最小限の合成名例だけを転記し、raw JSONLを移動・copy・stageしない。
15. cleanup後、`git status --short`で意図したsource / document以外のartifactがないことを確認する。

## Callback観測windowとsequence判定

### Callback観測window

fixtureに従い、cut対象操作はcut成功responseとatomic pairで隣接する自動action marker直後、navigationとlifecycle操作は明示的marker直後に行い、checkpoint種別ごとの観測anchorから必ず`5000 ms`観測します。checkpoint begin、cut response、navigation commandより前のmarkerからの経過時間で代用せず、最初のquiet periodで早期終了しません。

次の条件をすべて満たすcheckpointだけを`OBSERVED`にできます。

- 明示的final snapshotの最後のresponse / chunk後からcheckpoint終了までprobe messageもhost callbackも1件もなく、連続`1000 ms`以上quietである。snapshot完了直後にcheckpointを閉じず、後着messageがあればrunを停止する。
- UI ground truthを確認でき、access方式がsnapshot / ready状態を提供する場合はそれも取得できた。
- 5000 ms経過後、checkpoint終了前に必要なMixer Bank config（navigationは操作config、その他はALL / VISIBLE両方）の明示的final snapshotがcompleteになった。C15-COMBINED、およびDirectAccessがruntimeでsupported / activeだったC13 runではDirectAccessの明示的final snapshotもcompleteになり、その後の追加quiet periodも満たした。
- 直前checkpointのwindow終了後から次のUI操作前までにorphan callbackがない。
- 同一probe sessionのsequence検査にgap、duplicate、逆順がない。

期限までcallbackが続く、必要なsnapshotがない、final snapshot後にmessageが届く、またはwindow後にcallbackが届いた場合は`INCONCLUSIVE_CALLBACK_TIMEOUT`としてsequenceを停止します。遅延recordは`orphan_after_<checkpoint>`と記録し、次のUI操作を行いません。

collectorはcheckpoint終了時に上記`1000 ms` quiet periodをlast-message clockで検証します。final snapshotのresponse / chunkを含むprobe messageもclockを更新します。auditorはそれに加えて、final setの最初のsnapshot完了後からは後続する同じsetの明示的snapshot request / response / chunkだけを許し、set最後のsnapshotより後のprobe recordを1件でも拒否します。終了marker以前に受信され、終了処理後にqueueから取り出されたmessage / callbackもreceive timestampで判定し、異なるUI状態のprojectionや古いsnapshotをquietだけで成功扱いにしません。

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

| physical run / profile | exact host | API | access projection | fixture cases | status |
| --- | --- | --- | --- | --- | --- |
| C13-MB / `c13_mixer_bank` | Cubase 13.0.30.226 | v1.1 | Mixer Bank（runtimeでsupported / activeならDirectAccessも同runで条件付き取得） | E0 / E1 / E8 / C1 / M1 / R1 / R2。O1は別途許可時のみ | `PENDING` |
| C13.0.50 | `NOT_AVAILABLE` | v1.2 | Mixer Bank / DirectAccess | `NOT_AVAILABLE` | `NOT_AVAILABLE` |
| C15-COMBINED / `c15_combined` | Cubase 15.0.30.287 | v1.3 | Mixer Bank + DirectAccess | E0 / E1 / E8 / C1 / M1 / R1 / R2。O1は別途許可時のみ | `PENDING` |

Cubase 15のMixer BankとDirectAccessは別のCubase起動・collector process・run IDではありません。単一のC15-COMBINED physical runで、各UI操作、5000 ms window、session境界を共有し、同一checkpoint内にDirectAccess snapshotを先に、必要なMixer Bank snapshotを続けて取得します。以下のC15-MB / C15-DA列と節は、その同じraw JSONLから作る2つのprojectionです。一方のprojectionが不完全でも別runの結果で穴埋めしません。C13でもruntime capabilityがDirectAccessをsupported / activeと報告した場合は同じ順序と完全性条件を適用し、結果を条件付きC13 DirectAccess projectionとしてredacted reportへ残します。`supported: false, active: false`だけをunsupportedとして受理し、inactiveやactivation失敗をAPI versionから推測でunsupportedへ丸めません。

各physical runでedition、About表示、実施日時 / timezone、repository commit、probe digest、MixConsole surface / visibility sync、filter、callback window、reconnect deadlineを追記します。両profileともrevision 2のexactly 44 checkpoint IDを1回ずつ必要とします。

### E8 exact-width境界観測

E8はprimary bank幅とeligibleなProject由来channel数がexactly 8の独立fixtureです。最初の`E8` checkpointでprojectを開き、両bank configのfinal snapshotを取得します。続くnavigation各行では操作成功responseから5000 ms以上を観測した後、操作configの明示的bank final snapshotを取得します。C15-COMBINED、およびruntimeでDirectAccessがactiveだったC13 runではDirectAccess final snapshotを先に取得し、その後に必要なbank final snapshotを取得します。最後のsnapshot完了後に追加1000 msのmessage-free quiet periodを置き、同じUI ground truthに対する別projectionとして記録します。

| checkpoint ID | expected UI ground truth | C13-MB | C13-DA conditional | C15-MB projection | C15-DA projection |
| --- | --- | --- | --- | --- | --- |
| `E8` | E8 projectを開き、E8-01〜E8-08の8 Audioだけが存在 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_ALL-B0-reset` | E8-01〜E8-08、8 Audio、全visible | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_ALL-B1-next` | UI inventory不変。hostの境界挙動は未推測 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_ALL-B2-prev` | UI inventory不変 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_ALL-B3-reset` | E8-01〜E8-08、8 Audio、全visible | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_VISIBLE-B0-reset` | E8-01〜E8-08、8 Audio、全visible | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_VISIBLE-B1-next` | UI inventory不変。hostの境界挙動は未推測 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_VISIBLE-B2-prev` | UI inventory不変 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| `E8-MB_CORE_VISIBLE-B3-reset` | E8-01〜E8-08、8 Audio、全visible | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |

same / empty / no callbackのいずれか1つだけをbank終端として扱いません。E8、C1のbank幅超過、C1 final partial bankを合わせ、callbackとsnapshotが重複・欠落なく有限に収束するかを評価します。

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
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B0-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B1-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B2-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B3-extra-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B4-prev` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B5-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B0-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B1-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B2-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B3-extra-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B4-prev` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B5-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

### Scopeと完全性

| case / candidate | expected UI ground truth | observed inclusion / order | missing / duplicate / unknown | result |
| --- | --- | --- | --- | --- |
| E0 Project inventory | 0 Project Track | `PENDING` | `PENDING` | `PENDING` |
| E1 | Audio Track 1本 | `PENDING` | `PENDING` | `PENDING` |
| E8 exact-width | E8-01〜E8-08のAudio 8本 | `PENDING` | `PENDING` | `PENDING` |
| C1 Project inventory | P01〜P20 | `PENDING` | `PENDING` | `PENDING` |
| C1 all core channels | P02〜P20 | `PENDING` | `PENDING` | `PENDING` |
| C1 visible core channels | P02〜P07、P09〜P20 | `PENDING` | `PENDING` | `PENDING` |
| O1 Input / Output / VCA | audit v1では対象外 | `SKIPPED` | `SKIPPED` | `SKIPPED` |

primary Track Probeは`MB_CORE_ALL` / `MB_CORE_VISIBLE`だけを実装します。ambientなInput / Output / VCA titleやIDを未許可でraw JSONLへ取り込まないため、次の`MB_OPTIONAL_*` configは現行buildに存在せず、audit manifest v1も`optional_o1.status = "skipped"`と固定理由`not_separately_authorized`だけを受け付けます。

| future config | channel type | window zone | status |
| --- | --- | --- | --- |
| `MB_OPTIONAL_MAIN` | VCA / Input / Output | unlocked main | future separate profile only |
| `MB_OPTIONAL_LEFT` | VCA / Input / Output | left | future separate profile only |
| `MB_OPTIONAL_RIGHT` | VCA / Input / Output | right | future separate profile only |

O1を将来実装するときは、別途明示的な許可、専用Probe build、capability ID、audit profile、pre-run inventory、cleanup検証を先に追加します。primary Probeへoptional zoneを常時作成したり、core runのmanifestだけを`observed`へ変更したりしません。

`B3-extra-next`が同一snapshotに見えることだけを終端証拠にしません。repeat bank、前値維持、callback欠落を区別できなければ完全列挙は`PENDING`または`INCONCLUSIVE_*`のままです。

## Cubase 15.0.30 / API v1.3 Mixer Bank projection

この節はC15-COMBINED physical runのMixer Bank projectionです。

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
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B0-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B1-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B2-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B3-extra-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B4-prev` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_ALL` | `C1-MB_CORE_ALL-B5-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B0-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B1-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B2-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B3-extra-next` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B4-prev` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| `MB_CORE_VISIBLE` | `C1-MB_CORE_VISIBLE-B5-reset` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

### ScopeとID寿命

| 項目 | 観測値 | result |
| --- | --- | --- |
| E0 / E1 / E8の件数と順序 | `PENDING` | `PENDING` |
| C1のscope / hidden P08 | `PENDING` | `PENDING` |
| 同名P02 / P03のID分離 | `PENDING` | `PENDING` |
| Unicode / 長いtitle | `PENDING` | `PENDING` |
| bank移動後の同一Track ID | `PENDING` | `PENDING` |
| rename後のID | `PENDING` | `PENDING` |
| project切替後のID | `PENDING` | `PENDING` |
| reload / restart後のID | `PENDING` | `PENDING` |

## Cubase 13.0.30 conditional DirectAccess projection

INIT capabilityがDirectAccessを`supported: false, active: false`と報告し、固定のunavailable / incomplete reasonと整合した場合だけ、この節を`UNSUPPORTED_RUNTIME`として固定し、DA snapshotやeventを捏造しません。supported / activeだった場合は同じC13 physical runの全44 checkpointから作る`mix_console_root_children_v1` projectionをここへ記録し、後続のC15詳細表と同じlifecycle、root-child scope、metadata、change callback項目を評価します。supportedだがinactive、activation error、または組合せ不整合はこの表へ`UNSUPPORTED`として記録せずrunを停止します。

| 項目 | 観測値 | result |
| --- | --- | --- |
| runtime capability summary | `PENDING` | `PENDING_CONDITIONAL` |
| activation初期snapshot / ready順 | `PENDING` | `PENDING_CONDITIONAL` |
| E0 / E1 / E8 / C1 root-child scope、child index順、repeat edge | `PENDING` | `PENDING_CONDITIONAL` |
| unique ID / title / type等metadata | `PENDING` | `PENDING_CONDITIONAL` |
| rename / add / delete / visibility / state callback | `PENDING` | `PENDING_CONDITIONAL` |
| reload / restart後の回復とID寿命 | `PENDING` | `PENDING_CONDITIONAL` |
| redacted reportの44 final projections | `PENDING` | `PENDING_CONDITIONAL` |

## Cubase 15.0.30 / API v1.3 DirectAccess projection

この節は同じC15-COMBINED physical runのDirectAccess projectionです。

### Lifecycleとroot-child projection

| 項目 | 観測値 | result |
| --- | --- | --- |
| feature detection結果 | `PENDING` | `PENDING` |
| activate → base object取得順 | `PENDING` | `PENDING` |
| update / object change callback順 | `PENDING` | `PENDING` |
| deactivate後のcallback有無 | `PENDING` | `PENDING` |
| `projection = mix_console_root_children_v1` / `scope_depth = 1` | `PENDING` | `PENDING` |
| `scope_complete = true` / `host_graph_complete = false` / non-authoritative | `PENDING` | `PENDING` |
| E0 root / child count | `PENDING` | `PENDING` |
| E1 root / child count /順序 | `PENDING` | `PENDING` |
| E8 root / child count /順序 | `PENDING` | `PENDING` |
| C1 root-child scopeとchild index順序 | `PENDING` | `PENDING` |
| root / root childの`children_expanded` | `PENDING` | `PENDING` |
| Folder / hidden / Group / FX inclusion（root-child観測範囲のみ） | `PENDING` | `PENDING` |
| Input / Output / VCA inclusion | `PENDING` | `PENDING` |
| root self-reference / shared child reference / invalid child ID | `PENDING` | `PENDING` |
| root-child projectionの収束時間 | `PENDING` | `PENDING` |

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

| 操作 | object change | will-be-removed | root-child / metadata再取得 | seq integrity | result |
| --- | --- | --- | --- | --- | --- |
| rename | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| add | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| delete | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| visibility | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| selection | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |
| mute / solo | `PENDING` | `PENDING` | `PENDING` | `PENDING` | `PENDING` |

## Mutation、project切替、reload、restart

各checkpointはfixtureのM1手順どおりに分離し、selection準備操作をrename / add / deleteと同じwindowへ混ぜません。

| checkpoint | 確認対象 | C13-MB physical run | C13-DA conditional | C15-COMBINED: MB projection | C15-COMBINED: DA projection |
| --- | --- | --- | --- | --- | --- |
| S0 | C1初期snapshot、ID、state | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S1-select | P10単独selection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S1-rename | title callback、ID寿命 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S2-select-anchor | P12単独selection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S2-add | add、自動selection、後続index | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S3-select-delete | P11単独selection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S3-delete | removal、自動selection、後続index | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S4-show | hidden P08をshow、bank / root-child差分 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S5-select-anchor | P12比較元selection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S5-select-change | P12 false / P13 trueの順 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S6-mute | active Solo中のP04 Mute control操作、explicit / effective差と意図しないselection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S7-solo | P03 soloと意図しないselection | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S8-project-only-hide | visibility同期off時のsurface分離 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S8-restore | 同期とP08 hiddenの原状回復 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S9-empty | E0へproject切替 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S9-mutation | M1へ戻る | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| S9-baseline | C1へ戻る | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |

R1 / R2のreconnect deadlineはaction markerからreadyと再discover完了まで既定`30000 ms`です。final snapshotはmarkerから5000 ms以上かつready / discovery後に開始し、それらの完了から`10000 ms`以内に取得します。deadlineを延長して成功したように見せず、変更したrunはprimary comparisonから分離します。

| phase | 確認対象 | C13-MB physical run | C13-DA conditional | C15-COMBINED: MB projection | C15-COMBINED: DA projection |
| --- | --- | --- | --- | --- | --- |
| R1 pre-reload | `S9-baseline` finalを参照 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| R1 reload | clean `probe.ready(false)` / 新IDの`probe.loaded` seq 1 / `probe.mapping_active` / 初期snapshot / `probe.ready(true)`順 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| R1 post-reload | 30秒以内のready、ID、初期callback | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| R2 pre-restart | R1 finalを参照 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| R2 restart | 正常終了、新process、新session境界 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| R2 post-restart | 30秒以内のready、ID、初期callback | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |

## Access方式比較

この表はruntime完了後にだけ更新します。静的なmethod存在だけで「可」にしません。

| 判断項目 | C13 v1.1 Mixer Bank | C13 DirectAccess conditional | C15 v1.3 Mixer Bank | C15 v1.3 DirectAccess |
| --- | --- | --- | --- | --- |
| boundedな0件 / 1件 / 複数bank列挙 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| 終端を重複・欠落なく判定 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| 同名を分離するopaque ID | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| rename中のID一貫性 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| title / Unicode保持 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| typeを推測せず取得 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING`（v1.3だけ） |
| selected / mute / solo初期値 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| delayed / out-of-order callback耐性 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| mutation中snapshotの失効検出 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| hidden / zone / I/O scopeの明確さ | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| Project Track全体 / Folderの完全列挙authority | `PENDING` | `false`（root-child scope固定） | `PENDING` | `false`（root-child scope固定） |
| reload / restart後の回復 | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |
| API feature detectionによる安全なfallback | `PENDING` | `PENDING_CONDITIONAL` | `PENDING` | `PENDING` |

## 推奨方式の判定基準

最終推奨: `PENDING`

Issue #4へ方式を提案するには、候補が公開しようとするscopeについて次のgateをすべて満たす必要があります。DirectAccessの`mix_console_root_children_v1`は`authoritative_for_track_enumeration = false`なので、それ単独をProject全体の`tracks.list`候補としてgate通過させません。Mixer Bankもprimary Probeが明示的にfilterしたcore MixConsole channelだけが候補scopeであり、Project windowのFolderを含む全Track authorityは別の証拠がない限り主張しません。

1. **観測integrity**: callback windowとsequence検査を通過し、drop、duplicate、逆順、orphanがない。
2. **bounded completeness**: Mixer BankではE0、E1、E8のbank幅ちょうど、C1のbank幅超過と最終partial pageについて有限回の操作からcore MixConsole channelの終端を重複・欠落なく判定できる。DirectAccessではrootと宣言された全root childだけを重複・欠落なく取得できる。どちらも未観測surfaceへ完全性を拡張しない。
3. **opaque identity**: 同名Trackを区別でき、name、index、type、routingからIDを合成しない。ID寿命をsession / rename / project / reload / restart単位で明記できる。
4. **truthful metadata**: titleとstateの初期値到着前をreadyにせず、取得不能なtypeやstateを推測しない。
5. **snapshot consistency**: add / rename / delete / visibility / state変更中に異なる時点のTrackを正常な単一snapshotへ混ぜない。変更検知またはgeneration失効を実装できる。
6. **scope determinism**: Project、MixConsole、visibility、window zone、Input / Output / VCAの包含規則と、Project Folder completenessが未証明である境界を明示できる。
7. **lifecycle safety**: activate / deactivate / reload / restartでpending stateを破棄でき、unsupported hostでも既存Transport 6 Toolを壊さない。
8. **version evidence**: 正確なruntime hostで確認した能力だけを対応表へ載せる。Cubase 15 / API v1.3のtype結果をAPI v1.2へ一般化しない。

判定時は次のfail-closed規則を適用します。

- API v1.1 Mixer Bankで完全な終端またはopaque IDを保証できなければ、そのhostで`tracks.list`を成功扱いせずCapabilityを`false`とする案を選ぶ。
- DirectAccessがroot-child scopeのgateを満たす場合もfeature detectionとlifecycle guardを必須にし、`scope_complete = true`をhost graphまたはProject Track全体のcompleteへ読み替えない。
- Mixer Bankでcore MixConsole channelを完全に取得できても、Folderを含むProject Track全体の列挙成功へ読み替えない。Project-wide APIを提供するには別surfaceによるFolder completenessの証拠か、公開scopeをcore MixConsole channelへ限定する明示契約が必要である。
- API v1.2の最低対応versionを主張するには、v1.2公式契約だけで十分な項目と、正確なv1.2 runtime runを必要とする項目を分離する。現在13.0.50 runtimeは`NOT_AVAILABLE`である。
- どの候補も完全性を証明できなければ、推測によるfallbackを追加せず`tracks.list`を未対応のままにする。

## Issue #3完了checklist

- [ ] Cubase 13.0.30とCubase 15について、edition / version / build、MIDI Remote API、OS / build / architectureをruntime runごとに記録した
- [ ] repository commit、collector binary digestを固定し、probe source / installer embedded / deployedの3 digestが一致している
- [ ] fixture revision 2のE0 / E1 / E8 / C1 / M1、R1 / R2を再現し、audit v1のO1は`skipped / not_separately_authorized`にした
- [ ] C13 Mixer Bankのphysical runと、C15 Mixer Bank / DirectAccessを同時取得した単一physical runの完全な観測tableがある
- [ ] 各profileのexactly 44 checkpointについて、cut対象では成功response直後のatomic pairに同request / epochの自動action markerがあり、manual markerを重ねず、その直後にUI操作を行い、種別ごとの観測anchorから5000 ms以上の観測、明示的final snapshot、その後にmessageがない追加1000 ms quiet period、UI annotation、sequence gap判定を適用した
- [ ] input guardの全recordがv3 `consequential_input_only` contractと一致し、inventoryとarmed / resultのID列が一致し、全resultの15 consequential deltaが0である。move-only acceptanceとphysical click / key / scroll / drag rejectionを含むexact-context calibrationも保存した
- [ ] 各UI操作のfresh pre-state、semantic identityまたは明示座標へ束縛したexactly 1 call、action-specific fresh postconditionを照合し、focus / window移動、対象外への作用、postcondition不一致または誤clickの疑いがあるrunをguard成功だけで救済していない
- [ ] redacted reportがraw run IDの`run-<16-hex>` alias、raw / manifest digest、allowlist済みsemantic projection、run-local ID aliasを含み、raw run ID / raw host ID / path / port / unknown titleを含まない
- [ ] 同名、Unicode、長い名前、hidden、Folder、複数type、bank幅超過をUI ground truthと比較した
- [ ] add / rename / delete / selection / mute / solo / visibility / project切替を個別windowで観測した
- [ ] ID寿命、callback順、空slot、bank終端、`mix_console_root_children_v1` scope / repeat edgeを観測値と未確認事項に分けた
- [ ] Mixer BankとDirectAccessの制約を比較した
- [ ] Mixer Bankのauthorityをcore MixConsole channelに限定し、DirectAccessはTrack列挙non-authoritative、Project-wide Folder completenessは未証明と明記した
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
