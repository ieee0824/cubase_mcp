# 入力ガード校正の復旧手順

これはTrack API調査用の操作手順であり、本番MCP機能ではありません。[校正matrix](track-api-fixture.md)の8プロセス・14操作と決定的なsampling contractテスト、物理入力の確認、freshな前後stateと画像、guardの拒否条件を変更しません。校正だけではCubaseのTrack API対応を証明できません。

## 再試行の単位

### 2026-09-13: 受け入れ設計 v4

ユーザー承認により、偶然の物理入力が短いsample内に重なることを必須とする旧sample-race controlを、決定的な異常系テストへ分離しました。これは旧15項目の最後を実測成功に変更したものではありません。新しい分母は実測14操作 / 8プロセスに、別枠のsampling contractテストを加えたものです。

- 実機: automation、move-only、click/key/scroll/drag、wrong-target、held-stateの各既存条件を維持します。通常のOS入力カウンター取得・held検出・UI postconditionを確認します。
- 決定的テスト: test buildに限りOS readの戻り値を置換し、実際のworker、sampling判定、command処理、error serializationを通します。arm/checkそれぞれで同値、変化、wraparound、key held、button held、raceとheldの優先順位の計12 caseを実行します。エラーコード自体を注入して成功扱いにはしません。
- 境界: OS APIの原子性や、実機sample中の入力競合を再現・保証するテストではありません。runtimeの`INPUT_DURING_SAMPLE`拒否、timeout、held判定は維持し、sleep追加や本番への注入オプションは導入しません。
- 校正reportはv4、operator calibration summaryはv2とします。`sampling_contract.runtime_physical_race_reproduced`は必ずfalseです。旧reportの書き換え、旧race失敗の再ラベル付けはしません。final checkerはcleanな同一commitのsource/lock/runner digestも照合します。
- 既存8 bundleは原本のまま保持します。再利用には従来のexact-context確認とbinaryの一致が必要です。test-only変更でもreleaseの同一性を推測せず、隔離buildとのbyte/digest比較を行います。不一致なら旧guardのbundleを新guardの証拠にはしません。

単独の実行は`bash scripts/check-input-guard-sampling.sh`です。校正checkerが同じrunnerを毎回実行して結果をreportに含めるため、証拠directoryに置いたpass JSONでは代替できません。Cargoの依存は事前取得し、実行時は`--offline --locked`を使います。final checkerでは同じ検証呼出しの隔離target directoryでtest executableも再buildします。Rust toolchainとCargo home configurationは従来どおり信頼境界です。

同日の移行検証では、完了済み8 bundleの80 file（24 process fileと28+28 capture）を別の候補directoryへexclusive copyし、元とcopyのbyte/digest一致、8 distinct identity、時刻、capture、各controlの条件をv4 checkerで一括検証しました。実測14/14と決定的12/12は機械検証済みです。旧race失敗、各bundleの旧report、copy provenance、原本はrepository外に保持しています。

隔離offline release buildと既存の凍結guardはbyte単位で一致し、SHA-256は引き続き`a2d5e521b0df561c6d0a350ea557cf87c2d53c160be2a3e94080ed2418e015c4`です。全Rust target、校正checker、final checkerのcontract、sampling runnerの偽成功拒否、Finder parserの回帰テストを確認しました。実測の成功を、未確認のcontextへ自動で広げません。操作ツール実装・権限・表示条件の最終照合、およびformal runへのclean commit / inventoryの結び付けは別の残件です。Cubase正式実測は0/2であり、この機械検証だけで#35や#3をcloseしません。

### 同日: automationの起動条件差を再採取で解消

旧automationだけがNode parent / pipeだったため、その6操作全体を、他の実測と同じ権限付きshell / PTY・guard起動前3秒待機の独立processで再採取しました。固定Finder open helperはguard起動前に準備した別processであり、guardのparentではありません。captureは継続中のSky clientによるfull AXと同一responseの画像、UI操作はdocumented CUA Finder APIです。release guardは変更していません。

採用候補の新automationは6操作すべてで前後条件が成立し、14連続guard record、各resultの全16 delta 0、正常finish、exit 0、空stderrを確認しました。残り7 bundleと新規directoryで再結合し、80 file・8 distinct identity・実測14/14・決定的12/12をv4 checkerで再検証してexit 0でした。旧automationも原本のまま保持しています。最初の検証用copyではtraceのファイル名が非canonicalでcheckerが拒否したため、copy側の名前だけを修正し、元bytesと名前対応のprovenanceを別途保存しました。

