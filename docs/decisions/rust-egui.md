# Rustとeguiを採用する

状態: 採用

## 背景

macOSだけではなくWindows / Linuxでも動くAndroid自動化アプリが必要です。GUI / 通信 / 自動化ロジックを可能な限り同じ言語で保守したいという条件があります。

## 判断

Rust 2024を主言語にして、デスクトップGUIにはegui / eframeを使います。並行I/Oへ着手するときはTokioを追加します。

## 理由

- 3OSで同じCargo workspaceを使える
- 所有権と型でFrameやWorkerの寿命を明確にできる
- eguiは映像 / overlay / ログを同じ画面へ置く開発ツール型UIと相性がよい
- C APIを持つFFmpeg / OpenCV / ONNX Runtimeと接続できる

## 影響

- GUIの複雑なネイティブ操作感より、開発速度とデバッグ機能を優先する
- FFI依存は専用crateへ隔離する
- eframeの更新はGUI crateだけで吸収する

