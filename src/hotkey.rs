//! Parsing for the user-assigned hotkey spec (`PROFILE_TOGGLE_HOTKEY`) that
//! manually toggles which profile is active -- replaces automatic IME-based
//! detection as the daemon's real switching signal, per explicit
//! instruction: screen/IME state proved too unreliable in some games (Apex
//! Legends in particular) to trust over a key the user presses on purpose.
//!
//! The actual registration and event handling live in `windows_events.rs`
//! now, alongside HID device-change notifications, on one event-driven
//! thread with no polling; this module is just the pure spec-string ->
//! `(modifiers, virtual-key code)` parsing, kept separate because it has
//! nothing to do with Win32 event plumbing and is easy to unit test on its
//! own.
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VK_BACK, VK_CAPITAL, VK_DELETE,
    VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_NUMPAD0, VK_OEM_1,
    VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS,
    VK_OEM_PERIOD, VK_OEM_PLUS, VK_PAUSE, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RSHIFT, VK_SCROLL,
    VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
};

/// Parses a spec like `"Home"`, `"Ctrl+Alt+P"`, or `"F13"` into the
/// `(modifiers, virtual-key code)` pair `RegisterHotKey` wants.
/// Case-insensitive, `+`-separated; every part but the last must name a
/// modifier, and the last must name the key itself. Returns `None` for
/// anything unrecognized rather than guessing.
pub fn parse_hotkey(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let (key_part, modifier_parts) = parts.split_last()?;

    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for m in modifier_parts {
        modifiers |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "windows" => MOD_WIN,
            _ => return None,
        };
    }
    let vk = vk_from_name(key_part)?;
    Some((modifiers, vk))
}

/// Named virtual-key codes a hotkey spec's final token can reference.
/// Covers the keys with no plain ASCII letter/digit of their own (a
/// single alphanumeric character is handled separately below, since its
/// own char code already equals its VK code for 0-9/A-Z) plus the ones
/// likely to actually get assigned as a profile-toggle key -- not an
/// exhaustive list of every VK_* constant Windows defines.
fn vk_from_name(name: &str) -> Option<u32> {
    let vk = match name.to_ascii_uppercase().as_str() {
        "HOME" => VK_HOME,
        "END" => VK_END,
        "INSERT" | "INS" => VK_INSERT,
        "DELETE" | "DEL" => VK_DELETE,
        "PAGEUP" | "PGUP" => VK_PRIOR,
        "PAGEDOWN" | "PGDN" => VK_NEXT,
        "UP" => VK_UP,
        "DOWN" => VK_DOWN,
        "LEFT" => VK_LEFT,
        "RIGHT" => VK_RIGHT,
        "SPACE" => VK_SPACE,
        "TAB" => VK_TAB,
        "ENTER" | "RETURN" => VK_RETURN,
        "BACKSPACE" | "BACK" => VK_BACK,
        "ESC" | "ESCAPE" => VK_ESCAPE,
        "CAPSLOCK" => VK_CAPITAL,
        "SCROLLLOCK" => VK_SCROLL,
        "PAUSE" | "BREAK" => VK_PAUSE,
        "PRINTSCREEN" | "PRTSC" => VK_SNAPSHOT,
        "RSHIFT" => VK_RSHIFT,
        // The OEM_* names below are US-keyboard-layout-specific (as
        // Windows' own VK_OEM_* constants are); good enough for the
        // common symbol keys, not a full layout-independent mapping.
        ";" | "SEMICOLON" => VK_OEM_1,
        "/" | "SLASH" => VK_OEM_2,
        "`" | "GRAVE" | "BACKTICK" => VK_OEM_3,
        "[" => VK_OEM_4,
        "\\" | "BACKSLASH" => VK_OEM_5,
        "]" => VK_OEM_6,
        "'" | "QUOTE" | "APOSTROPHE" => VK_OEM_7,
        "," | "COMMA" => VK_OEM_COMMA,
        "." | "PERIOD" => VK_OEM_PERIOD,
        "-" | "MINUS" => VK_OEM_MINUS,
        "=" | "PLUS" | "EQUALS" => VK_OEM_PLUS,
        _ => {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return numbered_key(name);
            };
            if c.is_ascii_alphanumeric() {
                // VK codes for '0'..'9' and 'A'..'Z' are their own ASCII
                // values -- windows-rs has no named constant for these,
                // unlike every other key here.
                return Some(c.to_ascii_uppercase() as u32);
            }
            return numbered_key(name);
        }
    };
    Some(vk.0 as u32)
}

/// Handles the two families of keys that are a letter/prefix plus a
/// number: function keys (F1..=F24) and the numeric keypad (Numpad0..=9).
fn numbered_key(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    if let Some(n) = upper.strip_prefix('F').and_then(|s| s.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(VK_F1.0 as u32 + (n - 1));
        }
    }
    if let Some(n) = upper
        .strip_prefix("NUMPAD")
        .or_else(|| upper.strip_prefix("NUM"))
        .and_then(|s| s.parse::<u32>().ok())
    {
        if n <= 9 {
            return Some(VK_NUMPAD0.0 as u32 + n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_key_has_no_modifiers() {
        let (modifiers, vk) = parse_hotkey("Home").unwrap();
        assert_eq!(modifiers, HOT_KEY_MODIFIERS(0));
        assert_eq!(vk, VK_HOME.0 as u32);
    }

    #[test]
    fn modifiers_combine() {
        let (modifiers, vk) = parse_hotkey("Ctrl+Alt+P").unwrap();
        assert_eq!(modifiers, MOD_CONTROL | MOD_ALT);
        assert_eq!(vk, b'P' as u32);
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(parse_hotkey("home"), parse_hotkey("HOME"));
        assert_eq!(parse_hotkey("ctrl+p"), parse_hotkey("CTRL+P"));
    }

    #[test]
    fn function_keys_parse() {
        let (_, vk) = parse_hotkey("F13").unwrap();
        assert_eq!(vk, VK_F1.0 as u32 + 12);
    }

    #[test]
    fn numpad_keys_parse() {
        let (_, vk) = parse_hotkey("Numpad5").unwrap();
        assert_eq!(vk, VK_NUMPAD0.0 as u32 + 5);
    }

    #[test]
    fn an_unknown_key_name_does_not_parse() {
        assert!(parse_hotkey("NotAKey").is_none());
    }

    #[test]
    fn an_unknown_modifier_does_not_parse() {
        assert!(parse_hotkey("Super+P").is_none());
    }
}
