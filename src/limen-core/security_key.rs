//! A YubiKey answering HMAC-SHA1 challenge-response from one of its two OTP
//! slots — the security key a vault can need besides its password.
//!
//! Spoken over the key's OTP interface as HID feature reports, as `ykman` and
//! KeePassXC do: nothing is detached from the system, so the key's keyboard,
//! FIDO and smart-card functions keep working while Limen asks it. The
//! exchange follows Yubico's: a 70-byte frame (the challenge, the slot's
//! command, a CRC) sent as 8-byte reports, and a 22-byte answer (20 bytes of
//! HMAC-SHA1 and a CRC) read back the same way. After the frame logic of the
//! `challenge_response` crate (MIT / Apache-2.0).
//!
//! A slot set up to need a touch makes the key blink and wait; the answer
//! comes once it is touched, or not at all after [`TOUCH_WAIT`].

use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::vault::{ChallengeResponse, VaultError};

const YUBICO: u16 = 0x1050;
/// The YubiKeys with an OTP interface: 1–2, NEO, 4, 5.
const PRODUCTS: &[u16] = &[0x0010, 0x0110, 0x0111, 0x0114, 0x0116, 0x0401, 0x0403, 0x0405, 0x0407, 0x0410];

/// The challenge-response commands, slot 1 and slot 2.
const CHAL_HMAC: [u8; 2] = [0x30, 0x38];
/// "What is your serial number?" — answered without a touch.
const DEVICE_SERIAL: u8 = 0x10;
const PAYLOAD: usize = 64;
const FRAME: usize = PAYLOAD + 1 + 2 + 3;
/// The answer: 20 bytes of HMAC, its CRC, and what the reports pad it with.
const ANSWER: usize = 36;

/// In the last byte of a status report.
const WRITING: u8 = 0x80;
const PENDING: u8 = 0x40;
const AWAITING_TOUCH: u8 = 0x20;
const RESET: u8 = 0x8f;

/// How long the key may take to start answering when it is not waiting for a
/// touch: an untouched slot answers at once, so this is a slot that does not.
const NO_ANSWER: Duration = Duration::from_secs(3);
/// How long a touch is waited for.
pub const TOUCH_WAIT: Duration = Duration::from_secs(20);

const CRC_OK: u16 = 0xf0b8;

fn err(why: impl Into<String>) -> VaultError {
    VaultError::SecurityKey(why.into())
}

/// CRC-16 as the YubiKey keeps it (reflected 0x1021, from 0xffff).
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            let low = crc & 1;
            crc >>= 1;
            if low != 0 {
                crc ^= 0x8408;
            }
        }
    }
    crc
}

/// The frame for asking `slot` about `challenge`.
fn frame(slot: u8, challenge: &[u8]) -> Result<[u8; FRAME], VaultError> {
    let command = match slot {
        1 | 2 => CHAL_HMAC[slot as usize - 1],
        _ => return Err(err(format!("there is no slot {slot}: a YubiKey has slots 1 and 2"))),
    };
    if challenge.len() > PAYLOAD {
        return Err(err("the challenge is longer than 64 bytes"));
    }
    Ok(command_frame(command, challenge))
}

/// A command's frame: the payload padded to 64 bytes, the command, the
/// payload's CRC (little-endian), and filler.
fn command_frame(command: u8, payload: &[u8]) -> [u8; FRAME] {
    let mut f = [0u8; FRAME];
    f[..payload.len()].copy_from_slice(payload);
    f[PAYLOAD] = command;
    let crc = crc16(&f[..PAYLOAD]).to_le_bytes();
    f[PAYLOAD + 1..PAYLOAD + 3].copy_from_slice(&crc);
    f
}

/// The reports a frame is sent as: 7 bytes each, numbered, the all-zero ones
/// in the middle left out (the key fills them in).
fn reports(f: &[u8; FRAME]) -> Vec<[u8; 8]> {
    let chunks: Vec<&[u8]> = f.chunks(7).collect();
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .filter(|(seq, c)| *seq == 0 || *seq == last || c.iter().any(|&b| b != 0))
        .map(|(seq, c)| {
            let mut r = [0u8; 8];
            r[..7].copy_from_slice(c);
            r[7] = WRITING | seq as u8;
            r
        })
        .collect()
}

