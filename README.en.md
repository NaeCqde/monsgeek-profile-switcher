# MonsGeek Profile Switcher

[日本語](README.md) | English

A resident app that lets you assign a keyboard shortcut to switching the onboard profile of a MonsGeek FUN68 (RY5088 family). It switches by sending a 64-byte Feature Report to the keyboard's vendor HID interface.

## Behavior

| OS      | Right after connecting     | Afterwards                                                                          |
| ------- | -------------------------- | ----------------------------------------------------------------------------------- |
| Windows | Sends `ON_CONNECT_PROFILE` | Every press of the hotkey (default `Home`) toggles `PROFILE_CHAT` and `PROFILE_GAME` |
| macOS   | Sends `ON_CONNECT_PROFILE` | Does nothing by itself (switch by hand with `--set-profile`)                        |

- Plugging the keyboard in and out is detected at once through a Windows notification. Every time it is plugged in again it
  starts from `ON_CONNECT_PROFILE`.
- Switching is always immediate. Sending a profile makes the keyboard treat the keys that are held as released, so if you are
  holding some key at the moment you press the hotkey, the repeat of that key stops (see below).
- The interface may not be openable right after connecting, so `ON_CONNECT_PROFILE` is retried up to 8 times at 200 ms intervals.

### What decides the switch: the hotkey

Every press of the one key the user assigned (default `Home`) toggles `PROFILE_CHAT` and `PROFILE_GAME`. Windows notifies the app directly at the
moment the key is pressed, and the switch happens at once ([windows_events.rs](src/windows_events.rs); how the key is specified is parsed in
[hotkey.rs](src/hotkey.rs)).

