# ゲームごとにcompile-time pluginを使う

## 状態

採用

## 背景

複数のゲームタイトルへ対応する場合、状態名 / Trigger / Task / assetはゲームごとに異なります。一方でAndroid接続 / 映像 / 認識 / 入力queueは共通です。ゲーム固有コードからADBやGUIを直接呼ぶと、実機なしのテストとbackend交換が難しくなります。

## 判断

ゲーム固有実装は`GamePlugin`としてcompile時に登録します。`GameRegistry`は安定した`GameId`でpluginを管理し、重複IDを拒否します。動的libraryのloadやplugin downloadは初期対象にしません。

各pluginはゲーム専用の`Dispatcher`を生成します。Dispatcherは`GameState` / priority付き`Trigger` / lifecycleを持つ`Task`を管理します。ゲーム側の処理は`DispatchReport`へ正規化座標の入力意図と状態遷移を返し、ADBを直接実行しません。

1回のtickで出力できる入力は最大1件とします。高priorityのTriggerが`Consume`を返した場合、そのtickでは低priorityのTriggerと通常Taskを実行しません。

## 結果

- ゲームタイトルごとのcrateを独立して追加できます
- Android端末なしで状態遷移と実行順をテストできます
- 入力backendを変更してもゲームコードを変更せずに済みます
- pluginの追加にはアプリの再buildが必要です
- assetとタスク設定の外部読み込みは別の段階で設計します
