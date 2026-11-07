//! Primitive wire types: big-endian integers, VarInt/VarLong, strings, UUIDs,
//! packed block positions, bit sets. See the "Data types" page of the protocol
//! documentation.

use crate::{nbt::Nbt, text::Text};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("packet ended early (needed {needed} more bytes)")]
    Eof { needed: usize },
    #[error("VarInt is longer than 5 bytes")]
    VarIntTooLong,
    #[error("VarLong is longer than 10 bytes")]
    VarLongTooLong,
    #[error("string is not valid UTF-8")]
    Utf8,
    #[error("string of {len} bytes exceeds the limit of {max} characters")]
    StringTooLong { len: usize, max: usize },
    #[error("negative length {0}")]
    NegativeLength(i32),
    #[error("invalid NBT: {0}")]
    Nbt(String),
    #[error("invalid value for {field}: {value}")]
    Invalid { field: &'static str, value: i64 },
    #[error("invalid JSON text: {0}")]
    Json(String),
}

pub type DResult<T> = Result<T, DecodeError>;

/// Block coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn pack(self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38) | ((self.z as i64 & 0x3FF_FFFF) << 12) | (self.y as i64 & 0xFFF)
    }

    pub fn unpack(v: i64) -> Self {
        Self { x: (v >> 38) as i32, y: (v << 52 >> 52) as i32, z: (v << 26 >> 38) as i32 }
    }
}

/// Cursor over a received packet body.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

