# ロードマップ

## 基盤

- [x] Cargo workspaceを作る
- [x] CoreのFrame / 正規化座標 / ポートを定義する
- [x] eguiアプリの起動骨格を作る
- [x] Ubuntuの通常CIと手動のWindows / macOS確認を作る
- [x] ログと設定ファイルの形式を決める
- [x] Workerの起動 / 停止を管理するruntimeを作る

## Android映像

- [x] ADB端末の列挙と選択を作る
- [x] scrcpy-serverのバージョンと配布方法を決める
- [x] serverの転送 / 起動 / 停止を作る
- [x] video socketを受信する
- [x] H.264をFFmpegでデコードする
- [x] 最新フレームをeguiへ表示する

## 入力

- [x] ADB tap / swipe / keyを作る
- [x] 入力queueと停止保証を作る
- [x] 正規化座標から端末座標への変換を検証する
- [ ] scrcpy controlを追加するか評価する

## 認識

- [x] 保存画像のVideoSourceを作る
- [x] ROIと矩形型をCoreへ追加する
- [x] テンプレートマッチングを作る
- [x] 検出overlayをeguiへ表示する
- [x] 録画から抽出したFrame列を使う回帰テストを作る

## 汎用自動化

- [x] TOMLのAutomationProfileを作る
- [x] Condition / Action / Rule engineを作る
- [x] priority / cooldown / consumeを作る
- [x] 1 tickにつき入力を1件へ制限する
- [x] profile内で複数templateを定義できるようにする
- [x] runtimeの認識結果と入力queueへ接続する
- [x] GUIでprofileの選択 / 開始 / 停止を操作できるようにする
- [x] dry-runを作る
- [x] 保存Frame列を使うオフライン実行を作る
- [x] profileとassetを実機なしで検証できるようにする
- [x] rule editorを作る
- [x] 実行履歴をJSONLへ保存できるようにする
- [x] 実行履歴をGUIで絞り込み表示できるようにする

## タスク作成GUI

実装内容と受入条件は[作業指示書](task-studio/implementation-plan.md)を参照してください。

- [x] 画面への配置 / 操作記録 / 状態モデル / OCRの要件と設計をまとめる
- [ ] 映像接続と自動化開始を分離する
- [ ] 入力座標系とFrame / 観測の対応を整備する
- [ ] v2文書とプロジェクト保存 / 復旧を作る
- [ ] タップ点 / スワイプ矢印 / 画像領域を画面上で編集する
- [ ] 状態 / 遷移 / 観測項目 / 変数を編集する
- [ ] OCRで数値と文字を読み取り、値の品質と有効期間を扱う
- [ ] 入力完了を待つ順序付きタスクと待機 / 分岐 / 繰り返しを作る
- [ ] 実操作を記録してタスクへ変換する
- [ ] ドライラン / 単一手順の試行 / オフライン検証を統合する
- [ ] 実画像のOCR評価と3OSの実機確認を行う

## ゲーム別拡張

- [x] game-api crateを作る
- [x] GameState / Trigger / Task / Dispatcherを作る
- [x] 複数ゲームを登録するGameRegistryを作る
- [x] 1 tickにつき入力を1件へ制限する
- [ ] 汎用自動化の縦切りが完了するまで追加実装を保留する
- [ ] 例示用ゲームプラグインを作る
- [ ] 設定 / asset / taskの読み込みを作る
- [ ] 復旧Triggerと優先度制御を作る

## 配布

- [ ] 3OSのパッケージ形式を決める
- [ ] 外部ライブラリとscrcpyのライセンス表示を作る
- [ ] 署名 / notarization / Windows signingを検討する
- [ ] リリースartifactをCIで作る
