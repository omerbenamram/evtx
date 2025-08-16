use std::io::{Result as IoResult, Write};

/// Minimal, allocation-avoiding JSON writer used by streaming output.
///
/// Responsibilities:
/// - Escapes and streams strings without building large intermediates
/// - Writes numbers via itoa/ryu without heap allocations
/// - Exposes tiny helpers for common tokens to keep callsites terse
pub struct JsonWriter<W: Write> {
    pub(crate) writer: W,
}

impl<W: Write> JsonWriter<W> {
    #[inline]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    #[inline]
    pub fn flush(&mut self) -> IoResult<()> {
        self.writer.flush()
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> IoResult<()> {
        self.writer.write_all(bytes)
    }

    #[inline]
    pub fn write_str(&mut self, s: &str) -> IoResult<()> {
        self.write_bytes(s.as_bytes())
    }

    /// Writes a JSON-escaped string surrounded by quotes without allocating.
    pub fn write_quoted_str(&mut self, s: &str) -> IoResult<()> {
        use memchr::memchr2;
        let bytes = s.as_bytes();
        self.writer.write_all(b"\"")?;
        let mut i = 0usize;
        let len = bytes.len();
        while i < len {
            let slice = &bytes[i..];
            let pos = memchr2(b'"', b'\\', slice);
            let next = pos.map(|p| i + p).unwrap_or(len);
            // Scan for control bytes within the safe run
            let mut j = i;
            while j < next {
                let b = bytes[j];
                if b < 0x20 { // control
                    if i < j {
                        self.writer.write_all(&bytes[i..j])?;
                    }
                    // write control escape
                    match b {
                        b'\n' => self.writer.write_all(b"\\n")?,
                        b'\r' => self.writer.write_all(b"\\r")?,
                        b'\t' => self.writer.write_all(b"\\t")?,
                        0x00..=0x1F => {
                            const HEX: &[u8; 16] = b"0123456789ABCDEF";
                            let esc = [
                                b'\\', b'u', b'0', b'0', HEX[(b >> 4) as usize], HEX[(b & 0x0F) as usize],
                            ];
                            self.writer.write_all(&esc)?;
                        }
                        _ => {}
                    }
                    j += 1;
                    i = j;
                } else {
                    j += 1;
                }
            }
            if i < next {
                self.writer.write_all(&bytes[i..next])?;
            }
            if next >= len {
                break;
            }
            // Handle special at next
            match bytes[next] {
                b'"' => self.writer.write_all(b"\\\"")?,
                b'\\' => self.writer.write_all(b"\\\\")?,
                _ => {}
            }
            i = next + 1;
        }
        self.writer.write_all(b"\"")
    }

    #[inline]
    pub fn write_i64(&mut self, n: i64) -> IoResult<()> {
        let mut buf = itoa::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    pub fn write_u64(&mut self, n: u64) -> IoResult<()> {
        let mut buf = itoa::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    pub fn write_f32(&mut self, n: f32) -> IoResult<()> {
        let mut buf = ryu::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    pub fn write_f64(&mut self, n: f64) -> IoResult<()> {
        let mut buf = ryu::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    pub fn write_bool(&mut self, b: bool) -> IoResult<()> {
        if b {
            self.write_bytes(b"true")
        } else {
            self.write_bytes(b"false")
        }
    }

    #[inline]
    pub fn write_null(&mut self) -> IoResult<()> {
        self.write_bytes(b"null")
    }

    #[inline]
    pub fn colon(&mut self) -> IoResult<()> {
        self.write_bytes(b":")
    }

    #[inline]
    pub fn comma(&mut self) -> IoResult<()> {
        self.write_bytes(b",")
    }

    #[inline]
    pub fn open_object(&mut self) -> IoResult<()> {
        self.write_bytes(b"{")
    }

    #[inline]
    pub fn close_object(&mut self) -> IoResult<()> {
        self.write_bytes(b"}")
    }

    #[inline]
    pub fn open_array(&mut self) -> IoResult<()> {
        self.write_bytes(b"[")
    }

    #[inline]
    pub fn close_array(&mut self) -> IoResult<()> {
        self.write_bytes(b"]")
    }

    #[inline]
    pub fn write_key(&mut self, key: &str) -> IoResult<()> {
        self.write_quoted_str(key)?;
        self.colon()
    }

    /// Generic array writer that works with any error type convertible from io::Error.
    pub fn write_array_generic<T, I, E, F>(&mut self, iter: I, mut write_elem: F) -> Result<(), E>
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&mut Self, T) -> Result<(), E>,
        E: From<std::io::Error>,
    {
        self.open_array().map_err(E::from)?;
        let mut first = true;
        for elem in iter {
            if !first {
                self.comma().map_err(E::from)?;
            }
            first = false;
            write_elem(self, elem)?;
        }
        self.close_array().map_err(E::from)
    }

    /// Convenience: array writer returning IoResult.
    pub fn write_array<T, I, F>(&mut self, iter: I, mut write_elem: F) -> IoResult<()>
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&mut Self, T) -> IoResult<()>,
    {
        self.open_array()?;
        let mut first = true;
        for elem in iter {
            if !first {
                self.comma()?;
            }
            first = false;
            write_elem(self, elem)?;
        }
        self.close_array()
    }

    /// Generic object writer over an iterator of entries, where the closure writes `key:value` pairs.
    pub fn write_object_pairs_generic<T, I, E, F>(
        &mut self,
        iter: I,
        mut write_pair: F,
    ) -> Result<(), E>
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&mut Self, T) -> Result<(), E>,
        E: From<std::io::Error>,
    {
        self.open_object().map_err(E::from)?;
        let mut first = true;
        for entry in iter {
            if !first {
                self.comma().map_err(E::from)?;
            }
            first = false;
            write_pair(self, entry)?;
        }
        self.close_object().map_err(E::from)
    }

    /// Convenience: object writer returning IoResult.
    pub fn write_object_pairs<T, I, F>(&mut self, iter: I, mut write_pair: F) -> IoResult<()>
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&mut Self, T) -> IoResult<()>,
    {
        self.open_object()?;
        let mut first = true;
        for entry in iter {
            if !first {
                self.comma()?;
            }
            first = false;
            write_pair(self, entry)?;
        }
        self.close_object()
    }

    /// Writes `key: { ...pairs... }` where pairs are produced by the closure.
    pub fn write_key_object_pairs<T, I, F>(
        &mut self,
        key: &str,
        iter: I,
        mut write_pair: F,
    ) -> IoResult<()>
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&mut Self, T) -> IoResult<()>,
    {
        self.write_key(key)?;
        self.write_object_pairs(iter, |w, t| write_pair(w, t))
    }

    /// Accepts UTF-16 code units, decodes and writes a quoted JSON string.
    /// First implementation decodes to a String; can be optimized to stream in the future.
    pub fn write_quoted_utf16_units(&mut self, units: &[u16]) -> IoResult<()> {
        let s = crate::utils::utf16_opt::decode_utf16_trim(units)?;
        self.write_quoted_str(&s)
    }
}
