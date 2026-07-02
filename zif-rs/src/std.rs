use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{Chunk, ReadStatus, Reader, Request, WriteBatch, Zif};

pub struct RangeReader<R = File> {
    reader: R,
}

pub type FileRangeReader = RangeReader<File>;

impl RangeReader<File> {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
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
    pub fn read_zif(&mut self) -> std::io::Result<Zif> {
        let mut reader = Reader::new();
        let mut chunk = Chunk::default();

        loop {
            match reader.advance(chunk).map_err(io_invalid_data)? {
                ReadStatus::Need { req, .. } => chunk = self.fetch(req)?,
                ReadStatus::Done { zif } => return Ok(zif.as_zif().clone()),
            }
        }
    }

    pub fn fetch(&mut self, req: Request) -> std::io::Result<Chunk> {
        self.reader.seek(SeekFrom::Start(req.start()))?;
        let mut bytes = vec![
            0;
            usize::try_from(req.len()).map_err(|_| std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "range too large"
            ))?
        ];
        self.reader.read_exact(&mut bytes)?;
        Chunk::new(req.range(), bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

pub fn read_zif(reader: impl Read + Seek) -> std::io::Result<Zif> {
    RangeReader::wrap(reader).read_zif()
}

pub struct RangeWriter<W = File> {
    writer: W,
}

pub type FileRangeWriter = RangeWriter<File>;

impl RangeWriter<File> {
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            writer: std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(path)?,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
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
    pub fn apply(&mut self, batch: WriteBatch) -> std::io::Result<()> {
        for op in batch.into_ops() {
            self.writer.seek(SeekFrom::Start(op.offset))?;
            self.writer.write_all(&op.bytes)?;
        }
        self.writer.flush()
    }
}

fn io_invalid_data(err: crate::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, err)
}
