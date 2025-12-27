#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum Utf16LeDecodeError {
    OddLength,
    InvalidData,
}

/// Decode a UTF-16LE byte slice until the first NUL (0x0000), if present.
pub(crate) fn decode_utf16le_bytes_z(bytes: &[u8]) -> Result<String, Utf16LeDecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }

    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    decode_utf16_units_z(&units)
}

/// Decode UTF-16 code units until the first NUL (0x0000), if present.
pub(crate) fn decode_utf16_units_z(units: &[u16]) -> Result<String, Utf16LeDecodeError> {
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    let slice = &units[..end];
    String::from_utf16(slice).map_err(|_| Utf16LeDecodeError::InvalidData)
}
