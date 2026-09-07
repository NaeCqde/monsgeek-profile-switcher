# FUN68 のプロファイル切替プロトコル

実装は [hid.rs](../src/hid.rs) です。

## レポート

公式ドライバ（V4）は、キーボードのベンダー設定用 HID インターフェース
（Usage Page `0xFFFF` / Usage `0x02`、有線では通常インターフェース 2）へ 64 バイトの
Feature Report を送ります。hidapi はどのプラットフォームでも先頭にレポート ID を付ける
ので、こちらのバッファは 65 バイトで、`buf[0]` がレポート ID（`0` = 非番号レポート）です。

| バイト | 内容 |
| --- | --- |
| 0 | レポート ID（`0x00`） |
| 1 | コマンド。`0x04` = SET_PROFILE、`0x84` = GET_PROFILE |
| 2 | 引数。プロファイル番号 `0`〜`3`（ドライバ UI の 1〜4） |
| 3–6 | 未使用、`0x00` |
| 7 | チェックサム。`255 - (バイト 0..6 の合計 & 255)` |
| 8– | 未使用、`0x00` |

送るバイト列:

```
プロファイル 1 へ:  00 04 00 00 00 00 00 fb  (以降 0 埋め)
プロファイル 2 へ:  00 04 01 00 00 00 00 fa
現在値の取得:       00 84 00 00 00 00 00 7b
```

VID は `0x3151`。実機 FUN68（有線）の PID は `0x5030` でした。

## 検証状況

FUN68 実機（有線、VID `3151` / PID `5030`）での `--list-hid`:

```
pid=5030 iface=2 usage_page=ffff usage=0002 [config]  product="Monsgeek Multi-modes Keyboard"
pid=5030 iface=1 usage_page=ffff usage=0001
pid=5030 iface=1 usage_page=0001 usage=0006 (KBD)
pid=5030 iface=0 usage_page=0001 usage=0006 (KBD)
pid=5030 iface=1 usage_page=0001 usage=0002 / 0080、usage_page=000c usage=0001
```

| 項目 | 状態 |
| --- | --- |
| ベンダーインターフェース（`ffff:0002` / iface 2） | 実機で確認済み |
| `GET_PROFILE` (`0x84`)、チェックサム `0x7b` | 実機で確認済み |
| `GET_PROFILE` の応答の並び（バイト 1 コマンド、バイト 2 値） | 実機で確認済み |
| `SET_PROFILE` (`0x04`) と引数が 0 始まりであること | 実機で確認済み |

確認したときの実行結果:

```
--get-profile   -> Active profile: 1
--set-profile 2 -> Set profile 2
--get-profile   -> Active profile: 2
--set-profile 1 -> Set profile 1
--get-profile   -> Active profile: 1
```

つまり `04 01` が UI のプロファイル 2 で、引数は 0 始まりです。USB キャプチャは
このモデルについては不要になりました（以下は、別モデルやドングル接続で合わなく
なったときのための手順です）。

`--get-profile` が
「The keyboard answered, but not in the shape we expect」と出た場合は、応答の並びが
仮定と違います。`RUST_LOG=debug` を付けると先頭 8 バイトが出るので、それを見て
`hid.rs` の `get_profile` を直してください。

## USB キャプチャでの確認手順

1. USBPcap 付きの Wireshark を入れる。
2. キーボードを挿した状態で、該当の USB ルートハブのキャプチャを開始する。
3. **公式 MonsGeek ドライバは起動したまま**、UI でプロファイルを 1 → 2 → 1 とだけ操作する。
   （キャプチャ中はドライバが HID を掴んでいてよい。こちらから送るときだけ終了する。）
4. フィルタ例: `usb.idVendor == 0x3151 && usb.setup.bRequest == 9`
   （`bRequest == 9` = SET_REPORT。Feature Report は `usb.setup.wValue` の上位バイトが `0x03`）

見たいのは 64 バイトのデータの先頭で、上の表のとおりなら

```
04 00 ...  → プロファイル 1
04 01 ...  → プロファイル 2
```

の 2 バイト目だけが違うパケットになります（Windows では先頭にレポート ID `00` が付く
ことがあります）。

## 違っていたときに直す場所

| キャプチャで分かったこと | 直す場所 |
| --- | --- |
| PID / インターフェースが想定と違う | `hid.rs` の `VENDOR_USAGE_PAGE` / `VENDOR_USAGE` / `VENDOR_INTERFACE` |
| コマンド番号が違う | `hid.rs` の `SET_PROFILE` / `GET_PROFILE` |
| チェックサムの計算が違う | `hid.rs` の `bit7_checksum`（テストの期待値も合わせて更新） |
| 引数が 1 始まり（`04 01` がプロファイル 1） | `hid.rs` の `ui_profile_to_wire` / `wire_profile_to_ui` |
| Feature Report ではなく Output Report だった | `hid.rs` の `send_feature_report` を `write` に変更 |

キャプチャファイルはこのリポジトリに置いてもらえれば、そのバイト列に合わせられます。

## 2.4GHz ドングル

現状の実装は VID `0x3151` のベンダーインターフェースを見つけたら送る、というだけで、
有線とドングルを区別していません。ドングル接続では PID も転送手順も変わることがある
ので、ドングルでも使いたい場合は同じ手順でキャプチャが要ります。
