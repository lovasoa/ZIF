use crate::{Chunk, ReadStatus, Reader, Request, Zif};

pub struct HttpRangeReader {
    client: reqwest::Client,
    url: String,
}

impl HttpRangeReader {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_client(reqwest::Client::new(), url)
    }

    pub fn with_client(client: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }

    pub async fn fetch(&self, req: Request) -> crate::Result<Chunk> {
        let header = if req.is_empty() {
            String::from("bytes=0-0")
        } else {
            alloc::format!("bytes={}-{}", req.start(), req.end() - 1)
        };
        let res = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, header)
            .send()
            .await
            .map_err(|_| crate::Error::InvalidInput("http request failed"))?;
        let status = res.status();
        let range = if status == reqwest::StatusCode::OK {
            None
        } else if status == reqwest::StatusCode::PARTIAL_CONTENT {
            parse_content_range(
                res.headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok()),
            )
        } else {
            return Err(crate::Error::InvalidInput("unexpected http status"));
        };
        let bytes = res
            .bytes()
            .await
            .map_err(|_| crate::Error::InvalidInput("failed to read http body"))?
            .to_vec();
        match range {
            Some(r) => Chunk::new(r, bytes),
            None => Chunk::from_start(0, bytes),
        }
    }

    pub async fn read_zif(&self) -> crate::Result<Zif> {
        let mut reader = Reader::new();
        let mut chunk = Chunk::default();

        loop {
            match reader.advance(chunk)? {
                ReadStatus::Need { req, .. } => chunk = self.fetch(req).await?,
                ReadStatus::Done { zif } => return Ok(zif.as_zif().clone()),
            }
        }
    }
}

pub async fn read_zif(url: impl Into<String>) -> crate::Result<Zif> {
    HttpRangeReader::new(url).read_zif().await
}

fn parse_content_range(value: Option<&str>) -> Option<core::ops::Range<u64>> {
    let value = value?;
    let rest = value.strip_prefix("bytes ")?;
    let (range, _) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end: u64 = end.parse().ok()?;
    Some(start..end.checked_add(1)?)
}
