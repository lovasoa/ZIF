use core::marker::PhantomData;

use http::header;
use http::uri::Uri;
use http::{Request as HttpRequest, Response as HttpResponse, StatusCode};
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use tower_service::Service;

use crate::{Chunk, ReadStatus, Reader, Request, Result, Zif};

/// A ZIF reader backed by any [`tower_service::Service`] that can execute
/// HTTP requests.
///
/// The service must accept a request type `R` convertible from
/// [`http::Request<Vec<u8>>`] and return a response convertible to
/// [`http::Response<B>`] where `B: http_body::Body`.
///
/// ```ignore
/// use zif_tiff::http::HttpRangeReader;
///
/// let client = reqwest::Client::new();
/// let url: http::Uri = "https://example.com/slide.zif".parse()?;
/// let mut reader = HttpRangeReader::new(client, url);
/// let zif = reader.read_zif().await?;
/// ```
pub struct HttpRangeReader<S, R = HttpRequest<Vec<u8>>, ResBody = Vec<u8>> {
    service: S,
    url: Uri,
    _phantom: PhantomData<fn(R, ResBody)>,
}

impl<S, R, ResBody> HttpRangeReader<S, R, ResBody> {
    pub fn new(service: S, url: Uri) -> Self {
        Self {
            service,
            url,
            _phantom: PhantomData,
        }
    }

    pub fn into_service(self) -> S {
        self.service
    }
}

impl<S, R, ResBody> HttpRangeReader<S, R, ResBody>
where
    S: Service<R>,
    R: TryFrom<HttpRequest<Vec<u8>>>,
    R::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::error::Error + Send + Sync + 'static,
    S::Response: Into<HttpResponse<ResBody>>,
    ResBody: HttpBody,
    ResBody::Error: std::error::Error + Send + Sync + 'static,
{
    pub async fn fetch(&mut self, req: Request) -> Result<Chunk> {
        if req.is_empty() {
            return Chunk::from_start(req.start(), Vec::new());
        }

        let range_header = format!("bytes={}-{}", req.start(), req.end() - 1);

        let http_req = HttpRequest::builder()
            .uri(&self.url)
            .header(header::RANGE, &range_header)
            .body(Vec::new())
            .map_err(|e| crate::Error::Http(Box::new(e)))?;

        let service_req = R::try_from(http_req).map_err(|e| crate::Error::Http(Box::new(e)))?;

        let res = self
            .service
            .call(service_req)
            .await
            .map_err(|e| crate::Error::Http(Box::new(e)))?;

        let response: HttpResponse<ResBody> = res.into();
        let (parts, body) = response.into_parts();
        let bytes = body
            .collect()
            .await
            .map_err(|e| crate::Error::Http(Box::new(e)))?
            .to_bytes()
            .to_vec();

        Chunk::try_from(HttpResponse::from_parts(parts, bytes))
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

impl TryFrom<HttpResponse<Vec<u8>>> for Chunk {
    type Error = crate::Error;

    fn try_from(response: HttpResponse<Vec<u8>>) -> Result<Self> {
        let status = response.status();
        let range = if status == StatusCode::OK {
            None
        } else if status == StatusCode::PARTIAL_CONTENT {
            parse_content_range(
                response
                    .headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok()),
            )
        } else {
            return Err(crate::Error::InvalidInput("unexpected http status"));
        };

        let bytes = response.into_body();
        match range {
            Some(r) => Self::new(r, bytes),
            None => Self::from_start(0, bytes),
        }
    }
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