この再採取の前に中止した2 processは未採用です。1つは会話継続をまたいだarmed windowをUI操作前にcancelし、もう1つは2操作目でFinderが以前のpathを復元して、事前に定めた「空欄」という事後条件と不一致になり、clean result保存後にrejectしました。成功actionの切り出し・接合はしていません。新processでは履歴復元を踏まえた前後条件を実行前に固定し、set-valueで別の専用subfolder pathへの変化を確認しました。原本、失敗記録、実行前plan、画像、copy provenance、checker reportはすべてローカルのみ保存します。この結果で解消したのは既知のguard起動元の差であり、exact-context全体の最終採用や#32の根本原因特定を意味しません。

`no_retry_within_process`は**同じguardプロセス内で再試行しない**という契約です。別プロセスの失敗で、既に完全に終了した独立プロセスの記録まで自動的に無効にはしません。ただし共通のbinary、操作ツール、起動条件、時刻の信頼性などに問題が判明した場合は、影響する全記録を再評価します。

- 失敗元directoryは失敗のまま保持します。raw JSONL、stderr、trace、capture、既存lockを削除・編集して復旧しません。
- 不完全なプロセスはfresh processと新しい採取先で、そのプロセスの先頭から測り直します。成功したactionだけを切り出して別プロセスのactionと接合しません。
- UI操作は成功していても、guard result、終了記録、実入力確認などが欠けているプロセスは採用しません。過去の入力区間やattestationを推測で補いません。
- 完了済みプロセスは、guard JSONL、stderr、trace、traceが参照する全state / screenshotを一つのbundleとして扱います。

## 完了済みbundleの再利用

1. 採取プロセスが終了していることを実process handle等で確認します。観測timeoutやlockの存在だけで終了と判断しません。
2. 同じguard binary SHA-256と、同じ操作ツール実装・起動権限・表示/fixture条件で得た記録であることを確認します。由来が不明、またはtool update等でcontextが変わった場合は、互換性を推測して再利用しません。guardのSHAだけではexact-context一致を証明できません。
3. bundleごとに元directory、stem、guard identity、全参照fileの相対pathとSHA-256、採用理由をlocal provenance記録へ残します。元directoryの失敗原因がそのbundleにも影響する場合は採用しません。
4. 8つの完全なbundleを、元とは別の新規final directoryへ**byte-for-byteでcopy**します。既存fileを上書きせず、canonical名とtrace内の相対pathを維持します。symlinkやhardlinkは使いません。名前・captureの衝突は修正して通さず、採用元の選択を見直します。
5. 元とcopyのdigest一致を再確認します。provenance、失敗記録、checker出力はfinal directoryの外へ置きます。元の失敗記録は隔離保存したままです。
6. repositoryの`scripts/check-input-guard-calibration.sh`へfinal directory、prefix、guard binary、固定したguard SHAを渡して検証します。checkerは同じrepositoryの`check-input-guard-sampling.sh`も実行し、自己申告のpass記録では代替しません。8 distinct identity、各プロセス内のno-retry、guard/traceの完全性、時刻、28+28 captureとdigest、closed setなどの既存検査をすべて通すことが必要です。

bundleのコピーは校正の合格ではありません。checkerは操作ツールの実装versionや実際の対象画面を認証しないため、手順2のcontext照合と、実測時の画像・操作固有postcondition確認を省略できません。校正時の取得元が別directoryであることだけを理由に、全8プロセスを再実行する必要はありません。

## UI判定を先にオフライン検証する

`scripts/lib/finder-calibration-state.js`は保存済みの**full** AX textを扱う純粋関数です。GUIやguardを起動せず、stateがfreshであること、画像の一致、操作成功を保証するものでもありません。

対応範囲は観測済みの日本語Finder、tab階層、ListView形式です。英語版等の別AX形式への対応は未確認です。window名やsidebarの翻訳を推測して合わせるのではなく、専用fixtureのexactな名前とURLを渡します。

- `requireScratchWindow(text, windowTitle)`で専用scratchのwindowを確認します。残留GoToWindow等を見つけたら、guard起動前の準備へ戻します。
- `resolveGoToPathField(text, expectedValue)`でGoToWindow内のPathTextFieldを毎回解決します。空値のAX表現に`ID:`がない場合も扱い、空欄と異なる既存値を混同しません。
- `resolveFixtureTarget(text, {windowTitle, filename, expectedUrl})`でclick前の対象を解決します。選択状態は問わず、専用windowのcontent内にあるexactな合成file / folderを要求します。
- `resolveSelectedFixture(text, {windowTitle, filename, expectedUrl})`で専用fixtureの名前、exact URL、content内の選択を照合します。日本語の周辺ラベルに依存せず、sidebarだけの選択を成功扱いしません。