/// A YubiKey found plugged in.
pub struct YubiKey {
    path: std::ffi::CString,
    /// What it calls itself: "YubiKey OTP+FIDO+CCID", say.
    pub name: String,
    /// Its serial number, if it tells (a YubiKey can be set not to).
    pub serial: Option<u32>,
}

fn is_otp(d: &hidapi::DeviceInfo) -> bool {
    d.vendor_id() == YUBICO
        && PRODUCTS.contains(&d.product_id())
        // The OTP interface: a keyboard, interface 0.
        && (d.interface_number() == 0 || (d.usage_page() == 0x01 && d.usage() == 0x06))
}

/// Every YubiKey plugged in — a primary and its backup may both be.
pub fn find_all() -> Result<Vec<YubiKey>, VaultError> {
    let api = hidapi::HidApi::new().map_err(|e| err(format!("USB devices could not be listed: {e}")))?;
    let mut keys: Vec<YubiKey> = Vec::new();
    for d in api.device_list().filter(|d| is_otp(d)) {
        let mut key = YubiKey {
            path: d.path().to_owned(),
            name: d.product_string().unwrap_or("YubiKey").to_string(),
            serial: None,
        };
        key.serial = key.read_serial().ok();
        // One key can show more than one OTP-like interface.
        if !keys.iter().any(|k| k.path == key.path || (key.serial.is_some() && k.serial == key.serial)) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// The first YubiKey plugged in, or None.
pub fn find() -> Result<Option<YubiKey>, VaultError> {
    Ok(find_all()?.into_iter().next())
}

impl YubiKey {
    fn open(&self) -> Result<hidapi::HidDevice, VaultError> {
        let api = hidapi::HidApi::new().map_err(|e| err(format!("USB devices could not be listed: {e}")))?;
        api.open_path(&self.path).map_err(|e| {
            err(format!("the YubiKey could not be opened ({e}) — is it still plugged in, and may Limen use it?"))
        })
    }

    fn read_serial(&self) -> Result<u32, VaultError> {
        let dev = self.open()?;
        let (answer, at) = ask(&dev, &command_frame(DEVICE_SERIAL, &[]), "")?;
        if at < 6 || crc16(&answer[..6]) != CRC_OK {
            return Err(err("the YubiKey's serial number was garbled"));
        }
        Ok(u32::from_be_bytes([answer[0], answer[1], answer[2], answer[3]]))
    }
}

/// The last 8-byte status report.
fn status(dev: &hidapi::HidDevice) -> Result<[u8; 8], VaultError> {
    let mut buf = [0u8; 9]; // report id 0, then the report
    let n = dev.get_feature_report(&mut buf).map_err(|e| err(format!("the YubiKey stopped answering: {e}")))?;
    let mut r = [0u8; 8];
    // hidapi counts the report id in; some platforms leave it out.
    let body = if n >= 9 { &buf[1..9] } else { &buf[..8] };
    r.copy_from_slice(body);
    Ok(r)
}

fn send(dev: &hidapi::HidDevice, report: &[u8; 8]) -> Result<(), VaultError> {
    let mut buf = [0u8; 9];
    buf[1..].copy_from_slice(report);
    dev.send_feature_report(&buf).map_err(|e| err(format!("the YubiKey could not be written to: {e}")))
}

/// Wait for the key to take the next report.
fn wait_writable(dev: &hidapi::HidDevice) -> Result<(), VaultError> {
    let until = Instant::now() + Duration::from_secs(2);
    while status(dev)?[7] & WRITING != 0 {
        if Instant::now() > until {
            return Err(err("the YubiKey is busy"));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Send a frame, and read the key's answer: as soon as it has one, or once it
/// is touched. `slot_hint` names the slot in the error for a key that never
/// answers. Returns the answer and how much of it came.
fn ask(dev: &hidapi::HidDevice, f: &[u8; FRAME], slot_hint: &str) -> Result<(Zeroizing<[u8; ANSWER]>, usize), VaultError> {
    for r in reports(f) {
        wait_writable(dev)?;
        send(dev, &r)?;
    }
    let started = Instant::now();
    let mut report = loop {
        let s = status(dev)?;
        if s[7] & PENDING != 0 {
            break s;
        }
        let waited = started.elapsed();
        if s[7] & AWAITING_TOUCH != 0 {
            if waited > TOUCH_WAIT {
                let _ = send(dev, &[0, 0, 0, 0, 0, 0, 0, RESET]);
                return Err(err("the YubiKey was not touched in time"));
            }
        } else if waited > NO_ANSWER {
            return Err(err(format!("the YubiKey did not answer{slot_hint}")));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut answer = Zeroizing::new([0u8; ANSWER]);
    let mut at = 0;
    loop {
        let take = (ANSWER - at).min(7);
        answer[at..at + take].copy_from_slice(&report[..take]);
        at += take;
        if at >= ANSWER {
            break;
        }
        report = status(dev)?;
        let flags = report[7];
        // Done when the key stops marking reports pending, or starts over.
        if flags & PENDING == 0 || (flags & 0x1f) == 0 {
            break;
        }
    }
    report.iter_mut().for_each(|b| *b = 0);
    send(dev, &[0, 0, 0, 0, 0, 0, 0, RESET])?;
    let _ = wait_writable(dev);
    Ok((answer, at))
}

impl ChallengeResponse for YubiKey {
    fn respond(&self, slot: u8, challenge: &[u8]) -> Result<Zeroizing<[u8; 20]>, VaultError> {
        let f = frame(slot, challenge)?;
        let dev = self.open()?;
        let hint = format!(" — is its slot {slot} set up for HMAC-SHA1 challenge-response?");
        let (answer, at) = ask(&dev, &f, &hint)?;
        if at < 22 || crc16(&answer[..22]) != CRC_OK {
            return Err(err("the YubiKey's answer was garbled"));
        }
        let mut hmac = Zeroizing::new([0u8; 20]);
        hmac.copy_from_slice(&answer[..20]);
        Ok(hmac)
    }

    fn serial(&self) -> Option<u32> {
        self.serial
    }

    fn name(&self) -> String {
        match self.serial {
            Some(n) => format!("{} · {n}", self.name),
            None => self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_is_the_yubikeys() {
        // A frame's own CRC, appended little-endian, leaves the residual.
        let data = b"limen";
        let mut with = data.to_vec();
        with.extend_from_slice(&(!crc16(data)).to_le_bytes());
        assert_eq!(crc16(&with), CRC_OK);
    }

    #[test]
    fn a_frame_says_which_slot_and_carries_its_crc() {
        let f = frame(2, &[7u8; 64]).unwrap();
        assert_eq!(f[64], 0x38, "slot 2");
        assert_eq!(frame(1, &[7u8; 64]).unwrap()[64], 0x30, "slot 1");
        assert_eq!(u16::from_le_bytes([f[65], f[66]]), crc16(&f[..64]));
        assert!(frame(3, &[0u8; 4]).is_err());
        assert!(frame(1, &[0u8; 65]).is_err());
    }

    #[test]
    fn a_frame_goes_as_numbered_reports_the_zero_ones_left_out() {
        let mut challenge = [0u8; 64];
        challenge[0] = 1; // only the first report has anything in the challenge
        let f = frame(1, &challenge).unwrap();
        let r = reports(&f);
        let seqs: Vec<u8> = r.iter().map(|r| r[7] & 0x1f).collect();
        // 70 bytes are reports 0 to 9: the first, and the last — which holds
        // the command and the CRC. The empty ones between are left out.
        assert_eq!(seqs, vec![0, 9]);
        assert!(r.iter().all(|r| r[7] & WRITING != 0));
        assert_eq!(r[0][0], 1);
    }

    /// With a YubiKey plugged in whose `LIMEN_YUBIKEY_SLOT` is set up for
    /// challenge-response: it answers, and the same question the same way.
    /// Never prints the answer.
    #[test]
    #[ignore = "needs a YubiKey plugged in; set LIMEN_YUBIKEY_SLOT"]
    fn a_real_yubikey_answers_the_same_question_the_same_way() {
        let slot: u8 = std::env::var("LIMEN_YUBIKEY_SLOT").expect("LIMEN_YUBIKEY_SLOT").parse().unwrap();
        let key = find().unwrap().expect("no YubiKey plugged in");
        assert!(key.serial.is_some(), "a YubiKey 5 tells its serial");
        let a = key.respond(slot, &[0x5a; 64]).unwrap();
        let b = key.respond(slot, &[0x5a; 64]).unwrap();
        let c = key.respond(slot, &[0xa5; 64]).unwrap();
        assert_eq!(*a, *b, "the same question, the same answer");
        assert_ne!(*a, *c, "another question, another answer");
        println!("{}: slot {slot} answered", key.name);
    }
}
