# Changes

## テンプレート（open-remote-url）からの置き換え

- HTTP プロキシと URL オープン、`sender` クレートを削除。残したのは常駐デーモン、
  インストーラ、状態表示 GUI の骨格。
- `receiver` クレート（ディレクトリも `switcher/` に改名）を `monsgeek_profile_switcher` に改名（バイナリは
  `monsgeek-profile-switcher`）。`app_type` は `switcher` のみ。
- URL ハンドラ登録の残骸を除去: Windows の `StartMenuInternet` / `RegisteredApplications`
  レジストリ、Linux の `x-scheme-handler` desktop エントリ、macOS の `CFBundleURLTypes`、
  空になっていた `scheme_handler.rs`。
- 設定を `SwitcherConfig`（`ON_CONNECT_PROFILE` / `PROFILE_WHEN_CHAT` /
  `PROFILE_OTHERWISE` / `STATUS_PORT`）に置き換え。

## 追加した機能

- `hid.rs`: ベンダー HID への 64 バイト Feature Report で `SET_PROFILE` / `GET_PROFILE`。
  Usage Page `0xFFFF` / Usage `0x02` を優先し、usage を報告しないプラットフォームでのみ
  インターフェース 2 にフォールバック。
- `daemon.rs`: 250 ms ポーリングで抜き差しを検出。接続時は `ON_CONNECT_PROFILE` を
  リトライ付きで送り、Windows ではカーソル表示 / 日本語 IME の状態に応じて
  変化したときだけ送信。同じ失敗を毎回ログに出さないよう抑制。
- `windows_chat.rs`: `GetCursorInfo` の `CURSOR_SHOWING` と、フォアグラウンドウィンドウの
  IME 状態で判定。
- macOS のデーモンは HID ループを別スレッドに置き、メインスレッドは
  `-[NSApplication run]` に留まる。これで常駐中に .app をダブルクリックしても
  状態ウィンドウが開く（Apple Event を受けられる）。
- CLI: `--list-hid` / `--get-profile` / `--set-profile N` / `--help`。
- デーモンがステータスポートに現在の状態（`connected` / `profile` / `chat`）を出すように
  し、GUI とコンソールの状態表示に「Current: profile N - 理由」を追加。表示側が HID を
  開かないので、デーモンのポーリングとデバイスを取り合わない（`shared/src/status.rs`）。
  GUI はこの 3 項目（Keyboard / Profile / Chat）だけを出す。詳細が要るのは切り分けの
  ときだけなので `--probe` に分けている。
  応答は `application/json` で `{"connected":true,"profile":2,"chat":true}` の形。
  不明な項目は `null`。フィールドが 3 つの固定スカラーなので、HTTP を手書きしているのに
  合わせて JSON も依存なしで扱っている。

## 直したもの

- **日本語入力の判定が常に true だった。** 他プロセスのウィンドウに `ImmGetContext` を
  呼んでいたが、入力コンテキストはスレッドのものなので null しか返らない。旧コードは
  そこで「日本語レイアウトなら IME 対応とみなす」と true を返していたため、日本語配列の
  PC では常にチャット扱い＝常に `PROFILE_WHEN_CHAT` に張り付いていた。
  `ImmGetDefaultIMEWnd` ＋ `WM_IME_CONTROL`（`IMC_GETOPENSTATUS` /
  `IMC_GETCONVERSIONMODE`）に変更。これはプロセスをまたいで届く。応答しないアプリで
  ポーリングが詰まらないよう `SendMessageTimeoutW` に 100 ms の期限を付けた。
  IME ウィンドウを持たないアプリは「日本語入力中ではない」扱い。
- 判定に使っている生の値を出す `--probe` を追加（Windows のみ）。
- ステータスサーバがリクエストを読まずに応答して閉じていたため、Windows では
  クライアントの送信が RST になり、応答を読めなかった。返す前にリクエストを読むように
  した（`Invoke-WebRequest` や `curl` でも見られるようになった）。

- `attach_console`（Windows）が、シェルが用意したリダイレクト先を握りつぶしていた。
  `AttachConsole` は標準ハンドルをコンソールのものに差し替えるので、呼ぶ前に
  継承ハンドルを読んでおき、有効なら戻すようにした。これで
  `monsgeek-profile-switcher --list-hid > out.txt` が効く。

## 実機で確認できたところ

FUN68（有線、VID `3151` / PID `5030`）で、プロトコルは一通り確認済み。