The key may be combined with modifiers such as `Ctrl+Alt+P`. See [Configuration](#configuration). Every time it switches, a small notification
(such as `Chat (profile 2)`) that disappears after a few seconds appears at the bottom of the display that holds the focused window
([overlay.rs](src/overlay.rs)).

## Sending a profile releases held keys

This is behavior confirmed on a real FUN68. **Even re-sending the profile that is already set makes the keys that are pressed at that moment count as released**
(nothing changed in the settings, yet the repeat of all keys stopped). The keyboard resets its key state when it applies a profile, and there is no way to avoid
it from our side.

So **if you are holding some key at the moment you press the hotkey, the repeat of that key stops.** The hotkey is something the user presses on purpose, so
this is an accepted limitation and the switch is made at once, without waiting.

## Configuration

Put it in `.env`. The location can be opened with `--config`
(Windows: `%APPDATA%\monsgeek-profile-switcher\switcher\.env`,
macOS: `~/Library/Application Support/monsgeek-profile-switcher/switcher/.env`).

The `.env` that comes with the download is copied to this folder at install time (an existing `.env` is never overwritten). Uninstalling does not remove this folder or the `.env`. Even when the app is not installed, the `.env` in this folder is shown if there is one.

```env
ON_CONNECT_PROFILE=2
PROFILE_CHAT=2
PROFILE_GAME=1
STATUS_PORT=38415
PROFILE_TOGGLE_HOTKEY=Home
```

| Variable                | Meaning                                                                                                                                                                                                                       |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ON_CONNECT_PROFILE`    | Profile (1–4) set at start-up and at connection                                                                                                                                                                               |
| `PROFILE_CHAT`          | Profile when the hotkey has put it on the chat side                                                                                                                                                                           |
| `PROFILE_GAME`          | The other one (while playing, and so on)                                                                                                                                                                                      |
| `STATUS_PORT`           | Loopback port the GUI uses to check whether the daemon is running                                                                                                                                                            |
| `PROFILE_TOGGLE_HOTKEY` | The key that toggles `PROFILE_CHAT`/`PROFILE_GAME`. Written like `Home`, `F13` or `Ctrl+Alt+P`: modifiers (`Ctrl`/`Alt`/`Shift`/`Win`) separated by `+`, and the key name last (`parse_hotkey` in [hotkey.rs](src/hotkey.rs)) |

The daemon must be restarted after a change (the GUI detects the change and tells you).

## Checking the state

The daemon writes its current state as JSON to the status port.

```json
{ "connected": true, "profile": 2, "chat": true, "game": false }
```

`chat`/`game` tell whether the current `profile` equals `PROFILE_CHAT`/`PROFILE_GAME`; both are recomputed every time from the keyboard's actual profile number. If
another profile has been set with `--set-profile` and so on, both are `false`. When the value is unknown (keyboard not connected or sending failed), `profile`,
`chat` and `game` are all `null` together. All four keys are always present.

The status window that opens when the app is started without arguments shows **these items**.

```
Keyboard:  Connected
Profile:   2
Chat:      Yes
```

When the daemon is stopped, Keyboard shows `(daemon not running)`, and items whose value is unknown show `-`.

## Commands

```
monsgeek-profile-switcher                  Show the state (the GUI when double-clicked)
monsgeek-profile-switcher --daemon         Run the resident process in the foreground
monsgeek-profile-switcher --install        Install, register for start at login, and start
monsgeek-profile-switcher --uninstall      Stop and uninstall
monsgeek-profile-switcher --start/--stop   Start / stop the installed daemon
monsgeek-profile-switcher --config         Open the folder that holds .env
monsgeek-profile-switcher --list-hid       List the MonsGeek HID interfaces that are visible
monsgeek-profile-switcher --get-profile    Read the current profile (1–4)
monsgeek-profile-switcher --set-profile N  Switch to profile N (1–4)
monsgeek-profile-switcher --help           Show this list of commands
```

Where it installs: on Windows `%LOCALAPPDATA%\Programs\monsgeek-profile-switcher\switcher\` plus a registration in
`HKCU\...\CurrentVersion\Run`; on macOS `~/Applications/MonsGeekProfileSwitcher.app` plus a LaunchAgent.

## Build

```
cargo build --release
cargo test
```

## Protocol verification status

Confirmed on a real FUN68 (wired, VID `3151` / PID `5030`).

- Detection of the vendor configuration interface (`usage_page=ffff usage=0002`, interface 2)
- `GET_PROFILE` (`0x84`) reads the current profile
- `SET_PROFILE` (`0x04`) really switches (`--set-profile 2` → `--get-profile` returns `2`)

For the byte layout, the checksum, and how to investigate when it stops matching on another model or the 2.4 GHz dongle, see
[docs/PROTOCOL.md](docs/PROTOCOL.md).

## Methods already given up

The current way is switching by hand with a hotkey (see [Configuration](#configuration)), but before that two kinds of automatic detection were tried, and both were
given up in the end. They are kept here as a record.

### Cursor type: watching the mouse cursor and the IME state

The first detection that was built. It read two pieces of information from the OS, and if either applied it judged that the chat box was open.

- **Whether the mouse cursor is shown.** The assumption is that a 3D game normally hides the mouse cursor, and the cursor appears when the chat box or a menu is opened.
- **Whether the IME is in Japanese input.** The IME state of another application cannot normally be looked at, but with a Windows mechanism the foreground application can be
  asked whether it is in Japanese input. It was judged "in Japanese input" only when all three conditions held:
    - The keyboard input method is Japanese
    - The IME is on
    - The conversion mode is for Japanese input (not a temporary half-width alphanumeric input)

**Dealing with the emote picker problem.** In Fortnite a vicious circle happened:

1. Hold down the key for an emotion (emote)
2. The selection screen (picker) appears and the mouse cursor is shown
3. The shown cursor is detected and the profile is switched to the chat one
4. Switching the profile releases held keys (see above), so the held emote key counts as released and the selection screen closes

Holding the key itself triggers the switch, and the switch breaks the holding. This was solved by waiting a little before switching profiles while a key is held, and
sending it after the key is released (it was verified by running only the keyboard enumeration and the IME state check on their own, which confirmed that the cause
is the sending of the profile itself). Now that switching is a deliberate hotkey operation, this contradiction cannot happen, so the waiting mechanism itself has been removed.

**Problem in Apex Legends.** Observing whether the chat box is open was not easy. In Fortnite the cursor is shown when the chat box is opened, and the IME automatically
goes back from Japanese to English when the chat box is closed, so it was easy to detect.

In Apex Legends the cursor does not appear when the chat box is opened, and the Japanese state of the IME is kept even after the chat box is closed, so this method could not be used.

### Caret type: taking screenshots and looking for the blinking caret

Without relying on OS information such as the IME or the cursor, this method regularly takes screenshots of the screen and looks directly for the blinking text cursor
(caret) in the chat box (it never touches the game process). Roughly, it judged as follows:

1. Take the screen at regular intervals, divide it into small cells and record brightness and color.
2. First find the rectangular area that looks like the chat box itself in the image (from the border lines and a plain background that differs from the surroundings). If it is not found, no further judgement is made.
3. In the found chat box, extract only the parts whose brightness changed compared with the previous capture.
4. If almost the whole inside of the chat box changed at once (for example when a button changes color), ignore that capture.
5. Examine the shape and color of the changed parts and keep only those that look like a real caret, such as thin, straight and tall. Whether it blinks is checked by comparing with the previous capture to see if the brightness switches.

**Measures for stability.** While typing, the caret position keeps moving because characters are added and deleted, so going back to the other profile after losing it even once is unstable. The rule
was to return to the game-side profile only when it was not found several times in a row (3 times), and to switch to the chat-side profile at once at the moment it was found.

**In Fortnite and Genshin Impact detection could fail when the caret moved often, and in Apex Legends, on the contrary, false detections kept happening, so it was finally given up.** Examples of things
that were actually mistaken for a caret:

- A scroll bar (a tall band that extends through the border of the chat box)
- Rows of a mail or file list
- Noise of floor or wall patterns (which happen to satisfy the shape conditions)
- Animations in which the background color of a button changes (such as the "collect all" button)
- Pillars of decorated buildings (tall, but with a width that is not constant)

Each time the conditions were narrowed, other scenes that had been detected before were missed, and this went on without end.

A method of cutting out and examining only the surroundings of the chat box for Apex was also tried, but something else in that range was falsely detected as a caret and it was useless.

In parallel, a method was tried that reads, with the standard Windows text recognition (OCR), the text "発言:" (in the English version "Say:") that Apex always shows at the moment the chat box is opened.
Even when the whole screen was put through recognition, this text was almost never read (0 out of 19 in measurement), and even when only that part was cut out and enlarged, the moving scenery could be seen through the
translucent background of the chat box, so the shading of the text changed from frame to frame and recognition was not stable, so this too was given up.

In the end, all of these automatic detections ran into the wall that "for some games there is in principle no reliable signal", so they were replaced with a hotkey that the user presses explicitly.
