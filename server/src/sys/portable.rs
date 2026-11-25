//! Cryptography for the systems that do not bring their own: the RustCrypto crates.

pub mod crypto {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};

    pub fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
        // HMAC takes a key of any length, so this cannot fail.
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    pub fn sha256(data: &[u8]) -> [u8; 32] {
        Sha256::digest(data).into()
    }

    pub fn fill_random(buf: &mut [u8]) {
        // Without random numbers nothing here is safe: tokens, IVs and keys all come from them.
        getrandom::getrandom(buf).expect("the system has no random numbers");
    }

    /// AES-256 in CBC mode with PKCS#7 padding; None for a wrong IV length or bad padding.
    pub fn aes_cbc(encrypt: bool, key: &[u8; 32], iv: &[u8], input: &[u8]) -> Option<Vec<u8>> {
        if iv.len() != 16 || (!encrypt && (input.is_empty() || input.len() % 16 != 0)) {
            return None;
        }
        if encrypt {
            Some(cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv).ok()?.encrypt_padded_vec_mut::<Pkcs7>(input))
        } else {
            cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv).ok()?.decrypt_padded_vec_mut::<Pkcs7>(input).ok()
        }
    }
}

pub mod ec {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::{PublicKey, SecretKey};

    pub const POINT_LEN: usize = 65;

    /// An ephemeral P-256 key pair. The secret is wiped when it is dropped.
    pub struct EcKey {
        secret: SecretKey,
        pub public: [u8; POINT_LEN],
    }

    impl EcKey {
        pub fn new() -> Option<EcKey> {
            // A random 32 bytes is a valid key except with negligible probability; try again if not.
            for _ in 0..8 {
                let mut bytes = [0u8; 32];
                super::crypto::fill_random(&mut bytes);
                let secret = SecretKey::from_slice(&bytes);
                bytes.fill(0);
                let Ok(secret) = secret else { continue };
                let point = secret.public_key().to_encoded_point(false);
                let Ok(public) = <[u8; POINT_LEN]>::try_from(point.as_bytes()) else { continue };
                return Some(EcKey { secret, public });
            }
            None
        }

        /// The X coordinate of our private key times their point; None if the point is not on the curve.
        pub fn shared(&self, theirs: &[u8]) -> Option<[u8; 32]> {
            if theirs.len() != POINT_LEN || theirs[0] != 4 {
                return None;
            }
            let public = PublicKey::from_sec1_bytes(theirs).ok()?;
            let secret = p256::ecdh::diffie_hellman(self.secret.to_nonzero_scalar(), public.as_affine());
            <[u8; 32]>::try_from(secret.raw_secret_bytes().as_slice()).ok()
        }
    }
}
