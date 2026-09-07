# Changes

直近のコミット（`Initial commit: hotkey-driven MonsGeek profile switcher`）の内容。

## このコミットで入ったもの

MonsGeek FUN68（RY5088 系）のオンボードプロファイルを、ユーザーが割り当てた
グローバルホットキーで切り替える常駐アプリ一式。テンプレートは
open-remote-url。切り替えはキーボードのベンダー HID インターフェースへ
64 バイトの Feature Report を送って行う（`src/hid.rs`）。

- `config`: `SwitcherConfig`（`ON_CONNECT_PROFILE` / `PROFILE_CHAT` /
  `PROFILE_GAME` / `STATUS_PORT` / `PROFILE_TOGGLE_HOTKEY`）。
- クレート名 `monsgeek_profile_switcher`、バイナリ `monsgeek-profile-switcher`、
  `app_type` は `switcher` のみ。URL ハンドラ登録の残骸（`scheme_handler.rs`、
  Windows レジストリ、Linux desktop エントリ、macOS `CFBundleURLTypes`）は削除済み。
- インストール先: Windows は
  `%LOCALAPPDATA%\Programs\monsgeek-profile-switcher\switcher\` ＋ `HKCU\...\Run`、
  macOS は `~/Applications/MonsGeekProfileSwitcher.app` ＋ LaunchAgent。

## 切り替えの判定: ホットキー

自動判定はやめて、ユーザーが割り当てた 1 つのキー（既定 `Home`）を押すたびに
`PROFILE_CHAT` と `PROFILE_GAME` をトグルする方式にした。理由は「すでに諦めた方法」の項。

- Windows は完全にイベント駆動（`src/windows_events.rs`）。1 スレッド・1 メッセージ
  ループで次の 2 つを受ける。
  - `RegisterHotKey`（`MOD_NOREPEAT` 付き。押しっぱなしで OS のキーリピート速度で
    トグルが暴れるのを防ぐ）。押された瞬間に `WM_HOTKEY` が来て即座に切り替える。
  - `RegisterDeviceNotificationW`（HID デバイスインターフェースクラスに絞る）。
    キーボードの抜き差しを `WM_DEVICECHANGE` で即検出。挿し直すたびに
    `ON_CONNECT_PROFILE` を送り直す。
- ホットキー文字列のパースだけ `src/hotkey.rs` に分離（Win32 のイベント配線と無関係で
  単体テストしやすいため）。`Home` / `F13` / `Ctrl+Alt+P` のように `+` 区切りで
  修飾キー（Ctrl/Alt/Shift/Win）＋最後にキー名。大文字小文字問わず。不明な綴りは
  推測せず `None`。F1〜F24、Numpad0〜9、US 配列の記号キーに対応。
- 起動時にキーボードがすでに挿さっていれば、デバイス通知を待たず即
  `ON_CONNECT_PROFILE` を適用。接続直後はインターフェースが開けないことがあるので
  200 ms 間隔で最大 8 回リトライ（`hid::set_profile_with_retry`）。
- macOS はホットキーもデバイス監視の配線もないので 250 ms ポーリングのフォールバック
  （`daemon::hid_loop`）。やることは新規接続を見て `ON_CONNECT_PROFILE` を送るだけで、
  チャット/ゲームの切り替えは一切しない。

## プロファイル送信はキーの押しっぱなしを解除する（実機挙動、未対処）

FUN68 は**同じプロファイルの再送だけでも、そのとき押されているキーを離した扱いにする**。
以前の自動判定ではこれを避けるため保留ロジック（HoldOff 状態機械）を入れていたが、
切り替えがホットキーという意図的操作になったので自己矛盾（押しっぱなし → 誤検知で切替
→ 押しっぱなしが壊れる）が起きなくなり、保留ロジックは削除した。ホットキーを押した
瞬間に何かキーを押していればそのリピートが止まる、というのは受け入れた制限として
待たずに切り替える。

## 通知オーバーレイ

切り替えるたびに、フォアグラウンドウィンドウのあるモニタ下部に
`Chat (profile 2)` / `Game (profile 1)` の小さな半透明ポップアップを出す
（`src/overlay.rs`）。音量 OSD 相当。Windows のトースト通知は「毎回出るものとしては
目立ちすぎる」「使わないでほしい」との指示で不採用。`show()` 呼び出しごとに
クリックスルーの短命ウィンドウを別スレッドで作り、1500 ms で自分を閉じる。
マルチモニタでプレイヤーの見ている画面に出すため `MonitorFromWindow(GetForegroundWindow())`
基準（フォアグラウンドが無いときだけプライマリにフォールバック）。

## 状態 API と表示

デーモンが `STATUS_PORT` の loopback に現在状態を JSON で出す（`src/daemon.rs` の
手書き HTTP）。

```json
{ "connected": true, "profile": 2, "chat": true, "game": false }
```

- `chat` / `game` は現在の `profile` が `PROFILE_CHAT` / `PROFILE_GAME` と一致するかで
  毎回計算する（フラグとして持たない）。`--set-profile` など別経路で変えても正しく出る。
  両方 `false`（どちらの設定値でもない）もありうる。不明時は `profile` / `chat` / `game`
  がまとめて `null`。キーは常に 4 つ出る。フィールドが固定スカラーなので読み取り側
  （`src/status.rs`）も依存なしでパースし、手書き HTTP と揃えている。
- 表示側は HID を開かない（デーモンのポーリング/イベント処理とデバイスを取り合わない）。
- GUI の状態ウィンドウは Keyboard / Profile / Chat の 3 項目のみ。ウィンドウは
  スクロール可能・リサイズ可能にして、モニタごとの DPI スケールに合う固定サイズを
  当てずっぽうで決めなくて済むようにした。デーモン停止時は Keyboard が
  `(daemon not running)`、不明な項目は `-`。
- 状態サーバは応答前にリクエストを読む。読まずに返して閉じると Windows で接続が
  RST になり、クライアントが応答を読めない。

## macOS の常駐

HID ループを別スレッドに置き、メインスレッドは `-[NSApplication run]` に留める。
これでこのプロセスが Launch Services に「動作中のバンドル実体」として登録され続け、
常駐中に .app をダブルクリックしても状態ウィンドウが開く（`src/mac_apple_events.rs` が
`kAEOpenApplication` / `kAEReopenApplication` を受ける）。

## コマンド

```
（引数なし）        状態表示（ダブルクリック時は GUI）
--daemon           常駐処理をフォアグラウンド実行
--install/--uninstall
--start/--stop     インストール済みデーモンの起動/停止
--config           .env のフォルダを開く
--list-hid         見えている MonsGeek の HID インターフェース一覧
--get-profile      現在のプロファイルを読む（1〜4）
--set-profile N    プロファイル N に切り替える（1〜4）
--help
```

## 実機で確認できたところ

FUN68（有線、VID `3151` / PID `5030`）。

- ベンダー設定インターフェース（`usage_page=ffff usage=0002`、インターフェース 2）の検出。
- `GET_PROFILE`（`0x84`）で現在のプロファイルを読める。
- `SET_PROFILE`（`0x04`）で実際に切り替わる（`--set-profile 2` → `--get-profile` が `2`）。
- ホットキーによる `PROFILE_CHAT` / `PROFILE_GAME` のトグルと、接続時の
  `ON_CONNECT_PROFILE` 適用。

バイト列・チェックサム・別モデル / 2.4GHz ドングルでの調べ方は `docs/PROTOCOL.md`。
macOS 側の常駐は未確認。

## すでに諦めた方法（記録）

自動判定を 2 系統試して断念した。詳細は README。

- **カーソル型**: マウスカーソルの表示状態＋他プロセスの IME 状態
  （`ImmGetDefaultIMEWnd` ＋ `WM_IME_CONTROL`、プロセスをまたいで届く）で判定。
  Fortnite のエモートピッカーが閉じる問題は保留ロジックで対処できたが、Apex Legends は
  チャットを開いてもカーソルが出ず、閉じても IME の日本語状態が残るため判定不能。
- **キャレット型**: 画面を定期撮影してチャット欄内の点滅キャレットを直接探す
  （ゲームプロセスには触れない）。スクロールバー・行・柱・ボタンのアニメーション等の
  誤検知と、条件を絞ると別の場面を取りこぼすいたちごっこで断念。Apex の「発言:」を
  OCR で読む案も、半透明背景越しの景色でコントラストが安定せず不可（実測 19 回中 0 回）。

「ゲームによっては信頼できる合図が原理的に存在しない」ため、明示的なホットキーに置換した。

## テスト

`cargo test` 18 件パス（`hotkey` パース 7、`hid` チェックサム/往復 4、`status` パース/
表示 6、`config` 1）。
