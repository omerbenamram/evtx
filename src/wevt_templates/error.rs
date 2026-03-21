use thiserror::Error;

#[derive(Debug, Error)]
pub enum WevtTemplateExtractError {
    #[error("input is not a valid PE file: {message}")]
    InvalidPe { message: &'static str },

    #[error("malformed PE file: {message}")]
    MalformedPe { message: &'static str },

    #[error("failed to map RVA 0x{rva:08x} to a file offset")]
    UnmappedRva { rva: u32 },

    #[error("resource directory is malformed: {message}")]
    MalformedResource { message: &'static str },

    #[error("failed to decode UTF-16 resource name")]
    InvalidResourceName,
}
