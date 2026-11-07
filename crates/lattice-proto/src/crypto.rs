//! Login encryption: RSA for the shared secret, AES-128/CFB8 for the stream,
//! and Minecraft's signed SHA-1 hex digest for the session server.

use aes::Aes128;
use cfb8::cipher::KeyIvInit;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey};
use sha1::{Digest, Sha1};

pub type Encryptor = cfb8::Encryptor<Aes128>;
pub type Decryptor = cfb8::Decryptor<Aes128>;

pub fn ciphers(secret: &[u8; 16]) -> (Encryptor, Decryptor) {
    (Encryptor::new(secret.into(), secret.into()), Decryptor::new(secret.into(), secret.into()))
}

/// Stateful in-place encryption; CFB8 keeps its state across calls.
pub fn encrypt(c: &mut Encryptor, buf: &mut [u8]) {
    use cfb8::cipher::BlockEncryptMut;
    for b in buf.chunks_mut(1) {
        c.encrypt_block_mut(b.into());
    }
}

pub fn decrypt(c: &mut Decryptor, buf: &mut [u8]) {
    use cfb8::cipher::BlockDecryptMut;
    for b in buf.chunks_mut(1) {
        c.decrypt_block_mut(b.into());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the server's public key is malformed")]
    BadKey,
    #[error("RSA encryption failed")]
    Rsa,
}

/// Encrypts `data` with the server's DER (SubjectPublicKeyInfo) public key, PKCS#1 v1.5.
pub fn rsa_encrypt(public_key_der: &[u8], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key = RsaPublicKey::from_public_key_der(public_key_der).map_err(|_| CryptoError::BadKey)?;
    key.encrypt(&mut rsa::rand_core::OsRng, Pkcs1v15Encrypt, data).map_err(|_| CryptoError::Rsa)
}

/// The `serverId` sent to the session server: SHA-1 over server ID, shared
/// secret and public key, printed as a signed two's-complement hex number.
pub fn server_hash(server_id: &str, secret: &[u8], public_key_der: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(server_id.as_bytes());
    h.update(secret);
    h.update(public_key_der);
    signed_hex_digest(&h.finalize())
}

pub fn signed_hex_digest(digest: &[u8]) -> String {
    let negative = digest[0] & 0x80 != 0;
    let mut bytes = digest.to_vec();
    if negative {
        // Two's complement negation.
        let mut carry = true;
        for b in bytes.iter_mut().rev() {
            *b = !*b;
            if carry {
                let (v, c) = b.overflowing_add(1);
                *b = v;
                carry = c;
            }
        }
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let trimmed = hex.trim_start_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    if negative { format!("-{trimmed}") } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc_sha1(s: &str) -> String {
        signed_hex_digest(&Sha1::digest(s.as_bytes()))
    }

    #[test]
    fn documented_digests() {
        assert_eq!(mc_sha1("Notch"), "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48");
        assert_eq!(mc_sha1("jeb_"), "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
        assert_eq!(mc_sha1("simon"), "88e16a1019277b15d58faf0541e11910eb756f6");
    }

    #[test]
    fn cfb8_streams_across_calls() {
        let key = [7u8; 16];
        let (mut enc, mut dec) = ciphers(&key);
        let plain = b"hello minecraft protocol, this is a long message".to_vec();
        let mut a = plain.clone();
        encrypt(&mut enc, &mut a[..10]);
        encrypt(&mut enc, &mut a[10..]);
        // One-shot encryption must give the same bytes.
        let (mut enc2, _) = ciphers(&key);
        let mut b = plain.clone();
        encrypt(&mut enc2, &mut b);
        assert_eq!(a, b);
        assert_ne!(a, plain);
        decrypt(&mut dec, &mut a[..3]);
        decrypt(&mut dec, &mut a[3..]);
        assert_eq!(a, plain);
    }

    #[test]
    fn cfb8_known_vector() {
        // NIST SP 800-38A F.3.7 CFB8-AES128.Encrypt
        let key: [u8; 16] = [0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c];
        let iv: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut enc = Encryptor::new(&key.into(), &iv.into());
        let mut data = [0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17, 0x2a, 0xae, 0x2d];
        encrypt(&mut enc, &mut data);
        assert_eq!(data, [0x3b, 0x79, 0x42, 0x4c, 0x9c, 0x0d, 0xd4, 0x36, 0xba, 0xce, 0x9e, 0x0e, 0xd4, 0x58, 0x6a, 0x4f, 0x32, 0xb9]);
    }
}
