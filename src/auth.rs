use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn code6() -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("rng");
    let n = u32::from_le_bytes(b) % 1_000_000;
    format!("{n:06}")
}

pub fn otp_hash(secret: &str, email: &str, code: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(email.trim().to_lowercase().as_bytes());
    mac.update(b":");
    mac.update(code.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
