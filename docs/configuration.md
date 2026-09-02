# 設定

アプリは起動したディレクトリの`better-e7.toml`を読み込みます。ファイルが存在しない場合は初期値で作成します。

```toml
adb_path = "adb"
device_refresh_interval_ms = 2000
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
