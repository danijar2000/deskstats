//! The primitives from CommonCrypto, which every Mac has.

use std::ffi::c_void;

const KCC_ENCRYPT: u32 = 0;
const KCC_DECRYPT: u32 = 1;
const KCC_ALGORITHM_AES: u32 = 0;
const KCC_OPTION_PKCS7: u32 = 1;
const KCC_HMAC_SHA256: u32 = 2;
const KCC_SUCCESS: i32 = 0;

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
    fn CC_SHA256(data: *const c_void, len: u32, md: *mut u8) -> *mut u8;
    fn arc4random_buf(buf: *mut c_void, nbytes: usize);
}

pub fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    unsafe {
        CCHmac(KCC_HMAC_SHA256, key.as_ptr() as *const c_void, key.len(), data.as_ptr() as *const c_void, data.len(), out.as_mut_ptr() as *mut c_void)
    };
    out
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    unsafe { CC_SHA256(data.as_ptr() as *const c_void, data.len() as u32, out.as_mut_ptr()) };
    out
}

pub fn fill_random(buf: &mut [u8]) {
    unsafe { arc4random_buf(buf.as_mut_ptr() as *mut c_void, buf.len()) };
}

/// AES-256 in CBC mode with PKCS#7 padding; None for a wrong key or IV length or bad padding.
pub fn aes_cbc(encrypt: bool, key: &[u8; 32], iv: &[u8], input: &[u8]) -> Option<Vec<u8>> {
    if iv.len() != 16 || (!encrypt && (input.is_empty() || input.len() % 16 != 0)) {
        return None;
    }
    let mut out = vec![0u8; input.len() + 16];
    let mut moved = 0usize;
    let rc = unsafe {
        CCCrypt(
            if encrypt { KCC_ENCRYPT } else { KCC_DECRYPT },
            KCC_ALGORITHM_AES,
            KCC_OPTION_PKCS7,
            key.as_ptr() as *const c_void,
            key.len(),
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
