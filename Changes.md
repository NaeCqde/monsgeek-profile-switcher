# Changes

直近のコミット（`fix: seed the hotkey toggle state from ON_CONNECT_PROFILE`）の内容。
コミットごとにまっさらにして書き直している。

## 直したもの

- **ホットキーの 1 回目の押下が、すでに chat なのに「chat に切り替えた」と表示していた。**
  `EventContext.chat`（chat/game どちらのトグル状態かを表す bool）を固定で `false` に
  初期化していた。既定設定は `ON_CONNECT_PROFILE=2` かつ `PROFILE_CHAT=2` なので、
  接続時にキーボードは profile 2（＝chat）に入るのに、デーモン内部の `chat` は `false`
  のまま。そのため最初のホットキー押下で `chat = !false = true` となり、
  `PROFILE_CHAT` を送って（実際には既に profile 2 なので `ProfileState::apply` が
  早期 return し無送信）オーバーレイに `Chat (profile 2)` を出していた。ユーザーから
  見ると「chat にいるのに、ホットキーを押したら chat と言われる／game に行かない」。

  対処（`src/windows_events.rs`）:
  - `EventContext` 構築時に `chat` を `on_connect_profile == profile_chat` で初期化。
  - `apply_on_connect` が `ON_CONNECT_PROFILE` の送信に成功したら、そのプロファイルに
    合わせて `chat` を再セット。抜き差しのたびに走るので、game にトグルした状態で
    挿し直しても次回接続で正しく揃う。
  - どちらの設定プロファイルでもない値が `ON_CONNECT_PROFILE` の場合は `chat=false`
    始まり（1 回目の押下で chat 側へ）。

## 実機で確認できたところ

FUN68（有線、VID `3151` / PID `5030`）、既定設定（`ON_CONNECT_PROFILE=2` /
`PROFILE_CHAT=2` / `PROFILE_GAME=1`）。

- 起動直後: status = `{"connected":true,"profile":2,"chat":true,"game":false}`。
- ホットキー `Home` 1 回目 → `Profile 1 (hotkey: game)`、status が
  `profile:1, chat:false, game:true` に。修正前はここが `chat` になっていた。
- `Home` 2 回目 → `Profile 2 (hotkey: chat)`、status が `profile:2, chat:true` に戻る。

（`Home` の押下は `SendKeys {HOME}` で合成。グローバル `RegisterHotKey` が拾うことも
併せて確認。）

## テスト

`cargo test` 18 件パス（`hotkey` 7 / `hid` 4 / `status` 6 / `config` 1）。今回の修正は
Windows 専用の `EventContext`（`HidApi` を持つため単体テストしづらい）に対する 2 行の
状態初期化で、上記の実機確認で検証した。
