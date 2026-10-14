//! Sealing of locked payout addresses (Phase 5).
//!
//! A stream's payout address has to be stored so the worker can withdraw on
//! demand without re-entering it (Phase 3 deliberately stored none). It is
//! stored *sealed*: XChaCha20-Poly1305 under `PAYOUT_ADDRESS_KEY`, a key held
//! only by payroll-api, with the stream id as associated data (a sealed
//! address copied onto another stream does not open). A dump of the database
//! alone therefore reveals no address. Format: `v1.` + hex(nonce ‖ ciphertext).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const AAD_PREFIX: &[u8] = b"hushush-payout-address-v1\0";

pub struct AddressSealer(XChaCha20Poly1305);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn aad(stream_id: &str) -> Vec<u8> {
    [AAD_PREFIX, stream_id.as_bytes()].concat()
}

impl AddressSealer {
    /// From 64 hex characters (32 bytes).
    pub fn from_hex(key: &str) -> Result<Self, String> {
        let k = unhex(key.trim())
            .filter(|k| k.len() == 32)
            .ok_or("PAYOUT_ADDRESS_KEY must be 64 hex characters (32 random bytes)")?;
        if k.iter().all(|&b| b == k[0]) {
            return Err("PAYOUT_ADDRESS_KEY must be random".into());
        }
        Ok(Self(XChaCha20Poly1305::new(k.as_slice().into())))
    }

    pub fn seal(&self, stream_id: &str, address: &str) -> Result<String, String> {
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|e| format!("no randomness: {e}"))?;
        let ct = self
            .0
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: address.as_bytes(),
                    aad: &aad(stream_id),
                },
            )
            .map_err(|_| "sealing failed".to_string())?;
        Ok(format!("v1.{}{}", hex(&nonce), hex(&ct)))
    }

    pub fn open(&self, stream_id: &str, sealed: &str) -> Result<String, String> {
        let raw = sealed
            .strip_prefix("v1.")
            .and_then(unhex)
            .filter(|r| r.len() > 24)
            .ok_or("unrecognised sealed address")?;
        let (nonce, ct) = raw.split_at(24);
        let pt = self
            .0
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: &aad(stream_id),
                },
            )
            .map_err(|_| "sealed address does not open with this key and stream".to_string())?;
        String::from_utf8(pt).map_err(|_| "sealed address is not text".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn round_trip_is_randomised_and_bound_to_the_stream() {
        let s = AddressSealer::from_hex(KEY).unwrap();
        let a = s.seal("stream-1", "utest1abc").unwrap();
        let b = s.seal("stream-1", "utest1abc").unwrap();
        assert_ne!(a, b, "fresh nonce per seal");
        assert!(!a.contains("utest1abc"));
        assert_eq!(s.open("stream-1", &a).unwrap(), "utest1abc");
        assert!(s.open("stream-2", &a).is_err(), "bound to its stream");
        let other = AddressSealer::from_hex(&KEY.replace("00", "ff")).unwrap();
        assert!(other.open("stream-1", &a).is_err(), "needs the key");
        let mut tampered = a.clone();
        tampered.pop();
        tampered.push(if a.ends_with('0') { '1' } else { '0' });
        assert!(s.open("stream-1", &tampered).is_err());
    }

    #[test]
    fn key_must_be_32_random_bytes() {
        assert!(AddressSealer::from_hex("abcd").is_err());
        assert!(AddressSealer::from_hex(&"00".repeat(32)).is_err());
        assert!(AddressSealer::from_hex(KEY).is_ok());
    }
}