indexは同じ操作のfresh pre-stateから解決し、前回のindexを再利用しません。postは別のfresh captureで確認します。diffやsummaryだけでは判定しません。座標controlではこれに加え、画像から得た座標・window境界・対象固有のpostconditionを確認します。未知のAX形式は成功と推測せず、記録を保存してオフラインのparser/testを修正します。

Desktopやユーザーの通常folderへ移動せず、専用scratchの合成fileとbenign decoyだけを使います。予期しないdialogや判定不一致を調べるために、そのまま追加clickして採取を続けません。

```sh
node tests/finder_calibration_state.test.js
node tests/input_guard_calibration_checker.test.js
```

後者はmacOSの`sips`等を使う既存のsynthetic evidence testです。合格しても実際の物理入力を校正したことにはなりません。controllerへ渡す入力schemaと出力JSONLのschemaも混同しないよう、採取前にpure builderで検証します（例: controller入力の`timing.callStartedAt`と出力の`call_started_at`は別です）。

### Computer Use captureの保存

`scripts/record-cua-capture.js`は、Computer Useが**すでに同一観測で取得した**full AX textとscreenshot bytesを、校正checkerが要求する別々のstate JSONとPNG/JPEGへ保存するローカル補助です。画面を独自に取得せず、UIを操作せず、`screencapture`等の別の画面取得経路も使いません。stdinには次の4 keyだけを持つ1個のJSON objectを渡します。

```json
{
  "app": "Finder",
  "captured_at": "2026-09-12T20:48:00.123+09:00",
  "text": "full AX text from the same fresh Computer Use capture",
  "screenshot_base64": "base64 of that capture's PNG or JPEG bytes"
}
```

呼び出しはabsolute output directoryと、再利用しないASCII capture IDを指定します。

```sh
node scripts/record-cua-capture.js \
  --output-directory /absolute/local/evidence/calibration \
  --capture-id cal.automation.open-pre
```

標準出力の`state_path` / `state_sha256` / `screenshot_path` / `screenshot_sha256`と`captured_at` / `app`を、そのcaptureを参照するoperator traceへそのまま記録します。既存ID、出力directory自身または直下の`states` / `screenshots`のsymbolic link、非canonical base64、PNG/JPEG以外のsignature、範囲外timestamp、またはsize上限超過は拒否します。stdinはJSON解析前に128 MiBで制限し、AX textは4 MiB、画像bytesは64 MiBを上限とします。画像signatureの確認は完全な画像デコードではありません。破損・切断した画像のデコード検証は既存のevidence checkerが担当します。

出力先とその祖先directoryは信頼できるローカル管理下に置き、書込み中に別プロセスから差し替えないでください。この補助は祖先のsymlinkや並行したdirectory差替えを防ぐsandboxではありません。ID衝突や部分書込みが起きたdirectoryを成功bundleとして再利用せず、新しいdirectoryから採取します。

この補助はguard結果、freshness、target binding、physical input、またはpostconditionを証明しません。`arm`後のstate保存はUIへinputを注入しないローカルI/Oとして行い、Computer Useのsingle target-bound callを置き換えたり、別のUI callを追加したりしません。

#### ローカルSDKからの取得・受け渡し

操作経路として`@oai/sky`の利用を許可された環境では、`sky.get_app_state({ app: "Finder", disableDiff: true })`で得た同じresponseの`text`と`screenshot.url`を使用できます。後者は観測済み環境ではローカル画像のfile URLです。URLを再構築せず、返されたURLのbytesを読み、base64として記録器へ渡します。画面やAXの再取得で片方だけを差し替えません。`captured_at`はこの取得call完了時のoperator側時刻を記録します。

このSDKのoption名は`disableDiff`です。`cua_repl`の`disableDiffing`と混同すると差分が返ることがあります。保存前に対象windowと全文の構造を確認してください。記録器も観測済みの`The following is a diff from the previous accessibility tree`で始まる差分を拒否しますが、あらゆるAX形式の完全性を保証するparserではありません。差分から過去stateを合成してfresh full captureの代わりにしません。

