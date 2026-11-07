//! Named Binary Tag, as used on the network since 1.20.2: the root tag has a
//! type byte but no name. Strings are Java "modified UTF-8".

use crate::codec::{DResult, DecodeError, Reader, Writer};

#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

const MAX_DEPTH: usize = 512;

impl Nbt {
    /// Written the way commands take it.
    pub fn to_snbt(&self) -> String {
        fn quoted(s: &str) -> String {
            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
        }
        match self {
            Nbt::End => String::new(),
            Nbt::Byte(v) => format!("{v}b"),
            Nbt::Short(v) => format!("{v}s"),
            Nbt::Int(v) => v.to_string(),
            Nbt::Long(v) => format!("{v}L"),
            Nbt::Float(v) => format!("{v}f"),
            Nbt::Double(v) => format!("{v}d"),
            Nbt::String(s) => quoted(s),
            Nbt::ByteArray(v) => format!("[B;{}]", v.iter().map(|b| format!("{}b", *b as i8)).collect::<Vec<_>>().join(",")),
            Nbt::IntArray(v) => format!("[I;{}]", v.iter().map(i32::to_string).collect::<Vec<_>>().join(",")),
            Nbt::LongArray(v) => format!("[L;{}]", v.iter().map(|n| format!("{n}L")).collect::<Vec<_>>().join(",")),
            Nbt::List(v) => format!("[{}]", v.iter().map(Nbt::to_snbt).collect::<Vec<_>>().join(",")),
            Nbt::Compound(v) => {
                let plain = |k: &str| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'));
                format!("{{{}}}", v.iter().map(|(k, t)| format!("{}:{}", if plain(k) { k.clone() } else { quoted(k) }, t.to_snbt())).collect::<Vec<_>>().join(","))
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any numeric tag as f64.
    pub fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Nbt::Byte(v) => v as f64,
            Nbt::Short(v) => v as f64,
            Nbt::Int(v) => v as f64,
            Nbt::Long(v) => v as f64,
            Nbt::Float(v) => v as f64,
            Nbt::Double(v) => v,
            _ => return None,
        })
    }

    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Nbt::Byte(v) => v as i64,
            Nbt::Short(v) => v as i64,
            Nbt::Int(v) => v as i64,
            Nbt::Long(v) => v,
            _ => return None,
        })
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Nbt::Byte(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Nbt]> {
        match self {
            Nbt::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_compound(&self) -> Option<&[(String, Nbt)]> {
        match self {
            Nbt::Compound(v) => Some(v),
            _ => None,
        }
    }

    fn type_id(&self) -> u8 {
        match self {
            Nbt::End => 0,
            Nbt::Byte(_) => 1,
            Nbt::Short(_) => 2,
            Nbt::Int(_) => 3,
            Nbt::Long(_) => 4,
            Nbt::Float(_) => 5,
            Nbt::Double(_) => 6,
            Nbt::ByteArray(_) => 7,
            Nbt::String(_) => 8,
            Nbt::List(_) => 9,
            Nbt::Compound(_) => 10,
            Nbt::IntArray(_) => 11,
            Nbt::LongArray(_) => 12,
        }
    }
}

/// Reads a nameless root tag. `TAG_End` as the root means "no value".
pub fn read_network(r: &mut Reader) -> DResult<Nbt> {
    let ty = r.u8()?;
    if ty == 0 {
        return Ok(Nbt::End);
    }
    read_payload(r, ty, 0)
}

fn bad(msg: impl Into<String>) -> DecodeError {
    DecodeError::Nbt(msg.into())
}

fn read_payload(r: &mut Reader, ty: u8, depth: usize) -> DResult<Nbt> {
    if depth > MAX_DEPTH {
        return Err(bad("nesting too deep"));
    }
    Ok(match ty {
        1 => Nbt::Byte(r.i8()?),
        2 => Nbt::Short(r.i16()?),
        3 => Nbt::Int(r.i32()?),
        4 => Nbt::Long(r.i64()?),
        5 => Nbt::Float(r.f32()?),
        6 => Nbt::Double(r.f64()?),
        7 => {
            let n = read_count(r)?;
            Nbt::ByteArray(r.take(n)?.to_vec())
        }
        8 => Nbt::String(read_string(r)?),
        9 => {
            let elem = r.u8()?;
            let n = read_count(r)?;
            if elem == 0 && n > 0 {
                return Err(bad("non-empty list of TAG_End"));
            }
            let mut v = Vec::with_capacity(n.min(r.remaining()));
            for _ in 0..n {
                v.push(read_payload(r, elem, depth + 1)?);
            }
            Nbt::List(v)
        }
        10 => {
            let mut v = Vec::new();
            loop {
                let t = r.u8()?;
                if t == 0 {
                    break;
                }
                let name = read_string(r)?;
                v.push((name, read_payload(r, t, depth + 1)?));
            }
            Nbt::Compound(v)
        }
        11 => {
            let n = read_count(r)?;
            Nbt::IntArray((0..n).map(|_| r.i32()).collect::<DResult<_>>()?)
        }
        12 => {
            let n = read_count(r)?;
            Nbt::LongArray((0..n).map(|_| r.i64()).collect::<DResult<_>>()?)
        }
        t => return Err(bad(format!("unknown tag type {t}"))),
    })
}

fn read_count(r: &mut Reader) -> DResult<usize> {
    let n = r.i32()?;
    if n < 0 {
        return Err(bad("negative length"));
    }
    Ok(n as usize)
}

fn read_string(r: &mut Reader) -> DResult<String> {
    let n = r.u16()? as usize;
    decode_mutf8(r.take(n)?)
}

/// Java modified UTF-8: U+0000 is 0xC0 0x80 and supplementary characters are
/// encoded as two 3-byte surrogates.
pub fn decode_mutf8(bytes: &[u8]) -> DResult<String> {
    if let Ok(s) = std::str::from_utf8(bytes) {
        // Plain ASCII/BMP text without surrogates is identical in both encodings.
        if !s.contains('\u{0}') {
            return Ok(s.to_owned());
        }
    }
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xE0 == 0xC0 {
            let b2 = *bytes.get(i + 1).ok_or_else(|| bad("truncated string"))?;
            units.push((((b & 0x1F) as u16) << 6) | (b2 & 0x3F) as u16);
            i += 2;
        } else if b & 0xF0 == 0xE0 {
            let (b2, b3) = match (bytes.get(i + 1), bytes.get(i + 2)) {
                (Some(&b2), Some(&b3)) => (b2, b3),
                _ => return Err(bad("truncated string")),
            };
            units.push((((b & 0x0F) as u16) << 12) | (((b2 & 0x3F) as u16) << 6) | (b3 & 0x3F) as u16);
            i += 3;
        } else {
            return Err(bad("invalid modified UTF-8"));
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

pub fn encode_mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007F => out.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                out.push(0xC0 | ((unit >> 6) & 0x1F) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | ((unit >> 12) & 0x0F) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    out
}

/// Writes a nameless root tag.
pub fn write_network(w: &mut Writer, tag: &Nbt) {
    w.u8(tag.type_id());
    if !matches!(tag, Nbt::End) {
        write_payload(w, tag);
    }
}

fn write_string(w: &mut Writer, s: &str) {
    let b = encode_mutf8(s);
    w.u16(b.len() as u16);
    w.bytes(&b);
}

fn write_payload(w: &mut Writer, tag: &Nbt) {
    match tag {
        Nbt::End => {}
        Nbt::Byte(v) => {
            w.i8(*v);
        }
        Nbt::Short(v) => {
            w.i16(*v);
        }
        Nbt::Int(v) => {
            w.i32(*v);
        }
        Nbt::Long(v) => {
            w.i64(*v);
        }
        Nbt::Float(v) => {
            w.f32(*v);
        }
        Nbt::Double(v) => {
            w.f64(*v);
        }
        Nbt::ByteArray(v) => {
            w.i32(v.len() as i32);
            w.bytes(v);
        }
        Nbt::String(s) => write_string(w, s),
        Nbt::List(v) => {
            w.u8(v.first().map_or(0, Nbt::type_id));
            w.i32(v.len() as i32);
            for e in v {
                write_payload(w, e);
            }
        }
        Nbt::Compound(v) => {
            for (k, e) in v {
                w.u8(e.type_id());
                write_string(w, k);
                write_payload(w, e);
            }
            w.u8(0);
        }
        Nbt::IntArray(v) => {
            w.i32(v.len() as i32);
            for e in v {
                w.i32(*e);
            }
        }
        Nbt::LongArray(v) => {
            w.i32(v.len() as i32);
            for e in v {
                w.i64(*e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let tag = Nbt::Compound(vec![
            ("name".into(), Nbt::String("Bananrama".into())),
            ("list".into(), Nbt::List(vec![Nbt::Int(1), Nbt::Int(2)])),
            ("nested".into(), Nbt::Compound(vec![("x".into(), Nbt::Double(1.5))])),
            ("longs".into(), Nbt::LongArray(vec![i64::MIN, 0, i64::MAX])),
            ("emoji".into(), Nbt::String("a\u{0}b😀".into())),
        ]);
        let mut w = Writer::new();
        write_network(&mut w, &tag);
        let back = read_network(&mut Reader::new(&w.buf)).unwrap();
        assert_eq!(back, tag);
    }

    #[test]
    fn mutf8_encoding_of_supplementary_and_nul() {
        // U+1F600 → surrogates D83D DE00 → ED A0 BD ED B8 80; NUL → C0 80.
        assert_eq!(encode_mutf8("😀"), vec![0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
        assert_eq!(encode_mutf8("\u{0}"), vec![0xC0, 0x80]);
        assert_eq!(decode_mutf8(&[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]).unwrap(), "😀");
    }

    #[test]
    fn string_root() {
        // Text components may be sent as a bare string tag.
        let mut w = Writer::new();
        write_network(&mut w, &Nbt::String("hi".into()));
        assert_eq!(w.buf, vec![8, 0, 2, b'h', b'i']);
        assert_eq!(read_network(&mut Reader::new(&w.buf)).unwrap(), Nbt::String("hi".into()));
    }
}
