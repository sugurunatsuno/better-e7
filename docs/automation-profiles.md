# AutomationProfile

AutomationProfileは認識結果から入力を生成する汎用的なTOML設定です。ゲームタイトルやADBの実装を含みません。

最小のprofileは次の形式です。

```toml
name = "confirm-buttons"

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

座標は0.0から1.0で指定します。不正な座標 / 重複ID / 空のlabel / 空のCondition groupはprofileの読み込み時に拒否します。

現時点ではloaderとengineまで実装しています。runtime / GUIとの接続、複数templateの読み込み、editor / dry-runは次の実装範囲です。
