# monsgeek-profile-switcher 仕様

MonsGeek FUN68(RY5088 系)のキーボードのオンボードプロファイルを、キーボードショートカットで切り替える常駐アプリ(Rust)。
切り替えは、キーボードのベンダー HID インターフェースへ 64 バイトの Feature Report を送って行う。
仕様を変えたときは、このファイルも必ず更新する。詳しい説明は `README.md`、バイト列は `docs/PROTOCOL.md`。

## 構成

| ファイル | 役割 |
|---|---|
| `main.rs`, `cli.rs` | 起動、コマンドの解釈 |
| `daemon.rs` | 常駐処理。接続の検出、ホットキーの処理、プロファイルの送信、ステータスの待ち受け |
| `hid.rs` | HID の列挙、`SET_PROFILE`/`GET_PROFILE` の送受信 |
| `hotkey.rs` | ホットキー文字列(`Ctrl+Alt+P` など)の解析 |
| `windows_events.rs` | Windows でのキー入力と、デバイスの抜き差しの通知 |
| `mac_apple_events.rs` | macOS の Apple Events |
| `overlay.rs` | 切り替えたときの小さな通知の表示(`Chat (profile 2)`) |
| `status.rs`, `gui.rs` | 状態の取得・表示(引数なしで開く状態ウィンドウ) |
| `config.rs` | `.env` の読み込みと保存 |
| `installer.rs`, `installer_utils.rs`, `uninstaller.rs`, `utils.rs` | インストール、ログイン時起動の登録、解除 |

## 設定(`.env`)

場所: Windows `%APPDATA%\monsgeek-profile-switcher\switcher\.env`、macOS `~/Library/Application Support/monsgeek-profile-switcher/switcher/.env`(`--config` で開く)。変更後はデーモンの再起動が必要。

配布物には同じ内容のテンプレート `.env` が付く。インストール時にそれを設定フォルダーの `.env` にコピーするが、設定フォルダーにすでに `.env` があれば上書きしない。アンインストールしても設定フォルダーと `.env` は消さない。サービスをインストールしていなくても、設定フォルダーに `.env` があれば設定の表示・読み込みにそれを使う(なければ配布物のテンプレート)。

| 変数 | 意味 |
|---|---|
| `ON_CONNECT_PROFILE` | 起動時・接続時に設定するプロファイル(1〜4) |
| `PROFILE_CHAT` | ホットキーでチャット側にしたときのプロファイル |
| `PROFILE_GAME` | それ以外(ゲーム中など)のプロファイル |
| `STATUS_PORT` | 状態を JSON で返す loopback のポート(既定 38415) |
| `PROFILE_TOGGLE_HOTKEY` | トグルするキー(既定 `Home`)。`+` 区切りで修飾キー(`Ctrl`/`Alt`/`Shift`/`Win`)と最後にキー名 |

## 動作

- 接続直後に `ON_CONNECT_PROFILE` を送る。インターフェースが開けないことがあるので、200 ms 間隔で最大 8 回リトライする。
- Windows: ホットキーを押すたびに `PROFILE_CHAT` と `PROFILE_GAME` をトグルする。切り替えるたびに、フォーカスしているウィンドウのディスプレイの下部に通知を数秒出す。キーボードの抜き差しは Windows の通知で検出し、挿し直すたびに `ON_CONNECT_PROFILE` から始める。
- macOS: 接続直後の送信だけ。自動では切り替えない(`--set-profile` で手動)。
- 送る前に、キーボードの現在のプロファイルと違うときだけ送る(`ProfileState`)。失敗は同じものを一度だけログに出す。
- プロファイルを送ると、そのとき押しているキーが離した扱いになる(キーボード側の仕様。避けられない)。

## ステータス

デーモンは `127.0.0.1:<STATUS_PORT>` で TCP を待ち受け、どんなリクエストにも HTTP 200 で次の JSON を返す(認証なし)。

```json
{ "connected": true, "profile": 2, "chat": true, "game": false }
```

`chat`/`game` は、現在のプロファイルが `PROFILE_CHAT`/`PROFILE_GAME` と一致するか。値が不明なら `profile`・`chat`・`game` がまとめて `null`(キーは常に 4 つ出す)。GUI と `status` はこのポートで常駐の有無を調べる。

## コマンド

```
monsgeek-profile-switcher                  状態を表示(ダブルクリック時は GUI)
--daemon         常駐処理をフォアグラウンドで実行
--install        インストールしてログイン時起動に登録し、起動
--uninstall      停止してアンインストール
--start / --stop インストール済みデーモンの起動 / 停止
--config         .env のあるフォルダを開く
--list-hid       見えている MonsGeek の HID インターフェース一覧
--get-profile    現在のプロファイルを読む(1〜4)
--set-profile N  プロファイル N に切り替える(1〜4)
--help           コマンド一覧
```

インストール先: Windows `%LOCALAPPDATA%\Programs\monsgeek-profile-switcher\switcher\` と `HKCU\...\CurrentVersion\Run`、macOS `~/Applications/MonsGeekProfileSwitcher.app` と LaunchAgent。

## HID の仕様

VID `0x3151`、実機(有線)の PID `0x5030`。ベンダー設定インターフェース(Usage Page `0xFFFF`、Usage `0x02`)へ、65 バイトのバッファ(先頭がレポート ID `0`)の Feature Report を送る。コマンド `0x04` = SET_PROFILE、`0x84` = GET_PROFILE。引数はプロファイル番号 0〜3(UI の 1〜4)。チェックサムは `255 - (バイト 0〜6 の合計 & 255)`(バイト 7)。

## セキュリティ上の注意

- 通信先は loopback のステータスポートだけ。ネットワークには出さない。ステータスの JSON はキーボードの接続状態とプロファイルだけで、秘密は含まない。
- ホットキーのために、OS のキー入力を監視する(Windows)。入力した内容は保存・送信しない。
- 送る HID のコマンドは、プロファイルの切り替えだけ(ファームウェアの書き換えなどはしない)。
