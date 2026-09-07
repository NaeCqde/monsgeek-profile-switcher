//! Reads the live state the daemon publishes on its loopback status port.
//!
//! The GUI and the console status display use this instead of opening the HID
//! device themselves: the daemon polls that device four times a second, and a
//! second opener would race it for no benefit -- the daemon already knows
//! which profile it last applied.
//!
//! The daemon answers with JSON:
//!
//! ```json
//! {"connected":true,"profile":2,"chat":true,"game":false}
//! ```
//!
//! `chat`/`game` say whether `profile` equals `PROFILE_CHAT`/`PROFILE_GAME`
//! (from the daemon's own config) -- both are computed from the profile
//! number itself, not tracked separately, so a profile set some other way
//! (`--set-profile`, say) is still reported accurately. Both can be `false`
//! at once if the keyboard is on neither configured profile; `profile` and
//! both booleans are `null` together when unknown. The payload has fixed
//! scalar fields, so it is read here without pulling in a JSON parser,
//! matching the hand-written HTTP on the daemon side.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_millis(300);

/// What the running daemon reports right now. `None` fields mean the daemon
/// reported `null` (unknown, e.g. the keyboard isn't connected).
#[derive(Debug, Clone, Default)]
pub struct DaemonStatus {
    pub keyboard_connected: bool,
    pub profile: Option<u8>,
    pub chat: Option<bool>,
    pub game: Option<bool>,
}

/// One line describing what the daemon has the keyboard set to.
/// `None` means no daemon answered.
pub fn describe(status: Option<&DaemonStatus>) -> String {
    let Some(status) = status else {
        return "(daemon not running)".to_string();
    };
    if !status.keyboard_connected {
        return "(keyboard not connected)".to_string();
    }
    let profile = match status.profile {
        Some(p) => format!("profile {}", p),
        None => "unknown (could not send to the keyboard)".to_string(),
    };
    match (status.chat, status.game) {
        (Some(true), _) => format!("{} - chat", profile),
        (_, Some(true)) => format!("{} - game", profile),
        _ => profile,
    }
}

/// `None` when no daemon answered on `port`.
pub fn query(port: u16) -> Option<DaemonStatus> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;

    let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(parse(&response))
}

fn parse(response: &str) -> DaemonStatus {
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(response);

    let bool_field = |key: &str| match field(body, key) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };

    DaemonStatus {
        keyboard_connected: field(body, "connected") == Some("true"),
        profile: field(body, "profile").and_then(|v| v.parse().ok()),
        chat: bool_field("chat"),
        game: bool_field("game"),
    }
}

/// The raw text of `"key": <value>`, up to the next `,` or `}`. Values here
/// are only ever `true`, `false`, `null` or a small number, so no string or
/// nesting handling is needed.
fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let quoted = format!("\"{}\"", key);
    let after_key = &body[body.find(&quoted)? + quoted.len()..];
    let value = after_key.trim_start().strip_prefix(':')?.trim_start();
    let end = value.find([',', '}']).unwrap_or(value.len());
    Some(value[..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n";

    #[test]
    fn parses_a_full_response() {
        let s = parse(&format!(
            "{}{}",
            HEAD, r#"{"connected":true,"profile":2,"chat":true,"game":false}"#
        ));
        assert!(s.keyboard_connected);
        assert_eq!(s.profile, Some(2));
        assert_eq!(s.chat, Some(true));
        assert_eq!(s.game, Some(false));
    }

    #[test]
    fn null_fields_read_as_none() {
        let s = parse(&format!(
            "{}{}",
            HEAD, r#"{"connected":true,"profile":null,"chat":null,"game":null}"#
        ));
        assert!(s.keyboard_connected);
        assert_eq!(s.profile, None);
        assert_eq!(s.chat, None);
        assert_eq!(s.game, None);
    }

    #[test]
    fn disconnected_keyboard() {
        let s = parse(&format!(
            "{}{}",
            HEAD, r#"{"connected":false,"profile":null,"chat":null,"game":null}"#
        ));
        assert!(!s.keyboard_connected);
        assert_eq!(s.chat, None);
    }

    #[test]
    fn tolerates_whitespace_between_tokens() {
        let s = parse(&format!(
            "{}{}",
            HEAD, "{ \"connected\" : true , \"profile\" : 3 , \"chat\" : false , \"game\" : true }"
        ));
        assert!(s.keyboard_connected);
        assert_eq!(s.profile, Some(3));
        assert_eq!(s.chat, Some(false));
        assert_eq!(s.game, Some(true));
    }

    /// Anything we cannot read must degrade to "nothing known", never panic.
    #[test]
    fn unknown_body_is_not_an_error() {
        let s = parse(&format!("{}{}", HEAD, "alive"));
        assert!(!s.keyboard_connected);
        assert_eq!(s.profile, None);
        assert_eq!(s.chat, None);

        let s = parse(&format!("{}{}", HEAD, r#"{"connected":"#));
        assert!(!s.keyboard_connected);
    }

    #[test]
    fn describe_covers_the_states_the_gui_shows() {
        assert_eq!(describe(None), "(daemon not running)");
        assert_eq!(
            describe(Some(&DaemonStatus::default())),
            "(keyboard not connected)"
        );
        let on_game = DaemonStatus {
            keyboard_connected: true,
            profile: Some(1),
            chat: Some(false),
            game: Some(true),
        };
        assert_eq!(describe(Some(&on_game)), "profile 1 - game");

        let on_neither = DaemonStatus {
            keyboard_connected: true,
            profile: Some(3),
            chat: Some(false),
            game: Some(false),
        };
        assert_eq!(describe(Some(&on_neither)), "profile 3");
    }
}
