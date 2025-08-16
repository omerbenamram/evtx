use std::io::Result as IoResult;

#[inline]
pub fn decode_units_to_string_trim(units: &[u16]) -> IoResult<String> {
	crate::utils::utf16_opt::decode_utf16_trim(units)
}