# 設定

アプリは起動したディレクトリの`better-e7.toml`を読み込みます。ファイルが存在しない場合は初期値で作成します。

```toml
adb_path = "adb"
ffmpeg_path = "ffmpeg"
device_refresh_interval_ms = 2000
scrcpy_server_path = "third_party/scrcpy/scrcpy-server-v4.1"
scrcpy_local_port = 27183
scrcpy_max_size = 1920
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
