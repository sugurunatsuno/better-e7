# タスク作成GUIのデータ形式

状態: 新しいv2形式の設計。現行developは以下のv2を読み込めません。例は設計確認用であり、現時点で配布する実行サンプルではありません。

## 保存単位

| ファイル | 内容 |
|---|---|
| `automation.toml` | 実行定義。テンプレート、状態、観測項目、変数、タスク、遷移 |
| `.studio/editor/<revision>.toml` | そのrevisionの状態カード位置、選択、ズーム、手順と参照画面の対応 |
| `assets/templates/*.png` | 実行に必要なテンプレート |
| `assets/references/*.png` | 編集時に使う参照画面 |
| `.studio/recovery/` | 未保存の下書きとその画像 |
| `history/*.jsonl` | 任意の実行履歴 |

画像pathはプロジェクトからの相対pathにします。v2では絶対path、上位へ抜けるpath、symlinkを解決した結果がプロジェクト外になるpathを拒否します。画像を外部から取り込む場合は内部へコピーします。旧v1の絶対path対応は維持します。

実行には`automation.toml`とその実行assetだけを必要とします。編集用ファイルが欠落した場合は図の配置を再生成し、参照画像を未設定として表示します。実行条件や図形の実座標を編集用ファイルだけに保存してはいけません。

## バージョンと互換性

- `schema_version`のない既存ファイルはv1として読む。v1の型とRule engineの意味は変えない。
- `schema_version = 2`は別の`AutomationProfileV2`へ読む。最初にversionを調べてから厳密な型へdeserializeする。
- 未対応version、未知のtype、未知フィールドはpath付きエラーとして拒否する。未知項目を読み飛ばして保存時に失わないようにする。
- v1を開いて従来Ruleだけを編集した場合はv1で保存する。v2の機能を追加する場合は移行内容を表示し、元ファイルを残して新しいプロジェクトへ保存する。
- 移行ではtemplates / rulesをそのまま取り込む。Ruleのpriority順をタスク順へ変換しない。移行後もRulesまたは特定Taskを明示的に選んで実行する。
- v2で`rules`を保持する場合も、既存Condition / Actionの形式をそのまま使う。TaskのPredicateとは型を分ける。
- 保存と再読込で安定ID、型付き値、順序、領域、条件が一致する。コメントの保持は初期保証に含めず、GUI生成ファイルであることを示す。

## ルートと定義

| 型 | フィールドと制約 |
|---|---|
| AutomationProfileV2 | schema_version=2、project_id、revision、name、任意のeditor_metadata、reference_display、templates、rules、state_model、observations、variables、tasks、transitions |
| ReferenceDisplay | width、height、rotation。テンプレートを作成した映像のサイズと向き。画面全体を基準とする |
| TemplateDefinition | 既存のid / path / threshold / regionを再利用 |
| StateModel | unknown_state=`unknown`、default_stable_ms=300、lost_after_ms=500、states |
| ScreenState | id、name、priority=0、stable_ms、省略可能なreference_image、condition。conditionは既存の画像Condition。stable_ms省略時はStateModelのdefault_stable_msを使う |
| ObservationDefinition | id、name、data_type、source、active_states、interval_ms=500、ttl_ms=2000、minimum_quality=80、必要に応じてmin / max / choices |
| VariableDefinition | id、name、data_type、initial、必要に応じてmin / max / choices。runごとに初期化 |
| TaskDefinition | id、name、enabled=true、start_when、start_timeout_ms=10000、max_duration_ms=600000、任意のexpected_state、end_timeout_ms=10000、steps |
| TransitionDefinition | id、name、from_state、to_state、task_id。双方の状態とタスクの参照が必要 |

各collectionの省略値は空です。state_modelの省略はunknownのみのモデルです。transitionsが空なら状態の変化は観察だけに使い、1件以上なら未定義の遷移で実行中タスクを一時停止します。reference_displayは画像や座標付き手順を使うv2プロジェクトで必須です。widthとheightは1以上、rotationは0 / 90 / 180 / 270を受け付けます。

IDは既存規則と同じ小文字ASCII / 数字 / `-_.`を使います。表示名は日本語を使えます。IDは生成後に名前を変えても維持します。step IDはプロジェクト内で一意、他のIDはcollection内で一意とします。`unknown`は画面状態の予約IDです。

画面状態のconditionはAlways / DetectionPresent / DetectionAbsent / All / Any / Notを再利用します。ただし通常状態でAlwaysだけの条件は拒否し、不在条件だけの場合は広く一致する旨を警告します。空のgroupを拒否し、テンプレート参照を検証します。

