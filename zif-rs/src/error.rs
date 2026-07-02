use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    MalformedFile(&'static str),
    InvalidInput(&'static str),
    Unsupported(&'static str),
    #[cfg(feature = "std")]
    Io(std::io::Error),
    #[cfg(feature = "http")]
    Http(Box<dyn std::error::Error + Send + Sync>),
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
            #[cfg(feature = "http")]
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
            #[cfg(feature = "http")]
            Self::Http(err) => Some(err.as_ref()),
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

#[cfg(feature = "http")]
impl From<http::Error> for Error {
    fn from(value: http::Error) -> Self {
        Self::Http(Box::new(value))
    }
}

#[cfg(feature = "http")]
impl From<http::uri::InvalidUri> for Error {
    fn from(value: http::uri::InvalidUri) -> Self {
        Self::Http(Box::new(value))
    }
}
