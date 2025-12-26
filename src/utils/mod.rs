mod binxml_utils;
pub(crate) mod bytes;
pub(super) mod hexdump;
mod read_ext;
mod time;
pub(crate) mod utf16;

pub use self::binxml_utils::{
    read_ansi_encoded_string, read_len_prefixed_utf16_string, read_null_terminated_utf16_string,
    read_utf16_by_size,
};
pub use self::hexdump::dump_stream;
pub(crate) use self::read_ext::ReadExt;
pub use self::time::read_systemtime;
pub(crate) use self::utf16::{decode_utf16_units_z, decode_utf16le_bytes_z};
