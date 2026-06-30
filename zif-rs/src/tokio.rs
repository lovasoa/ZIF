use std::path::Path;

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

use crate::{Chunk, Request, WriteBatch, WriteOp};

pub struct FileRangeReader {
    file: File,
}

impl FileRangeReader {
    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: File::open(path).await?,
        })
    }

    pub async fn fetch(&mut self, req: Request) -> std::io::Result<Chunk> {
        self.file.seek(SeekFrom::Start(req.start())).await?;
        let mut bytes = vec![
            0;
            usize::try_from(req.len()).map_err(|_| std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "range too large"
            ))?
        ];
        self.file.read_exact(&mut bytes).await?;
        Chunk::new(req.range(), bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

pub struct FileRangeWriter {
    file: File,
}

impl FileRangeWriter {
    pub async fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(path)
                .await?,
        })
    }

    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().read(true).write(true).open(path).await?,
        })
    }

    pub async fn apply(&mut self, batch: WriteBatch) -> std::io::Result<()> {
        for op in batch.into_ops() {
            match op {
                WriteOp::InitHeader(bytes) => {
                    self.file.seek(SeekFrom::Start(0)).await?;
                    self.file.write_all(&bytes).await?;
                }
                WriteOp::Append(bytes) => {
                    self.file.seek(SeekFrom::End(0)).await?;
                    self.file.write_all(&bytes).await?;
                }
                WriteOp::PatchU64 { offset, value } => {
                    self.file.seek(SeekFrom::Start(offset.get())).await?;
                    self.file.write_all(&value.to_le_bytes()).await?;
                }
            }
        }
        self.file.flush().await
    }
}