## 型付き値と観測ソース

値は`{ kind = "integer", value = 123 }`のように型を持たせます。kindはboolean / integer / string / enumです。integerはi64、stringは4096文字以下、enumのvalueは宣言済みchoicesの1つです。浮動小数、暗黙の型変換、任意のobjectは初期対象外です。

| source.type | 内容 |
|---|---|
| template_presence | labelとminimum_confidenceを持つ。結果型はboolean。正常に認識した画像で不在ならfalse |
| ocr | region、language、page_mode、preprocess、parserを持つ。integer / string / enumへ変換 |

`active_states`が空なら全画面、指定があればvalidな現在状態が一致する場合だけ読みます。StateEstimatorはobservationsを参照しないため循環しません。観測項目同士の参照も初期対象外です。

OCRの`page_mode`はsingle_line / single_word / block、初期値はsingle_lineです。`preprocess`はscale=1 / grayscale=false / invert=false / optional thresholdを持ち、scaleは1〜4、thresholdは0〜255に制限します。

| parser.type | 変換 |
|---|---|
| integer | 文字列全体を整数として読む。全角数字 / 指定の桁区切り / 前後の空白だけを正規化 |
| integer_before_separator | 指定separatorで分割した前半を整数にする。区切りは1つだけで、後半も整数として検証 |
| integer_after_separator | 同じ形式の後半を整数にする |
| text | 前後の空白を除去した文字列 |
| enum_text | 正規化後の全文をchoicesと照合する。あいまい一致はしない |

`minimum_quality`はOCRの利用tokenの最小品質0〜100へ適用し、template_presenceには使いません。テンプレートのconfidenceとOCR品質を同じ確率として扱いません。

## Predicate

| type | フィールド | 評価 |
|---|---|---|
| always | なし | True |
| image | condition | 既存画像Conditionを有効な認識snapshotで評価 |
| state_is | state_id | validな現在状態と比較 |
| compare | scope、id、operator、value | observationまたはvariableの型付き値を比較 |
| all / any | predicates | 三値論理でAND / OR |
| not | predicate | True / Falseを反転し、Unknownを維持 |

compareのscopeはobservation / variableです。operatorはeq / ne / lt / lte / gt / gteで、大小比較はintegerだけに許可します。参照先とvalueの型が異なる場合は保存前にエラーにします。式文字列やshellを受け付けません。

無効または期限切れの観測値はUnknownです。現在状態の品質がvalidでなければ、`state_is unknown`を含めてUnknownです。unknownを契機に復旧する機能は初期対象外とし、診断と手動停止に使います。

## 手順の形式

全手順はid、name、enabled=true、typeを持ちます。enabled=falseは実行時に飛ばし、無効な参照も保存検証の対象には残します。各手順のtimeout_msは指定時1〜600000msとします。

| type | 固有フィールド | 完了条件 |
|---|---|---|
| action | action、任意のtimeout_ms | 対応する入力成功、またはlogの発行 |
| delay | duration_ms | 0〜600000msの経過 |
| wait_until | predicate、timeout_ms=10000 | Trueになる |
| if | predicate、timeout_ms=10000、then_steps、else_steps | 真偽が確定して選んだ子手順が完了 |
| repeat | count、steps | 子手順がcount回完了。countは1〜1000 |
| set_variable | variable_id、value | 型と範囲が正しい代入が完了 |
| increment | variable_id、amount | 整数加算が完了。overflowと範囲外は失敗 |

actionは既存Actionのtap / tap_detection / swipe / key / logを再利用します。GUIの長押しは同一点のswipeとして保存します。swipeのduration_msはGUIでは1〜60000msに制限します。

tap_detectionは、その手順を評価する時点の有効なsnapshotから位置を決めます。対象がない場合は手順の期限まで待ちます。既存Ruleのtap_detectionが未検出でエラーになる意味は変えません。

Stepの失敗 / タイムアウト / 入力結果不明はタスク全体をFailedにし、残りを実行しません。暗黙の再試行や失敗時skipは初期形式に含めません。Taskのstart_whenは省略時always、expected_stateがある場合は手順完了後にend_timeout_ms以内で到着を確認します。

## v2の例

ホーム画面でスタミナを読み、メニューを開いて到着を確認する例です。画像は説明用pathで、同梱していません。

