# 入力ガード・証跡保存・校正の復旧手順

この手順は[分割計画A / Issue #41](https://github.com/ieee0824/cubase_mcp/issues/41)の独立した調査補助を対象にします。本番MCP Toolから自動起動されません。Track Probe / collector / auditorとの正式run統合と証跡checkerは[Track fixture](track-api-fixture.md)を参照してください。I/O調査基盤は後続の[計画C](https://github.com/ieee0824/cubase_mcp/issues/43)で扱います。

過去の実測校正採用は[Issue #35](https://github.com/ieee0824/cubase_mcp/issues/35)に記録されています。このコード抽出やCI合格は、新しい操作サービス・権限・起動contextでの校正合格を意味しません。既存原本の所在と再利用可否は[Issue #44](https://github.com/ieee0824/cubase_mcp/issues/44)、起動時KEY_HELDの未確定原因は[Issue #38](https://github.com/ieee0824/cubase_mcp/issues/38)で追跡します。

## ガードの範囲

`cubase_input_guard`はmacOSのCoreGraphics HID-system tableから、入力種別ごとの累積countと通常key / mouse buttonのheld booleanを読みます。入力文字、個別key番号、pointer座標、対象applicationを記録しません。Linux / Windowsの実入力観測は未対応で、起動はエラーで終了します。各platformの単体テストはOS readを置換したロジック検証です。

protocolはversion 5、`source = hid_system_state`、`coverage = action_windows`、`privacy = counts_and_held_state_boolean`、`policy = consequential_input_only`です。session ID / process ID / 開始時刻と連続record sequenceは記録の対応付けに使い、actor認証や改ざん耐性を与える署名ではありません。

`arm`と`check`の間ではmouse_moved単独を干渉として扱いません。残る15 field（mouse down/up/drag、key down/up、modifier、scroll、tablet）の増加は干渉としてlatchします。counterはwraparound差分を使います。OS readの短いsample中にaggregateが変化した場合は、mouse moveだけでも`INPUT_DURING_SAMPLE`で停止します。通常key / mouse buttonが押下中、counter取得が2秒以内に完了しない場合も拒否します。

ガードはUI callをinterceptせず、click座標・target・操作結果を検査しません。通知、application自身のfocus変更、一部hardware keyやremote controlを完全検出するものでもありません。freshな前後画面、対象へ束縛したcall、操作固有のpostconditionを別に確認します。

## 起動と1操作の順序

```sh
cargo build --release --locked --bin cubase_input_guard
shasum -a 256 target/release/cubase_input_guard
bash scripts/start-input-guard.sh /absolute/path/cubase_input_guard EXPECTED_SHA256 /absolute/private/new-launch.jsonl
```

上のpathとdigestは対象binaryから確定します。保存先はrepository外の継続保存領域を使用し、原本・画像・provenanceを公開しません。必要なOS / tool承認をlauncher起動前に済ませます。launcherはdigest確認後に新規logへ`awaiting_start`を書き、stdinからASCIIの`start`と改行を待ちます。sample外で3秒待ち、digest再確認後に同じPIDでguardへexecします。stdout / stderrはguardの記録用、launch logは校正directory外に保存します。

`start`や3秒待機はキー解放の認証ではありません。実際の`ready`を確認するまでUIを操作しません。起動拒否を待機・キー解放注入・自動再起動で成功へ変更しません。

1. `arm`を送り、同じaction IDの`armed`を確認する。
2. 対象application / window / controlをfreshな全文AXと画像から解決する。
3. 対象に束縛したUI callをexactly 1回行う。座標操作はwindow範囲とclick countを同じcallへ束縛する。
4. freshな全文AXと画像を取得し、操作固有postconditionを確認する。
5. 同じIDで`check`を送り、`result`を保存する。
6. UI call前に中止した場合は`cancel`、UI実行後にpostconditionが成立しなかった場合はcheck後のclean resultへ`reject`する。干渉resultはlatch自体が継続を拒否する。
7. 全操作が終了したら`finish`を送り、`finished`とexit 0を確認する。

```json
{"command":"arm","action_id":"scratch.select"}
{"command":"check","action_id":"scratch.select"}
{"command":"finish"}
```

各JSONは応答・観測を待って個別に送ります。cancel / reject / protocol error / sampling error / 干渉 / finishなしのEOFは成功として採用しません。guardのJSONLには補足ログを混ぜません。action IDは1〜128 bytesのASCII英数字・dot・underscore・hyphen、commandは最大512 bytesです。idle区間は次のarmへ繰り越さず、ping / finishもidle差分を検査しません。

## 実測校正matrix

同じbinary、操作ツール実装、権限、parent / stdio、表示・fixture条件で校正します。専用Finder scratchと合成file / benign decoyを使用します。application名は`Finder`、openのbundle pathは`/System/Library/CoreServices/Finder.app`です。物理入力は個別の準備確認と開始・終了合図で行い、synthetic入力で代替しません。

| process suffix | 操作数 | 必須確認 |
| --- | ---: | --- |
| automation | 6 | exact document open、shortcut、set_value、semantic click、single / double coordinate click。各15 consequential deltaが0、各postconditionが成立 |
| move | 2 | semantic / coordinateの両方で物理mouse moveが1以上、他15 deltaが0、正しい対象へ作用 |
| wrong-target | 1 | 意図的に誤った有効座標で別項目をclick。clean result後にpostcondition_failedとしてreject |
| positive-click | 1 | 物理clickを検出しinterferenceをlatch |
| positive-key | 1 | 物理key入力を検出しinterferenceをlatch |
| positive-scroll | 1 | 物理scrollを検出しinterferenceをlatch |
| positive-drag | 1 | 物理dragを検出しinterferenceをlatch |
| held-state-rejection | 1 | arm sample全体でkey / mouse buttonを保持しKEY_HELD / MOUSE_BUTTON_HELD。armedに進まない |

8個の独立processで14操作です。許可controlと意図的拒否controlを同じlatched processへ継ぎ足しません。held controlの物理入力区間は失敗sampleの開始〜終了を包含する必要があります。

決定的sampling contractは実測matrixとは別です。test buildでOS readだけを置換し、実際のsampler / command / error処理を通す12 case（arm/check両方の同値・変化・wraparound・key held・button held・race優先順位）を検証します。実機raceの再現や発生確率、OS readの原子性を証明しません。

## 証跡の保存

`scripts/record-cua-capture.js`は既に取得した同一観測の全文AXとPNG/JPEG bytesを保存するローカル補助です。UI操作や画面取得APIを呼びません。stdinへ次の4 fieldのJSONを渡します。

```json
{
  "app": "Finder",
  "captured_at": "2026-09-19T10:00:00.123+09:00",
  "text": "full AX text from the same fresh observation",
  "screenshot_base64": "base64 of that observation's PNG or JPEG bytes"
}
```

```sh
node scripts/record-cua-capture.js --output-directory /absolute/private/calibration --capture-id cal.automation.open-pre
```

stdoutの相対path / SHA-256 / app / captured_atをoperator traceへそのまま記録します。出力directoryは事前に用意し、`states/`と`screenshots/`は保存器が作成します。既存ID・直下directoryのsymlink・非canonical base64・不正timestamp・画像signature不一致を拒否します。stdin上限は128 MiB、全文AXは4 MiB、画像は64 MiBです。既知のAX差分prefixも拒否しますが、全形式のAX完全性を保証するparserではありません。

保存器のsignature確認は完全な画像デコードではありません。校正checkerがmacOSの`sips`で画像形式と正のpixel寸法を検査します。片方だけ保存された場合は成功として扱わず、新規directoryへ採り直します。出力先と祖先directoryは信頼できるローカル管理下に置き、書込み中に差し替えないでください。祖先symlinkや並行directory差替えを防ぐsandboxではありません。

校正directoryのclosed setは、各suffixにつき`PREFIX-SUFFIX.jsonl`、`PREFIX-SUFFIX.stderr`、`PREFIX-SUFFIX-trace.jsonl`の24 process fileと、28個ずつのstate JSON / screenshotです。stderrは空であることを確認します。失敗記録・provenance・checker report・launch logは別directoryへ保存します。

traceは各captureの相対path、digest、同一取得時刻を持ち、`pre < call started <= call ended < post`とguardのsample / record順序を照合します。8つのguard identity、sequence、期待delta、physical-input確認、target binding、全fileのhashと画像decodeをcheckerが検査します。実際の画面内容と操作ツールcontextの確認はoperatorの責務です。共通system wall clockを信頼境界とし、署名された時刻証明として扱いません。

## checkerと復旧

```sh
bash scripts/check-input-guard-sampling.sh
bash scripts/check-input-guard-calibration.sh /absolute/private/calibration PREFIX /absolute/path/cubase_input_guard EXPECTED_SHA256
```

repository側のcheckerを実行します。必要なコマンドはBash、Node.js（テスト・保存器）、Cargo、jq、shasum、macOS sipsと標準Unix utilityです。Cargo依存を先に取得し、sampling runnerは`--offline --locked`でテストします。Rust toolchainとCargo home設定は信頼する環境です。checkerはrunnerを実行して結果を校正report v4へ含め、自己申告のpassed JSONでは代替しません。

失敗したprocessは新規process・新規保存先で先頭から再測定します。原本のraw / trace / captureを編集せず、成功actionだけを切り出して接合しません。完成済み独立processは、原本とcontextが確認できた場合に限り再利用できます。

1. 採取processの終了とbundle完全性を確認する。
2. binary SHA-256・操作ツール・権限・起動・fixture条件を照合する。SHAだけから互換性を推測しない。
3. 原本directory、stem、identity、相対pathとdigest、採用理由をlocal provenanceへ記録する。
4. 8つの完全bundleを新規directoryへbyte-for-byte copyする。canonical名とtraceの相対pathを保持し、衝突や不足を編集で補わない。
5. 原本とcopyのdigest一致を確認し、上記checkerを実行する。
6. reportのvalidに加えて、保存画面・物理入力確認・操作固有postcondition・contextを確認する。

原本が見つからない場合は採用済みというIssue記録だけで代替しません。共通binaryや時刻等の問題が見つかった場合は、影響する完成済みbundleも再評価します。

## オフライン検証

`scripts/lib/finder-calibration-state.js`は保存済み全文AXの純粋parserです。観測済み日本語Finderのtab階層・ListViewを扱い、専用window、GoToWindowのPathTextField、content内のexact file / URLと選択を確認します。未知形式は拒否し、sidebarの選択をtarget成功に読み替えません。freshnessや画像一致は別に確認します。

```sh
cargo test --locked --bin cubase_input_guard
node tests/finder_calibration_state.test.js
node --test tests/cua_capture_recorder.test.js
node --test tests/input_guard_launcher.test.js
node --test tests/input_guard_sampling_checker.test.js
node tests/input_guard_calibration_checker.test.js
```

launcher試験はPOSIX環境、校正checker試験はmacOSで実施します。これらの試験は模擬データと置換したOS readを使い、ユーザー入力の採取やCubase操作を行いません。正式Track runへの採用、実機再校正、Input / Outputの観測はそれぞれの後続Issueで確認します。
