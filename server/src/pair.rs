//! Pairing over Wi‑Fi. The phone and the Mac agree on a secret by ECDH (P-256), both show a six-digit code made from it,
//! and the Mac asks its user whether to let the phone in. Only then does the Mac hand over its token, sealed with the secret.
//! Someone in the middle would get other secrets on both sides, and so other codes.

use crate::crypto::{hex, hmac, random_hex, Keys};
use crate::ffi::*;
use crate::util::log;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const POINT_LEN: usize = 65;
const DIALOG: Duration = Duration::from_secs(60);
const FORGET_AFTER: Duration = Duration::from_secs(90);
const AFTER_DENIAL: Duration = Duration::from_secs(5);
pub const WAIT: Duration = Duration::from_secs(20);

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

/// The six digits both screens show. Both public keys go in, so a swapped key changes the code.
pub fn code(secret: &[u8], phone: &[u8], mac: &[u8]) -> String {
    let mut input = b"deskstats-pair-code".to_vec();
    input.extend_from_slice(phone);
    input.extend_from_slice(mac);
    let tag = hmac(secret, &input);
    let n = u32::from_be_bytes([tag[0], tag[1], tag[2], tag[3]]) % 1_000_000;
    format!("{n:06}")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Answer {
    Waiting,
    Allowed,
    Denied,
}

struct Pending {
    session: String,
    secret: [u8; 32],
    started: Instant,
    answer: Answer,
}

#[derive(Default)]
pub struct Pairing {
    pending: Mutex<(Option<Pending>, Option<Instant>)>,
    decided: Condvar,
}

pub enum Start {
    Started { mac_key: [u8; POINT_LEN], session: String, code: String },
    Busy,
    BadKey,
}

pub enum Outcome {
    Token(String),
    Denied,
    Waiting,
    Unknown,
}

impl Pairing {
    /// Begins a pairing with the phone's public key. Only one at a time, so nobody can fill the screen with questions.
    pub fn start(&self, phone_key: &[u8]) -> Start {
        let mut state = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = &state.0 {
            if p.started.elapsed() < FORGET_AFTER && p.answer == Answer::Waiting {
                return Start::Busy;
            }
        }
        if state.1.is_some_and(|denied| denied.elapsed() < AFTER_DENIAL) {
            return Start::Busy;
        }
        let Some(key) = EcKey::new() else { return Start::BadKey };
        let Some(secret) = key.shared(phone_key) else { return Start::BadKey };
        let session = random_hex(16);
        let code = code(&secret, phone_key, &key.public);
        state.0 = Some(Pending { session: session.clone(), secret, started: Instant::now(), answer: Answer::Waiting });
        Start::Started { mac_key: key.public, session, code }
    }

    pub fn decide(&self, session: &str, allowed: bool) {
        let mut state = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = state.0.as_mut().filter(|p| p.session == session && p.answer == Answer::Waiting) {
            p.answer = if allowed { Answer::Allowed } else { Answer::Denied };
            if !allowed {
                state.1 = Some(Instant::now());
            }
        }
        self.decided.notify_all();
    }

    /// Waits up to [wait] for the answer. An allowed phone gets the token sealed with the shared secret, once.
    pub fn wait(&self, session: &str, token: &str, wait: Duration) -> Outcome {
        let until = Instant::now() + wait;
        let mut state = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let Some(p) = state.0.as_ref().filter(|p| p.session == session && p.started.elapsed() < FORGET_AFTER) else {
                return Outcome::Unknown;
            };
            match p.answer {
                Answer::Allowed => {
                    let sealed = Keys::for_pairing(&p.secret).seal(token.as_bytes());
                    state.0 = None;
                    return sealed.map_or(Outcome::Unknown, |s| Outcome::Token(hex(&s)));
                }
                Answer::Denied => {
                    state.0 = None;
                    return Outcome::Denied;
                }
                Answer::Waiting => {}
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Outcome::Waiting;
            }
            state = self.decided.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

/// Asks the user of this Mac in a dialog; true only for a click on Allow.
pub fn ask_user(phone: &str, code: &str, language: &str) -> bool {
    let (message, deny, allow) = words(language);
    let spaced = format!("{} {}", &code[..3], &code[3..]);
    let text = message.replace("{phone}", phone).replace("{code}", &spaced);
    let script = [
        "on run argv",
        "set r to display dialog (item 1 of argv) with title \"Deskstats\" buttons {item 2 of argv, item 3 of argv} default button 2 with icon caution giving up after (item 4 of argv as integer)",
        "if gave up of r then return \"\"",
        "return button returned of r",
        "end run",
    ];
    let mut args: Vec<&str> = Vec::new();
    for line in &script {
        args.push("-e");
        args.push(line);
    }
    let seconds = DIALOG.as_secs().to_string();
    args.extend_from_slice(&["--", &text, deny, allow, &seconds]);
    let answer = crate::util::run("/usr/bin/osascript", &args, DIALOG + Duration::from_secs(10));
    let allowed = answer.as_ref().is_some_and(|(ok, out)| *ok && out.trim() == allow);
    log(if allowed { "pair: the user let a phone in over Wi-Fi" } else { "pair: a phone was not let in" });
    allowed
}

fn words(language: &str) -> (&'static str, &'static str, &'static str) {
    match language {
        "ru" => ("Телефон «{phone}» хочет показывать статистику и уведомления этого Mac.\n\nКод: {code}\n\nРазрешайте, только если на телефоне тот же код.", "Запретить", "Разрешить"),
        "de" => ("Das Telefon „{phone}“ möchte die Werte und Mitteilungen dieses Mac anzeigen.\n\nCode: {code}\n\nNur erlauben, wenn das Telefon denselben Code zeigt.", "Ablehnen", "Erlauben"),
        "es" => ("El teléfono «{phone}» quiere mostrar los datos y las notificaciones de este Mac.\n\nCódigo: {code}\n\nPermítelo solo si el teléfono muestra el mismo código.", "Rechazar", "Permitir"),
        "fr" => ("Le téléphone « {phone} » veut afficher les données et les notifications de ce Mac.\n\nCode : {code}\n\nN’autorisez que si le téléphone affiche le même code.", "Refuser", "Autoriser"),
        "pt" => ("O telefone “{phone}” quer mostrar os dados e as notificações deste Mac.\n\nCódigo: {code}\n\nPermita apenas se o telefone mostrar o mesmo código.", "Recusar", "Permitir"),
        "zh" => ("手机“{phone}”想要显示这台 Mac 的状态和通知。\n\n代码：{code}\n\n只有手机上显示相同的代码时才允许。", "拒绝", "允许"),
        "ja" => ("スマートフォン「{phone}」がこの Mac の状態と通知を表示しようとしています。\n\nコード：{code}\n\nスマートフォンに同じコードが表示されている場合のみ許可してください。", "拒否", "許可"),
        _ => ("The phone “{phone}” wants to show this Mac’s stats and notifications.\n\nCode: {code}\n\nAllow only if the phone shows the same code.", "Deny", "Allow"),
    }
}

/// Decodes a query value the phone percent-encoded; drops control characters and keeps at most [limit] characters.
pub fn decode(value: &str, limit: usize) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match value.get(i + 1..i + 3).and_then(crate::crypto::unhex) {
                Some(b) => {
                    out.push(b[0]);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).chars().filter(|c| !c.is_control()).take(limit).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_get_the_same_secret() {
        let (a, b) = (EcKey::new().unwrap(), EcKey::new().unwrap());
        assert_eq!(a.public[0], 4);
        let secret = a.shared(&b.public).unwrap();
        assert_eq!(Some(secret), b.shared(&a.public));
        assert_ne!(EcKey::new().unwrap().shared(&b.public), Some(secret));
        let mut off_curve = b.public;
        off_curve[40] ^= 1;
        assert_eq!(a.shared(&off_curve), None);
        assert_eq!(a.shared(&b.public[..64]), None);
        let fixed: Vec<u8> = (0..32).collect();
        assert_eq!(super::code(&fixed, &[1], &[2]), "642103");
        let code = code(&secret, &a.public, &b.public);
        assert_eq!(code.len(), 6);
        assert_ne!(code, super::code(&secret, &b.public, &a.public));
    }

    #[test]
    fn the_token_goes_only_to_an_allowed_phone() {
        let pairing = Pairing::default();
        let phone = EcKey::new().unwrap();
        let Start::Started { mac_key, session, .. } = pairing.start(&phone.public) else { panic!() };
        assert!(matches!(pairing.start(&EcKey::new().unwrap().public), Start::Busy));
        assert!(matches!(pairing.wait(&session, "abc", Duration::from_millis(10)), Outcome::Waiting));
        assert!(matches!(pairing.wait("other", "abc", Duration::from_millis(10)), Outcome::Unknown));
        pairing.decide(&session, true);
        let Outcome::Token(sealed) = pairing.wait(&session, "0123456789abcdef0123456789abcdef", Duration::ZERO) else { panic!() };
        let keys = Keys::for_pairing(&phone.shared(&mac_key).unwrap());
        assert_eq!(keys.open(&crate::crypto::unhex(&sealed).unwrap()).unwrap(), b"0123456789abcdef0123456789abcdef");
        assert!(matches!(pairing.wait(&session, "abc", Duration::ZERO), Outcome::Unknown));
        let Start::Started { session, .. } = pairing.start(&phone.public) else { panic!() };
        pairing.decide(&session, false);
        assert!(matches!(pairing.wait(&session, "abc", Duration::ZERO), Outcome::Denied));
        assert!(matches!(pairing.start(&phone.public), Start::Busy));
        assert!(matches!(pairing.start(&[4u8; 65]), Start::Busy));
    }

    #[test]
    fn a_bad_key_is_refused() {
        assert!(matches!(Pairing::default().start(&[4u8; 65]), Start::BadKey));
        assert!(matches!(Pairing::default().start(b"x"), Start::BadKey));
    }

    #[test]
    fn decodes_the_phone_name() {
        assert_eq!(decode("Samsung+SM-J710F", 40), "Samsung SM-J710F");
        assert_eq!(decode("%D0%A2%D0%B5%D0%BB", 40), "Тел");
        assert_eq!(decode("a%0Ab%2", 40), "ab%2");
        assert_eq!(decode("abcdef", 3), "abc");
        assert_eq!(decode("%zz", 3), "%zz");
    }
}
