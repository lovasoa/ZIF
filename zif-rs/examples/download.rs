use std::{env, io};

use zif_tiff::http::HttpRangeReader;
use zif_tiff::tokio::FileRangeWriter;

fn main() -> io::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

type HttpReader = HttpRangeReader<reqwest::Client, reqwest::Request, reqwest::Body>;

async fn run() -> io::Result<()> {
    let (url, output) = args()?;
    let client = reqwest::Client::new();
    let uri: http::Uri = url
        .parse()
        .map_err(|e: http::uri::InvalidUri| invalid_data(e.to_string()))?;
    let mut http: HttpReader = HttpRangeReader::new(client, uri);

    let zif = http.read_zif().await.map_err(io_error)?;
    let mut writer = writer_for(&zif)?;
    let mut file = FileRangeWriter::create(output).await.map_err(io_error)?;

    for level_index in 0..zif.level_count() {
        for tile in zif.get_level_tiles(level_index).map_err(io_error)? {
            let bytes = fetch_tile(&mut http, tile.req()).await?;
            let batch = writer
                .put_tile_at_level(level_index, (tile.col(), tile.row()), bytes)
                .map_err(io_error)?;
            file.apply(batch).await.map_err(io_error)?;
        }
    }

    Ok(())
}

fn args() -> io::Result<(String, String)> {
    let mut args = env::args().skip(1);
    let Some(url) = args.next() else {
        return Err(usage());
    };
    let Some(output) = args.next() else {
        return Err(usage());
    };
    if args.next().is_some() {
        return Err(usage());
    }
    Ok((url, output))
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: cargo run --example download --features http,tokio -- <url> <output.zif>",
    )
}

fn writer_for(zif: &zif_tiff::Zif) -> io::Result<zif_tiff::Writer> {
    let mut builder = zif_tiff::Writer::new()
        .codec(zif.codec())
        .color_model(zif.color_model())
        .channels(zif.channels())
        .map_err(io_error)?;

    if let Some(subsampling) = zif.level(0).map_err(io_error)?.ycbcr_subsampling() {
        builder = if subsampling == (1, 1) || subsampling == (2, 2) {
            builder.ycbcr_subsampling(subsampling).map_err(io_error)?
        } else {
            // Compatibility path for non-conforming files noted in specification section 6.6.
            // The example rewrites existing tile streams, so preserving the original tag keeps
            // libtiff/ImageMagick in sync with the JPEG sampling actually present in those tiles.
            builder
                .preserve_nonstandard_ycbcr_subsampling(subsampling)
                .map_err(io_error)?
        };
    }

    for level in zif.levels() {
        let (tile_width, tile_height) = level.tile_size();
        let tile_size = (
            u32::try_from(tile_width)
                .map_err(|_| invalid_data("tile width does not fit in u32"))?,
            u32::try_from(tile_height)
                .map_err(|_| invalid_data("tile height does not fit in u32"))?,
        );
        let spec = zif_tiff::LevelSpec::new(level.dimensions(), tile_size).map_err(io_error)?;
        builder = builder.level(spec);
    }

    builder.build().map_err(io_error)
}

async fn fetch_tile(http: &mut HttpReader, req: zif_tiff::Request) -> io::Result<Vec<u8>> {
    if req.is_empty() {
        return Ok(Vec::new());
    }

    let requested = req.range();
    let chunk = http.fetch(req).await.map_err(io_error)?;
    if chunk.start() > requested.start || chunk.end() < requested.end {
        return Err(invalid_data("http response did not cover requested tile"));
    }

    let start = usize::try_from(requested.start - chunk.start())
        .map_err(|_| invalid_data("tile start does not fit in usize"))?;
    let len = usize::try_from(requested.end - requested.start)
        .map_err(|_| invalid_data("tile length does not fit in usize"))?;
    Ok(chunk.bytes()[start..start + len].to_vec())
}

fn io_error(err: zif_tiff::Error) -> io::Error {
    invalid_data(err.to_string())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
