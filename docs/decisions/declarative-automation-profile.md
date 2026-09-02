# 汎用自動化を宣言型profileで表す

## 状態

採用

## 背景

ゲームタイトルごとにRustコードを追加すると、似た認識条件 / tap / cooldownを何度も実装することになります。一般的な画面自動化は、認識labelと入力の組み合わせで表現できます。

## 判断

ゲーム固有実装より先に、TOMLの`AutomationProfile`を実装します。Profileはpriority付きRuleを持ち、RuleはCondition / Action / cooldown / consumeで構成します。

Conditionは認識結果だけを参照します。Actionは`InputCommand`またはlogへ変換し、ADBやscrcpyを直接呼びません。1回のtickでは入力を最大1件だけ返します。時間はengineの外から単調増加値として渡します。

game-apiは削除しません。長い状態遷移や手続き的なTaskが必要な場合の拡張先として残します。ただし、まず汎用profileへ機能を追加できないか確認します。

## 結果

- Rustを変更せず自動化Ruleを追加できます
- 同じRule engineを複数の用途で使えます
- cooldownをsleepなしでテストできます
- profileの誤りを実行前に検出できます
- runtime / GUIへ接続するまではprofileだけで端末を操作できません
