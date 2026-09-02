# AutomationProfile

AutomationProfileは認識結果から入力を生成する汎用的なTOML設定です。ゲームタイトルやADBの実装を含みません。

最小のprofileは次の形式です。

```toml
name = "confirm-buttons"

[[templates]]
id = "confirm"
path = "assets/confirm.png"
threshold = 0.9

[templates.region]
left = 0.0
top = 0.0
right = 1.0
bottom = 1.0

[[rules]]
id = "confirm"
priority = 100
cooldown_ms = 1000
consume = true

[rules.condition]
type = "detection_present"
label = "confirm"
minimum_confidence = 0.9

[rules.action]
type = "tap_detection"
label = "confirm"
```

完全な例は`automation.example.toml`にあります。

## Template

| 項目 | 内容 | 初期値 |
|---|---|---|
| id | 検出結果のlabelとして使うprofile内で重複しないID | 必須 |
| path | PNGまたはJPEGのtemplate画像 | 必須 |
| threshold | 一致率のしきい値 | 0.9 |
| region | 認識する正規化ROI | 画面全体 |

`path`の相対パスはprofileファイルがあるdirectoryを基準に解決します。複数の`[[templates]]`を定義すると、同じFrameに対してすべてを認識し、検出結果をまとめてRule engineへ渡します。

## Rule

| 項目 | 内容 | 初期値 |
|---|---|---|
| id | profile内で重複しないID | 必須 |
| enabled | Ruleを評価するか | true |
| priority | 評価順。大きい値を先に評価 | 0 |
| cooldown_ms | 前回実行後に待つ時間 | 0 |
| consume | 実行後に低priorityのRuleを止めるか | true |
| condition | 実行条件 | 必須 |
| action | 条件一致時の処理 | 必須 |

入力Actionを生成した場合は`consume`に関係なくtickを終了します。これにより1つの認識結果から複数の入力を同時に作りません。

## Condition

| type | 内容 |
|---|---|
| always | 常に一致する |
| detection_present | 指定labelがしきい値以上で存在する |
| detection_absent | 指定labelがしきい値以上で存在しない |
| all | 内部Conditionがすべて一致する |
| any | 内部Conditionのどれかが一致する |
| not | 内部Conditionの結果を反転する |

`minimum_confidence`は0.0から1.0で指定します。初期値は0.9です。

## Action

| type | 内容 |
|---|---|
| tap_detection | 指定labelで最もconfidenceが高い検出の中心をtapする |
| tap | 正規化した固定座標をtapする |
| swipe | 正規化した始点から終点へswipeする |
| key | Android key codeを送る |
| log | 入力せずmessageを記録する |

座標は0.0から1.0で指定します。不正な座標 / 重複ID / 空のlabel / 空のCondition group / 未定義templateの参照はprofileの読み込み時に拒否します。

## Runtimeでの実行

`better-e7.toml`の`automation_profile_path`へprofileを指定します。開始時にengineのcooldown状態をresetし、認識結果ごとに1回tickします。生成された入力は手動入力と同じ順序付きqueueを通り、GUIには実行Rule / profileのlog / 最後に実行したRuleを表示します。

GUIでは停止中にprofile pathを変更できます。dry-runを有効にすると入力をqueueへ送らず、予定入力だけを表示します。

profileを検証を押すと、TOMLの構造、Ruleの参照、template assetの読み込みをAndroidへ接続せずに確認します。成功時はprofile名とtemplate / Ruleの件数を表示し、失敗時は対象pathを含むエラーをログへ表示します。

rule editorではTemplateのID / 画像path / threshold / ROIと、RuleのCondition / Action / priority / cooldown / consumeを編集できます。Conditionはall / any / notを入れ子にできます。検証して保存を押すと、参照とassetの読み込みに成功した場合だけTOMLを保存します。保存後に実行内容を変更する場合は、profileを読み込むを押します。

profileにtemplateがなくてもRule engineは動きます。この構成は`always`や固定座標のActionをmock Frameで検証するときに使えます。rule editorは今後の実装範囲です。

`automation_history_path`を設定すると、Ruleの発火と入力の処理結果をJSONLへ追記します。dry-runでは`input_planned`、通常実行では`input_queued`として区別します。

履歴を表示を押すと、最新10000件をGUIへ読み込みます。profile / Rule / eventのfilterを組み合わせて、必要なrecordだけを確認できます。再読み込みを押すと実行中に追記されたrecordも反映します。

## オフライン実行

GUIのオフライン実行へ録画から抽出したFrameのdirectoryを指定すると、PNG / JPEGを名前順に読みます。読み込んだFrameは通常実行と同じtemplate認識とRule engineを通りますが、Android入力は行わず、Actionは予定入力として表示します。

profileとFrameのdirectoryを入力して保存Frameを実行を押します。処理中は同じ場所の停止で中断できます。`automation_history_path`が設定されている場合は、Ruleと予定入力も通常のJSONLへ保存します。