- ベンダー設定インターフェース（`usage_page=ffff usage=0002`、インターフェース 2）を検出。
- `GET_PROFILE`（`0x84`、チェックサム `0x7b`）が応答し、応答の並びも仮定どおり
  （バイト 1 コマンド、バイト 2 値）。
- `SET_PROFILE`（`0x04`）で実際に切り替わり、引数は 0 始まり
  （`04 01` = UI のプロファイル 2）。

Windows の判定（カーソル表示 / 日本語 IME）も実機で確認済み。macOS 側の常駐は未確認。

## 直したもの: 切り替えがキーの押しっぱなしを解除していた

FUN68 は、**同じプロファイルを再送するだけでも押されているキーを落とす**。
`--stress send`（設定を変えずに 250 ms ごとに再送）で全キーのリピートが止まることを
確認した。`--stress hid`（列挙のみ）と `--stress ime`（IME クエリのみ）は無害だったので、
原因は送信に絞られる。

実害の出方は Fortnite のエモートピッカー。エモートキーを押しっぱなしにする → ピッカーが
出てカーソルが表示される → デーモンが切り替えを送信 → 押しているキーが落ちてピッカーが
閉じる。押しっぱなしが切り替えを誘発し、その切り替えが押しっぱなしを壊す循環。

対処として、**切り替えが押しっぱなしによって引き起こされた形のときだけ**送信を保留する
（`daemon::HoldOff` と `switcher/src/windows_keys.rs`）。保留の条件は次の 2 段。

- 前のポーリング: 何かキーが押されていて、チャット判定ではない
- このポーリング: その同じキーがまだ押されていて、チャット判定になった

そのキーが離れるまで保留を継続する（1 回で解除すると次のポーリングで送信されて
押しっぱなしが壊れるため）。次はいずれも即時に送る。キーが押されていない状態で始まった
チャット、チャット開始後に押されたキー、前後で別々のキーのタップ、そしてチャットから
抜けるとき。マウスボタンは対象外（落ちるのはキーであり、ゲーム中は長時間押されうるため）。

戻る側（`PROFILE_OTHERWISE`）を即時にしているのは、ゲームに復帰する瞬間は移動キーを
押していることが多く、そこで待つと押している間ずっとチャット用プロファイルのままに
なるため。判定はプロファイル番号ではなく「チャット状態から抜けるか」で行っているので、
設定値を変えても意図どおりに動く。

規則は 2 段階で絞り込んだ。最初は「連続する 2 回のポーリングで同じキーが押されていたら
押しっぱなし」としてタップを保留の対象外にしたが、押しっぱなしの最初の 1 回が必ず
「タップかもしれない」と判定されて無防備になり、実機でピッカーが閉じた。次に「1 つでも
押されていれば保留」に広げるとピッカーは直ったが、キーを押していないときに始まった
チャットまで最大 250 ms 遅れる。いまの規則は壊れる原因になる形だけを狙うので、その
遅れがない。

`HoldOff` は純粋な状態機械としてテストしている（8 件）。

実機で修正を確認済み（エモートピッカーが閉じなくなった）。

切り分け用に `--stress <hid|ime|both|open|send|switch> [秒]` を追加した。

## 分かった制限（Apex Legends での実測）

- ゲーム内チャットの開閉は OS から観測できない。`--probe` の全項目がチャット中と
  キャラ操作中で一致した（カーソル、`hwnd_capture`、`cursor_clipped`、`hwnd_caret`、
  `gui_flags` すべて）。チャット欄をゲームが自前描画しているため。
- IME 状態はフルスクリーンのゲームでも読める（`open_status` が 0 / 1 で変化する）。
- ただし Apex はチャットを閉じても IME をオンのまま維持するので、キャラ操作中も
  `chat=true` と判定されてプロファイル 2 に張り付く。IME のオン/オフはスレッド単位の
  状態で、アプリの都合で自動的には戻らない。
- 対処は運用で行う方針（チャットを閉じたら半角/全角で IME をオフにする）。
  ホットキーによる手動オーバーライドは実装していない。詳細は README。

## 診断コマンド

`--probe`（Windows のみ）は判定に使う値に加えて、カーソル詳細（フラグ / ハンドル /
座標 / クリップ範囲）、フォアグラウンドのクラス・タイトル・PID、`GetGUIThreadInfo`
（フォーカス / キャプチャ / メニュー / キャレット）を出す。判定ロジックはこれらを
使っていない（切り分け用）。