macro_rules! be {
    ($name:ident, $t:ty, $n:expr) => {
        pub fn $name(&mut self) -> DResult<$t> {
            Ok(<$t>::from_be_bytes(self.take($n)?.try_into().unwrap()))
        }
    };
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// The bytes read since `start`, a value of [`Reader::position`].
    pub fn since(&self, start: usize) -> &'a [u8] {
        &self.buf[start..self.pos]
    }

    pub fn take(&mut self, n: usize) -> DResult<&'a [u8]> {
        if self.remaining() < n {
            return Err(DecodeError::Eof { needed: n - self.remaining() });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }

    pub fn u8(&mut self) -> DResult<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn i8(&mut self) -> DResult<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn bool(&mut self) -> DResult<bool> {
        Ok(self.u8()? != 0)
    }
    be!(u16, u16, 2);
    be!(i16, i16, 2);
    be!(i32, i32, 4);
    be!(u32, u32, 4);
    be!(i64, i64, 8);
    be!(u64, u64, 8);
    be!(f32, f32, 4);
    be!(f64, f64, 8);

    pub fn var_i32(&mut self) -> DResult<i32> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            value |= ((b & 0x7F) as u32) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(DecodeError::VarIntTooLong)
    }

    pub fn var_i64(&mut self) -> DResult<i64> {
        let mut value: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            value |= ((b & 0x7F) as u64) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i64);
            }
        }
        Err(DecodeError::VarLongTooLong)
    }

    /// A VarInt used as a length or count.
    pub fn len(&mut self) -> DResult<usize> {
        let n = self.var_i32()?;
        if n < 0 {
            return Err(DecodeError::NegativeLength(n));
        }
        Ok(n as usize)
    }

    pub fn string(&mut self, max_chars: usize) -> DResult<String> {
        let len = self.len()?;
        if len > max_chars * 3 {
            return Err(DecodeError::StringTooLong { len, max: max_chars });
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::Utf8)
    }

    pub fn identifier(&mut self) -> DResult<String> {
        self.string(32767)
    }

    pub fn uuid(&mut self) -> DResult<Uuid> {
        Ok(Uuid::from_u128(u128::from_be_bytes(self.take(16)?.try_into().unwrap())))
    }

    pub fn block_pos(&mut self) -> DResult<BlockPos> {
        Ok(BlockPos::unpack(self.i64()?))
    }

    pub fn byte_array(&mut self) -> DResult<Vec<u8>> {
        let n = self.len()?;
        Ok(self.take(n)?.to_vec())
    }

    /// `BitSet`, returned as 64-bit words (bit `i` is `words[i / 64] >> (i % 64) & 1`).
    ///
    /// Since 26.x the game encodes it as a length-prefixed byte array
    /// (`BitSet.toByteArray`, little-endian bit order), not as longs. Verified
    /// against the 26.3 server's `ByteBufCodecs.BIT_SET` and real packets.
    pub fn bitset(&mut self) -> DResult<Vec<u64>> {
        let n = self.len()?;
        let bytes = self.take(n)?;
        Ok(bytes
            .chunks(8)
            .map(|c| c.iter().enumerate().fold(0u64, |acc, (i, &b)| acc | (b as u64) << (8 * i)))
            .collect())
    }

    /// Angle in 1/256ths of a turn, returned in degrees.
    pub fn angle(&mut self) -> DResult<f32> {
        Ok(self.u8()? as f32 * 360.0 / 256.0)
    }

    pub fn nbt(&mut self) -> DResult<Nbt> {
        crate::nbt::read_network(self)
    }

    /// Text component in its network (NBT) form.
    pub fn text(&mut self) -> DResult<Text> {
        Ok(Text::from_nbt(&self.nbt()?))
    }

    /// Text component sent as a JSON string (login disconnect, server list).
    pub fn json_text(&mut self) -> DResult<Text> {
        let s = self.string(262_144)?;
        Text::from_json_str(&s).map_err(DecodeError::Json)
    }

    pub fn option<T>(&mut self, f: impl FnOnce(&mut Self) -> DResult<T>) -> DResult<Option<T>> {
        if self.bool()? { Ok(Some(f(self)?)) } else { Ok(None) }
    }

    pub fn array<T>(&mut self, mut f: impl FnMut(&mut Self) -> DResult<T>) -> DResult<Vec<T>> {
        let n = self.len()?;
        // Guard against absurd counts before allocating: every element is at least one byte.
        let mut v = Vec::with_capacity(n.min(self.remaining()));
        for _ in 0..n {
            v.push(f(self)?);
        }
        Ok(v)
    }

    /// Low-precision vector used for entity velocities (see "LpVec3").
    pub fn lp_vec3(&mut self) -> DResult<[f64; 3]> {
        const MAX: f64 = 32766.0;
        fn unpack(v: u64) -> f64 {
            ((v & 32767) as f64).min(MAX) * 2.0 / MAX - 1.0
        }
        let b1 = self.u8()? as u64;
        if b1 == 0 {
            return Ok([0.0; 3]);
        }
        let b2 = self.u8()? as u64;
        let b3_6 = self.u32()? as u64;
        let packed = (b3_6 << 16) | (b2 << 8) | b1;
        let mut scale = b1 & 3;
        if b1 & 4 != 0 {
            scale |= (self.var_i32()? as u32 as u64) << 2;
        }
        let s = scale as f64;
        Ok([unpack(packed >> 3) * s, unpack(packed >> 18) * s, unpack(packed >> 33) * s])
    }
}

/// Builder for an outgoing packet body.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

