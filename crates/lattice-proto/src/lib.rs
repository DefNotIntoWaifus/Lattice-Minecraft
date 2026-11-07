//! Minecraft: Java Edition network protocol, version 26.3 (protocol 777).

pub mod codec;
pub mod crypto;
pub mod frame;
pub mod item;
pub mod nbt;
pub mod packets;
pub mod recipes;
pub mod signing;
pub mod text;

/// Packet IDs generated from Mojang's packet report.
pub mod ids {
    include!(concat!(env!("OUT_DIR"), "/ids.rs"));
}

pub use codec::{BlockPos, DecodeError, Reader, Writer};
pub use item::{DataValue, ItemStack};
pub use nbt::Nbt;
pub use packets::handshake::{GAME_VERSION, PROTOCOL_VERSION};
pub use text::{Span, Text, Translator};

/// UUID the vanilla server assigns to offline-mode players: v3 of "OfflinePlayer:<name>".
pub fn offline_uuid(name: &str) -> uuid::Uuid {
    let digest = md5_compat(format!("OfflinePlayer:{name}").as_bytes());
    uuid::Builder::from_md5_bytes(digest).into_uuid()
}

fn md5_compat(data: &[u8]) -> [u8; 16] {
    // Java's UUID.nameUUIDFromBytes is MD5 over the raw bytes (no namespace).
    // uuid's v3 helper prepends a namespace, so compute MD5 ourselves.
    md5::compute(data)
}

mod md5 {
    //! Minimal MD5 (RFC 1321), only for offline-mode UUIDs.
    pub fn compute(input: &[u8]) -> [u8; 16] {
        const S: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4,
            11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
        let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
        let mut msg = input.to_vec();
        let bit_len = (input.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_le_bytes());
        for chunk in msg.chunks(64) {
            let m: Vec<u32> = chunk.chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
            let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
            for i in 0..64 {
                let (f, g) = match i {
                    0..=15 => ((b & c) | (!b & d), i),
                    16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                    32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                    _ => (c ^ (b | !d), (7 * i) % 16),
                };
                let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
                a = d;
                d = c;
                c = b;
                b = b.wrapping_add(f.rotate_left(S[i]));
            }
            a0 = a0.wrapping_add(a);
            b0 = b0.wrapping_add(b);
            c0 = c0.wrapping_add(c);
            d0 = d0.wrapping_add(d);
        }
        let mut out = [0u8; 16];
        out[..4].copy_from_slice(&a0.to_le_bytes());
        out[4..8].copy_from_slice(&b0.to_le_bytes());
        out[8..12].copy_from_slice(&c0.to_le_bytes());
        out[12..].copy_from_slice(&d0.to_le_bytes());
        out
    }

    #[test]
    fn rfc1321_vectors() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(compute(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(compute(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(compute(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offline_uuid_is_v3() {
        let u = super::offline_uuid("Notch");
        assert_eq!(u.get_version_num(), 3);
        // Known value: offline UUID of "Notch" is b50ad385-829d-3141-a216-7e7d7539ba7f.
        assert_eq!(u.to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
}
