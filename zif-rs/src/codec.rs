use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Jpeg,
    Png,
    JpegXr,
    Jpeg2000,
}

impl Codec {
    pub(crate) fn from_code(code: u16) -> Result<Self> {
        match code {
            7 => Ok(Self::Jpeg),
            34933 => Ok(Self::Png),
            34934 => Ok(Self::JpegXr),
            34712 => Ok(Self::Jpeg2000),
            _ => Err(Error::MalformedFile("unsupported codec code")),
        }
    }

    pub(crate) fn code(self) -> u16 {
        match self {
            Self::Jpeg => 7,
            Self::Png => 34933,
            Self::JpegXr => 34934,
            Self::Jpeg2000 => 34712,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorModel {
    WhiteIsZero,
    BlackIsZero,
    Rgb,
    YCbCr,
}

impl ColorModel {
    pub(crate) fn from_code(code: u16) -> Result<Self> {
        match code {
            0 => Ok(Self::WhiteIsZero),
            1 => Ok(Self::BlackIsZero),
            2 => Ok(Self::Rgb),
            6 => Ok(Self::YCbCr),
            _ => Err(Error::MalformedFile("unsupported color model")),
        }
    }

    pub(crate) fn code(self) -> u16 {
        match self {
            Self::WhiteIsZero => 0,
            Self::BlackIsZero => 1,
            Self::Rgb => 2,
            Self::YCbCr => 6,
        }
    }
}
