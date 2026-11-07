//! Packet framing: length prefix, optional zlib compression, optional
//! AES/CFB8 encryption of the whole byte stream.

use crate::codec::{Reader, Writer, var_i32_len};
use crate::crypto::{self, Decryptor, Encryptor};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::io::{self, Read, Write};

/// A frame may not exceed a 3-byte VarInt.
pub const MAX_FRAME: usize = 2_097_151;
/// Upper bound we accept for a decompressed packet (the vanilla client has none;
/// this protects against decompression bombs).
pub const MAX_DECOMPRESSED: usize = 64 * 1024 * 1024;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub struct FrameReader<R> {
    inner: R,
    cipher: Option<Decryptor>,
    threshold: Option<usize>,
}

impl<R: Read> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, cipher: None, threshold: None }
    }

    pub fn enable_encryption(&mut self, d: Decryptor) {
        self.cipher = Some(d);
    }

    pub fn set_compression(&mut self, threshold: i32) {
        self.threshold = (threshold >= 0).then_some(threshold as usize);
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.inner.read_exact(buf)?;
        if let Some(c) = &mut self.cipher {
            crypto::decrypt(c, buf);
        }
        Ok(())
    }

    fn read_varint(&mut self) -> io::Result<i32> {
        let mut value: u32 = 0;
        for i in 0..3 {
            let mut b = [0u8];
            self.read_exact(&mut b)?;
            value |= ((b[0] & 0x7F) as u32) << (7 * i);
            if b[0] & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(invalid("frame length longer than 3 bytes"))
    }

    /// Returns one packet: VarInt ID followed by the body.
    pub fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let len = self.read_varint()? as usize;
        if len > MAX_FRAME {
            return Err(invalid(format!("frame of {len} bytes is too large")));
        }
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        let Some(_) = self.threshold else { return Ok(buf) };
        let mut r = Reader::new(&buf);
        let data_len = r.var_i32().map_err(|e| invalid(e.to_string()))?;
        let header = r.position();
        if data_len == 0 {
            buf.drain(..header);
            return Ok(buf);
        }
        if data_len < 0 || data_len as usize > MAX_DECOMPRESSED {
            return Err(invalid(format!("bad decompressed length {data_len}")));
        }
        let mut out = Vec::with_capacity(data_len as usize);
        ZlibDecoder::new(&buf[header..]).take(data_len as u64).read_to_end(&mut out)?;
        if out.len() != data_len as usize {
            return Err(invalid("decompressed size mismatch"));
        }
        Ok(out)
    }
}

pub struct FrameWriter<W> {
    inner: W,
    cipher: Option<Encryptor>,
    threshold: Option<usize>,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, cipher: None, threshold: None }
    }

    pub fn enable_encryption(&mut self, e: Encryptor) {
        self.cipher = Some(e);
    }

    pub fn set_compression(&mut self, threshold: i32) {
        self.threshold = (threshold >= 0).then_some(threshold as usize);
    }

    /// Frames and sends `id` + `body`.
    pub fn write_packet(&mut self, id: i32, body: &[u8]) -> io::Result<()> {
        let payload_len = var_i32_len(id) + body.len();
        let mut frame = Writer::new();
        match self.threshold {
            None => {
                frame.var_i32(payload_len as i32).var_i32(id).bytes(body);
            }
            Some(t) if payload_len < t => {
                frame.var_i32(payload_len as i32 + 1).var_i32(0).var_i32(id).bytes(body);
            }
            Some(_) => {
                let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
                let mut idbuf = Writer::new();
                idbuf.var_i32(id);
                z.write_all(&idbuf.buf)?;
                z.write_all(body)?;
                let compressed = z.finish()?;
                let inner_len = var_i32_len(payload_len as i32) + compressed.len();
                frame.var_i32(inner_len as i32).var_i32(payload_len as i32).bytes(&compressed);
            }
        }
        if frame.buf.len() > MAX_FRAME + 3 {
            return Err(invalid("outgoing packet too large"));
        }
        if let Some(c) = &mut self.cipher {
            crypto::encrypt(c, &mut frame.buf);
        }
        self.inner.write_all(&frame.buf)?;
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(threshold: Option<i32>, encrypted: bool, body: &[u8]) {
        let mut wire = Vec::new();
        {
            let mut w = FrameWriter::new(&mut wire);
            if let Some(t) = threshold {
                w.set_compression(t);
            }
            if encrypted {
                w.enable_encryption(crypto::ciphers(&[9; 16]).0);
            }
            w.write_packet(0x2e, body).unwrap();
            w.write_packet(0x00, b"").unwrap();
        }
        let mut r = FrameReader::new(&wire[..]);
        if let Some(t) = threshold {
            r.set_compression(t);
        }
        if encrypted {
            r.enable_encryption(crypto::ciphers(&[9; 16]).1);
        }
        let a = r.read_frame().unwrap();
        let mut rd = Reader::new(&a);
        assert_eq!(rd.var_i32().unwrap(), 0x2e);
        assert_eq!(rd.rest(), body);
        assert_eq!(r.read_frame().unwrap(), vec![0x00]);
    }

    #[test]
    fn frames() {
        let big: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for t in [None, Some(256), Some(0)] {
            for enc in [false, true] {
                round_trip(t, enc, b"small");
                round_trip(t, enc, &big);
            }
        }
    }
}
