# FFmpegを外部processとして使う

## 状態

採用

## 判断

最初のH.264 decoderは、FFmpeg実行ファイルへ標準入出力で接続する`FfmpegProcessDecoder`として実装します。入力はscrcpy-serverのraw H.264、出力は連結されたP6 PPMです。PPMはRGB8の`Frame`へ変換します。

`VideoDecoder`と`VideoDecoderFactory`を公開境界にし、runtimeやGUIからprocess APIとFFmpeg固有型を隠します。

## 理由

- Rust crateのbuild時にFFmpeg native libraryを要求しない
- macOS / Windows / Linuxで同じprocess境界を使える
- CIではFFmpegやAndroid端末を起動せず、模擬PPM streamでparserを検証できる
- 実測後にFFIやhardware decoderへ交換できる

## 影響

実行環境にはFFmpeg実行ファイルが必要です。PPMは非圧縮なのでprocess間の転送量が大きくなります。最初の縦切りと診断性を優先した判断であり、映像遅延やCPU使用率を測定した後にbackendの変更を評価します。
