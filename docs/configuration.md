# 設定

アプリは起動したディレクトリの`better-e7.toml`を読み込みます。ファイルが存在しない場合は初期値で作成します。

```toml
adb_path = "adb"
ffmpeg_path = "ffmpeg"
device_refresh_interval_ms = 2000
scrcpy_server_path = "third_party/scrcpy/scrcpy-server-v4.1"
scrcpy_local_port = 27183
scrcpy_max_size = 1920
# automation_profile_path = "automation.toml"
automation_dry_run = false
recognition_threshold = 0.9
# recognition_template_path = "assets/confirm.png"
```

## adb_path

ADB実行ファイルのパスです。PATHから実行できる場合は`adb`のまま使えます。PATHにない場合は絶対パスを指定します。

Windowsでは次のようにバックスラッシュを重ねるか、スラッシュを使います。

```toml
adb_path = "C:/Android/platform-tools/adb.exe"
```

## device_refresh_interval_ms

端末一覧を更新する間隔です。単位はミリ秒で、最小値は250です。通常は初期値の2000で十分です。

未知の設定項目や不正な値がある場合は設定を採用せず、初期値で起動してエラーをログへ出します。

## ffmpeg_path

FFmpeg実行ファイルのパスです。PATHから実行できる場合は`ffmpeg`のまま使えます。PATHにない場合は絶対パスを指定します。初期実装はH.264を標準入力へ渡し、標準出力のPPM streamをRGB Frameへ変換します。

## scrcpy_server_path

固定対象のscrcpy-server v4.1を置くパスです。配布元とchecksumは`third_party/scrcpy/README.md`にあります。

## scrcpy_local_port

ADB forwardでPC側に割り当てるTCP portです。ほかのprocessが使っている場合は変更します。

## scrcpy_max_size

Androidから送る映像の最大辺です。初期値は1920です。0を指定するとscrcpy側でサイズを制限しません。

## recognition_template_path

検出するtemplate画像のパスです。PNGとJPEGを読み込めます。未指定の場合は認識workerを起動せず、映像previewと入力だけを動かします。相対パスはアプリを起動したディレクトリから解決されます。

検出labelにはファイル名から拡張子を除いた部分を使います。たとえば`assets/confirm.png`は`confirm`になります。

## automation_profile_path

汎用自動化profileのパスです。相対パスはアプリを起動したdirectoryから解決されます。profile内のtemplate画像はprofileファイルがあるdirectoryを基準に解決されます。

指定した場合はprofile内の複数templateとRule engineを使い、`recognition_template_path` / `recognition_threshold`による単一template設定より優先します。未指定の場合は従来の単一template認識をそのまま使えます。

停止中であればGUIから別のprofile pathを入力して読み直せます。読み込みと検証に成功した場合だけ`better-e7.toml`へ保存します。
起動時のprofileが読めない場合もADB端末の監視とGUIは起動するため、タスク欄からpathを修正できます。

## automation_dry_run

`true`にするとRuleの認識 / priority / cooldown / logを通常どおり実行しますが、Ruleが生成した入力はAndroidへ送りません。予定していたRuleと入力はGUIの状態欄とログへ表示します。previewのclickやHome / Backなどの手動入力には影響しません。

## recognition_threshold

templateの一致率に対するしきい値です。0.0から1.0の範囲で指定し、初期値は0.9です。誤検出がある場合は上げ、画像圧縮や色の差で検出できない場合は少し下げます。
