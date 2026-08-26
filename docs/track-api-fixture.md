# Track API実機検証fixture

この文書は、Track列挙の実機調査と受け入れテストで同じCubase project状態を再現するための手順を定義します。`.cpr`、audio、MIDI file、presetはrepositoryへ追加しません。fixtureは各検証者が専用の一時projectとして作成します。

主な利用先は[Issue #3](https://github.com/ieee0824/cubase_mcp/issues/3)のhost API調査と[Issue #22](https://github.com/ieee0824/cubase_mcp/issues/22)の受け入れテストです。列挙方式は[Issue #4](https://github.com/ieee0824/cubase_mcp/issues/4)、DTO / ID / pagination契約は[Issue #5](https://github.com/ieee0824/cubase_mcp/issues/5)で決定し、全体の依存関係は[Issue #24](https://github.com/ieee0824/cubase_mcp/issues/24)で管理します。

fixture revision: `1`

## この文書で固定するもの

- Project window上のTrack種別、名前、上からの順序
- 明示的なselected / mute / solo / visibility状態
- rename / add / delete / project切替の操作順
- 実機観測時に記録する環境情報とログ項目

次の事項は、このfixtureでは決定しません。

- `cubase.get_tracks`がProject全体、MixConsole channel、visible channelのどれを列挙するか
- Folder、Input、Output、VCAを公開結果へ含めるか
- hostが返すopaque IDの形式と寿命
- Track `type`、pagination、snapshot generationのBridge / MCP契約
- Mixer BankとDirectAccessのどちらを採用するか

これらは実機調査の結果を使ってIssue #4と#5で決定します。この文書の`P01`等はfixture内の行ラベルであり、Track IDではありません。

## 安全条件

開始前に次をすべて満たしてください。

1. 編集中の通常projectを保存して閉じる。
2. Cubaseを1 instanceだけ起動する。
3. 新規の空projectと専用の一時directoryを使う。既存projectをfixtureへ転用しない。
4. audio / MIDI / video fileをimportせず、event、part、automationを書かない。
5. Audio / Instrument / MIDI TrackのRecord EnableとMonitorをすべてoffにする。
6. input / output routingは`No Bus`または`Not Connected`を優先する。実機入力をmonitorしない。
7. TransportのPlay / Recordを実行しない。Track列挙調査に再生や録音は不要。
8. user preset、third-party plug-in、個人名、顧客名、通常project由来の名前を使わない。
9. baseline保存後の変更は必ず別名保存したcopyで行い、baselineを上書きしない。
10. repositoryのcommitと、実際にCubaseへ配備したprobe scriptのSHA-256を照合する。古い配備scriptで実行しない。

専用directory名の例:

```text
CMCP_TrackFixture_r1_<YYYYMMDD>
```

このdirectoryと`.cpr`はlocal artifactです。Gitへ追加しないでください。Gitへcommitする結果や共有ログにはこの文書の合成Track名だけを含め、absolute path、raw MIDI device名、audio data、credentialは記録しません。原状回復にraw device名が必要なO1のpre-run inventoryだけはlocal artifactとして保持し、共有しません。

repositoryの`.gitignore`はCubase projectやaudio artifactを包括的には除外しません。`.cpr`、`.bak`、`Audio/`、`Edits/`、`Images/`、autosave、raw SysEx / JSON logをstage前に手動で確認してください。

## 検証matrix

各runの開始時に次を記録します。versionはAbout画面等に表示される省略なしの値を使います。

| 項目 | 記録例 | 必須 |
| --- | --- | --- |
| run ID | `mac-c13-r1-001` | yes |
| fixture revision | `1` | yes |
| 実施日時とtimezone | `2026-08-26T22:00:00+09:00` | yes |
| OS / build / architecture | `macOS 26.5.1 / build ... / arm64` | yes |
| Cubase edition / version / build | `Cubase Pro 13.0.30.226` | yes |
| MIDI Remote API | `1.1` | yes |
| repository commit | 40桁のGit commit SHA | yes |
| probe source / deployed SHA-256 | 2つのdigest | yes |
| daemon version / build | `cubase_mcp 0.1.0 / commit ...` | 使用時 |
| Bridge設定 | mode、標準port roleまたはredacted alias、address、timeout | 使用時 |
| Track access方式 | `MixerBankZone` / `DirectAccess` | yes |
| probe bank幅 | `8` | Mixer Bank runのみ |
| Mixer type / window-zone filter、followVisibility | include/exclude設定、true / false | Mixer Bank runのみ |
| explicit main-zone filter capability | `available` / `not available` | Mixer Bank runのみ |
| MixConsole観測surface | `lower-zone` / `separate` | yes |
| Project / MixConsole visibility同期 | `automatic` / `on` / `off` / `n/a` / `unsupported` | yes |
| run前のseparate MixConsole同期 | `on` / `off` / `not open` | yes |
| callback観測window | `5000 ms` | yes |
| reload / reconnect deadline | `30000 ms` | R1 / R2 |
| Instrument / Effectの代替plug-in | 名前または`none` | 該当時 |
| optional caseの省略理由 | edition非対応等 | 該当時 |

最低限Cubase 13.0.30で実施し、利用できる場合は13.0.50以降の正確なbuildでも同じ手順を実施します。13.0.50はMIDI Remote API 1.2導入境界ですが、現在の調査環境にはinstallされていません。Cubase 15等のrunは補足情報であり、Issue #4が決定するまでは13.0.50以降のrunを代替した扱いにしません。未installまたは未確認のversionを「検証済み」と記載せず、`not available`として残してください。

Cubase versionごとにfixtureを新規作成します。新しいCubaseで保存した`.cpr`をCubase 13で開いて使い回してはいけません。

## 作成するlocal project

| case | local file名 | 用途 |
| --- | --- | --- |
| E0 | `CMCP_TrackFixture_Empty.cpr` | project Trackが0本の状態 |
| E1 | `CMCP_TrackFixture_One.cpr` | 通常Audio Trackが1本だけの状態 |
| C1 | `CMCP_TrackFixture_Core_Baseline.cpr` | 変更前の基準状態 |
| M1 | `CMCP_TrackFixture_Mutation.cpr` | rename / add / delete用copy |
| O1 | `CMCP_TrackFixture_Optional_IO_VCA.cpr` | Input / Output / VCAの任意調査 |

## 共通のTrack追加方法

Project windowのTrack listにある`Add Track`を使い、Track type、name、countを指定して`Add Track`を実行します。Trackは選択中Trackの直下へ追加されるため、作成後に表の順番へ並べ替えてください。

- countは常に`1`にする。
- Track presetは使わない。
- Audio / Group / Effect（FX channel）はStereoにする。
- Group / Effect / VCAに専用folderの選択肢がある場合は、project直下へ作成する設定を選ぶ。
- FolderにGroup Channelの選択肢がある場合はoffにし、通常の空Folderとして作成する。
- Instrumentは`No VST Instrument`を選べる場合はそれを使う。作成時に必須の場合だけ同梱Steinberg instrumentを使い、作成後に未割当へ戻せる場合は戻す。残る場合はdisable / bypassして名前をrun情報へ記録する。
- Effect Trackは`No Effect`を選べる場合はそれを使う。作成時に必須の場合だけ同梱Steinberg effectを使い、作成後にinsertから外す。外せない場合はbypassして名前を記録する。
- routingは列挙条件ではないため、可能な限り`No Bus` / `Not Connected`にする。

Cubase 13.0.30と後続versionでラベルの配置が異なる場合も、global `Add Track` dialog内の同じTrack typeを使用します。存在しないtypeを別typeで代用しません。

## Case E0: 空project

1. Hubからtemplateを使わず空projectを作成する。
2. Project windowのTrack listが0本であることを確認する。
3. Record Enable、Monitor、Transport Recordがactiveでないことを確認する。
4. `CMCP_TrackFixture_Empty.cpr`として保存する。
5. Project windowとMixConsoleを別々に観測する。

UI上の基準値:

| surface | 基準 |
| --- | --- |
| Project window Track list | 0本 |
| MixConsole | audio connection由来のInput / Output等が存在してもよい。Project Trackとは別に記録する |
| host access | 返却対象と順序は未決定。観測値をそのまま記録する |

空projectでInput / Output channelが見えることを、Project Trackが存在する証拠として扱わないでください。

## Case E1: 1 Track project

1. E0を開き、すぐに`CMCP_TrackFixture_One.cpr`として別名保存する。
2. Stereo Audio Track `CMCP_E1_ONLY_AUDIO`を1本だけ追加する。
3. routingを`No Bus` / `Not Connected`にし、Record Enable、Monitor、Mute、Soloをoffにする。
4. `CMCP_E1_ONLY_AUDIO`だけを選択して保存する。
5. Project windowにこの1本以外のTrackがないことを確認する。

UI上の基準値:

| surface | 基準 |
| --- | --- |
| Project window Track list | `CMCP_E1_ONLY_AUDIO`の1本 |
| Project上からの順序 | 1番目 |
| selected / mute / solo | true / false / false |
| visibility | Projectと同期対象MixConsoleでvisible |
| host access | inclusion、ID、type、順序は観測し、推測しない |

## Case C1: core baseline

### 正確なTrack inventory

1. E0を開き、Project windowのTrack listが0本であることを再確認する。
2. Trackを追加する前に`CMCP_TrackFixture_Core_Baseline.cpr`として別名保存する。
3. 次の20本だけを作成し、Project windowで上から`P01`から`P20`の順へ並べる。全Trackをproject直下へ置き、`P01` Folderは空のままにする。

`P09`の`<LONG_NAME>`は次の117 ASCII文字です。省略せずpasteしてください。

```text
CMCP_09_LONG_ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ
```

`P05`の`é`は単一code point `U+00E9`（NFC）です。要求名は15 Unicode scalar values / 25 UTF-8 bytesです。

| label | Track type | requested Track name | selected | mute | solo | Project visibility | sync対象MixConsole visibility |
| --- | --- | --- | --- | --- | --- | --- | --- |
| P01 | Folder | `CMCP_01_FOLDER_EMPTY` | false | false | false | visible | n/a |
| P02 | Audio | `CMCP_DUPLICATE` | false | true | false | visible | visible |
| P03 | Audio | `CMCP_DUPLICATE` | false | false | true | visible | visible |
| P04 | MIDI | `CMCP_04_MIDI_ASCII` | false | false | false | visible | visible |
| P05 | Instrument | `CMCP_05_日本語_é_🎹` | false | false | false | visible | visible |
| P06 | Group | `CMCP_06_GROUP` | false | false | false | visible | visible |
| P07 | Effect (FX channel) | `CMCP_07_FX` | false | false | false | visible | visible |
| P08 | Audio | `CMCP_08_HIDDEN` | false | false | false | hidden | hidden |
| P09 | Audio | `<LONG_NAME>` | false | false | false | visible | visible |
| P10 | Audio | `CMCP_10_MUTATE_RENAME` | false | false | false | visible | visible |
| P11 | Audio | `CMCP_11_MUTATE_DELETE` | false | false | false | visible | visible |
| P12 | Audio | `CMCP_12_MUTATION_ANCHOR` | false | false | false | visible | visible |
| P13 | Audio | `CMCP_13_STATE_S0_M0_SO0` | false | false | false | visible | visible |
| P14 | Audio | `CMCP_14_STATE_S0_M0_SO1` | false | false | true | visible | visible |
| P15 | Audio | `CMCP_15_STATE_S0_M1_SO0` | false | true | false | visible | visible |
| P16 | Audio | `CMCP_16_STATE_S0_M1_SO1` | false | true | true | visible | visible |
| P17 | Audio | `CMCP_17_STATE_S1_M0_SO0` | true | false | false | visible | visible |
| P18 | Audio | `CMCP_18_STATE_S1_M0_SO1` | true | false | true | visible | visible |
| P19 | Audio | `CMCP_19_STATE_S1_M1_SO0` | true | true | false | visible | visible |
| P20 | Audio | `CMCP_20_STATE_S1_M1_SO1` | true | true | true | visible | visible |

P13〜P20はselected / mute / soloの8通りを1回ずつ表します。すべてのRecord Enable、Monitor、Automation Writeはoffです。P02とP03は同名ですが、上側のP02だけをMute、下側のP03だけをSoloにします。状態確認で名前だけを使わず、Project順と明示control値を併記してください。

Track作成後、Cubase UIが実際に受理したP05/P09の文字列、Unicode scalar数、UTF-8 byte数をrun記録へ転記します。truncationやUnicode normalizationが起きた場合は`SETUP_VARIANCE`として要求値と実値を両方残し、そのrunのUI ground truthには実値を使います。文字列を黙って補正したり、host結果を要求値へ置換したりしません。

### baseline状態の設定

1. 作成したTrackを表の順番へ並べる。
2. globalのDeactivate All Mute StatesとDeactivate All Solo Statesを実行する。
3. P02、P15、P16、P19、P20のMuteだけをonにする。
4. P03、P14、P16、P18、P20のSoloだけをonにする。
5. P17を単独選択し、Cmd/Ctrl-clickでP18、P19、P20の順に追加選択する。P17〜P20だけがselectedで、P20が最後に選択したTrackであることを確認する。
6. Project windowのTrack Visibilityで全Trackを表示する。
7. 観測surfaceを記録する。lower-zone MixConsoleはProject visibilityへ自動追従するため同期設定を`automatic`と記録する。separate MixConsoleでは`Sync Visibility of Project and MixConsole`をonにし、同期設定を`on`と記録する。
8. MixConsoleのZones表示でP02〜P20をleft / rightへlockせず、中央のscrolling fader sectionへ置く。P01 FolderはMixer Bank channelではないため対象外とする。
9. Track VisibilityからP08だけをhideする。
10. P08がVisibility listには残り、Project windowと対象MixConsoleでは非表示であることを確認する。separate MixConsoleで同期toggleを利用できない場合はvisibility-sync subcaseだけを`UNSUPPORTED`と記録し、main runはProject-onlyの観測として続行する。lower-zoneにはtoggleがないため、`UNSUPPORTED`ではなく`automatic`とする。
11. `CMCP_TrackFixture_Core_Baseline.cpr`を保存して閉じ、再度開いて状態が復元されることを確認する。

Soloによって他channelが聴感上muteされても、実機ログでは表の明示的なMute / Solo control値とeffective audio stateを混同しません。

### bank幅を超える条件

Mixer Bank調査ではprobeのbank幅を`8` slotに固定します。

| UI集合 | 個数 | 意味 |
| --- | ---: | --- |
| Project window Track | 20 | P01からP20 |
| channelを持つcore entry | 19 | P02からP20。hostの実際の対象scopeとは限らない |
| synchronized visibilityで表示されるcore channel | 18 | P08を除くP02からP20 |

どの候補scopeでも8 slotを超えるためPrev / Next / Resetと複数bankを観測できます。Input / Output等が追加される場合やFolderの扱いによって最終bankの件数は変わるため、終端を件数から推測せずcallbackとslot状態を記録します。

Mixer Bank runでは次の2設定だけをprimary comparisonに使います。これはprobe用設定であり、Issue #4の製品scope決定ではありません。

| config ID | width | include type | exclude type | unlocked / main scope | left zone | right zone | followVisibility |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| `MB_CORE_ALL` | 8 | Audio、Instrument、MIDI、Group、FX | Sampler、VCA、Input、Output | API 1.1ではimplicit、対応hostではinclude | exclude | exclude | false |
| `MB_CORE_VISIBLE` | 8 | Audio、Instrument、MIDI、Group、FX | Sampler、VCA、Input、Output | API 1.1ではimplicit、対応hostではinclude | exclude | exclude | true |

FolderにはMixer Bankのinclude対象がないため、P01はこの候補countへ含めません。probe実装で上表の各channel typeとleft / right zoneを明示的にinclude / excludeし、default type filter値へ依存しないでください。

Cubase 13.0.30のMIDI Remote API 1.1にはexplicit main-zone filterがありません。両primary configは`excludeWindowZoneLeftChannels()`と`excludeWindowZoneRightChannels()`でleft / rightを明示的に除外し、lockされていない中央のscrolling fader sectionをimplicit対象にします。`includeWindowZoneMainChannels()`相当のmethodはfeature detectionで存在を確認できたhostだけで追加し、API 1.1では呼び出しません。capabilityと実際の呼出値をrun情報へ記録します。追加のfilter実験は別config IDで記録し、primary comparisonへ混ぜません。

同じC1から次の固定sequenceを1回ずつ実施します。

1. configと実際に呼び出したfilter値が上表に一致することを記録する。
2. Resetを実行し、8 slotを`B0-reset`として記録する。
3. Nextを1回ずつ実行して`B1-next`、`B2-next`、さらに終端確認用の`B3-extra-next`を記録する。
4. Prevを1回実行して`B4-prev`を記録する。
5. Resetを実行して`B5-reset`を記録する。
6. `MB_CORE_ALL`と`MB_CORE_VISIBLE`で同じsequenceを繰り返す。

各snapshotはslot 1〜8を省略せず、title / ID / selected / mute / solo callbackの到着順と、空slotが無通知・空文字・null・前値維持のどれに見えるかを記録します。`B3-extra-next`が同じに見えても、それだけで終端や完全性を証明した扱いにはしません。sequenceは固定回数で終了し、各操作後は後述のcallback観測windowを省略せず適用します。

| snapshot | config ID | slot 1 | slot 2 | slot 3 | slot 4 | slot 5 | slot 6 | slot 7 | slot 8 | callback count | result |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | ---: | --- |
| B0-reset | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B1-next | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B2-next | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B3-extra-next | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B4-prev | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B5-reset | `MB_CORE_ALL` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B0-reset | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B1-next | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B2-next | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B3-extra-next | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B4-prev | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |
| B5-reset | `MB_CORE_VISIBLE` | observed | observed | observed | observed | observed | observed | observed | observed | observed | OBSERVED / INCONCLUSIVE |

## UI ground truthとhost観測の分離

fixtureの正解は次のUI ground truthです。host APIの結果がどの候補に一致するかをIssue #3で観測します。

| candidate | 順序とscope | hidden P08 | 用途 |
| --- | --- | --- | --- |
| Project inventory | P01〜P20のProject上からの順 | 含む | fixture自体の正解 |
| all core channels | P02〜P20のProject上からの順 | 含む | visibility非追従Mixer候補 |
| visible core channels | P02〜P07、P09〜P20の順 | 除外 | visibility追従Mixer候補 |
| Input / Output / VCA | coreとは別表で順序を記録 | n/a | optional zone候補 |

観測時は次を守ります。

- Folderやhidden Trackが返らないことだけで不具合と判定しない。
- APIが返さない`type`を名前やroutingから推測しない。
- P02/P03を同じTrackとしてまとめない。
- host IDをfixture labelへ置換せず、対応関係をrun単位で記録する。
- IDが空、重複、変更された場合も補正せず観測結果として残す。
- callbackが欠けた場合にpolling結果からcallbackを捏造しない。

## Callback観測window

bank操作とmutation操作は、操作直前から連続してcallbackを記録し、UIで期待状態を確認した後も操作開始から`5000 ms`まで観測を続けます。1秒のquiet periodへ早く到達してもwindowを短縮しません。window終了時にUI / host snapshotを採取し、次の条件をすべて満たした場合だけ次の操作へ進みます。

- window最後の1000 msに、probe streamのcallbackがない。
- UIの期待状態を確認でき、利用中access方式がsnapshotまたはready状態を提供する場合はそれも取得できた。
- 前stepのwindow終了後に遅延callbackを検出していない。

期限までcallbackが続く、必要な状態を取得できない、またはwindow終了後から次のUI操作前までにcallbackを検出した場合は`INCONCLUSIVE_CALLBACK_TIMEOUT`とします。遅延callbackは`orphan_after_<step>`として記録し、次のUI操作を行わず、そのsequenceを停止します。callbackが来ないことやboolean値を推測で補いません。異なるwindowを使うrunは、値と理由を記録し、primary comparisonから分離します。

## Case M1: mutation sequence

1. C1 baselineを開く。
2. すぐに`CMCP_TrackFixture_Mutation.cpr`として別名保存する。
3. probeを開始し、安定した初期snapshotを`S0`として記録する。
4. P10だけを選択し、準備操作のselection差分を`S1-select`として記録する。
5. P10を`CMCP_10_RENAMED_変更後`へrenameし、`S1-rename`を記録する。
6. P12だけを選択し、`S2-select-anchor`を記録する。
7. Audio Track `CMCP_21_ADDED`を1本追加する。P12直下、P13直上にあることを確認し、追加時の自動selectionを含めて`S2-add`を記録する。
8. P11だけを選択し、`S3-select-delete`を記録する。
9. P11 `CMCP_11_MUTATE_DELETE`を削除し、削除後の自動selectionを含めて`S3-delete`を記録する。
10. baselineのvisibility連動（separateは`on`、lower-zoneは`automatic`）のままTrack VisibilityでP08をshowし、`S4-show`を記録する。
11. P12だけを選択し、`S5-select-anchor`を記録する。
12. P13だけを選択し、P12からP13へのselection変更を`S5-select-change`として記録する。
13. P04を意図的に選択せず、P04のMute controlをonにして`S6-mute`を記録する。UIがselectionも変更した場合は、その差分を同じcheckpointへ記録する。
14. P03を意図的に選択せず、P03のSolo controlをoffにして`S7-solo`を記録する。UIがselectionも変更した場合は、その差分を同じcheckpointへ記録する。
15. separate MixConsoleの`Sync Visibility of Project and MixConsole`をoffにし、P08をProject windowだけでhideして`S8-project-only-hide`を記録する。separate MixConsoleまたは同期toggleを利用できないrunでは、このsubcaseを`n/a`または`UNSUPPORTED`としてstep 16へ進む。lower-zoneだけのrunはtoggleが存在しない正常な`n/a`であり、`UNSUPPORTED`ではない。
16. separate MixConsoleでは`Sync Visibility of Project and MixConsole`をonへ戻し、P08がProject windowとMixConsoleの両方でhiddenになるbaseline visibilityを復元して`S8-restore`を記録する。lower-zoneではProject側のP08をhiddenへ戻して自動追従を確認する。
17. M1を保存する。
18. E0へ切り替えて`S9-empty`、M1へ戻って`S9-mutation`、C1 baselineへ戻って`S9-baseline`を記録する。

各checkpointにCallback観測windowを個別に適用します。rename / add / deleteのためのselection操作を同じcheckpointへ混ぜず、途中で`INCONCLUSIVE_CALLBACK_TIMEOUT`になった場合は次の操作へ進みません。

期待するUI差分:

| step | Project inventoryの差分 | 確認対象 |
| --- | --- | --- |
| S0 | C1と同一 | 初期callback、ID、状態 |
| S1-select | P10だけをselectedへ変更 | rename前selection callback |
| S1-rename | P10の名前だけ変更 | ID維持または変更、title callback順 |
| S2-select-anchor | P12だけをselectedへ変更 | add前selection callback |
| S2-add | P12とP13の間へ1本追加 | 既存ID、index、addと自動selection callback |
| S3-select-delete | P11だけをselectedへ変更 | delete前selection callback |
| S3-delete | P11を除外 | removal、削除後selection、後続index |
| S4-show | P08を表示 | visibility追従有無、bank再配置 |
| S5-select-anchor | P12だけをselectedへ変更 | 比較元selection callback |
| S5-select-change | selectedをP12からP13へ変更 | selected callbackのfalse / true順 |
| S6-mute | P04 Muteをfalseからtrueへ変更 | mute callback、意図しないselection、他Trackへの影響 |
| S7-solo | P03 Soloをtrueからfalseへ変更 | solo callback、意図しないselection、明示control値 |
| S8-project-only-hide | visibility同期offでP08をProject-only hide | Project / separate MixConsole状態の分離 |
| S8-restore | visibility同期onとP08 hiddenを復元 | project切替前のvisibility原状回復 |
| S9 | E0 / M1 / C1を切替 | project間ID寿命、activate / deactivate順 |

script reloadとCubase再起動はbaseline作成へ混ぜず、C1を開いた独立phaseとして実施します。

| phase | 手順 | checkpoint |
| --- | --- | --- |
| R1 | MIDI Remote Script Consoleにprobeが表示されていることを確認 → C1 snapshot → Reload Scripts → 新しいload / reinitialize markerを確認 → 最大`reconnect_deadline_ms`まで再接続待ち → C1 snapshot | callback再初期化、reload前後のID比較 |
| R2 | C1 snapshot → Cubaseを正常終了 → 同じversionを1 instance起動 → C1を開く → 最大`reconnect_deadline_ms`までready待ち → snapshot | restart後のID、初期callback、接続状態 |

R1/R2の`reconnect_deadline_ms`は既定`30000`とし、run情報へ記録します。期限内に接続と必要な初期snapshotを確認できなければ、そのphaseを`INCONCLUSIVE_RECONNECT_TIMEOUT`として停止し、待ち続けたり後続phaseへ進んだりしません。R1/R2は未保存の通常projectがないことを再確認してから行います。IDの維持・変更は観測値であり、このfixtureのpass条件にはしません。

## Case O1: Input / Output / VCA（任意）

このcaseはC1からSave Asしたcopyで行い、core baselineへ混ぜません。

期待するoptional entity:

| label | entity | name | location / order | selected | mute | solo | visibility |
| --- | --- | --- | --- | --- | --- | --- | --- |
| O-P21 | VCA Track | `CMCP_OPT_VCA` | Project上でP20の直下 | false | false | false | Project / MixConsoleでvisible |
| O-I01 | Stereo Input bus/channel | `CMCP_OPT_INPUT` | Audio Connections Input。host順は観測値 | false | false | false | MixConsoleでvisible |
| O-O01 | Stereo Output bus/channel | `CMCP_OPT_OUTPUT` | Audio Connections Output。host順は観測値 | false | false | false | MixConsoleでvisible |

利用できないcontrolは`not available`とし、値を推測しません。VCA、Input、Outputの一部だけが安全に作成できるrunも許可しますが、存在しない行と理由を明示します。

1. C1を開き、変更前に`CMCP_TrackFixture_Optional_IO_VCA.cpr`として別名保存する。
2. Audio Connectionsを変更する前に、Input / Output busの名前、format、port割当を含む完全なpre-run inventoryをlocal記録する。raw device名はlocalにだけ保存し、commitする結果ではredacted aliasを使う。
3. `CMCP_OPT_INPUT`または`CMCP_OPT_OUTPUT`が既に存在する場合は衝突として中止し、既存busを変更・削除しない。
4. Cubase editionがVCAを提供する場合、project直下のP20直下へVCA `CMCP_OPT_VCA`を1本追加する。
5. Audio Connectionsでdevice portを割り当てないStereo Input `CMCP_OPT_INPUT`とStereo Output `CMCP_OPT_OUTPUT`を安全に作成できる場合だけ追加する。
6. 実audio deviceへ接続していないことを確認する。
7. O-P21 / O-I01 / O-O01の利用可能な行を表の状態にし、coreのstateと順序がC1から変わっていないことを確認する。
8. O1を保存する。
9. Project window、MixConsole、Mixer Bank zone、DirectAccess treeでそれぞれ存在と順序を記録する。DirectAccess非対応versionではそのaccess方式だけを`UNSUPPORTED`とする。
10. Mixer Bankではzoneごとに次の3 configを使い、それぞれcoreのB0〜B5と同じ固定sequenceおよびCallback観測windowを適用する。3 configの結果を連結して単一のcross-zone順序を推測しない。

全configでwidthは`8`、include typeはVCA / Input / Output、exclude typeはAudio / Instrument / Sampler / MIDI / Group / FX、followVisibilityは`false`です。

| config ID | 対象zone | API 1.1 window-zone method | explicit main filter対応host |
| --- | --- | --- | --- |
| `MB_OPTIONAL_MAIN` | lockされていない中央section | `excludeWindowZoneLeftChannels()` + `excludeWindowZoneRightChannels()` | `includeWindowZoneMainChannels()`を追加 |
| `MB_OPTIONAL_LEFT` | left | `includeWindowZoneLeftChannels()` | `excludeWindowZoneMainChannels()` + `excludeWindowZoneRightChannels()`を追加 |
| `MB_OPTIONAL_RIGHT` | right | `includeWindowZoneRightChannels()` | `excludeWindowZoneMainChannels()` + `excludeWindowZoneLeftChannels()`を追加 |

API 1.1のleft / right configは、同一filter categoryの`include*`が対象zoneへ絞る契約を使います。mainをimplicitのままleft / right includeと併用して「全zone」と扱ってはいけません。explicit main-zone methodはfeature detectionで存在を確認した場合だけ呼び出します。API 1.1 runは`explicit_main_filter=not available`と記録し、それだけを理由に`UNSUPPORTED`とはしません。pre-runから存在するInput / Outputも各zoneへ現れ得るため、fixture busだけに絞ったと推測せず、全slotを記録します。

11. bus構成をdefault presetとして保存しない。観測後、今回作成した`CMCP_OPT_INPUT`と`CMCP_OPT_OUTPUT`だけを削除し、既存busには触れない。
12. Input / Output bus inventoryをpre-run記録と照合し、visibility同期をrun前の状態へ戻す。完全に一致しない場合は通常projectを開かず`RESTORE_FAILED`として停止し、手動復旧が完了するまで続行しない。
13. inventory一致後、bus削除によるO1のcleanup差分は保存せずにO1を閉じる。必要ならE0を開いてinventoryを再確認してから通常projectへ戻る。O1を後日再度開いた場合もstep 11〜13のcleanupを繰り返す。

VCA非対応、bus変更が通常環境へ影響する、または`Not Connected`にできない場合は省略します。省略理由をrun情報へ記録し、別typeで代用しません。

## Run終了時のcleanup

fixture作成前にseparate MixConsoleの同期toggleが`on` / `off` / `not open`のどれだったかを記録します。すべてのcaseまたは中断したsequenceの終了時に、次を確認してから通常projectへ戻ります。

1. separate MixConsoleを使った場合は`Sync Visibility of Project and MixConsole`をrun前と同じ値へ戻す。run前に開いていなければfixture用windowを閉じる。lower-zoneはtoggleを持たないため操作しない。
2. O1を実施した場合はInput / Output bus inventoryがpre-run記録と完全に一致していることを再確認する。
3. fixture project以外を変更・保存していないことを確認する。
4. 原状回復を確認できない場合は`RESTORE_FAILED`として停止し、通常projectを開かない。

## 観測ログ

raw probe logはlocal artifactとし、repositoryには結論と再現に必要な最小例だけを`docs/track-api-host-spike.md`へ転記します。各recordは最低限次を持ちます。

```text
run_id
fixture_revision
sequence
monotonic_timestamp_ms
request_started_ms
request_finished_ms
elapsed_ms
configured_timeout_ms
capability_result
source                 # mixer_bank / direct_access / ui_snapshot
event                  # initial / title / selected / mute / solo / added / removed / activated
bank_or_tree_index
slot_index
fixture_label          # 対応を確認できた場合だけ
host_id                # local raw logのみ。APIが返した値を推測しない
host_id_alias          # committed result用のrun-local alias
host_id_byte_length
host_id_sha256         # committed resultで同一性比較が必要な場合
requested_title
accepted_ui_title
returned_title
type                   # APIが返した値。推測しない
selected
mute
solo
project_visible
mixconsole_visible
mixconsole_surface
visibility_sync
follow_visibility
included_channel_types
excluded_channel_types
included_window_zones
excluded_window_zones
explicit_main_filter_capability
result_class           # PASS / FAIL / UNSUPPORTED / INCONCLUSIVE
notes
```

APIが提供しないfieldは`null`または`not available`とし、直前値で埋めません。callbackの受信順を保ち、同一timestampへ並べ替えません。raw host IDはlocal logから外へ出さず、committed resultではrun-local alias、byte length、必要な場合だけSHA-256 digestで同一性を表現します。名前、index、typeからIDを生成してはいけません。

snapshotごとに、期待するfixture label、観測できたlabel、missing、duplicate、scope外またはunknownを別々に集計します。`UNSUPPORTED`とtimeout等による`INCONCLUSIVE`を`PASS`へ含めません。

## 再現性チェックリスト

実機run前に次を確認します。

- [ ] run情報に正確なOS、Cubase version/build、MIDI Remote APIを記録した
- [ ] E0のProject Trackが0本である
- [ ] C1が20本で、P01〜P20の順番・type・Cubaseが受理したUI ground truth名が一致する
- [ ] E1がAudio Track 1本だけである
- [ ] P02/P03が同名で、MuteとSoloが別々に設定されている
- [ ] P13〜P20がselected / mute / soloの8通りを1回ずつ表している
- [ ] Unicode名と117文字のASCII要求名について、要求値とCubaseが受理した実値・長さを記録した
- [ ] P01 Folderが空で、Group / Effect Trackがproject直下にある
- [ ] P08だけがhiddenで、P17〜P20だけがselected、P20が最後に選択したTrackである
- [ ] Record Enable、Monitor、Automation Writeが全てoffである
- [ ] event、part、automation、imported mediaがない
- [ ] 8-slot bankを超えるTrack数がある
- [ ] Mixer Bank configでchannel typeとleft / right zoneを明示し、main filter capabilityとimplicit / explicit scopeを記録した
- [ ] mutationはM1 copyで行い、C1 baselineを変更していない
- [ ] optional caseの実施結果または省略理由を記録した
- [ ] separate MixConsole同期とoptional bus inventoryをrun前の状態へ戻した
- [ ] `.cpr`、raw log、device名、absolute path、credentialをGitへ追加していない
- [ ] `git status --short`と`git diff --name-only`に意図した文書以外のfixture artifactがない

## 公式操作資料

- [Cubase 13: Choosing a Project Location](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/project_handling/project_handling_project_location_choosing_t.html)
- [Cubase 13: Adding Tracks via the Add Track Dialog](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/track_handling/track_handling_tracks_via_the_project_menu_adding_t.html)
- [Cubase 13: Track Controls](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/tracks_about/tracks_about_track_controls_r.html)
- [Cubase 13: Opening Track Visibility](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/project_window/project_window_inspector_visibility_opening_track_visibility_t.html)
- [Cubase 13: Showing/Hiding Individual Tracks](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/project_window/project_window_showinghiding_individual_tracks_c.html)
- [Cubase 13: Selecting Tracks](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/track_handling/track_handling_selecting_tracks_t.html)
- [Cubase 13: Renaming Tracks](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/track_handling/track_handling_renaming_tracks_t.html)
- [Cubase 13: Removing Selected Tracks](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/track_handling/track_handling_removing_tracks_t.html)
- [Cubase 13: Audio Connections](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/vst_connections/vst_connections_c.html)
- [Cubase 13: MixConsole Zones](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/mixconsole/mixconsole_zones_c.html)
- [Cubase 13: Adding Input and Output Busses](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/vst_connections/vst_connections_input_and_output_busses_adding_t.html)
- [Cubase 13: Bus Configurations](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/vst_connections/vst_connections_bus_configurations_editing_c.html)
- [Cubase 13: Saving Project Files](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/project_handling/project_handling_saving_project_files_c.html)
- [Cubase 13: MIDI Remote Script Console](https://www.steinberg.help/r/cubase-pro/13.0/en/cubase_nuendo/topics/midi_remote/midi_remote_script_console_r.html)
- [MIDI Remote API releases and compatibility](https://steinbergmedia.github.io/midiremote_api_doc/versions/)
- [MIDI Remote API 1.2 changes](https://steinbergmedia.github.io/midiremote_api_doc/new_in_v1.2/)
- [MIDI Remote API Reference](https://steinbergmedia.github.io/midiremote_api_doc/codedoc_api_reference/)

このfixtureはCubase 13.0.30でも存在するglobal Add Track、Track controls、Visibilityの共通操作だけを必須手順に使い、version固有機能はoptionalとして扱います。現行MIDI Remote API Referenceには新しいCubase向け機能も含まれるため、13.0.30の能力を現行Referenceだけから推測しません。
