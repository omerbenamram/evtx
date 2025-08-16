use std::io::{Error, ErrorKind, Result as IoResult};
use std::char::decode_utf16;

/// Optimized UTF-16 units → String decoding with:
/// - early stop on NUL (0x0000)
/// - reserved capacity to minimize reallocations
/// - in-place trim_end without extra allocation
#[allow(dead_code)]
pub fn decode_utf16_trim(units: &[u16]) -> IoResult<String> {
    if units.is_empty() {
        return Ok(String::new());
    }

    // Heuristic: UTF-16 to UTF-8 worst-case can expand. Reserve `units.len()` bytes as a good start.
    let mut out = String::with_capacity(units.len());
    for r in decode_utf16(units.iter().copied()) {
        match r {
            Ok(ch) => {
                if ch == '\0' {
                    break;
                }
                out.push(ch);
            }
            Err(_) => return Err(Error::from(ErrorKind::InvalidData)),
        }
    }
    // In-place right trim
    let trimmed_len = out.trim_end().len();
    if trimmed_len < out.len() {
        out.truncate(trimmed_len);
    }
    Ok(out)
}


