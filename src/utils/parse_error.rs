use std::io;

use crate::err::DeserializationError;

#[inline]
pub(crate) fn invalid_data(what: &'static str, offset: u64) -> DeserializationError {
    DeserializationError::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{what} at offset {offset}: invalid data"),
    ))
}
