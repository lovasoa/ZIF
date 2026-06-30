use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    MalformedFile(&'static str),
    InvalidInput(&'static str),
    Unsupported(&'static str),
    Incomplete,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedFile(msg) => write!(f, "malformed ZIF file: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::Unsupported(msg) => write!(f, "unsupported ZIF feature: {msg}"),
            Self::Incomplete => f.write_str("more data is required"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
