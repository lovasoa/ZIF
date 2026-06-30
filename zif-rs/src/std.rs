use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{Chunk, Request, WriteBatch, WriteOp};

pub struct FileRangeReader {
    file: File,
}

impl FileRangeReader {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
        })
    }

    pub fn fetch(&mut self, req: Request) -> std::io::Result<Chunk> {
        self.file.seek(SeekFrom::Start(req.start()))?;
        let mut bytes = vec![
            0;
            usize::try_from(req.len()).map_err(|_| std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "range too large"
            ))?
        ];
        self.file.read_exact(&mut bytes)?;
        Chunk::new(req.range(), bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

pub struct FileRangeWriter {
    file: File,
}

impl FileRangeWriter {
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(path)?,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().read(true).write(true).open(path)?,
        })
    }

    pub fn apply(&mut self, batch: WriteBatch) -> std::io::Result<()> {
        for op in batch.into_ops() {
            match op {
                WriteOp::InitHeader(bytes) => {
                    self.file.seek(SeekFrom::Start(0))?;
                    self.file.write_all(&bytes)?;
                }
                WriteOp::Append(bytes) => {
                    self.file.seek(SeekFrom::End(0))?;
                    self.file.write_all(&bytes)?;
                }
                WriteOp::PatchU64 { offset, value } => {
                    self.file.seek(SeekFrom::Start(offset.get()))?;
                    self.file.write_all(&value.to_le_bytes())?;
                }
            }
        }
        self.file.flush()
    }
}