```toml
schema_version = 2
project_id = "sample-project"
revision = "rev-a"
name = "メニュー操作"
editor_metadata = ".studio/editor/rev-a.toml"

[reference_display]
width = 1920
height = 1080
rotation = 0

[[templates]]
id = "home-marker"
path = "assets/templates/home-marker.png"
threshold = 0.9

[[templates]]
id = "menu-marker"
path = "assets/templates/menu-marker.png"
threshold = 0.9

[state_model]
unknown_state = "unknown"
default_stable_ms = 300
lost_after_ms = 500

[[state_model.states]]
id = "home"
name = "ホーム"
priority = 10
condition = { type = "detection_present", label = "home-marker", minimum_confidence = 0.9 }

[[state_model.states]]
id = "menu"
name = "メニュー"
priority = 20
condition = { type = "detection_present", label = "menu-marker", minimum_confidence = 0.9 }

[[observations]]
id = "stamina"
name = "スタミナ"
data_type = "integer"
active_states = ["home"]
interval_ms = 500
ttl_ms = 2000
minimum_quality = 80
min = 0
max = 9999

[observations.source]
type = "ocr"
language = "eng"
page_mode = "single_line"
region = { left = 0.70, top = 0.02, right = 0.85, bottom = 0.08 }
preprocess = { scale = 2, grayscale = true, invert = false }
parser = { type = "integer_before_separator", separator = "/" }

[[variables]]
id = "opened-count"
name = "開いた回数"
data_type = "integer"
initial = { kind = "integer", value = 0 }
min = 0
max = 1000

[[tasks]]
id = "open-menu"
name = "メニューを開く"
max_duration_ms = 30000
expected_state = "menu"
end_timeout_ms = 10000

[tasks.start_when]
type = "all"
predicates = [
  { type = "state_is", state_id = "home" },
  { type = "compare", scope = "observation", id = "stamina", operator = "gte", value = { kind = "integer", value = 10 } },
]

[[tasks.steps]]
id = "step-open"
name = "メニューボタンを押す"
type = "action"
action = { type = "tap", x = 0.92, y = 0.12 }

[[tasks.steps]]
id = "step-wait-menu"
name = "メニューが表示されるまで待つ"
type = "wait_until"
timeout_ms = 10000
predicate = { type = "state_is", state_id = "menu" }

[[tasks.steps]]
id = "step-count"
name = "回数を増やす"
type = "increment"
variable_id = "opened-count"
amount = 1

[[transitions]]
id = "home-to-menu"
name = "メニューを開く"
from_state = "home"
to_state = "menu"
task_id = "open-menu"
```

## 編集情報の例

この形式は実行ロジックを持ちません。状態グラフ上の座標は任意のcanvas座標で、Android正規化座標とは混ぜません。

```toml
editor_schema_version = 1
project_id = "sample-project"
document_revision = "rev-a"

[state_positions.home]
x = 100.0
y = 100.0

[state_positions.menu]
x = 100.0
y = 300.0

[step_references.step-open]
path = "assets/references/home.png"
source_width = 1920
source_height = 1080
rotation = 0
```

実機のserial、選択中の端末、liveの値、run ID、process IDは実行定義へ保存しません。画像の元Frame情報や記録した時刻は編集情報へ保存でき、実行条件の代用にはしません。

## 保存前の検証

- version、IDの重複、空の名前、未知のtype、無効な数値、非有限な座標を拒否する。
- 全参照を解決し、状態 / 観測 / 変数 / task / stepの削除による破損を検出する。
- 条件の型、enumの選択肢、整数の範囲、initialと代入値を確認する。
- 画像が存在してdecodeできること、ROIが正の面積を持つこと、テンプレートが認識領域へ収まることを確認する。
- active_statesに存在しない状態を指定できない。OCRの設定を持つプロジェクトでもbackend未導入なら保存は可能だが、OCRの実評価と依存する実行は開始できない。
- taskの最大時間、待機期限、繰り返し回数、深さ、展開後の最大操作数を検証する。
- 遷移に指定したtaskのexpected_stateがto_stateと異なる場合はエラーにする。省略時は遷移の実行要求に到着条件を付ける。
- 正規化座標だけでテンプレートの異なる解像度対応を保証しない。実行時の映像サイズや向きがreference_displayと異なる場合は再検証を要求する。

v2のTOML例は、実装時に新loaderでparse / validate / serialize / parseできるfixtureにします。文書作成時のTOML構文確認は、Rustの新loaderが動くことの検証ではありません。
