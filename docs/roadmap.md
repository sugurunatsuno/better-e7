# ロードマップ

## 基盤

- [x] Cargo workspaceを作る
- [x] CoreのFrame / 正規化座標 / ポートを定義する
- [x] eguiアプリの起動骨格を作る
- [x] Ubuntuの通常CIと手動のWindows / macOS確認を作る
- [ ] ログと設定ファイルの形式を決める
- [ ] Workerの起動 / 停止を管理するruntimeを作る

## Android映像

- [ ] ADB端末の列挙と選択を作る
- [ ] scrcpy-serverのバージョンと配布方法を決める
- [ ] serverの転送 / 起動 / 停止を作る
- [ ] video socketを受信する
- [ ] H.264をFFmpegでデコードする
- [ ] 最新フレームをeguiへ表示する

## 入力

- [ ] ADB tap / swipe / keyを作る
- [ ] 入力queueと停止保証を作る
- [ ] 正規化座標から端末座標への変換を検証する
- [ ] scrcpy controlを追加するか評価する

## 認識

- [ ] 保存画像のVideoSourceを作る
- [ ] ROIと矩形型をCoreへ追加する
- [ ] テンプレートマッチングを作る
- [ ] 検出overlayをeguiへ表示する
- [ ] 録画を使った回帰テストを作る

## ゲーム自動化

- [ ] game-api crateを作る
- [ ] GameState / Trigger / Task / Dispatcherを作る
- [ ] 例示用ゲームプラグインを作る
- [ ] 設定 / asset / taskの読み込みを作る
- [ ] 復旧Triggerと優先度制御を作る

## 配布

- [ ] 3OSのパッケージ形式を決める
- [ ] 外部ライブラリとscrcpyのライセンス表示を作る
- [ ] 署名 / notarization / Windows signingを検討する
- [ ] リリースartifactをCIで作る
