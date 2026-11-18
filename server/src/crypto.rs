//! Encryption of what the phone and the server say to each other, with macOS CommonCrypto.
//!
//! Format: IV (16 bytes) | AES-256-CBC ciphertext (PKCS#7) | HMAC-SHA256 of IV and ciphertext.
//! Both keys are derived from the shared token, which itself is never sent: the phone receives
//! it once over USB. Each side checks the HMAC before decrypting.

use std::ffi::c_void;

const KCC_ENCRYPT: u32 = 0;
const KCC_DECRYPT: u32 = 1;
const KCC_ALGORITHM_AES: u32 = 0;
const KCC_OPTION_PKCS7: u32 = 1;
const KCC_HMAC_SHA256: u32 = 2;
const KCC_SUCCESS: i32 = 0;
pub const IV_LEN: usize = 16;
pub const MAC_LEN: usize = 32;

extern "C" {
    fn CCCrypt(
        op: u32,
        alg: u32,
        options: u32,
        key: *const c_void,
        key_length: usize,
        iv: *const c_void,
        data_in: *const c_void,
        data_in_length: usize,
        data_out: *mut c_void,
        data_out_available: usize,
        data_out_moved: *mut usize,
    ) -> i32;
    fn CCHmac(alg: u32, key: *const c_void, key_length: usize, data: *const c_void, data_length: usize, mac_out: *mut c_void);
    fn arc4random_buf(buf: *mut c_void, nbytes: usize);
}

pub fn hmac(key: &[u8], data: &[u8]) -> [u8; MAC_LEN] {
    let mut out = [0u8; MAC_LEN];
    unsafe {
        CCHmac(KCC_HMAC_SHA256, key.as_ptr() as *const c_void, key.len(), data.as_ptr() as *const c_void, data.len(), out.as_mut_ptr() as *mut c_void)
    };
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes of a hex string; None when it is not one.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    bytes.chunks(2).map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?)).collect()
}

/// Compares in constant time, so a secret cannot be guessed byte by byte from response times.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub struct Keys {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl Keys {
    pub fn from_token(token: &str) -> Self {
        Keys { enc: hmac(token.as_bytes(), b"deskstats-enc"), mac: hmac(token.as_bytes(), b"deskstats-mac") }
    }

    /// HMAC of a text with the MAC key, as hex; signs the Wi-Fi beacon.
    pub fn sign(&self, text: &str) -> String {
        hex(&hmac(&self.mac, text.as_bytes()))
    }

    pub fn seal(&self, plain: &[u8]) -> Option<Vec<u8>> {
        let mut iv = [0u8; IV_LEN];
        unsafe { arc4random_buf(iv.as_mut_ptr() as *mut c_void, IV_LEN) };
        let cipher = self.crypt(KCC_ENCRYPT, &iv, plain)?;
        let mut out = Vec::with_capacity(IV_LEN + cipher.len() + MAC_LEN);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&cipher);
        let tag = hmac(&self.mac, &out);
        out.extend_from_slice(&tag);
        Some(out)
    }

    /// The inverse of seal; None when the data was not sealed with these keys.
    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < IV_LEN + 16 + MAC_LEN {
            return None;
        }
        let (body, tag) = sealed.split_at(sealed.len() - MAC_LEN);
        if !same(&hmac(&self.mac, body), tag) {
            return None;
        }
        let (iv, cipher) = body.split_at(IV_LEN);
        self.crypt(KCC_DECRYPT, iv, cipher)
    }

    fn crypt(&self, op: u32, iv: &[u8], input: &[u8]) -> Option<Vec<u8>> {
        let mut out = vec![0u8; input.len() + 16];
        let mut moved = 0usize;
        let rc = unsafe {
            CCCrypt(
                op,
                KCC_ALGORITHM_AES,
                KCC_OPTION_PKCS7,
                self.enc.as_ptr() as *const c_void,
                self.enc.len(),
                iv.as_ptr() as *const c_void,
                input.as_ptr() as *const c_void,
                input.len(),
                out.as_mut_ptr() as *mut c_void,
                out.len(),
                &mut moved,
            )
        };
        if rc != KCC_SUCCESS {
            return None;
        }
        out.truncate(moved);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open() {
        let keys = Keys::from_token("0123456789abcdef0123456789abcdef");
        let sealed = keys.seal("{\"привет\":1}".as_bytes()).unwrap();
        assert_eq!(keys.open(&sealed).unwrap(), "{\"привет\":1}".as_bytes());
        let other = Keys::from_token("ffffffffffffffffffffffffffffffff");
        assert!(other.open(&sealed).is_none());
        let mut broken = sealed.clone();
        broken[20] ^= 1;
        assert!(keys.open(&broken).is_none());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex("00ff1A"), Some(vec![0, 255, 26]));
        assert_eq!(unhex(&hex(&[1, 2, 250])), Some(vec![1, 2, 250]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn hmac_matches_known_vector() {
        // RFC 4231 test case 2
        assert_eq!(hex(&hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }
}