macro_rules! put_be {
    ($name:ident, $t:ty) => {
        pub fn $name(&mut self, v: $t) -> &mut Self {
            self.buf.extend_from_slice(&v.to_be_bytes());
            self
        }
    };
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.u8(v as u8)
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }
    put_be!(u16, u16);
    put_be!(i16, i16);
    put_be!(i32, i32);
    put_be!(i64, i64);
    put_be!(u64, u64);
    put_be!(f32, f32);
    put_be!(f64, f64);

    pub fn var_i32(&mut self, v: i32) -> &mut Self {
        let mut v = v as u32;
        loop {
            if v & !0x7F == 0 {
                self.buf.push(v as u8);
                return self;
            }
            self.buf.push((v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
    }

    pub fn var_i64(&mut self, v: i64) -> &mut Self {
        let mut v = v as u64;
        loop {
            if v & !0x7F == 0 {
                self.buf.push(v as u8);
                return self;
            }
            self.buf.push((v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
    }

    /// Low-precision vector (see [`Reader::lp_vec3`]).
    pub fn lp_vec3(&mut self, v: [f64; 3]) -> &mut Self {
        let clean = |d: f64| if d.is_nan() { 0.0 } else { d.clamp(-1.7179869183e10, 1.7179869183e10) };
        let [x, y, z] = v.map(clean);
        let max = x.abs().max(y.abs()).max(z.abs());
        if max < 3.051944088384301e-5 {
            return self.u8(0);
        }
        let scale = max.ceil() as i64;
        let big = scale & 3 != scale;
        let low = if big { (scale & 3) | 4 } else { scale };
        let pack = |d: f64| ((d * 0.5 + 0.5) * 32766.0).round() as i64;
        let s = scale as f64;
        let packed = low | (pack(x / s) << 3) | (pack(y / s) << 18) | (pack(z / s) << 33);
        self.u8(packed as u8).u8((packed >> 8) as u8).i32((packed >> 16) as i32);
        if big {
            self.var_i32((scale >> 2) as i32);
        }
        self
    }

    pub fn string(&mut self, s: &str) -> &mut Self {
        self.var_i32(s.len() as i32);
        self.buf.extend_from_slice(s.as_bytes());
        self
    }

    pub fn uuid(&mut self, u: Uuid) -> &mut Self {
        self.buf.extend_from_slice(&u.as_u128().to_be_bytes());
        self
    }

    pub fn block_pos(&mut self, p: BlockPos) -> &mut Self {
        self.i64(p.pack())
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    pub fn byte_array(&mut self, b: &[u8]) -> &mut Self {
        self.var_i32(b.len() as i32);
        self.bytes(b)
    }
}

pub fn var_i32_len(v: i32) -> usize {
    let v = v as u32;
    match v {
        0..=0x7F => 1,
        0x80..=0x3FFF => 2,
        0x4000..=0x1F_FFFF => 3,
        0x20_0000..=0xFFF_FFFF => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_vectors() {
        // Vectors from the protocol documentation.
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (2, &[0x02]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for (v, bytes) in cases {
            let mut w = Writer::new();
            w.var_i32(*v);
            assert_eq!(&w.buf, bytes, "encode {v}");
            assert_eq!(Reader::new(bytes).var_i32().unwrap(), *v, "decode {v}");
            assert_eq!(var_i32_len(*v), bytes.len());
        }
    }

    #[test]
    fn varlong_vectors() {
        let cases: &[(i64, &[u8])] = &[
            (0, &[0x00]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            (9223372036854775807, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]),
            (-9223372036854775808, &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
        ];
        for (v, bytes) in cases {
            let mut w = Writer::new();
            w.var_i64(*v);
            assert_eq!(&w.buf, bytes);
            assert_eq!(Reader::new(bytes).var_i64().unwrap(), *v);
        }
    }

    #[test]
    fn varint_too_long() {
        assert!(matches!(Reader::new(&[0xff; 6]).var_i32(), Err(DecodeError::VarIntTooLong)));
    }

    #[test]
    fn position_example() {
        // Example from the documentation: x=18357644, z=-20882616, y=831.
        let p = BlockPos::new(18357644, 831, -20882616);
        let packed = p.pack();
        assert_eq!(packed as u64, 0b01000110000001110110001100_10110000010101101101001000_001100111111);
        assert_eq!(BlockPos::unpack(packed), p);
        for p in [BlockPos::new(-1, -64, -1), BlockPos::new(0, 319, 0), BlockPos::new(-30_000_000, -2048, 29_999_999)] {
            assert_eq!(BlockPos::unpack(p.pack()), p);
        }
    }
}
