use std::path::Path;

use tokio::fs::File;
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, SeekFrom,
};

use crate::{Chunk, ReadStatus, Reader, Request, WriteBatch, Zif};

pub struct AsyncRangeReader<R = File> {
    reader: R,
}

pub type FileRangeReader = AsyncRangeReader<File>;

impl AsyncRangeReader<File> {
    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            reader: File::open(path).await?,
        })
    }
}

impl<R: AsyncRead + AsyncSeek + Unpin> AsyncRangeReader<R> {
    pub fn wrap(reader: R) -> Self {
        Self { reader }
    }

    pub async fn fetch(&mut self, req: Request) -> std::io::Result<Chunk> {
        self.reader.seek(SeekFrom::Start(req.start())).await?;
        let mut bytes = vec![
            0;
            usize::try_from(req.len()).map_err(|_| std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "range too large"
            ))?
        ];
        self.reader.read_exact(&mut bytes).await?;
        Chunk::new(req.range(), bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    pub async fn read_zif(&mut self) -> std::io::Result<Zif> {
        let mut reader = Reader::new();
        let mut chunk = Chunk::default();

        loop {
            match reader.advance(chunk).map_err(io_invalid_data)? {
                ReadStatus::Need { req, .. } => chunk = self.fetch(req).await?,
                ReadStatus::Done { zif } => return Ok(zif.as_zif().clone()),
            }
        }
    }
}

pub async fn read_zif(reader: impl AsyncRead + AsyncSeek + Unpin) -> std::io::Result<Zif> {
    AsyncRangeReader::wrap(reader).read_zif().await
}

pub struct AsyncRangeWriter<W = File> {
    writer: W,
}

pub type FileRangeWriter = AsyncRangeWriter<File>;

impl AsyncRangeWriter<File> {
    pub async fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            writer: tokio::fs::OpenOptions::new()
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
            writer: tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .await?,
        })
    }
}

impl<W: AsyncWrite + AsyncSeek + Unpin> AsyncRangeWriter<W> {
    pub fn wrap(writer: W) -> Self {
        Self { writer }
    }

    pub async fn apply(&mut self, batch: WriteBatch) -> std::io::Result<()> {
        for op in batch.into_ops() {
            self.writer.seek(SeekFrom::Start(op.offset)).await?;
            self.writer.write_all(&op.bytes).await?;
        }
        self.writer.flush().await
    }
}

fn io_invalid_data(err: crate::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, err)
}
