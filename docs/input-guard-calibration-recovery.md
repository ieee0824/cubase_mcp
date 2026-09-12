# 入力ガード校正の復旧手順

これはTrack API調査用の操作手順であり、本番MCP機能ではありません。[校正matrix](track-api-fixture.md)の9プロセス・15操作、物理入力の確認、freshな前後stateと画像、guardの拒否条件を変更しません。校正だけではCubaseのTrack API対応を証明できません。

## 再試行の単位

`no_retry_within_process`は**同じguardプロセス内で再試行しない**という契約です。別プロセスの失敗で、既に完全に終了した独立プロセスの記録まで自動的に無効にはしません。ただし共通のbinary、操作ツール、起動条件、時刻の信頼性などに問題が判明した場合は、影響する全記録を再評価します。

- 失敗元directoryは失敗のまま保持します。raw JSONL、stderr、trace、capture、既存lockを削除・編集して復旧しません。
- 不完全なプロセスはfresh processと新しい採取先で、そのプロセスの先頭から測り直します。成功したactionだけを切り出して別プロセスのactionと接合しません。
- UI操作は成功していても、guard result、終了記録、実入力確認などが欠けているプロセスは採用しません。過去の入力区間やattestationを推測で補いません。
- 完了済みプロセスは、guard JSONL、stderr、trace、traceが参照する全state / screenshotを一つのbundleとして扱います。

## 完了済みbundleの再利用

1. 採取プロセスが終了していることを実process handle等で確認します。観測timeoutやlockの存在だけで終了と判断しません。
2. 同じguard binary SHA-256と、同じ操作ツール実装・起動権限・表示/fixture条件で得た記録であることを確認します。由来が不明、またはtool update等でcontextが変わった場合は、互換性を推測して再利用しません。guardのSHAだけではexact-context一致を証明できません。
3. bundleごとに元directory、stem、guard identity、全参照fileの相対pathとSHA-256、採用理由をlocal provenance記録へ残します。元directoryの失敗原因がそのbundleにも影響する場合は採用しません。
4. 9つの完全なbundleを、元とは別の新規final directoryへ**byte-for-byteでcopy**します。既存fileを上書きせず、canonical名とtrace内の相対pathを維持します。symlinkやhardlinkは使いません。名前・captureの衝突は修正して通さず、採用元の選択を見直します。
5. 元とcopyのdigest一致を再確認します。provenance、失敗記録、checker出力はfinal directoryの外へ置きます。元の失敗記録は隔離保存したままです。
6. repositoryの`scripts/check-input-guard-calibration.sh`へfinal directory、prefix、guard binary、固定したguard SHAを渡して検証します。9 distinct identity、各プロセス内のno-retry、guard/traceの完全性、時刻、30+30 captureとdigest、closed setなどの既存検査をすべて通すことが必要です。

bundleのコピーは校正の合格ではありません。checkerは操作ツールの実装versionや実際の対象画面を認証しないため、手順2のcontext照合と、実測時の画像・操作固有postcondition確認を省略できません。校正時の取得元が別directoryであることだけを理由に、全9プロセスを再実行する必要はありません。

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

## operatorとの同期と進捗

- 自動操作の準備が済んでから、対象の物理入力controlに対する現在の準備確認を得ます。過去の「準備OK」を新しい入力区間の確認に使いません。
- 各controlの入力内容、開始、終了を明示します。見落としやtiming違いがあれば、そのcontrolのプロセスは未成立として残します。ユーザーの操作ミスと断定しません。
- 「部分確認済みaction数 / 15」「完全bundle数 / 9」「校正全体の合否」と「採用済みCubase実測数 / 2」を分けて表示します。工程数を作業量の割合に換算しません。
- 同じoperator手順または判定コードの問題を検出したらlive再試行を止め、原因をオフラインで再現・修正してから再開します。

Input / Outputを含むIssue #3の元scopeはこの手順では変更しません。primary profileのO1 skip、校正完了、基盤PRのCI成功だけを根拠にIssueをcloseしません。
