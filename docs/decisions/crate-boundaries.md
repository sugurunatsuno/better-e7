# 外部ライブラリを専用crateへ隔離する

状態: 採用

## 背景

FFmpeg / OpenCV / ONNX Runtime / ADB / scrcpyは更新頻度と配布条件が異なります。これらの型がゲームコードまで広がると交換やテストが難しくなります。

## 判断

CoreはRust標準ライブラリだけに依存します。外部ライブラリはvideo / vision / androidの専用crateで扱い、CoreのFrameとtraitを通して利用します。

## 影響

- 模擬実装で単体テストできる
- FFIのunsafeを専用crateで監査できる
- 変換処理が増える可能性がある
- 性能問題が計測された場合だけzero-copyの境界を再設計する

