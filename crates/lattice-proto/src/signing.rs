//! Signing chat, for the servers that insist on knowing it's really from you, and the record of
//! what's been seen that goes with each message.

use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey};
use sha2::Sha256;
use uuid::Uuid;

pub type Signature = [u8; 256];

/// Reads a key from the text form Mojang hands out.
pub fn private_key_from_pem(pem: &str) -> Result<RsaPrivateKey, String> {
    use base64::Engine;
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body).map_err(|e| e.to_string())?;
    RsaPrivateKey::from_pkcs8_der(&der).or_else(|_| RsaPrivateKey::from_pkcs1_der(&der)).map_err(|e| e.to_string())
}

/// The bytes between the armour of a public key.
pub fn der_from_pem(pem: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
    base64::engine::general_purpose::STANDARD.decode(body).map_err(|e| e.to_string())
}

/// Signs one message after another: each signature counts on the one before.
pub struct ChatSigner {
    key: SigningKey<Sha256>,
    pub session: Uuid,
    player: Uuid,
    index: i32,
}

impl ChatSigner {
    pub fn new(key: RsaPrivateKey, player: Uuid, session: Uuid) -> ChatSigner {
        ChatSigner { key: SigningKey::<Sha256>::new(key), session, player, index: 0 }
    }

    /// What exactly gets signed.
    fn payload(&self, message: &str, millis: i64, salt: i64, seen: &[Signature]) -> Vec<u8> {
        let text = message.as_bytes();
        let mut out = Vec::with_capacity(60 + text.len() + seen.len() * 256);
        out.extend_from_slice(&1i32.to_be_bytes());
        out.extend_from_slice(self.player.as_bytes());
        out.extend_from_slice(self.session.as_bytes());
        out.extend_from_slice(&self.index.to_be_bytes());
        out.extend_from_slice(&salt.to_be_bytes());
        out.extend_from_slice(&millis.div_euclid(1000).to_be_bytes());
        out.extend_from_slice(&(text.len() as i32).to_be_bytes());
        out.extend_from_slice(text);
        out.extend_from_slice(&(seen.len() as i32).to_be_bytes());
        for s in seen {
            out.extend_from_slice(s);
        }
        out
    }

    pub fn sign(&mut self, message: &str, millis: i64, salt: i64, seen: &[Signature]) -> Signature {
        let payload = self.payload(message, millis, salt, seen);
        self.index = self.index.wrapping_add(1);
        let bytes = self.key.sign(&payload).to_vec();
        let mut out = [0u8; 256];
        let n = bytes.len().min(256);
        out[..n].copy_from_slice(&bytes[..n]);
        out
    }
}

/// Java's hash of a byte array, which the seen-list's checksum is built from.
fn java_bytes_hash(bytes: &[u8]) -> i32 {
    let mut h = 1i32;
    for b in bytes {
        h = h.wrapping_mul(31).wrapping_add(*b as i8 as i32);
    }
    h
}

/// What goes with a message to say what's been seen.
pub struct SeenUpdate {
    pub signatures: Vec<Signature>,
    pub offset: i32,
    pub acknowledged: [u8; 3],
    pub checksum: u8,
}

/// The last twenty signed messages, and how many have come since the server last heard.
pub struct SeenMessages {
    tracked: [Option<Signature>; 20],
    tail: usize,
    offset: i32,
    last: Option<Signature>,
}

impl Default for SeenMessages {
    fn default() -> Self {
        SeenMessages { tracked: [None; 20], tail: 0, offset: 0, last: None }
    }
}

impl SeenMessages {
    /// Notes a message. Returns how many to acknowledge right now, when too many have built up.
    pub fn add(&mut self, signature: Signature) -> Option<i32> {
        if self.last == Some(signature) {
            return None;
        }
        self.last = Some(signature);
        self.tracked[self.tail] = Some(signature);
        self.tail = (self.tail + 1) % 20;
        self.offset += 1;
        (self.offset > 64).then(|| std::mem::take(&mut self.offset))
    }

    /// The state of things, for sending with a message.
    pub fn update(&mut self) -> SeenUpdate {
        let offset = std::mem::take(&mut self.offset);
        let mut bits = 0u32;
        let mut signatures = Vec::new();
        for i in 0..20 {
            if let Some(s) = self.tracked[(self.tail + i) % 20] {
                bits |= 1 << i;
                signatures.push(s);
            }
        }
        let mut sum = 1i32;
        for s in &signatures {
            sum = sum.wrapping_mul(31).wrapping_add(java_bytes_hash(s));
        }
        let checksum = sum as u8;
        SeenUpdate { signatures, offset, acknowledged: [bits as u8, (bits >> 8) as u8, (bits >> 16) as u8], checksum: if checksum == 0 { 1 } else { checksum } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::VerifyingKey;
    use rsa::signature::Verifier;

    #[test]
    fn signatures_verify_and_each_one_differs() {
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let public = key.to_public_key();
        let player = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        let mut signer = ChatSigner::new(key, player, session);
        let payload = signer.payload("hello", 1_700_000_000_123, 42, &[]);
        // Fixed layout: version, sender, session, index, salt, seconds, length, text, count.
        assert_eq!(&payload[..4], &[0, 0, 0, 1]);
        assert_eq!(&payload[4..20], player.as_bytes());
        assert_eq!(&payload[20..36], session.as_bytes());
        assert_eq!(&payload[36..40], &[0, 0, 0, 0]);
        assert_eq!(&payload[40..48], &42i64.to_be_bytes());
        assert_eq!(&payload[48..56], &1_700_000_000i64.to_be_bytes());
        assert_eq!(&payload[56..60], &5i32.to_be_bytes());
        assert_eq!(&payload[60..65], b"hello");
        assert_eq!(&payload[65..69], &[0, 0, 0, 0]);
        let first = signer.sign("hello", 1_700_000_000_123, 42, &[]);
        let check = VerifyingKey::<Sha256>::new(public);
        check.verify(&payload, &rsa::pkcs1v15::Signature::try_from(first.as_slice()).unwrap()).unwrap();
        // The next signs its own place in the chain.
        let next = signer.payload("hello", 1_700_000_000_123, 42, &[]);
        assert_eq!(&next[36..40], &[0, 0, 0, 1]);
    }

    #[test]
    fn seen_list_counts_as_the_game_does() {
        let mut seen = SeenMessages::default();
        let empty = seen.update();
        assert_eq!((empty.offset, empty.acknowledged, empty.checksum), (0, [0, 0, 0], 1));
        let (a, b) = ([1u8; 256], [2u8; 256]);
        assert_eq!(seen.add(a), None);
        // The same message twice in a row counts once.
        assert_eq!(seen.add(a), None);
        assert_eq!(seen.add(b), None);
        let u = seen.update();
        assert_eq!(u.offset, 2);
        assert_eq!(u.signatures, vec![a, b]);
        // Filled from where the oldest would be: the two sit at the end of the window.
        assert_eq!(u.acknowledged, [0, 0, 0b1100]);
        assert_eq!(seen.update().offset, 0);
        // Too many unacknowledged, and the server's told.
        let mut total = None;
        for i in 0..65 {
            let mut s = [0u8; 256];
            s[0] = i as u8 + 3;
            total = seen.add(s).or(total);
        }
        assert_eq!(total, Some(65));
    }
}
