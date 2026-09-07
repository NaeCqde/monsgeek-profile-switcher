//! MonsGeek / RY5088-family vendor HID: profile get & set.
//!
//! The official V4 driver talks to the keyboard's vendor-config HID interface
//! (Usage Page 0xFFFF, Usage 0x02) with 64-byte feature reports:
//!
//! ```text
//! byte 0   report id (0 -- unnumbered; hidapi prepends it on every platform)
//! byte 1   command      0x04 = SET_PROFILE, 0x84 = GET_PROFILE
//! byte 2   argument     profile index, 0..=3 (driver UI shows 1..=4)
//! byte 3-6 unused, zero
//! byte 7   checksum     255 - (sum of bytes 0..=6 & 255)
//! byte 8+  unused, zero
//! ```
//!
//! Both commands, the checksum, the 0-based argument and the response layout
//! (which mirrors the request) are confirmed against a wired FUN68,
//! VID 0x3151 / PID 0x5030. See `docs/PROTOCOL.md` for the evidence and for
//! how to re-check another model with a USB capture.

use hidapi::{DeviceInfo, HidApi, HidDevice};

pub const MONSGEEK_VID: u16 = 0x3151;
const VENDOR_USAGE_PAGE: u16 = 0xFFFF;
pub const VENDOR_USAGE: u16 = 0x02;
/// The vendor-config interface on the wired keyboard. Used as a fallback on
/// platforms where hidapi does not report usage page / usage (Linux hidraw).
const VENDOR_INTERFACE: i32 = 2;
const REPORT_LEN: usize = 65;
const SET_PROFILE: u8 = 0x04;
const GET_PROFILE: u8 = 0x84;

pub fn ui_profile_to_wire(profile: u8) -> u8 {
    profile.saturating_sub(1).min(3)
}

pub fn wire_profile_to_ui(wire: u8) -> u8 {
    wire.min(3) + 1
}

pub fn bit7_checksum(header7: &[u8; 7]) -> u8 {
    let sum: u16 = header7.iter().map(|&b| b as u16).sum();
    (255 - (sum & 255)) as u8
}

fn build_report(command: u8, argument: u8) -> [u8; REPORT_LEN] {
    let mut buf = [0u8; REPORT_LEN];
    buf[1] = command;
    buf[2] = argument;
    let header = [buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6]];
    buf[7] = bit7_checksum(&header);
    buf
}

pub fn build_set_profile_report(ui_profile: u8) -> [u8; REPORT_LEN] {
    build_report(SET_PROFILE, ui_profile_to_wire(ui_profile))
}

pub fn build_get_profile_report() -> [u8; REPORT_LEN] {
    build_report(GET_PROFILE, 0)
}

fn matches_vendor_usage(info: &DeviceInfo) -> bool {
    info.vendor_id() == MONSGEEK_VID
        && info.usage_page() == VENDOR_USAGE_PAGE
        && info.usage() == VENDOR_USAGE
}

fn matches_vendor_interface(info: &DeviceInfo) -> bool {
    info.vendor_id() == MONSGEEK_VID && info.interface_number() == VENDOR_INTERFACE
}

pub fn is_vendor_config(info: &DeviceInfo) -> bool {
    matches_vendor_usage(info) || matches_vendor_interface(info)
}

/// The vendor-config interface, preferring a usage-page match. Only when no
/// device reports the vendor usage do we fall back to interface 2, so a
/// platform that does report usages never picks the wrong interface.
pub fn find_keyboard(api: &HidApi) -> Option<DeviceInfo> {
    api.device_list()
        .find(|d| matches_vendor_usage(d))
        .or_else(|| api.device_list().find(|d| matches_vendor_interface(d)))
        .cloned()
}

pub fn list_monsgeek_devices(api: &HidApi) -> Vec<String> {
    api.device_list()
        .filter(|d| d.vendor_id() == MONSGEEK_VID)
        .map(|d| {
            format!(
                "pid={:04x} iface={} usage_page={:04x} usage={:04x}{} product={:?} path={:?}",
                d.product_id(),
                d.interface_number(),
                d.usage_page(),
                d.usage(),
                if is_vendor_config(d) { " [config]" } else { "" },
                d.product_string(),
                d.path()
            )
        })
        .collect()
}

pub fn open_config_device(api: &HidApi, info: &DeviceInfo) -> Result<HidDevice, hidapi::HidError> {
    api.open_path(info.path())
}

pub fn set_profile(
    api: &HidApi,
    info: &DeviceInfo,
    ui_profile: u8,
) -> Result<(), hidapi::HidError> {
    let device = open_config_device(api, info)?;
    device.send_feature_report(&build_set_profile_report(ui_profile))?;
    Ok(())
}

/// Reads the keyboard's active profile as a UI number (1..=4).
///
/// `Ok(None)` means the keyboard answered but not in the shape we expect --
/// treat that as "protocol needs verifying", not as a transport failure.
pub fn get_profile(api: &HidApi, info: &DeviceInfo) -> Result<Option<u8>, hidapi::HidError> {
    let device = open_config_device(api, info)?;
    device.send_feature_report(&build_get_profile_report())?;

    let mut buf = [0u8; REPORT_LEN];
    let read = device.get_feature_report(&mut buf)?;
    if read < 3 || buf[1] != GET_PROFILE {
        log::debug!(
            "GET_PROFILE returned {} bytes: {:02x?}",
            read,
            &buf[..read.min(8)]
        );
        return Ok(None);
    }
    Ok(Some(wire_profile_to_ui(buf[2])))
}

pub fn set_profile_with_retry(
    api: &HidApi,
    info: &DeviceInfo,
    ui_profile: u8,
    attempts: u32,
) -> Result<(), hidapi::HidError> {
    let mut last = None;
    for i in 0..attempts {
        match set_profile(api, info, ui_profile) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if i + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    }
    Err(last.expect("at least one attempt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_set_profile_1() {
        let r = build_set_profile_report(1);
        assert_eq!(&r[0..8], &[0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xfb]);
    }

    #[test]
    fn checksum_set_profile_2() {
        let r = build_set_profile_report(2);
        assert_eq!(&r[0..8], &[0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xfa]);
    }

    #[test]
    fn checksum_get_profile() {
        let r = build_get_profile_report();
        assert_eq!(&r[0..8], &[0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7b]);
    }

    #[test]
    fn ui_and_wire_profiles_round_trip() {
        for ui in 1..=4u8 {
            assert_eq!(wire_profile_to_ui(ui_profile_to_wire(ui)), ui);
        }
        // Out-of-range input is clamped rather than wrapping.
        assert_eq!(ui_profile_to_wire(0), 0);
        assert_eq!(ui_profile_to_wire(9), 3);
    }
}
