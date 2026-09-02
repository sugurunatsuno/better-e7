# テスト

Android端末が必要な確認と、通常の自動テストを分けます。

## 通常の自動テスト

- ADBの`devices -l`出力は保存した文字列から解析する
- scrcpy sessionは`ScrcpyBackend`のmockを使う
- video socketはメモリ上のbyte列を使う
- H.264 Annex Bはchunk境界を変えたbyte列で検証する
- push / forward / server起動 / connect / stop / forward削除の順序を検証する
- 接続途中で失敗した場合の後片付けを検証する

このテストはUbuntu CIで実行し、ADB / scrcpy-server / FFmpegを必要としません。

## 実機確認

実機確認では次を確認します。

- GUIにADB端末が表示される
- 選択した端末だけへscrcpy-serverがpushされる
- 開始後に受信量が増える
- 停止後にADB forwardが削除される
- USBを抜いた場合にsessionが停止する

映像表示はFFmpeg decoderを追加した後に確認します。現時点のGUIはH.264の受信量だけを表示します。
