# 開発への参加

## 変更の単位

- CoreはOS / GUI / ADB / scrcpy / OpenCVの具象型へ依存させない
- OS固有処理は専用crateへ閉じ込める
- 新しい外部依存を追加するときは、必要性と代替案をADRへ記録する
- 認識処理には保存フレームを使った再現テストを用意する

## 提出前の確認

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

機能変更では、対応する要件またはADRも更新してください。

