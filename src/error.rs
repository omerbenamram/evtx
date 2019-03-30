use std::fmt;
use std::io;

use failure::{Context, Fail};

#[derive(Fail, Debug)]
pub struct Error {
    inner: Context<ErrorKind>,
    offset: Option<u64>,
}

#[derive(Fail, Debug)]
pub enum ErrorKind {
    #[fail(
        display = "Expected attribute token to follow attribute name at position {}",
        position
    )]
    ExpectedValue { position: u64 },
    #[fail(display = "{:2X} not a valid binxml token", token)]
    NotAValidBinXMLToken { token: u8 },
    #[fail(display = "{:2X} not a valid binxml token", token)]
    NotAValidValueType { token: u8 },
    #[fail(display = "Unexpected EOF")]
    UnexpectedEOF,
    #[fail(display = "Failed to decode UTF-16 string")]
    UTF16Decode,
    #[fail(display = "Unexpected IO error")]
    IO,
    #[fail(display = "{}", display)]
    Other { display: String },
}

impl Error {
    pub fn new(ctx: Context<ErrorKind>, offset: Option<u64>) -> Error {
        Error { inner: ctx, offset }
    }

    /// Error offset (relative to chunk start)
    pub fn offset(&self) -> Option<u64> {
        self.offset
    }

    pub(crate) fn unexpected_eof(e: impl Fail, offset: u64) -> Self {
        Error::new(e.context(ErrorKind::UnexpectedEOF), Some(offset))
    }

    pub(crate) fn io(e: io::Error) -> Self {
        Error::new(e.context(ErrorKind::IO), None)
    }

    pub(crate) fn not_a_valid_binxml_token(token: u8, offset: u64) -> Self {
        let err = ErrorKind::NotAValidBinXMLToken { token };
        Error::new(Context::new(err), Some(offset))
    }

    pub(crate) fn not_a_valid_binxml_value_type(token: u8, offset: u64) -> Self {
        let err = ErrorKind::NotAValidValueType { token };
        Error::new(Context::new(err), Some(offset))
    }
    pub(crate) fn utf16_decode_error(_e: impl Fail, offset: u64) -> Self {
        Error::new(Context::new(ErrorKind::UTF16Decode), Some(offset))
    }

    pub(crate) fn other(context: impl AsRef<str>, offset: u64) -> Self {
        let err = ErrorKind::Other {
            display: context.as_ref().to_owned(),
        };
        Error::new(Context::new(err), Some(offset))
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::io(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let repr = format!(
            "Error occurred during serialization at offset {:?} - {}",
            self.offset, self.inner
        );
        f.write_str(&repr)?;

        if let Some(bt) = self.backtrace() {
            f.write_str(&format!("{}", bt))?;
        }

        Ok(())
    }
}
