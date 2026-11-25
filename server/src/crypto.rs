pub use crate::sys::crypto::{hmac, sha256};
use crate::sys::crypto::{aes_cbc, fill_random};

pub const IV_LEN: usize = 16;
pub const MAC_LEN: usize = 32;

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    fill_random(&mut buf);
    hex(&buf)
}

/// The id the phone keeps a computer under: the start of the SHA-256 of its token. It tells which announced one is already paired.
pub fn id_of(token: &str) -> String {
    hex(&sha256(token.to_ascii_lowercase().as_bytes())[..8])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    bytes.chunks(2).map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?)).collect()
}

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

    /// Keys for handing the token over during pairing, from the secret both sides got by ECDH.
    pub fn for_pairing(secret: &[u8]) -> Self {
        Keys { enc: hmac(secret, b"deskstats-pair-enc"), mac: hmac(secret, b"deskstats-pair-mac") }
    }

    pub fn sign(&self, text: &str) -> String {
        hex(&hmac(&self.mac, text.as_bytes()))
    }

    pub fn seal(&self, plain: &[u8]) -> Option<Vec<u8>> {
        let mut iv = [0u8; IV_LEN];
        fill_random(&mut iv);
        let cipher = aes_cbc(true, &self.enc, &iv, plain)?;
        let mut out = Vec::with_capacity(IV_LEN + cipher.len() + MAC_LEN);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&cipher);
        let tag = hmac(&self.mac, &out);
        out.extend_from_slice(&tag);
        Some(out)
    }

    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < IV_LEN + 16 + MAC_LEN {
            return None;
        }
        let (body, tag) = sealed.split_at(sealed.len() - MAC_LEN);
        if !same(&hmac(&self.mac, body), tag) {
            return None;
        }
        let (iv, cipher) = body.split_at(IV_LEN);
        aes_cbc(false, &self.enc, iv, cipher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open() {
        let keys = Keys::from_token("0123456789abcdef0123456789abcdef");
        let sealed = keys.seal("{\"héllo ✓\":1}".as_bytes()).unwrap();
        assert_eq!(keys.open(&sealed).unwrap(), "{\"héllo ✓\":1}".as_bytes());
        let other = Keys::from_token("ffffffffffffffffffffffffffffffff");
        assert!(other.open(&sealed).is_none());
        let mut broken = sealed.clone();
        broken[20] ^= 1;
        assert!(keys.open(&broken).is_none());
        // AES-256-CBC with PKCS#7 padding and the tag over IV and ciphertext: the same bytes on every system.
        assert_eq!(hex(&aes_cbc(true, &[7u8; 32], &[9u8; 16], b"deskstats").unwrap()), "b889e6f951a0ab387f507e81c21619ad");
        assert_eq!(aes_cbc(false, &[7u8; 32], &[9u8; 16], &aes_cbc(true, &[7u8; 32], &[9u8; 16], b"deskstats").unwrap()).unwrap(), b"deskstats");
        assert!(aes_cbc(false, &[7u8; 32], &[9u8; 16], &[1u8; 15]).is_none());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex("00ff1A"), Some(vec![0, 255, 26]));
        assert_eq!(unhex(&hex(&[1, 2, 250])), Some(vec![1, 2, 250]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn sha256_and_id() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(id_of("ABC"), "ba7816bf8f01cfea");
        assert_eq!(random_hex(16).len(), 32);
        assert_ne!(random_hex(16), random_hex(16));
    }

    #[test]
    fn hmac_matches_known_vector() {
        assert_eq!(hex(&hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }
}
