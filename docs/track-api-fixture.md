# Track API実機検証fixture

この文書は、Track列挙の実機調査と受け入れテストで同じCubase project状態を再現するための手順を定義します。`.cpr`、audio、MIDI file、presetはrepositoryへ追加しません。fixtureは各検証者が専用の一時projectとして作成します。

主な利用先は[Issue #3](https://github.com/ieee0824/cubase_mcp/issues/3)のhost API調査と[Issue #22](https://github.com/ieee0824/cubase_mcp/issues/22)の受け入れテストです。列挙方式は[Issue #4](https://github.com/ieee0824/cubase_mcp/issues/4)、DTO / ID / pagination契約は[Issue #5](https://github.com/ieee0824/cubase_mcp/issues/5)で決定し、全体の依存関係は[Issue #24](https://github.com/ieee0824/cubase_mcp/issues/24)で管理します。

fixture revision: `2`

revision 2では、bank幅`8`と対象channel数がちょうど一致する境界fixture `E8`を追加しました。revision 1のlocal projectやrun recordをrevision 2の証拠として再利用しません。

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
CMCP_TrackFixture_r2_<YYYYMMDD>
```

このdirectoryと`.cpr`はlocal artifactです。Gitへ追加しないでください。Gitへcommitする結果や共有ログにはこの文書の合成Track名だけを含め、absolute path、raw MIDI device名、audio data、credentialは記録しません。原状回復にraw device名が必要なO1のpre-run inventoryだけはlocal artifactとして保持し、共有しません。

repositoryの`.gitignore`は`CMCP_TrackFixture_*`のproject / backup、`CMCP_TrackProbe_*`のmanifest / audit report / JSONL / SysEx、予約済みruntime directoryを追加防御として除外しますが、Cubase projectやaudio artifactを包括的には除外しません。`.cpr`、`.bak`、`Audio/`、`Edits/`、`Images/`、autosave、manifest、raw SysEx / JSON logをstage前に手動で確認してください。runtime artifactをrepository外へ置く原則は変わりません。

## 検証matrix

各runの開始時に次を記録します。versionはAbout画面等に表示される省略なしの値を使います。

| 項目 | 記録例 | 必須 |
| --- | --- | --- |
| run ID | `mac-c13-r2-001` | yes |
| fixture revision | `2` | yes |
| 実施日時とtimezone | `2026-08-26T22:00:00+09:00` | yes |
| OS / build / architecture | `macOS 26.5.1 / build ... / arm64` | yes |
| Cubase edition / version / build | `Cubase Pro 13.0.30.226` | yes |
| MIDI Remote API | `1.1` | yes |
| repository commit | 40桁のGit commit SHA | yes |
| probe source / installer embedded / deployed SHA-256 | 一致する3つのdigest | yes |
| collector binary SHA-256 | 実行したrelease binaryのdigest | yes |
| daemon version / build | `cubase_mcp 0.1.0 / commit ...` | 使用時 |
| Bridge設定 | mode、標準port roleまたはredacted alias、address、timeout | 使用時 |
| profile / Track access方式 | `c13_mixer_bank` / `MixerBankZone`、または`c15_combined` / 両projection | yes |
| probe bank幅 | `8` | Mixer Bank runのみ |
| Mixer type / window-zone filter、followVisibility | include/exclude設定、true / false | Mixer Bank runのみ |
| explicit main-zone filter capability | `available` / `not available` | Mixer Bank runのみ |
| MixConsole観測surface | `lower-zone` / `separate` | yes |
| Project / MixConsole visibility同期 | `automatic` / `on` / `off` / `n/a` / `unsupported` | yes |
| run前のseparate MixConsole同期 | `on` / `off` / `not open` | yes |
| callback観測window | `5000 ms` | yes |
| reload / reconnect deadline | `30000 ms` | R1 / R2 |
| Instrument / Effectの代替plug-in | 名前または`none` | 該当時 |
| O1 status / 理由 | `skipped / not_separately_authorized` | yes |

Probe capabilityの`host_version`はMIDI Remoteの`mDefaults.mAppVersion.getVersionString()`から取得する補助照合値です。API v1.1のCubase 13型定義にはこのsurfaceがないため、C13 profileでは`null`も正規の「取得不能」として受理します。runtime objectが値を提供した場合はbuild番号を含まない`13.0.30`、または対応するbuild込み完全値だけを受理します。C15 profileは`null`を拒否し、`15.0.30`または対応する完全値だけを受理します。別patch、別build、別major version、prefix / suffix一致は両profileで拒否します。この補助値または`null`をAbout画面とapp bundleから記録する上表の完全なedition / version / buildの代用にしてはいけません。manifestの完全値とprofileの一致は引き続き独立して必須です。

最低限Cubase 13.0.30で実施し、利用できる場合は13.0.50以降の正確なbuildでも同じ手順を実施します。13.0.50はMIDI Remote API 1.2導入境界ですが、現在の調査環境にはinstallされていません。Cubase 15等のrunは補足情報であり、Issue #4が決定するまでは13.0.50以降のrunを代替した扱いにしません。未installまたは未確認のversionを「検証済み」と記載せず、`not available`として残してください。

Cubase versionごとにfixtureを新規作成します。新しいCubaseで保存した`.cpr`をCubase 13で開いて使い回してはいけません。

Cubase 15ではMixer Bank用とDirectAccess用にfixtureやrunを複製しません。1つのCubase instance、collector process、run ID、manifestで各caseを1回だけ操作し、同じcheckpointからMixer BankとDirectAccessの2 projectionを取得します。DirectAccessは`projection = mix_console_root_children_v1`、`scope_depth = 1`としてbase objectとその直下childだけを取得し、root childのchild listを展開しません。root childの同一ID再出現やroot self-referenceは`shared_reference` / `ancestor_cycle` recordとして保持しますが、`scope_complete = true`はroot-child coordinateの完全性だけを意味し、完全なhost graphやProject Track全体の完全性を意味しません。Cubase 13でもruntime feature detectionがDirectAccessを`supported: true, active: true`と報告した場合は同じ条件付きcombined契約を適用し、初期DirectAccess snapshotと全checkpointのDirectAccess final snapshotを省略しません。`supported: false, active: false`かつ固定のunavailable / incomplete reasonの場合だけMixer Bank単独とし、DirectAccess eventがないことを確認します。supportedだがinactive、activation error、または組合せ不整合はunsupportedへ丸めずrun invalidとします。API versionだけからどちらかを決め打ちしません。

DirectAccess capabilityは`projection = "mix_console_root_children_v1"`、`scope_depth = 1`、`authoritative_for_track_enumeration = false`、traversal limitの`direct_access_nodes = 256`、`direct_access_depth = 1`、`direct_access_children = 128`を返します。`direct_access_nodes`は共通のfail-closed ceilingであり、このprojectionの形状上の有効なsemantic recordは最大129です。各snapshotは同じprojectionと`scope_depth = 1`、`scope_complete`、`host_graph_complete`、`root_child_count`、`authoritative_for_track_enumeration`を全chunkで固定します。nontruncated snapshotでは`scope_complete = true`、`host_graph_complete = false`、`authoritative_for_track_enumeration = false`です。root observationだけが`children_expanded = true`です。depth 1のroot-child observationはchild countをbounded metadataとして持ち得ますが、child object IDを列挙しないため`children_expanded = false`であり、その値をleafやdescendant completenessの証拠にしません。root child countは最大128、semantic recordはroot 1件と各root-child coordinate 1件の合計最大129で、host-ID fragmentだけが別のwire itemとして加算されます。

## 作成するlocal project

| case / support artifact | local file名 | 用途 |
| --- | --- | --- |
| INIT bootstrap support artifact（checkpointではない） | `CMCP_TrackFixture_Bootstrap_Empty.cpr` | revision 2 primary C13 / C15 runのINITで、MIDI Remote mapping pageを決定的にactivateするための空project |
| E0 | `CMCP_TrackFixture_Empty.cpr` | project Trackが0本の状態 |
| E1 | `CMCP_TrackFixture_One.cpr` | 通常Audio Trackが1本だけの状態 |
| E8 | `CMCP_TrackFixture_Eight.cpr` | Mixer Bank対象がbank幅と同じ8本の境界状態 |
| C1 | `CMCP_TrackFixture_Core_Baseline.cpr` | 変更前の基準状態 |
| M1 | `CMCP_TrackFixture_Mutation.cpr` | rename / add / delete用copy |
| O1 | `CMCP_TrackFixture_Optional_IO_VCA.cpr` | Input / Output / VCAの任意調査 |

INIT bootstrapはrevision 2のprimary C13 / C15 runで例外なく使用します。対象runとexactly同じCubase versionを使い、同じ専用directory内へ新規の空projectとして作成し、Project Trackが0本であることを確認します。audio / MIDI / video、event、part、automation、plug-in、user preset、通常project由来の設定や名前、新しいrouting設定を追加せず、実機inputをmonitorしません。別versionで保存したproject、既存の通常project、正式なE0をbootstrapへ転用しません。run前にbootstrapのSHA-256をlocal recordへ固定し、対象versionを起動するexactな製品、application名またはbundle path、およびbootstrapのabsolute pathもlocalで確定します。これらのabsolute pathをraw JSONL、manifest、run ID、共有report、repositoryへ転記しません。

bootstrapはfixture caseでも独立したraw checkpointでもなく、bootstrap用のmanifest annotationを追加しません。revision 2のexactly 44 checkpointとmanifest v1 schemaは変更せず、既存の`INIT` annotationだけを使用します。`INIT`の`action_confirmed`は、run前に確定したexactな製品をbootstrap documentと同じOS launch操作で起動した場合だけ`true`にします。`ui_ground_truth_confirmed`は、Project windowのbasenameがexactly `CMCP_TrackFixture_Bootstrap_Empty`、Project Trackが0本、他のprojectが開いていない、dirty / modified表示がなくbootstrapを変更していないことをUIで確認した場合だけ`true`にします。INITで得たbootstrapの初期snapshot、明示的final snapshot、またはINIT annotationを、正式なE0のsnapshot、UI ground truth、action確認へ流用してはいけません。

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

## Case E8: bank幅ちょうど8 Track project

E8は「対象がbank幅ちょうど」の境界を、C1の終端挙動から推測せず独立して観測するためのprojectです。

1. E0を開き、すぐに`CMCP_TrackFixture_Eight.cpr`として別名保存する。
2. 次のStereo Audio Trackを1本ずつ追加し、Project windowで表の順へ並べる。これ以外のProject Track、Folder、Group、FX、Instrument、MIDI、VCAは追加しない。
3. 全8本のroutingを`No Bus` / `Not Connected`にし、Record Enable、Monitor、Automation Write、Mute、Soloをoffにする。
4. `CMCP_E8_01`だけを選択し、残り7本が非選択であることを確認する。
5. 全8本をProject windowと同期対象MixConsoleでvisibleにし、left / right zoneへlockせず中央のscrolling fader sectionへ置く。
6. 保存して閉じ、再度開いて状態が復元されることを確認する。

UI ground truth:

| label | Track type | Track name | Project順 | selected | mute | solo | Project / sync対象MixConsole visibility |
| --- | --- | --- | ---: | --- | --- | --- | --- |
| E8-01 | Audio | `CMCP_E8_01` | 1 | true | false | false | visible |
| E8-02 | Audio | `CMCP_E8_02` | 2 | false | false | false | visible |
| E8-03 | Audio | `CMCP_E8_03` | 3 | false | false | false | visible |
| E8-04 | Audio | `CMCP_E8_04` | 4 | false | false | false | visible |
| E8-05 | Audio | `CMCP_E8_05` | 5 | false | false | false | visible |
| E8-06 | Audio | `CMCP_E8_06` | 6 | false | false | false | visible |
| E8-07 | Audio | `CMCP_E8_07` | 7 | false | false | false | visible |
| E8-08 | Audio | `CMCP_E8_08` | 8 | false | false | false | visible |

`MB_CORE_ALL`と`MB_CORE_VISIBLE`はいずれもAudioをincludeし、Input / Outputをexcludeし、main sectionだけを対象にするため、このfixtureでeligibleなProject由来channelはexactly `8`です。Audio Connections由来のchannelがUIに存在してもこの8本へ加算しません。実際のhost結果が8件になること自体はfixture作成条件ではなく観測対象であり、欠落、重複、scope外channelを補正しません。

runtimeではE1 checkpoint終了後に必須`E8` checkpointを開始し、`probe.observation.cut`の成功responseとcollectorが直後に自動生成するaction markerを隣接recordとして確認し、そのmarker直後にこのprojectを開きます。project load / optional same-source reactivation、UI ground truth確認、5000 ms window、`MB_CORE_ALL` / `MB_CORE_VISIBLE`（条件付きDirectAccessを含む）のfinal snapshot set、quiet periodをすべて`E8`内へ収めます。`E8`終了後にだけ次のB0 Reset checkpointへ進み、project openとResetを同じactionへ混ぜません。

各configで次の境界sequenceを実施します。各行を独立checkpointとし、後述の`5000 ms` windowと明示的final snapshotを適用します。

| checkpoint | 操作 | 境界で確認すること |
| --- | --- | --- |
| `E8-<CONFIG>-B0-reset` | Reset | 最初のbankにexactly 8本がどの順で現れるか |
| `E8-<CONFIG>-B1-next` | Next 1回 | ちょうど終端から進めた場合のcallback、空slot、repeat、no-op |
| `E8-<CONFIG>-B2-prev` | Prev 1回 | 最初のbankへ可逆に戻るか |
| `E8-<CONFIG>-B3-reset` | Reset | 操作後も初期bankを同じ規則で再取得できるか |

`<CONFIG>`は`MB_CORE_ALL`または`MB_CORE_VISIBLE`へ置換するため、E8の完全なcheckpoint IDは`E8-MB_CORE_ALL-B0-reset`から`E8-MB_CORE_VISIBLE-B3-reset`までの8個です。Next後に同じ8本が見える、空に見える、またはcallbackがないことだけで終端を断定しません。C1のbank幅超過・final partial pageの観測と組み合わせ、有限で重複・欠落のない列挙条件を別途評価します。

## Case C1: core baseline

### 正確なTrack inventory

1. E0を開き、Project windowのTrack listが0本であることを再確認する。
2. Trackを追加する前に`CMCP_TrackFixture_Core_Baseline.cpr`として別名保存する。
3. 次の20本だけを作成し、Project windowで上から`P01`から`P20`の順へ並べる。全Trackをproject直下へ置き、`P01` Folderは空のままにする。

`P09`の`<LONG_NAME>`は次の80 ASCII文字です。省略せずpasteしてください。

```text
CMCP_09_LONG_ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNO
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

Track作成後、Cubase UIが実際に受理したP05/P09の文字列、Unicode scalar数、UTF-8 byte数をrun記録へ転記します。P05は固定NFCまたはその固定NFD表現だけを受理し、NFDだった場合を`SETUP_VARIANCE`とします。P05がtruncationまたはそれ以外の変換を受けた場合はfixture不成立として停止します。P09は固定80文字のprefixのうち、予約marker `CMCP_09_LONG_`の13文字を完全に含むものだけを受理し、truncationが起きた場合を`SETUP_VARIANCE`として要求値と実値を両方残します。`C`や`CMCP`のような短いprefixはambient titleと区別できないため許可しません。いずれもrunのUI ground truthには実値を使い、文字列を黙って補正したり、host結果を要求値へ置換したりしません。

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
10. P08がVisibility listには残り、Project windowと対象MixConsoleでは非表示であることを確認する。primary runではseparate MixConsoleと利用可能な同期toggleが必須であり、toggleを利用できない場合はここでprimary auditを停止する。Project-onlyまたはlower-zoneだけの結果は別のsupplemental runとして扱い、44-checkpoint primary evidenceへ流用しない。lower-zoneの同期状態はtoggleを持たないため`UNSUPPORTED`ではなく`automatic`と記録する。
11. `CMCP_TrackFixture_Core_Baseline.cpr`を保存して閉じ、再度開いて状態が復元されることを確認する。

Soloによって他channelが聴感上muteされても、実機ログでは表の明示的なMute / Solo control値とeffective audio stateを混同しません。activeなSoloがある間、non-Solo channelのMixConsole上のMute表示とMixer Bankの`mValue.mMute`はSolo由来のeffective muteを表し、明示的なMute control値と区別できない場合があります。その状態でMute controlを操作した結果も明示Muteへの変更と決め付けず、Mute / Soloの両controlとcallbackを実測します。

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

同じC1から次の固定sequenceを1回ずつ実施します。raw checkpoint IDは`C1-<CONFIG>-<SNAPSHOT>`とし、例えば`MB_CORE_ALL`の`B0-reset`は`C1-MB_CORE_ALL-B0-reset`です。2 config × 6操作の12 IDを省略しません。

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
| E8 exact-width core channels | E8-01〜E8-08のProject上からの順 | n/a | bank幅ちょうどの境界fixture |
| Project inventory | P01〜P20のProject上からの順 | 含む | fixture自体の正解 |
| all core channels | P02〜P20のProject上からの順 | 含む | visibility非追従Mixer候補 |
| visible core channels | P02〜P07、P09〜P20の順 | 除外 | visibility追従Mixer候補 |
| Input / Output / VCA | coreとは別表で順序を記録 | n/a | optional zone候補 |

primary Mixer Bankのauthority候補は、明示したtype / main-zone filterに一致するcore MixConsole channelだけです。P01 Folderはそのsurfaceに存在しないため、Mixer BankでP02〜P20を欠落なく取得できてもProject inventory P01〜P20の完全性を証明しません。DirectAccessも`mix_console_root_children_v1`のroot-child scopeだけを完全性判定の対象とし、`authoritative_for_track_enumeration = false`、`host_graph_complete = false`を固定します。したがって、このcombined runだけからFolderを含むProject-wide Track enumerationを成功扱いにしません。

観測時は次を守ります。

- Folderやhidden Trackが返らないことだけで不具合と判定しない。
- APIが返さない`type`を名前やroutingから推測しない。
- P02/P03を同じTrackとしてまとめない。
- host IDをfixture labelへ置換せず、対応関係をrun単位で記録する。
- IDが空、重複、変更された場合も補正せず観測結果として残す。
- callbackが欠けた場合にpolling結果からcallbackを捏造しない。

## Callback観測window

初期化、bank操作、mutation、project切替、reload、restartを含むすべての観測checkpointは、操作直前から連続してcallbackを記録し、UIで期待状態を確認した後も操作開始から`5000 ms`以上観測を続けます。監査可能な観測anchorは、observation cutを使う21 checkpointと`INIT` / `R1` / `R2`ではaction marker時刻、bank navigationではReset / Next / Prev成功responseの受信時刻です。cut checkpointでは成功したcut responseと`boundary_source = probe.observation.cut_response`の自動action markerをcollectorが1つのatomic output pairとして隣接させ、同じrequest ID / epochをmarkerへ保持します。checkpoint開始時刻、cut response、navigation commandより前のaction markerを5000 ms windowの代用にしません。各操作の直前にはraw `collector_action` markerをexactly 1件記録し、1秒のquiet periodへ早く到達してもwindowを短縮しません。

`INIT`ではaction markerを書き、その直後に、run前に確定したexactな対象CubaseとINIT bootstrapを同じOS launch操作へ指定して起動します。macOSでは`open -a "<EXACT_CUBASE_APPLICATION_OR_BUNDLE_PATH>" "<ABSOLUTE_BOOTSTRAP_PROJECT_PATH>"`のようにapplication / bundleとdocumentの両方をquoteし、absolute pathはlocal commandにだけ使用します。Cubaseだけを先にHubへ起動してからbootstrapを手動で開く、Hub起動で始めたINITへbootstrapを後付けする、または別のprojectを経由して救済することを禁止します。bootstrapが同じlaunch操作で開かなかった場合はそのrunを停止し、collectorをgraceful drainした後、fresh run ID、fresh collector process、fresh JSONLでINITから再実施します。

INIT終了まで別projectを開かず、同じsourceからexactly 1回の完全な`probe.loaded(source_seq = 1)` → `probe.mapping_active` → `probe.capabilities` → `reason = page_activate`の`MB_CORE_ALL` / `MB_CORE_VISIBLE`初期snapshot完了 → runtime capabilityがDirectAccessをsupported / activeとした場合だけその初期snapshot完了 → `probe.ready(true, initial_snapshots_complete = true)`を必須とします。初期化中のcallbackはfeedbackとして順序どおり記録しますが、ready前に非command DirectAccess change snapshotをscheduleせず、初期状態は`page_activate` snapshotだけで覆います。INIT内の新しい`probe.loaded`の追加、`probe.ready(false)`、2回目の`probe.mapping_active` / capability / page-activate initial snapshot set / `probe.ready(true)`、partial activation、別sourceのactivationはrun invalidです。後続sequenceを待って成功へ戻さず、UI操作を進めず、fresh runで再実施します。INITのcold launch時間はR1 / R2用の`reconnect_deadline_ms`へ含めず、任意の上限で正しいrunを捨てません。R1 / R2でもaction markerより前のlifecycleや初期snapshotを新activationの証拠へ流用せず、同じ順序と完全性を再検証します。E0 / E1 / E8 / C1やS9等の通常checkpointで同sourceのpage reactivationが発生した場合は、旧activationのbounded feedbackを`ready(false)`より前に全件drainし、action後の`ready(false)` → mapping → capability → 初期snapshot → readyを同checkpoint内でexactly 1回完了し、target選択が破棄された後の再discoverを行ってからfinal snapshotへ進みます。旧mappingのcallbackを送信済みで次の`page_activate` snapshotが置換するcoalescedな非command DirectAccess change snapshotだけはpage境界でcancelできます。明示command、`page_activate`、bank snapshot、未知reason、未drain feedbackが残る場合は`probe.overflow`で停止します。通常checkpointで新しい`probe.loaded`、partial / 複数activation、action前sequenceの流用、再discover省略を許しません。

INIT終了後にexact ID `E0`を開始し、`probe.observation.cut`の成功responseと隣接する自動action markerを確認した直後に、bootstrapとは別fileの正式な`CMCP_TrackFixture_Empty.cpr`を開きます。Project windowでbasenameが`CMCP_TrackFixture_Empty`であり、Project Trackが0本であることを目視確認できた場合だけ、E0 annotationの`action_confirmed`と`ui_ground_truth_confirmed`を`true`にします。project切替によりsame-source reactivationが発生した場合は前段落のexactly 1 sequenceと再discoverをE0内で完了します。E0内の新しい`probe.loaded`、partialまたは複数のreactivation、正式E0を開かなかった状態、basenameまたは0本を確認できない状態はrun invalidです。

UI操作は対象Cubase instanceへの排他的な入力として実施します。semantic操作では、操作直前に再取得したactive application、window、project basename、dialog、対象controlのidentityを同じUI callへ直接束縛し、現在のmouse pointer位置から対象を決めません。座標操作しか使えない場合は、直前に再取得したwindow screenshotとwindow boundsから絶対座標を解決し、対象application / current window、明示座標、single / doubleのclick countを同じ1回のcallへ束縛します。現在のpointerを移動するcallとclickを分ける2段階入力や、取得済みwindowの外側を指す座標は使用しません。operator traceとstate dumpのComputer Use application識別子はprofileごとにexact `Cubase 15` / `Cubase 13`へ固定し、OS launchだけは別fieldのexact bundle path `/Applications/Cubase 15.app` / `/Applications/Cubase 13.app`へ固定します。各操作直後にfreshな同じUI surfaceを再取得し、操作ごとに定義したproject、selection、control値、visibility、dialog遷移またはprocess状態を確認します。入力guardの`mouse_moved`だけが増えたことは誤操作の証拠にしません。target-bound callは現在のpointer移動でsemantic identityまたは明示座標を変更しませんが、guardはautomation call自体の対象、注入座標、click countまたはUI outcomeを検査しないため、fresh pre-state、exactly 1 callのtool trace、freshなaction-specific postconditionの3つを省略しません。別operatorのbutton / keyboard / scroll / drag入力、focus移動、window移動、対象外controlの変化、予期しないcallback、またはpostcondition不一致を検出した場合は、そのannotationを`action_confirmed = false` / `ui_ground_truth_confirmed = false`のままrunを停止します。同じcheckpoint内でclickをやり直して成功へ戻しません。

macOSのprimary runでは、Cubaseを停止したままcollectorを起動して`collector_started`を確認し、その後、最初のcheckpointを開始する前に`cubase_input_guard`を起動して`version = 4`、`source = hid_system_state`、`coverage = action_windows`、`privacy = counts_and_held_state_boolean`、`policy = consequential_input_only`の`ready`を確認します。以後のguard response / errorも同じversion、coverage、policyを必須とし、欠落または不一致を拒否します。全recordは同じ64桁hexの`guard_session_id`、正の`guard_process_id`、同じ`guard_started_at_unix_ms`を持ち、`record_sequence`が1から欠落なく増えることを要求します。これらはprocess相関と誤ったstream結合を検出する識別子であり、actorの認証や改ざん耐性を与える署名ではありません。collector起動に必要なOS承認はguard startupより前に完了させ、guard startup後はcollectorを再起動しません。このsidecarはCoreGraphicsの`HID_SYSTEM_STATE`にあるevent category別の累積countと、通常keyまたはmouse buttonが押下中かというaggregate booleanだけを読みます。counterを進めたactorやeventの物理 / synthetic由来は認証せず、個別key codeや`other` mouse buttonの番号を保持・serialize・logせず、入力文字、pointer座標、対象applicationも取得しません。各UI操作では、freshなpre-stateを取得する**前**に`arm`してその時点のcountをbaselineとし、単一のtarget-bound UI callとfreshなaction-specific post-state確認を終えた直後に同じ`action_id`で`check`します。正常に完了したarm sampleとcheck sampleの間の`mouse_moved` deltaはinformationalとして結果へ残しますが、それだけではinterferenceをlatchしません。残る15 field、すなわちleft / right / otherのmouse down / upとdrag、key down / up、flags changed、scroll wheel、tablet pointer / proximityのいずれかが1件でも増えた場合は`interference_detected = true`としてそのactionとrunをfail closedにし、同じcheckpoint内でやり直しません。arm前のidle区間は次のbaselineへ繰り越しません。その区間にはautomationのUI actionを置かず、次のarm後にactive application、window、project、dialog、targetをfreshに再取得し、Probeのorphan callbackや予期しない状態変化があれば別途run invalidにします。guard processはcollectorの正常summaryまで同一processで維持し、その後にだけ`finish`を送り、同じpolicyと`interference_detected = false`の`finished`を確認します。`interference_detected = true`、`cancelled`、`rejected`、guard protocol error、process終了、action ID不一致、明示的`finish`なしのEOF、またはarmedのままの`finish`は、postconditionが偶然一致した場合でもrun invalidです。guardを途中で再起動してaction windowを継ぎ足しません。

`finished`が証明するのは、armed action windowがすべて閉じられ、そのwindow内で15種類のconsequential counter deltaがlatchされず、cancelもなかったことだけです。move-only eventがなかったこと、arm前のidle区間でcounterが変化しなかったこと、eventのactor、run全体への排他的入力、各automation callの対象またはUI outcomeは証明しません。`ping`と`finish`はidle差分を判定しません。`arm`と`check`の各sampleでは、per-type counterとheld-stateを読む区間全体を`kCGAnyInputEventType`のaggregate counterで前後から括ります。この短いsample中にaggregateが変化した場合は、mouse moveだけであっても`INPUT_DURING_SAMPLE`としてfail closedにします。通常key / mouse buttonのheld stateとcounter取得の`2000 ms` timeoutも同様に拒否します。move-onlyをinformationalとして除外するpolicyは、両sampleが正常に完了した後のarm→check差分だけに適用します。

run開始前に、実際に注入するUI callを順序付き・重複なしの`ui-action-inventory`としてrepository外へ固定します。menuを開く、menu itemを選ぶ、dialog値を設定する、確定する、windowをraiseする、scrollする、clickする、keyを送る等は、それぞれ別のaction IDです。semantic callでは対象application / window / element identity、座標clickでは対象application / current window、fresh screenshotから解決した明示座標、single / doubleのclick countをoperator tool traceへ記録します。run前には未確定なdynamic identityや座標をinventoryへ捏造せず、期待するAPI、操作、click count、precondition、action-specific postconditionを固定し、実行時にfresh pre-stateから解決した値を同じaction IDのtraceへ束縛します。1つのarm/checkへ複数の注入callを入れません。UI call前にpre-stateまたはtool bindingが失敗した場合はarmed中に`cancel`します。UI callを送った場合はpostconditionが失敗しても必ず先に同じIDを`check`し、clean resultを得た直後に同じIDの`reject`を送って`reason = postcondition_failed` / `after_clean_result = true`を記録します。interference resultならlatch自体がrunを停止します。いずれもretry用の別actionを追加しません。run後はguard JSONLの`armed`と`result`のaction ID列がinventoryとexactly一致し、各IDが隣接する1 pairで、欠落・追加・逆順がなく、全recordの`version = 4` / session identity / sequence / `policy = consequential_input_only`、各resultの15 consequential deltaが0であることを機械照合します。sidecar自体はComputer Useや別automation APIの直接callをinterceptせず、注入clickの座標やclick countも観測しないため、この照合、exactly 1 callのoperator tool trace、fresh pre / post-stateなしに「全UI callが正しい対象へguardされた」と主張しません。audit manifest v1 / report v2もこのinventoryをbindしないため、guard artifactはlocal operator evidenceに限ります。

```json
{"command":"arm","action_id":"S3-delete"}
{"command":"check","action_id":"S3-delete"}
{"command":"reject","action_id":"S3-delete"}
{"command":"finish"}
```

`policy`はguardがresponseで宣言する固定契約であり、`arm`へ追加fieldとして送りません。`hid_system_state`はCoreGraphicsの`HID_SYSTEM_STATE` counterを指しますが、counter値だけからactorや物理 / synthetic由来を認証しません。formal runのexactなautomation / launch contextごとに、次のcalibration matrixをFinderの専用scratch window上で先に完了します。state / targetのComputer Use application識別子はexact `Finder`、`open` controlのapplication pathはexact `/System/Library/CoreServices/Finder.app`へ固定します。許可controlと拒否controlを同じlatched processへ連結せず、interferenceを意図的に起こす各rowはfresh guard processで行います。

| control | exact context / input | 必須guard結果 | UI / tool確認 |
| --- | --- | --- | --- |
| automation negative | exact application / documentを指定する`open` | 15 consequential deltaがすべて0。`mouse_moved`は任意 | exactly 1 callと期待process / document postcondition |
| automation negative | `press_key` | 同上 | exactly 1 callとshortcut固有postcondition |
| automation negative | `set_value` | 同上 | exactly 1 callとfield値のfresh postcondition |
| automation negative | application / window / elementへ束縛したsemantic click | 同上 | exactly 1 callと対象control固有postcondition |
| automation negative | application / current window / 明示座標へ束縛したsingle coordinate click | 同上 | pointer初期位置と無関係にscratch targetがexactly 1 clickを受理する |
| automation negative | 同じ束縛を使うdouble coordinate click | 同上 | single clickでは生じないfolder open等のaction-specific postcondition |
| move-only acceptance | arm response後からcheck command前までに物理pointerを動かすがbutton、key、scroll、dragは使わない | `mouse_moved >= 1`、15 consequential deltaは0、`interference_detected = false` | semantic identityと明示座標の両controlで対象とpostconditionが変わらない |
| target-binding rejection | current window外の座標、または意図的に誤った有効座標 | guardだけの成功を採用しない | window外はcall自体が拒否され、誤座標はclean `result`を保存した後にaction-specific postcondition不一致として`reject`される |
| consequential positive | 物理mouse button click | 対応down / upが1以上で`interference_detected = true` | session latch後にrunを継続しない |
| consequential positive | 物理keyboard input | key down / upまたはflags changedが1以上で同上 | 同上 |
| consequential positive | 物理scroll | scroll wheelが1以上で同上 | 同上 |
| consequential positive | 物理drag | button / dragged / upのいずれかが1以上で同上 | 同上 |
| held / race | armまたはcheckのsample中に通常key / mouse buttonを保持するか、moveを含む任意のHID eventを発生させる | held-state errorまたは`INPUT_DURING_SAMPLE` | fail closed |

校正とformal runでは、各actionのfresh pre / post UI stateをJSON、同じ取得結果のscreenshotを画像fileとしてrepository外へ保存します。operator traceは各fileの相対path、SHA-256、state JSON内と一致する取得時刻を保持し、`pre captured_at < call started_at <= call ended_at < post captured_at`を満たします。校正directoryは成功した8 fresh guard processの24 JSONL / stderr fileと、exactly 28組のstate / screenshotだけを含むclosed setとし、失敗・再試行artifactは別directoryへ隔離します。formal runもexactly 56 actionの112組だけをclosed setとして保持します。拡張子と画像signatureが一致しないfile、参照されない追加file、state / screenshotの実byteから再計算したdigestがtraceと一致しないrunは採用しません。

機械検証のtrust rootは同じrepository commitにある`scripts/check-input-guard-calibration.sh`と`scripts/check-track-probe-evidence.sh`です。final checkerは必ずrepository側のscriptを直接起動し、evidence directory内の`check-evidence.sh`を実行入口にはしません。両scriptをevidence directoryへbyte-for-byte copyし、final checkerはcleanなexact HEAD、inventoryのcommit、repository側scriptとの一致、事前に固定したchecker digestを検証します。校正検証もwritableなevidence copyではなくrepository側の校正checkerを実行し、比較後の差し替えを実行へ持ち込みません。auditorはrepository rootをworking directoryとして、caller由来のRust wrapper / rustflags環境変数を除去したうえでcleanなinventory-bound commitから隔離target directoryへoffline rebuildし、run recordおよび通常のrelease binaryとdigest・byte単位で一致したものだけを実行します。この再buildはlocal Rust toolchainとCargo home configurationをtrusted environmentとする境界を持ちます。生成reportを含む全canonical regular fileは、自身だけを循環参照回避のため除外したdetached `artifact-sha256.txt`へ列挙します。guard session identityはstream相関にだけ使用し、operator認証やartifact署名として扱いません。

counter取得が`2000 ms`以内に完了しない、sample中のaggregate変化を拒否できない、通常keyまたはmouse buttonが押下されたまま、controlを再現できない、またはautomation自身が15 fieldのいずれかを増加させる環境ではguardを利用可能と推測せずprimary runを開始しません。`mouse_moved`は結果へ記録し、正常に完了したarm / check sample間では単独のaction interferenceに使いませんが、sample中のaggregate raceからは除外しません。CoreGraphicsはkey autorepeatをcountしないため、押下中keyを拒否するpreconditionを省略しません。volume / brightness等の一部special hardware keyやsoftware remote control、通知、Cubase自身によるfocus / window変更はconsequential counterだけでは検出できません。input guardは前後のsemantic target / screenshot / Probe差分、tool target binding、action-specific postconditionを置き換えず、すべてを満たした場合だけannotationをtrueにします。guard JSONL、guard binaryのSHA-256、exact-context calibration結果、ui-action-inventory、operator tool traceはrepository外のlocal operator recordとして保持し、raw Probe JSONLやmanifest v1へ混入させません。audit manifest v1とauditorはguard artifactを入力に取らないため、redacted audit report v2だけからguard使用を証明したと主張しません。

各checkpointでは次の順序を固定します。

1. checkpointを開始する。
2. `E0` / `E1` / `E8` / `C1`とS0〜S9のexactly 21 checkpointでは、最初のtargeted commandとして`probe.observation.cut`を`@selected`へ送る。成功responseをcollectorがraw `probe_response`として出力した直後、同じatomic pair内に同じrequest ID / `observation_epoch`を持つraw `collector_action` markerを自動生成するため、operatorは別の`collector.action`を送らない。隣接するmarkerを確認した直後にUI操作を行う。bank navigationではcutを送らず、先に明示的markerを書いてから`1000 ms`以内に対応するReset / Next / Prevを送る。`INIT` / `R1` / `R2`もcutを送らず、明示的marker直後に新activationを開始する。UI annotationの`action_confirmed`は、この順序で表の操作だけを行った場合にだけ`true`にする。
3. 上記の観測anchorから`5000 ms`以上を経過させる。cut checkpointではaction markerより前のcut待ち時間を、navigationでは操作responseより前の待ち時間をwindowへ算入せず、途中でquietになっても短縮しない。
4. checkpointを閉じる前に明示的なfinal snapshot commandを送る。C15、またはC13でruntime capabilityがDirectAccessをsupported / activeと報告したrunでは、最初に`probe.direct_access.snapshot`を取得し、response、同期`update()`から生じ得るfeedback、全chunkの完了まで待つ。その後、E8 / C1のbank navigation checkpointは操作したconfigの`probe.bank.snapshot`を1回取得し、それ以外は`MB_CORE_ALL`、続けて`MB_CORE_VISIBLE`を取得する。DirectAccessが固定のunsupported分岐であるrunでは同じbank snapshotだけを同じ順で取得する。自動初期snapshot、操作直後のcallback、または直前checkpointのsnapshotをfinal snapshotの代用にしない。
5. final snapshot setの最初のsnapshotが完了した後は、後続する同じsetの明示的snapshot request / response / chunk以外のprobe messageやhost callbackを1件も許さない。DirectAccess `update()`の同期feedbackは最初のDirectAccess snapshot完了前にだけ許され、完了後へ持ち越さない。set最後のresponse / chunkを受信してからは、probe messageもhost callbackもない追加のquiet periodを連続`1000 ms`以上待つ。final snapshotがcollectorのlast-message clockを更新するため、snapshot完了直後にcheckpointを終了しない。set途中または最後のsnapshot後に予期しないmessageが1件でも届いた場合はsnapshotが同じUI状態を表さない、または古くなり得るため、quiet timerだけを再開して成功扱いにせず、そのrunを停止する。
6. 文書で固定した期待状態とUI実測状態が一致したかを同じcheckpoint IDのannotationへ`result` / `ui_ground_truth_confirmed`として記録し、その後にだけcheckpointを終了する。個々の状態値は自由記述でsidecarへ複製せず、このrevisionの固定fixture表を期待値、boolean確認を実測照合結果とする。P05/P09の受理実値と代替plug-inだけは`fixture_acceptance`へ構造化して記録する。`S6-mute`の`action_confirmed = true`は指定したMute UI経路をexactly 1回実行したこと、`ui_ground_truth_confirmed = true`はpre / post UI状態を一意に確認したことだけを表し、Muteがtrueになったことや機能上のPASSを意味しない。状態を一意に確認できない場合は両値をtrueへせずrunを停止する。manifest v1はS6の結果カテゴリを保持しないため、explicit Mute on / no-op / Solo mappingの別はraw callback / projectionと監査後のredacted結果表に記録し、annotation booleanへ畳み込まない。

Cubase 15、およびruntime capabilityがDirectAccessをsupported / activeと報告したCubase 13 runでは、同じUI操作と同じwindowに対してMixer BankとDirectAccessのfinal snapshotを両方取得します。一方だけのsnapshotから他方のprojectionを推測しません。次の条件をすべて満たした場合だけ次の操作へ進みます。

- final snapshot完了後からcheckpoint終了まで、probe messageもhost callbackも1件もなく、1000 ms以上のquietを満たした。
- UIの期待状態を確認でき、利用中access方式がsnapshotまたはready状態を提供する場合はそれも取得できた。
- 前stepのwindow終了後に遅延callbackを検出していない。

期限までcallbackが続く、明示的final snapshotを取得できない、final snapshot後に新しいmessageが届く、必要な状態を取得できない、またはwindow終了後から次のUI操作前までにcallbackを検出した場合は`INCONCLUSIVE_CALLBACK_TIMEOUT`とします。遅延callbackは`orphan_after_<step>`として記録し、次のUI操作を行わず、そのsequenceを停止します。callbackが来ないことやboolean値を推測で補いません。異なるwindowを使うrunは、値と理由を記録し、primary comparisonから分離します。

## Case M1: mutation sequence

1. C1 baselineを開く。
2. `S0` checkpointを開始し、action marker直後に`CMCP_TrackFixture_Mutation.cpr`として別名保存する。Save Asに伴うcallbackをcheckpoint外へ出さない。
3. 保存後の安定した初期snapshotを同じ`S0`として記録する。
4. P10だけを選択し、準備操作のselection差分を`S1-select`として記録する。
5. P10を`CMCP_10_RENAMED_変更後`へrenameし、`S1-rename`を記録する。
6. P12だけを選択し、`S2-select-anchor`を記録する。
7. Audio Track `CMCP_21_ADDED`を1本追加する。P12直下、P13直上にあることを確認し、追加時の自動selectionを含めて`S2-add`を記録する。
8. P11だけを選択し、`S3-select-delete`を記録する。
9. P11 `CMCP_11_MUTATE_DELETE`を削除し、削除後の自動selectionを含めて`S3-delete`を記録する。
10. baselineのvisibility連動（separateは`on`、lower-zoneは`automatic`）のままTrack VisibilityでP08をshowし、`S4-show`を記録する。
11. P12だけを選択し、`S5-select-anchor`を記録する。
12. P13だけを選択し、P12からP13へのselection変更を`S5-select-change`として記録する。
13. P03 / P14 / P16 / P18 / P20の明示Soloをonのまま維持し、P04を意図的に選択せず、run前に固定した1つのP04 Mute UI経路をexactly 1回実行して`S6-mute`を記録する。P04は明示Mute=false / Solo=falseから開始しますが、Solo由来のeffective mute中にこの操作が明示Muteをtrueにするとは事前仮定しません。操作前後のP04の明示Mute / Solo control、effective mute表示、Mixer Bank / DirectAccess callback、selection、他Trackへの影響をそのまま記録します。結果がexplicit Mute on、no-op、またはeffective unmuteからSoloへのmappingでも、補正や別経路での再試行をせず、そのpost-stateを以後のM1 ground truthとします。
14. P03を意図的に選択せず、P03のSolo controlをoffにして`S7-solo`を記録する。UIがselectionも変更した場合は、その差分を同じcheckpointへ記録する。
15. primary runではseparate MixConsoleと利用可能な`Sync Visibility of Project and MixConsole`を必須とする。同期をoffにし、P08をProject windowだけでhideして`S8-project-only-hide`を記録する。toggleを利用できないrunはprimary auditを停止し、lower-zoneだけの結果は別のsupplemental runとして扱う。
16. separate MixConsoleの同期をonへ戻し、P08がProject windowとMixConsoleの両方でhiddenになるbaseline visibilityを復元して`S8-restore`を記録する。M1の保存も`S8-restore`のaction後・final snapshot前に完了し、保存に伴うcallbackを同checkpointへ含める。保存後のM1 SHA-256を`mutation_copy.sha256_after`としてrun recordへ固定し、final checkerで同じ絶対pathの実fileから再計算する。
17. `S8-restore`のfinal snapshotとquietが完了してから次へ進む。
18. E0へ切り替えて`S9-empty`、M1へ戻って`S9-mutation`、C1 baselineへ戻って`S9-baseline`を記録する。

各checkpointにCallback観測windowを個別に適用し、対象access projectionごとの明示的final snapshotを取得します。rename / add / deleteのためのselection操作を同じcheckpointへ混ぜず、途中で`INCONCLUSIVE_CALLBACK_TIMEOUT`になった場合は次の操作へ進みません。

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
| S6-mute | Project inventory不変。active Solo中のP04 state差分は観測値 | 1回のMute UI操作、明示Mute / Solo / effective state、callbackまたはno-op、selection、他Trackへの影響 |
| S7-solo | P03 Soloをtrueからfalseへ変更 | solo callback、意図しないselection、明示control値 |
| S8-project-only-hide | visibility同期offでP08をProject-only hide | Project / separate MixConsole状態の分離 |
| S8-restore | visibility同期onとP08 hiddenを復元 | project切替前のvisibility原状回復 |
| S9 | E0 / M1 / C1を切替 | project間ID寿命、activate / deactivate順 |

script reloadとCubase再起動はbaseline作成へ混ぜず、C1を開いた独立phaseとして実施します。

| phase | 手順 | checkpoint |
| --- | --- | --- |
| R1 | `S9-baseline`のfinal snapshotをpre-reload状態として固定 → action marker → Reload Scripts → 新しいload / reinitialize marker → 再discover → actionから5000 ms以上観測 → 明示的C1 final snapshot | callback再初期化、reload前後のID比較 |
| R2 | R1のfinal snapshotをpre-restart状態として固定 → action marker → Cubaseを正常終了 → 同じversionを1 instance起動 → C1を開く → ready → 再discover → actionから5000 ms以上観測 → 明示的final snapshot | restart後のID、初期callback、接続状態 |

R1/R2の`reconnect_deadline_ms`は既定`30000`とし、action markerから新sessionのreadyと再discover完了までへ適用します。final snapshotはactionから5000 ms以上かつready / discovery後に行い、ready / discovery完了から`10000 ms`以内に完了させます。30秒のreconnect期限へ追加観測時間を混ぜず、期限内にready / discoveryを確認できなければ、そのphaseを`INCONCLUSIVE_RECONNECT_TIMEOUT`として停止します。R1/R2内でpre-action snapshot commandを重複実行せず、直前checkpointの監査済みfinal snapshotをpre-stateとして参照します。R1/R2は未保存の通常projectがないことを再確認してから行います。IDの維持・変更は観測値であり、このfixtureのpass条件にはしません。

## Case O1: Input / Output / VCA（任意）

このcaseは将来の別profile用手順であり、audit manifest v1とprimary Track Probeでは実施できません。primary Probeは`MB_OPTIONAL_*` config自体を作成しません。DirectAccessの`mix_console_root_children_v1`はbase object直下にInput / Output / VCA等が現れるかを観測し得ますが、depth 1より下を探索せず、固定fixture allowlist外のtitle、unique name、host ID、自由形式type / error文字列はProbe内でframe生成前にredactし、raw JSONLへ収集しません。安全なroot-child位置、explicit repeat edge、boolean / numeric値、固定type categoryとredaction件数だけをscope観測へ残します。v1 manifestは`optional_o1.status = "skipped"`と固定理由`not_separately_authorized`だけを受け付け、O1 projectを作成せず、以下の手順1以降を実行しません。

将来O1を実施する場合は、通常fixtureとは別の明示的な許可、O1専用Probe build / capability / audit profile、pre-run inventory、local UI記録、cleanup後の完全一致を先に定義します。primary 44-checkpoint runへoptional configを追加したり、manifestのstatusだけを`observed`へ変更したりしてはいけません。

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
9. Project window、MixConsole、Mixer Bank zone、DirectAccess root-child projectionでそれぞれ存在、child index順序、repeat edgeを記録する。DirectAccess非対応versionではそのaccess方式だけを`UNSUPPORTED`とする。DirectAccessに現れないentityをdepth 1より下にも存在しないと推測しない。
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

現行v1では常に省略します。将来profileでも、別途許可がない、VCA非対応、bus変更が通常環境へ影響する、または`Not Connected`にできない場合は省略し、別typeで代用しません。

## Run終了時のcleanup

fixture作成前にseparate MixConsoleの同期toggleが`on` / `off` / `not open`のどれだったかを記録します。すべてのcaseまたは中断したsequenceの終了時に、次を確認してから通常projectへ戻ります。

1. separate MixConsoleを使った場合は`Sync Visibility of Project and MixConsole`をrun前と同じ値へ戻す。run前に開いていなければfixture用windowを閉じる。lower-zoneはtoggleを持たないため操作しない。
2. O1を実施した場合はInput / Output bus inventoryがpre-run記録と完全に一致していることを再確認する。
3. INIT bootstrapが開いていないこと、dirty / modified表示のあるbootstrapを保存していないこと、およびbootstrapのrun後SHA-256がrun前のlocal SHA-256と完全に一致することを確認する。不一致ならそのrunを監査へ使わず、bootstrapを手動確認・再作成してfresh runを開始できる状態へ戻すまで通常projectを開かない。
4. fixture project以外を変更・保存していないことを確認する。
5. 原状回復を確認できない場合は`RESTORE_FAILED`として停止し、通常projectを開かない。

## runtime evidenceの3 artifact

runtime evidenceは、用途の異なる次の3 artifactを混ぜません。

1. **raw collector JSONL v1**: collectorがstdoutへflushするmachine event stream。repository外に保持し、callback、request / response、chunk、raw host ID、checkpoint markerを受信順のまま含む。fixture revision、UI確認、host version、実施許可を各raw recordへ後付けしたり、UI値でhost payloadを補完したりしない。
2. **audit manifest v1 / UI annotation sidecar**: operatorがrun前にtemplateを用意し、最初のraw recordと各checkpointの確認後に完成させるversioned JSON。raw JSONLと同じ`run_id`を持ち、環境・digest・期待profile・UI ground truth確認を結び付ける。raw host ID、MIDI port名、absolute path、device名を含めない。
3. **redacted audit report v2**（`audit_report_version = 2`）: fail-closed auditorがraw JSONLとmanifestを両方検証して生成する共有可能な集計。raw `run_id`をechoせず、そのSHA-256先頭16 hexから作る`run-<16-hex>` alias、両inputのSHA-256、環境・artifact digest、checkpointごとのfinal snapshot projectionを含む。通常titleとP05はfixture revision 2の固定allowlistへexact一致した値だけ、P09は予約marker `CMCP_09_LONG_`全体を含む固定80文字のprefix policyへ一致した値だけを出し、host IDはrun-local aliasだけを出力する。それ以外の文字列は分類と件数へredactする。repositoryへ転記できるのは、この検証を通過し、別途secret / path確認を終えたreportの最小部分だけである。

raw JSONL単独の`collector_summary.exit_ok`は通信integrityだけを表し、fixture coverage、UI ground truth、exact host、digest一致、O1許可を証明しません。manifest単独も、実際のcallback、final snapshot、時間window、sequence integrityを証明しません。どちらか一方しかないrunを`OBSERVED`へ使いません。

audit manifest v1の論理schemaは次です。auditorは`profile`と`fixture_revision`から必須checkpoint集合を固定的に導出し、入力側がannotationを省略して検査範囲を狭めることを許しません。

```json
{
  "audit_manifest_version": 1,
  "fixture_revision": 2,
  "profile": "c13_mixer_bank",
  "run_id": "mac-c13-r2-001",
  "run_started_at": "2026-08-27T12:00:00.123+09:00",
  "environment": {
    "host": {
      "product": "Cubase Pro",
      "version": "13.0.30.226",
      "api_version": "1.1"
    },
    "os": {
      "name": "macOS",
      "version": "26.5.1",
      "build": "25F80",
      "architecture": "arm64"
    },
    "repository_commit": "<40-hex>",
    "probe_source_sha256": "<64-hex>",
    "installer_embedded_sha256": "<64-hex>",
    "deployed_probe_sha256": "<64-hex>",
    "collector_binary_sha256": "<64-hex>"
  },
  "callback_window_ms": 5000,
  "reconnect_deadline_ms": 30000,
  "mixconsole": {
    "surface": "separate",
    "visibility_sync_initial": "not_open",
    "visibility_sync_during_baseline": "on",
    "visibility_sync_restored": true
  },
  "filters": {
    "bank_width": 8,
    "core_all_follow_visibility": false,
    "core_visible_follow_visibility": true,
    "included_channel_types": ["audio", "instrument", "midi", "group", "fx"],
    "excluded_channel_types": ["sampler", "vca", "input", "output"],
    "left_zone": "excluded",
    "right_zone": "excluded",
    "main_filter": "implicit"
  },
  "fixture_acceptance": {
    "alternate_plugins": {
      "instrument": {
        "status": "none"
      },
      "effect": {
        "status": "none"
      }
    },
    "p05_title": {
      "policy": "nfc_or_nfd_exact",
      "accepted_title": "CMCP_05_日本語_é_🎹",
      "unicode_scalar_count": 15,
      "utf8_byte_length": 25,
      "setup_variance": false
    },
    "p09_title": {
      "policy": "fixed_name_prefix",
      "accepted_title": "CMCP_09_LONG_ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNO",
      "unicode_scalar_count": 80,
      "utf8_byte_length": 80,
      "setup_variance": false
    }
  },
  "optional_o1": {
    "status": "skipped",
    "reason": "not_separately_authorized"
  },
  "annotations": [
    {
      "checkpoint_id": "<required-checkpoint-id>",
      "result": "observed",
      "ui_ground_truth_confirmed": true,
      "action_confirmed": true
    }
  ]
}
```

Cubase 13 runは`c13_mixer_bank`、Cubase 15でMixer BankとDirectAccessを同時観測するrunは`c15_combined`を使います。`c13_mixer_bank`というprofile名はDirectAccess非対応を意味せず、INITのruntime capabilityがsupported / activeなら同じ44 checkpointへDirectAccess projectionも条件付きで必須化します。`run_started_at`はtimezone offsetとミリ秒を含むRFC 3339で記録し、そのUnix millisecond値がraw `collector_started.timestamp_unix_ms`と完全一致しなければなりません。最初のraw recordを受け取った後、その値からsidecarへ転記し、目視時刻やcollector起動commandの実行時刻で代用しません。primary runのMixConsole surfaceは`separate`固定で、baseline中のvisibility syncは`on`、終了時はrun前の`on` / `off` / `not_open`へ復元します。filter配列と順序は上例へ固定し、C13の`main_filter`は`implicit`、C15は`explicit`です。auditorはINITのcapability responseにあるbank幅、config ID、main filter、DirectAccess能力と照合します。

`fixture_acceptance`はUIが実際に受理したfixture値をraw snapshotへ結び付けます。InstrumentとEffectを別々に記録し、plug-in不要なら各`status = "none"`、作成に代替が必要だった側だけ`status = "used"`としてboundedな`accepted_name`をlocal sidecarへ記録します。両方が必要なら異なる2つの名前をそれぞれ保持し、1つへ連結しません。P05の`accepted_title`は上記固定NFCまたはその固定NFD表現だけ、P09は予約marker `CMCP_09_LONG_`を完全に含む上記80文字のprefixだけを許し、scalar / byte数と`setup_variance`を実値から検証します。auditorは対象Trackが含まれるfinal snapshotのtitleと照合し、欠落やhost側の表現差はintegrity failureへ変換せずsemantic comparisonへ明示します。redacted reportは代替plug-in名を出力せず、固定fixtureから導出できるallowlist titleだけを共有します。

`annotations`は自由記述の作業メモではなく必須UI確認sidecarです。各entryは同名のraw checkpointと`collector_action` markerへexactly once対応し、marker直後に指定操作だけを行い、UI上でこの文書の件数、順序、名前、state、visibilityを確認できた場合だけ`observed / true / true`にします。確認不能、差異、操作ミスは成功値へ変更せずrunを停止し、再現runを作ります。R1 / R2のaction時刻はmanifestへ手入力せず、raw markerのclockを唯一の基準にします。

revision 2の両profileで必須annotation / raw checkpointは次のexactly 44 IDです。以下の列挙順をchronological begin順として固定し、各checkpointを前の`end`後にだけ開始します。同じ集合でも並べ替え、重複、overlapしたrunは受理しません。

- setup / initial cases: `INIT`、`E0`、`E1`
- E8 project open / ground truth: `E8`
- E8 exact-width: `E8-MB_CORE_ALL-B0-reset`、`E8-MB_CORE_ALL-B1-next`、`E8-MB_CORE_ALL-B2-prev`、`E8-MB_CORE_ALL-B3-reset`と、同じ4 suffixの`E8-MB_CORE_VISIBLE-*`
- C1 baseline / bank-width-exceeded: `C1`、続けて`C1-MB_CORE_ALL-B0-reset`、`B1-next`、`B2-next`、`B3-extra-next`、`B4-prev`、`B5-reset`と、同じ6 suffixの`C1-MB_CORE_VISIBLE-*`
- mutation / project switch: `S0`、`S1-select`、`S1-rename`、`S2-select-anchor`、`S2-add`、`S3-select-delete`、`S3-delete`、`S4-show`、`S5-select-anchor`、`S5-select-change`、`S6-mute`、`S7-solo`、`S8-project-only-hide`、`S8-restore`、`S9-empty`、`S9-mutation`、`S9-baseline`
- lifecycle: `R1`、`R2`

profile名はcheckpoint IDへ重ねて付けず、manifestの`profile`でnamespaceを分離します。必須`E8` checkpointでprojectを開いてcase-level UI ground truthを確認し、続く8個のnavigation checkpointでも各境界状態を確認します。E8 / C1 navigationは操作対象configを、その他のcheckpointは`MB_CORE_ALL`と`MB_CORE_VISIBLE`の両方をsnapshotにします。C15 combined profile、およびDirectAccessがruntimeでsupported / activeだったC13 profileでは、44個をMixer Bank用とDirectAccess用に複製せず、各同一checkpoint内でDirectAccessの明示的final snapshotを先に、必要なbank snapshotをその後に取得します。

APIが提供しないfieldはraw logとredacted reportの双方で`null`または`not available`とし、直前値で埋めません。callbackの受信順を保ち、同一timestampへ並べ替えません。raw host IDはlocal JSONLから外へ出さず、redacted reportではrun-local alias、byte length、必要な場合だけSHA-256 digestで同一性を表現します。名前、index、typeからIDを生成してはいけません。

redacted reportでは最後の監査対象snapshotごとに、Mixer Bank slot順 / DirectAccess root-child index順、allowlistへ一致した合成title、nullable state、run-local host ID alias、missing、duplicate、scope外またはunknown / redacted件数を別々に集計します。wire / capability上のDirectAccess snapshotは`projection = mix_console_root_children_v1`、`scope_depth = 1`、`scope_complete = true`、`host_graph_complete = false`、`authoritative_for_track_enumeration = false`を固定し、`root_child_count`と全indexを照合します。sanitized reportではprojection種別tagを`projection = direct_access`、その具体的なDirectAccess契約を`scope = mix_console_root_children_v1`として別fieldに保持し、この2つを同名fieldへ潰しません。root observationだけが`children_expanded = true`、root-child observationは`children_expanded = false`であり、後者をleafの証拠へ読み替えません。repeat edgeはsource / targetのrun-local object alias、child index、depth 1、`ancestor_cycle` / `shared_reference`分類だけを出し、raw numeric object IDやambient文字列を共有しません。`observation_items`は初出object、`reference_items`はrepeat edgeとして別集計し、root observation 1件と`0..root_child_count-1`の全coordinateがobservationまたはreferenceでexactly once覆われた場合だけroot-child scopeをcompleteとします。Mixer Bank callback値はProbeがcallback発生時に刻んだfield別`observation_epoch`がそのcheckpointのcut responseと一致した場合だけ`fresh`とし、cut前のqueueが後からflushされても`fresh`へ繰り上げません。旧epochや未観測のfieldは`stale` / `not available`として値を`null`にし、source側redactionとは別に集計します。DirectAccessのcallbackとlive snapshotも各recordのepoch / statusを検証します。raw JSONL / manifestのSHA-256で同じrunへ固定し、同じ環境の別runで穴埋めしません。`UNSUPPORTED`とtimeout等による`INCONCLUSIVE`を`PASS`へ含めません。snapshot内容が期待と異なっても証拠stream自体が完全ならauditorのmachine-readable integrity `status`は`evidence_valid`、`semantic_assessment`は`observed_not_evaluated`になり得ますが、semantic projectionの差分を隠して機能上の`PASS`へ読み替えてはいけません。

## 再現性チェックリスト

実機run前に次を確認します。

- [ ] run情報に正確なOS、Cubase version/build、MIDI Remote APIを記録した
- [ ] audit manifest v1の`fixture_revision`が`2`で、raw JSONLと`run_id`が一致する
- [ ] DirectAccessがactiveなら、capabilityとsnapshotが`mix_console_root_children_v1` / `scope_depth = 1` / non-authoritativeで一致し、rootだけが`children_expanded = true`、全root-child indexがobservationまたはreferenceでexactly once覆われ、target ordinal、root self/shared分類、最大129 semantic record、fragmentを含む全count式をauditorが検証した
- [ ] 対象versionで新規作成したINIT bootstrapが正式E0とは別fileで、0 Project Trackかつmedia、event、part、automation、plug-in、user preset、新しいrouting設定を持たず、run前SHA-256をlocalに固定した
- [ ] exactなCubase application名またはbundle pathとbootstrap absolute pathをrun前にlocalで確定し、INIT action直後の同じOS launch操作へ両方をquoteして指定した。Hub先行起動やprimary INIT中のbootstrap後付け救済を行っていない
- [ ] INIT annotationはexact bootstrap basename、0 Project Track、他projectなし、dirty / modified表示なしをUI確認しており、full lifecycleがexactly 1回で、追加loaded、ready(false)、2回目のmapping / capability / page-activate snapshot set / ready、別source activationがない
- [ ] E0のProject Trackが0本である
- [ ] E8がAudio Track 8本だけで、E8-01〜E8-08の順・state・visibilityが一致する
- [ ] E8の両primary configでReset / Next / Prev / Resetを独立windowとして観測した
- [ ] C1が20本で、P01〜P20の順番・type・Cubaseが受理したUI ground truth名が一致する
- [ ] E1がAudio Track 1本だけである
- [ ] P02/P03が同名で、MuteとSoloが別々に設定されている
- [ ] P13〜P20がselected / mute / soloの8通りを1回ずつ表している
- [ ] Unicode名と80文字のASCII要求名について、要求値とCubaseが受理した実値・長さを記録した
- [ ] P01 Folderが空で、Group / Effect Trackがproject直下にある
- [ ] P08だけがhiddenで、P17〜P20だけがselected、P20が最後に選択したTrackである
- [ ] Record Enable、Monitor、Automation Writeが全てoffである
- [ ] event、part、automation、imported mediaがない
- [ ] 8-slot bankを超えるTrack数がある
- [ ] 全44 checkpointでexactly 1 action markerを記録し、cut対象21 checkpointでは成功response直後のatomic output pairに同request / epochの自動markerがあり、別のmanual markerを送らず、そのmarker直後にUI操作を行い、checkpoint種別ごとの観測anchorから5000 ms以上後に各access projectionの明示的final snapshotを取得した
- [ ] input guardの`ready`が`version = 4` / `source = hid_system_state` / `coverage = action_windows` / `privacy = counts_and_held_state_boolean` / `policy = consequential_input_only`と一致し、全recordのversion / coverage / policy / session identity / record sequence、inventoryとarmed / resultのID列、全resultの15 consequential deltaがそれぞれ一致する
- [ ] exact automation primitiveのnegative control、physical move-only acceptance、physical click / key / scroll / drag rejectionをcalibrationし、各UI操作のfresh pre-state、target-bound exactly 1 call、action-specific fresh postconditionを記録して、対象外への作用または誤clickの疑いがあるcheckpointを再試行や成功推測で救済していない
- [ ] final snapshot完了後にprobe messageが1件もなく、追加1000 msのquiet periodを満たした
- [ ] Mixer Bank configでchannel typeとleft / right zoneを明示し、main filter capabilityとimplicit / explicit scopeを記録した
- [ ] Mixer Bank completenessはcore MixConsole channelに限定し、DirectAccessの`scope_complete`をhost graph completeへ拡張せず、Project-wide Folder completenessは未証明として残した
- [ ] mutationはM1 copyで行い、C1 baselineを変更していない
- [ ] audit v1ではO1を実施せず、`skipped / not_separately_authorized`を記録した
- [ ] primary runでseparate MixConsoleを使い、visibility同期をrun前の状態へ戻し、optional bus inventoryへ触れていない
- [ ] cleanupでINIT bootstrapが閉じており、run後SHA-256がrun前SHA-256と一致し、bootstrapを変更・保存していない
- [ ] redacted reportの`run-<16-hex>` alias、raw JSONL digest、manifest digest、semantic projectionを保存した
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
