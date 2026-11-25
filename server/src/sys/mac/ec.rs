//! P-256 key agreement from the Security framework.

use super::ffi::*;
use std::ffi::c_void;

pub const POINT_LEN: usize = 65;

/// An ephemeral P-256 key pair from the Security framework.
pub struct EcKey {
    private: *const c_void,
    pub public: [u8; POINT_LEN],
}

unsafe impl Send for EcKey {}

impl Drop for EcKey {
    fn drop(&mut self) {
        unsafe { CFRelease(self.private) };
    }
}

unsafe fn release(objects: &[*const c_void]) {
    for object in objects {
        if !object.is_null() {
            CFRelease(*object);
        }
    }
}

unsafe fn data_bytes(data: *const c_void) -> Option<Vec<u8>> {
    if data.is_null() || CFGetTypeID(data) != CFDataGetTypeID() {
        return None;
    }
    Some(std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data).max(0) as usize).to_vec())
}

unsafe fn dictionary(pairs: &[(*const c_void, *const c_void)]) -> *const c_void {
    let keys: Vec<_> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<_> = pairs.iter().map(|p| p.1).collect();
    CFDictionaryCreate(
        std::ptr::null(),
        keys.as_ptr(),
        values.as_ptr(),
        pairs.len() as isize,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks,
    )
}

impl EcKey {
    pub fn new() -> Option<EcKey> {
        unsafe {
            let bits: i32 = 256;
            let size = CFNumberCreate(std::ptr::null(), CF_NUMBER_SINT32, &bits as *const i32 as *const c_void);
            let params = dictionary(&[(kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom), (kSecAttrKeySizeInBits, size)]);
            let private = SecKeyCreateRandomKey(params, std::ptr::null_mut());
            release(&[size, params]);
            if private.is_null() {
                return None;
            }
            let public = SecKeyCopyPublicKey(private);
            let data = if public.is_null() { std::ptr::null() } else { SecKeyCopyExternalRepresentation(public, std::ptr::null_mut()) };
            let bytes = data_bytes(data);
            release(&[public, data]);
            match bytes.and_then(|b| <[u8; POINT_LEN]>::try_from(b).ok()) {
                Some(public) => Some(EcKey { private, public }),
                None => {
                    CFRelease(private);
                    None
                }
            }
        }
    }

    /// The X coordinate of our private key times their point; None if the point is not on the curve.
    pub fn shared(&self, theirs: &[u8]) -> Option<[u8; 32]> {
        if theirs.len() != POINT_LEN || theirs[0] != 4 {
            return None;
        }
        unsafe {
            let bits: i32 = 256;
            let size = CFNumberCreate(std::ptr::null(), CF_NUMBER_SINT32, &bits as *const i32 as *const c_void);
            let attributes = dictionary(&[
                (kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom),
                (kSecAttrKeyClass, kSecAttrKeyClassPublic),
                (kSecAttrKeySizeInBits, size),
            ]);
            let data = CFDataCreate(std::ptr::null(), theirs.as_ptr(), theirs.len() as isize);
            let public = SecKeyCreateWithData(data, attributes, std::ptr::null_mut());
            let empty = dictionary(&[]);
            let secret = if public.is_null() {
                std::ptr::null()
            } else {
                SecKeyCopyKeyExchangeResult(self.private, kSecKeyAlgorithmECDHKeyExchangeStandard, public, empty, std::ptr::null_mut())
            };
            let bytes = data_bytes(secret);
            release(&[size, attributes, data, public, empty, secret]);
            bytes.and_then(|b| <[u8; 32]>::try_from(b).ok())
        }
    }
}