2026-09-12に専用Finder scratchで読み取りのみの受け渡しを確認しました。全文AXは`requireScratchWindow`を通り、保存後のtextは取得responseと一致し、JPEGも元bytesと一致しました。画像は71,052 bytes、`sips`でJPEG / 920×672としてデコードできました。保存stateのSHA-256は`7a32b3208ca0cf6560c98ad741db8fdbe304b2fee134d9d4fed79d04d9a132a7`、画像は`bbd08750a64e846850f251418fd6fc709df98adfa798dcf736ade976c144d668`です。差分だった先行captureは別directoryに不採用のまま保持しました。

同日の自動校正は、sandbox内の事前起動が`COUNTER_UNAVAILABLE`で停止した後、権限付きの独立した2回のguard起動がどちらも最初のsampleで`KEY_HELD`を返し、exit status 1 / 空stderrで停止しました。`ready`、`armed`、UI入力へ進んでいません。これはcontrolledな物理押下区間を持つheld-state校正の合格ではなく、誰が何を押したかの判定でもありません。ユーザーへの入力依頼やsynthetic入力による代用はせず、校正全体・Cubase正式runは未完了のままです。

## operatorとの同期と進捗

### Issue #32の無入力起動診断（2026-09-12）

macOS 26.5.1 / build 25F80 / arm64で、権限付きの読み取り専用診断を一度実行しました。CoreGraphicsのcombined-session / HID-system両tableでsample中のaggregate変化なし、通常範囲・拡張範囲のkey押下数0、mouse button押下数0、Caps Lock / modifier flagなしでした。key番号・入力内容・device識別子は出力していません。

続いて同じ未変更のrelease guard（SHA-256 `a2d5e521b0df561c6d0a350ea557cf87c2d53c160be2a3e94080ed2418e015c4`）を、権限付き`exec_command`から直接起動しました。tool sessionで`ready` → `arm` → `check` → `finish`、record sequence 1〜4、exit status 0を確認しました。arm / check sampleはそれぞれUnix-ms `1789218676033` / `1789218680288`で、全16 deltaが0、`interference_detected: false`でした。UIへの入力は行っていません。この診断のterminal出力はformal calibration用のraw bundleではなく、校正合格に算入しません。

