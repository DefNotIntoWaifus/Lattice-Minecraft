//! Whatever a server sends, decoding must end in a value or an error, never a crash or a hang.
//! The numbers come from a fixed seed, so a failure can be repeated.

use lattice_proto::Reader;
use lattice_proto::packets::play;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Random bytes, leaning towards what confuses decoders: small counts, huge counts, zeroes.
fn noise(rng: &mut Rng, out: &mut Vec<u8>) {
    out.clear();
    let len = match rng.below(4) {
        0 => rng.below(8),
        1 => rng.below(64),
        2 => rng.below(512),
        _ => rng.below(4096),
    };
    while (out.len() as u64) < len {
        match rng.below(6) {
            0 => out.push(0),
            1 => out.push(1),
            2 => out.push(rng.below(10) as u8),
            3 => out.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x07]),
            4 => out.extend_from_slice(&[0x0A, 0x00]),
            _ => out.push(rng.next() as u8),
        }
    }
}

#[test]
fn no_packet_crashes_on_noise() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut buf = Vec::new();
    let mut decoded = 0u64;
    for id in 0..160 {
        for _ in 0..3000 {
            noise(&mut rng, &mut buf);
            let mut r = Reader::new(&buf);
            if play::decode(id, &mut r).is_ok() {
                decoded += 1;
            }
        }
    }
    // Some must get through, or the test isn't reaching the decoders at all.
    assert!(decoded > 1000, "only {decoded} decoded");
}

#[test]
fn text_and_nbt_survive_noise() {
    let mut rng = Rng(0xD1B54A32D192ED03);
    let mut buf = Vec::new();
    for _ in 0..100_000 {
        noise(&mut rng, &mut buf);
        let mut r = Reader::new(&buf);
        let _ = r.nbt();
        let mut r = Reader::new(&buf);
        let _ = r.text();
        let mut r = Reader::new(&buf);
        let _ = lattice_proto::recipes::SlotDisplay::read(&mut r);
        let mut r = Reader::new(&buf);
        let _ = lattice_proto::recipes::RecipeDisplay::read(&mut r);
    }
}

/// Deeply nested lists mustn't run the stack out.
#[test]
fn nesting_is_bounded() {
    let mut bytes = vec![0x0A];
    for _ in 0..200_000 {
        // A compound holding a compound with an empty name.
        bytes.extend_from_slice(&[0x0A, 0x00, 0x00]);
    }
    let mut r = Reader::new(&bytes);
    let _ = r.nbt();
    let mut bytes = vec![0x09];
    for _ in 0..200_000 {
        // A list of one list.
        bytes.extend_from_slice(&[0x09, 0x00, 0x00, 0x00, 0x01]);
    }
    let mut r = Reader::new(&bytes);
    let _ = r.nbt();
}
