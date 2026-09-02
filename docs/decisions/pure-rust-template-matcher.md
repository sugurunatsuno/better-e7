# pure Rustのtemplate matcherから始める

## 状態

採用

## 背景

最初の認識縦切りでは、保存画像とAndroid映像の両方から同じ`Recognizer`を呼び、検出結果をeguiへ表示する必要があります。通常CIはUbuntuだけで実行し、Android端末やnative画像処理libraryを必要としない構成を維持します。

## 判断

最初のtemplate matcherはpure Rustで実装します。RGB8の絶対差から一致率を計算し、正規化ROI内を粗く探索した後、最良候補の周辺を1pixel単位で再探索します。PNG / JPEGの読み込みだけに`image` crateを使います。

認識APIは`better-e7-core`の`Recognizer` traitに置き、検出結果は外部libraryの型を含まない`Detection`として返します。runtimeでは映像workerと認識workerを分け、未処理Frameを常に最新の1枚へ置き換えます。

## 結果

- Android / FFmpeg / OpenCVなしで生成画像によるテストができます
- 3OSで同じ実装を使えます
- 高速な特徴量や複数scaleが必要になった場合はbackendを交換できます
- 初期実装は回転 / scale変化 / 大きな色変化を吸収しません

OpenCVやONNX Runtimeは性能測定と要件が揃ってから、`Recognizer`境界の内側へ追加します。