SDK headerの`CGEventSourceKeyState` / state IDとRustのFFI型・定数に不一致は見つかりませんでした。Appleは[HID-system tableをhardware event sourceの集約状態](https://developer.apple.com/documentation/coregraphics/cgeventsourcestateid)として説明しています。現在の実装はaggregateがsample中に変化した場合を先に拒否し、その後key stateがtrueなら`KEY_HELD`を返します。今回の成功は「現在は同じbinaryで起動できる」ことの確認であり、過去の押下主体や残留状態の原因を特定するものではありません。

#32の残件は、過去の失敗と今回の成功の条件差・再現条件の切り分けです。恒常的な故障と断定せず、無入力snapshot単独を安全保証にせず、実操作ごとのguard / target / postcondition検証は引き続き必要です。起動条件を調べるためにrelease guardを書き換えたり、キー解放を注入したり、ユーザー操作を依頼したりはしていません。

### 2026-09-13: 起動経路の比較

同じ読み取り専用診断を権限付きで3つの経路から各1回実行しました。直接起動では両OS tableの通常範囲key押下数0、即時のshell pipeでは1、起動前に3秒待ったshell pipeでは0でした。どのsampleもaggregate変化なし、拡張範囲keyとmouse buttonの押下数0、Caps Lock / modifier flagなしです。個別key番号、入力内容、device識別子は記録していません。

続いて同じ凍結guardを権限付きshell / pipe / PTYから起動前3秒待機で1回実行し、`ready → arm → check → finish`、連続4 record、全16 delta 0、exit 0、空stderrを確認しました。UI入力なし、別processの接合なし、raw JSONLと比較結果はローカルだけに保存しています。これはstartup診断であり正式校正の追加bundleではありません。

この比較は起動経路・タイミングへの依存を再観測したもので、ランダム化した同時測定ではありません。ユーザー無操作のattestationも取得していないため、誰が何を押したか、OSのsticky stateか、承認画面の影響かは判定できません。#32の根本原因を特定済みとはせず、必要ならその別調査を続けます。

現時点の運用候補は、必要な権限承認を先に済ませ、sampleの外側で3秒待ってからguardを開始し、実際の`ready`と各`arm` / `check`を必須にする方法です。3秒で必ず安全になるという保証ではありません。held / timeout / race等が返れば停止し、同じrunを待機や自動再起動で救済しません。異なるparent / pipe / tool contextへの互換性は#35で別途照合します。

### 承認と計測開始を分離するlauncher

`scripts/start-input-guard.sh GUARD_BINARY EXPECTED_SHA256 NEW_LAUNCH_LOG`は、権限承認後もガードを即時起動しません。絶対pathの実行可能な通常fileとSHA-256を照合し、別の新規logへ`awaiting_start`を書いて待機します。operatorが承認操作を終えてキー・ボタンを離した後に、stdinへASCIIの`start`と改行を送ります。launcherはsample外で3秒待ち、binaryを再照合してから同じPIDを未変更のguardへ`exec`します。

- launcherの待機通知はguardの`ready`ではありません。実際のguard `ready`を読むまで、`arm`やCubaseの操作を送ってはいけません。
- `start`はoperatorの開始合図であり、キー解放の検出・承認済み状態の認証ではありません。必要な承認はlauncherの起動前に済ませ、armed区間に新しい承認操作が入ったら通常の干渉として扱います。
- launcherのstdoutはguardのraw JSONL用、stderrはguard stderr用に分離して記録できます。launcher自身の起動失敗もstderrへ出るため、その場合は不採用です。launch logはclosed evidence directoryの**外**へ保存し、校正や正式runのraw streamへ混ぜません。
- logは新規作成のみで、既存file・symlinkを拒否します。EOF、不正な開始合図、待機中のbinary差し替えはguard起動前に失敗します。起動後の`KEY_HELD`等の出力とexit statusをそのまま返し、再試行やキー解放注入はしません。
- 新しいlauncherの採用は起動contextの変更として記録します。凍結guardが同一でも、旧校正のexact-contextを自動で採用済みにせず、正式run前の照合を残します。

承認キーを離す前にguardが起動したという仮説は、OS全体のheld-stateを読む実装と整合します。ただし以前の承認方法・キー解放時刻は記録していないため、過去の`KEY_HELD`の原因を特定済みにはしません。承認済みcontrollerから開始した、ユーザー確認付きの無操作診断2回では計84 sampleの押下数が0でしたが、これは以前の承認timingの再現実験ではありません。launcherの自動テストもprocess / stdio / handshakeの検証であり、OS入力の実測や正式校正を代替しません。

2026-09-13のlauncher導入検証では、自動テスト16/16に加え、未変更の凍結guardを使う新規の権限付きshell / PTY processを1回実行しました。承認後の`awaiting_start`時点でguard stdout / stderrが0 byteであること、開始合図後に同じPIDのguardが`ready`を返すことを確認し、UI操作なしで`arm → check → finish`を完了しました。rawは連続4 record、全16 delta 0、空stderr、exit 0でした。この新しい区間についてユーザー無操作の確認は取得していないため、先の84 sampleのattestationを流用しません。rawと別のlaunch logはローカルに保持し、正式runや校正bundleには算入しません。

その後、ユーザーから承認はいつもEnterで確定していると確認を得ました。これは承認方法についての事後申告であり、失敗sample時にEnterが押下中だったという確認ではありません。「承認のEnter key-down後、key-upより先にguardの初回sampleが走った」という起動時入力の競合を具体的な原因候補として扱えますが、当時のkey-upとsampleの対応時刻はなく、因果関係は未確定です。過去のrawや無操作区間のattestationを変更せず、guardの誤判定・OSのsticky state・ユーザーの指示違反を確定したものとも扱いません。

launcher変更`14e5a7d`のCI run `34726582682`はLinux / macOS / Windowsすべて成功しました。launcherのhandshakeテストはLinux / macOSで実施し、Windowsで同じshell起動経路を検証したという意味ではありません。旧`c955405`へ結び付けたrun前inventory・build記録はそのcommitの準備として保持し、新しいcommitやlauncher contextへ無条件に流用しません。

- 自動操作の準備が済んでから、対象の物理入力controlに対する現在の準備確認を得ます。過去の「準備OK」を新しい入力区間の確認に使いません。
- 各controlの入力内容、開始、終了を明示します。見落としやtiming違いがあれば、そのcontrolのプロセスは未成立として残します。ユーザーの操作ミスと断定しません。
- 「部分確認済み実測action数 / 14」「完全bundle数 / 8」「決定的テスト結果」「校正全体の合否」と「採用済みCubase実測数 / 2」を分けて表示します。工程数を作業量の割合に換算しません。
- 同じoperator手順または判定コードの問題を検出したらlive再試行を止め、原因をオフラインで再現・修正してから再開します。

Input / Outputを含むIssue #3の元scopeはこの手順では変更しません。primary profileのO1 skip、校正完了、基盤PRのCI成功だけを根拠にIssueをcloseしません。
