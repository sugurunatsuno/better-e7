# テスト

Android端末が必要な確認と、通常の自動テストを分けます。

## 通常の自動テスト

- ADBの`devices -l`出力は保存した文字列から解析する
- scrcpy sessionは`ScrcpyBackend`のmockを使う
- video socketはメモリ上のbyte列を使う
- H.264 Annex Bはchunk境界を変えたbyte列で検証する
- push / forward / server起動 / connect / stop / forward削除の順序を検証する
- 接続途中で失敗した場合の後片付けを検証する
- 正規化座標からpixel座標への変換を検証する
- ADB tap / swipe / keyのargumentをmock runnerで検証する
- 入力停止時に未実行のqueueが破棄されることを検証する
- 保存画像を`VideoSource`として1回だけ取得できることを検証する
- 生成したRGB画像からtemplateの位置と正規化矩形を検出する
- ROI外の一致を検出しないことを検証する
- 認識中に複数Frameが届いた場合、未処理Frameを最新のものへ置き換えることを検証する

このテストはUbuntu CIで実行し、ADB / scrcpy-server / FFmpegを必要としません。

## 実機確認

実機確認では次を確認します。

- GUIにADB端末が表示される
- 選択した端末だけへscrcpy-serverがpushされる
- 開始後に受信量が増える
- Android映像がpreviewへ表示される
- previewをclickした位置が端末でtapされる
- Home / Backボタンが選択した端末だけへ送られる
- 上へswipeボタンで下から上へのswipeが送られる
- 停止後にADB forwardが削除される
- 停止後に入力が送信されない
- USBを抜いた場合にsessionが停止する
- `recognition_template_path`を指定すると一致箇所にlabel / confidence / 矩形が表示される
