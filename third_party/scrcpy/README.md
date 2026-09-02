# scrcpy-server

better-e7はscrcpy-server v4.1へ固定します。scrcpyのclientとserverは同じversionでなければならないため、更新時はAndroid transportとテストを同時に更新します。

## 配布元

- Version: 4.1
- URL: https://github.com/Genymobile/scrcpy/releases/download/v4.1/scrcpy-server-v4.1
- SHA-256: `deacb991ed2509715160ffdc7907e47b4160eb30d1566217e9047fd5b8850cae`
- License: Apache License 2.0

server binaryはリポジトリへ含めません。上のURLから取得し、このディレクトリへ`scrcpy-server-v4.1`として置きます。配布packageへ同梱するときはchecksum検証とライセンス表示をbuild処理へ追加します。
