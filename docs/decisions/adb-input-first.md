# 最初の入力backendにADBを使う

## 状態

採用

## 判断

最初の入力backendは`adb shell input`を使います。tap / swipe / keyeventを`AdbInputController`へ閉じ込め、上位層は`InputController`だけを使います。

ゲーム側は正規化座標の`InputCommand`を作ります。runtimeが最新Frameのサイズを使ってpixel座標へ変換し、容量を制限したqueueからADBへ順番に送ります。

## 理由

- Android端末側へ追加アプリを入れずに確認できる
- macOS / Windows / Linuxで同じcommandを使える
- mock runnerでargumentと失敗処理をテストできる
- scrcpyの内部control protocolから入力設計を分離できる

## 影響

ADB processを操作ごとに起動するため、高頻度入力には遅延があります。最初の自動化と安全な停止を確認した後で、必要ならscrcpy control backendを追加します。`InputController`の境界は維持するため、ゲーム側の変更は不要です。
