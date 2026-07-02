use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{ByteRange, DataChunk, Error, Image, ParseState, Parser, Result, WriteBatch};

pub struct RangeReader<R = File> {
    reader: R,
}

pub type FileRangeReader = RangeReader<File>;

impl RangeReader<File> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            reader: File::open(path)?,
        })
    }
}

impl<R> RangeReader<R> {
    pub fn wrap(reader: R) -> Self {
        Self { reader }
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl From<Vec<u8>> for RangeReader<Cursor<Vec<u8>>> {
    fn from(bytes: Vec<u8>) -> Self {
        Self::wrap(Cursor::new(bytes))
    }
}

impl<R: Read + Seek> RangeReader<R> {
    pub fn read_zif(&mut self) -> Result<Image> {
        let mut parser = Parser::new();
        let mut chunk = DataChunk::default();

        while let ParseState::Need { range, .. } = parser.feed(chunk)? {
            chunk = self.fetch(range)?;
        }

        parser.finish()
    }

    pub fn fetch(&mut self, range: ByteRange) -> Result<DataChunk> {
        self.reader.seek(SeekFrom::Start(range.start()))?;
        let mut bytes = vec![
            0;
            usize::try_from(range.len())
                .map_err(|_| Error::InvalidInput("range too large"))?
        ];
        self.reader.read_exact(&mut bytes)?;
        DataChunk::new(range.range(), bytes)
    }
}

pub fn read_zif(reader: impl Read + Seek) -> Result<Image> {
    RangeReader::wrap(reader).read_zif()
}

pub struct RangeWriter<W = File> {
    writer: W,
}

pub type FileRangeWriter = RangeWriter<File>;

impl RangeWriter<File> {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            writer: std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(path)?,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            writer: std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?,
        })
    }
}

impl<W> RangeWriter<W> {
    pub fn wrap(writer: W) -> Self {
        Self { writer }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl From<Vec<u8>> for RangeWriter<Cursor<Vec<u8>>> {
    fn from(bytes: Vec<u8>) -> Self {
        Self::wrap(Cursor::new(bytes))
    }
}

impl<W: Write + Seek> RangeWriter<W> {
    pub fn apply(&mut self, batch: WriteBatch) -> Result<()> {
        for action in batch.into_actions() {
            self.writer.seek(SeekFrom::Start(action.offset))?;
            self.writer.write_all(&action.bytes)?;
        }
        Ok(self.writer.flush()?)
    }
}
