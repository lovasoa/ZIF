use std::path::Path;

use tokio::fs::File;
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, SeekFrom,
};

use crate::{Chunk, Error, ReadStatus, Reader, Request, Result, WriteBatch, Zif};

pub struct AsyncRangeReader<R = File> {
    reader: R,
}

pub type FileRangeReader = AsyncRangeReader<File>;

impl AsyncRangeReader<File> {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            reader: File::open(path).await?,
        })
    }
}

impl<R: AsyncRead + AsyncSeek + Unpin> AsyncRangeReader<R> {
    pub fn wrap(reader: R) -> Self {
        Self { reader }
    }

    pub async fn fetch(&mut self, req: Request) -> Result<Chunk> {
        self.reader.seek(SeekFrom::Start(req.start())).await?;
        let mut bytes = vec![
            0;
            usize::try_from(req.len())
                .map_err(|_| Error::InvalidInput("range too large"))?
        ];
        self.reader.read_exact(&mut bytes).await?;
        Chunk::new(req.range(), bytes)
    }

    pub async fn read_zif(&mut self) -> Result<Zif> {
        let mut reader = Reader::new();
        let mut chunk = Chunk::default();

        while let ReadStatus::Need { req, .. } = reader.advance(chunk)? {
            chunk = self.fetch(req).await?;
        }

        reader.into_zif()
    }
}

pub async fn read_zif(reader: impl AsyncRead + AsyncSeek + Unpin) -> Result<Zif> {
    AsyncRangeReader::wrap(reader).read_zif().await
}

pub struct AsyncRangeWriter<W = File> {
    writer: W,
}

pub type FileRangeWriter = AsyncRangeWriter<File>;

impl AsyncRangeWriter<File> {
    pub async fn create(path: impl AsRef<Path>) -> Result<Self> {
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

    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
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

    pub async fn apply(&mut self, batch: WriteBatch) -> Result<()> {
        for op in batch.into_ops() {
            self.writer.seek(SeekFrom::Start(op.offset)).await?;
            self.writer.write_all(&op.bytes).await?;
        }
        Ok(self.writer.flush().await?)
    }
}
