use std::path::Path;

use tokio::fs::File;
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, SeekFrom,
};

use crate::{ByteRange, DataChunk, Error, Image, ParseState, Parser, Result, WriteBatch};

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

    pub async fn fetch(&mut self, range: ByteRange) -> Result<DataChunk> {
        self.reader.seek(SeekFrom::Start(range.start())).await?;
        let mut bytes = vec![
            0;
            usize::try_from(range.len())
                .map_err(|_| Error::InvalidInput("range too large"))?
        ];
        self.reader.read_exact(&mut bytes).await?;
        DataChunk::new(range.range(), bytes)
    }

    pub async fn read_zif(&mut self) -> Result<Image> {
        let mut parser = Parser::new();
        let mut chunk = DataChunk::default();

        while let ParseState::Need { range, .. } = parser.feed(chunk)? {
            chunk = self.fetch(range).await?;
        }

        parser.finish()
    }
}

pub async fn read_zif(reader: impl AsyncRead + AsyncSeek + Unpin) -> Result<Image> {
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
        for action in batch.into_actions() {
            self.writer.seek(SeekFrom::Start(action.offset)).await?;
            self.writer.write_all(&action.bytes).await?;
        }
        Ok(self.writer.flush().await?)
    }
}
