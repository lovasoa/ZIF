use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    MalformedFile(&'static str),
    InvalidInput(&'static str),
    Unsupported(&'static str),
    #[cfg(feature = "std")]
    Io(std::io::Error),
    #[cfg(feature = "reqwest")]
    Http(reqwest::Error),
    Incomplete,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedFile(msg) => write!(f, "malformed ZIF file: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::Unsupported(msg) => write!(f, "unsupported ZIF feature: {msg}"),
            #[cfg(feature = "std")]
            Self::Io(err) => write!(f, "IO error: {err}"),
            #[cfg(feature = "reqwest")]
            Self::Http(err) => write!(f, "HTTP error: {err}"),
            Self::Incomplete => f.write_str("more data is required"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            #[cfg(feature = "reqwest")]
            Self::Http(err) => Some(err),
            Self::MalformedFile(_)
            | Self::InvalidInput(_)
            | Self::Unsupported(_)
            | Self::Incomplete => None,
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(feature = "reqwest")]
impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}
