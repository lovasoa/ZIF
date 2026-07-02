use alloc::vec::Vec;
use core::ops::Range;

use crate::codec::{Codec, ColorModel};
use crate::chunk::ByteRange;
use crate::tiff::ceil_div;
use crate::{Error, Result};

/// Classification of the image's directory chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Pyramid,
    TimeSeries,
    Collection,
}

/// Parsed ZIF image metadata.
///
/// ```
/// let image = zif_tiff::sample::image();
/// assert_eq!(image.dimensions(), (40, 40));
/// assert_eq!(image.width(), 40);
/// assert_eq!(image.height(), 40);
/// assert_eq!(image.level_count(), 1);
/// assert_eq!(image.levels().len(), 1);
/// assert_eq!(image.level(0)?.dimensions(), (40, 40));
/// assert_eq!(image.codec(), zif_tiff::Codec::Jpeg);
/// assert_eq!(image.color_model(), zif_tiff::ColorModel::YCbCr);
/// assert_eq!(image.channels(), 3);
/// assert_eq!(image.kind(), zif_tiff::ImageKind::Pyramid);
/// assert_eq!(image.level_tiles(0)?.count(), 9);
/// assert_eq!(image.viewport_tiles(0, (15..17, 0..16))?.count(), 2);
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Image {
    pub(crate) levels: Vec<Level>,
    pub(crate) kind: ImageKind,
}

impl Image {
    pub(crate) fn new(levels: Vec<Level>) -> Self {
        let kind = classify(&levels);
        Self { levels, kind }
    }

    /// Returns base-level dimensions as `(width, height)`.
    pub fn dimensions(&self) -> (u64, u64) {
        self.levels[0].dimensions()
    }

    /// Returns the base-level width in pixels.
    pub fn width(&self) -> u64 {
        self.levels[0].width
    }

    /// Returns the base-level height in pixels.
    pub fn height(&self) -> u64 {
        self.levels[0].height
    }

    /// Returns the number of image directories/levels.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Returns all parsed levels.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// Returns a level by index.
    pub fn level(&self, index: usize) -> Result<&Level> {
        self.levels
            .get(index)
            .ok_or(Error::InvalidInput("level index out of range"))
    }

    /// Returns the base-level codec.
    pub fn codec(&self) -> Codec {
        self.levels[0].codec
    }

    /// Returns the base-level color model.
    pub fn color_model(&self) -> ColorModel {
        self.levels[0].color_model
    }

    /// Returns the base-level channel count.
    pub fn channels(&self) -> u16 {
        self.levels[0].channels
    }

    /// Returns how the directory chain is classified.
    pub fn kind(&self) -> ImageKind {
        self.kind
    }

    /// Iterates every tile in a level in row-major order.
    pub fn level_tiles(&self, level: usize) -> Result<TileIter<'_>> {
        let level = self.level(level)?;
        Ok(TileIter::new(
            level,
            0,
            0,
            level.tiles_across,
            level.tiles_down,
        ))
    }

    /// Iterates tiles intersecting a pixel viewport at a level.
    ///
    /// The viewport is `(x_range, y_range)` in pixels at that level.
    pub fn viewport_tiles(
        &self,
        level: usize,
        viewport: (Range<u64>, Range<u64>),
    ) -> Result<TileIter<'_>> {
        let level = self.level(level)?;
        View::new(viewport.0, viewport.1).and_then(|v| level.tiles_in_view(v))
    }
}

/// Metadata for one image pyramid level.
///
/// ```
/// let image = zif_tiff::sample::image();
/// let level = image.level(0)?;
/// assert_eq!(level.dimensions(), (40, 40));
/// assert_eq!(level.width(), 40);
/// assert_eq!(level.height(), 40);
/// assert_eq!(level.tile_size(), (16, 16));
/// assert_eq!(level.tile_grid(), (3, 3));
/// assert_eq!(level.tile_count(), 9);
/// assert_eq!(level.codec(), zif_tiff::Codec::Jpeg);
/// assert_eq!(level.color_model(), zif_tiff::ColorModel::YCbCr);
/// assert_eq!(level.channels(), 3);
/// assert_eq!(level.tile(2, 2)?.size(), (8, 8));
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Level {
    pub(crate) index: usize,
    pub(crate) width: u64,
    pub(crate) height: u64,
    pub(crate) tile_width: u64,
    pub(crate) tile_height: u64,
    pub(crate) tiles_across: u64,
    pub(crate) tiles_down: u64,
    pub(crate) codec: Codec,
    pub(crate) color_model: ColorModel,
    pub(crate) channels: u16,
    pub(crate) ycbcr_subsampling: Option<(u16, u16)>,
    pub(crate) offsets: Vec<u64>,
    pub(crate) counts: Vec<u32>,
}

impl Level {
    pub(crate) fn new(
        index: usize,
        width: u64,
        height: u64,
        tile_width: u64,
        tile_height: u64,
        codec: Codec,
        color_model: ColorModel,
        channels: u16,
        ycbcr_subsampling: Option<(u16, u16)>,
        offsets: Vec<u64>,
        counts: Vec<u32>,
    ) -> Result<Self> {
        let (tiles_across, tiles_down, tile_count) =
            crate::tiff::tile_count(width, height, tile_width, tile_height)?;
        if u64::try_from(offsets.len()).map_err(|_| Error::MalformedFile("too many offsets"))?
            != tile_count
            || u64::try_from(counts.len())
                .map_err(|_| Error::MalformedFile("too many byte counts"))?
                != tile_count
        {
            return Err(Error::MalformedFile(
                "tile array length does not match tile count",
            ));
        }
        for (&offset, &count) in offsets.iter().zip(&counts) {
            offset
                .checked_add(u64::from(count))
                .ok_or(Error::MalformedFile("tile byte range overflows"))?;
        }
        Ok(Self {
            index,
            width,
            height,
            tile_width,
            tile_height,
            tiles_across,
            tiles_down,
            codec,
            color_model,
            channels,
            ycbcr_subsampling,
            offsets,
            counts,
        })
    }

    /// Returns level dimensions as `(width, height)`.
    pub fn dimensions(&self) -> (u64, u64) {
        (self.width, self.height)
    }

    /// Returns this level's width in pixels.
    pub fn width(&self) -> u64 {
        self.width
    }

    /// Returns this level's height in pixels.
    pub fn height(&self) -> u64 {
        self.height
    }

    /// Returns tile dimensions as `(width, height)`.
    pub fn tile_size(&self) -> (u64, u64) {
        (self.tile_width, self.tile_height)
    }

    /// Returns tile grid dimensions as `(tile_cols, tile_rows)`.
    pub fn tile_grid(&self) -> (u64, u64) {
        (self.tiles_across, self.tiles_down)
    }

    /// Returns the total tile count for this level.
    pub fn tile_count(&self) -> u64 {
        self.tiles_across * self.tiles_down
    }

    /// Returns this level's codec.
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Returns this level's color model.
    pub fn color_model(&self) -> ColorModel {
        self.color_model
    }

    /// Returns this level's channel count.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns the YCbCr subsampling factors recorded for JPEG YCbCr tiles.
    pub fn ycbcr_subsampling(&self) -> Option<(u16, u16)> {
        self.ycbcr_subsampling
    }

    /// Returns a tile by column and row.
    pub fn tile(&self, col: u64, row: u64) -> Result<Tile<'_>> {
        if col >= self.tiles_across || row >= self.tiles_down {
            return Err(Error::InvalidInput("tile coordinate out of range"));
        }
        Ok(Tile {
            level: self,
            index: row * self.tiles_across + col,
        })
    }

    fn tiles_in_view(&self, view: View) -> Result<TileIter<'_>> {
        let x0 = view.x.start.min(self.width);
        let x1 = view.x.end.min(self.width);
        let y0 = view.y.start.min(self.height);
        let y1 = view.y.end.min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return Ok(TileIter::new(self, 0, 0, 0, 0));
        }
        let start_col = x0 / self.tile_width;
        let end_col = ceil_div(x1, self.tile_width)?.min(self.tiles_across);
        let start_row = y0 / self.tile_height;
        let end_row = ceil_div(y1, self.tile_height)?.min(self.tiles_down);
        Ok(TileIter::new(
            self, start_col, start_row, end_col, end_row,
        ))
    }
}

/// A pixel viewport represented as `x_range` and `y_range`.
#[derive(Debug, Clone)]
pub struct View {
    x: Range<u64>,
    y: Range<u64>,
}

impl View {
    /// Creates a pixel viewport from `(x_range, y_range)`.
    pub fn new(x: Range<u64>, y: Range<u64>) -> Result<Self> {
        if x.start > x.end || y.start > y.end {
            return Err(Error::InvalidInput("viewport range start is after end"));
        }
        Ok(Self { x, y })
    }
}

/// Metadata for one encoded tile.
///
/// ```
/// let image = zif_tiff::sample::image();
/// let tile = image.level(0)?.tile(2, 2)?;
/// assert_eq!(tile.level(), 0);
/// assert_eq!(tile.index(), 8);
/// assert_eq!(tile.col(), 2);
/// assert_eq!(tile.row(), 2);
/// assert_eq!(tile.x(), 32);
/// assert_eq!(tile.y(), 32);
/// assert_eq!(tile.width(), 8);
/// assert_eq!(tile.height(), 8);
/// assert_eq!(tile.position(), (32, 32));
/// assert_eq!(tile.size(), (8, 8));
/// assert_eq!(tile.range().range(), tile.byte_range());
/// assert_eq!(tile.codec(), zif_tiff::Codec::Jpeg);
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Tile<'a> {
    level: &'a Level,
    index: u64,
}

impl Tile<'_> {
    /// Returns the level index containing this tile.
    pub fn level(&self) -> usize {
        self.level.index
    }

    /// Returns this tile's row-major index.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Returns the tile column.
    pub fn col(&self) -> u64 {
        self.index % self.level.tiles_across
    }

    /// Returns the tile row.
    pub fn row(&self) -> u64 {
        self.index / self.level.tiles_across
    }

    /// Returns the tile's left pixel coordinate.
    pub fn x(&self) -> u64 {
        self.col() * self.level.tile_width
    }

    /// Returns the tile's top pixel coordinate.
    pub fn y(&self) -> u64 {
        self.row() * self.level.tile_height
    }

    /// Returns the tile's clipped pixel width.
    pub fn width(&self) -> u64 {
        self.level.tile_width.min(self.level.width - self.x())
    }

    /// Returns the tile's clipped pixel height.
    pub fn height(&self) -> u64 {
        self.level.tile_height.min(self.level.height - self.y())
    }

    /// Returns the tile's `(x, y)` pixel position.
    pub fn position(&self) -> (u64, u64) {
        (self.x(), self.y())
    }

    /// Returns the tile's clipped `(width, height)`.
    pub fn size(&self) -> (u64, u64) {
        (self.width(), self.height())
    }

    /// Returns the encoded tile byte range in the ZIF file.
    pub fn byte_range(&self) -> Range<u64> {
        let i = usize::try_from(self.index)
            .expect("tile index fits usize because arrays are indexed by it");
        let start = self.level.offsets[i];
        start..start + u64::from(self.level.counts[i])
    }

    /// Returns a byte-range request for the encoded tile.
    pub fn range(&self) -> ByteRange {
        ByteRange::new(self.byte_range())
            .expect("tile byte ranges are validated when levels are built")
    }

    /// Returns the codec used by this tile's level.
    pub fn codec(&self) -> Codec {
        self.level.codec
    }
}

/// Row-major iterator over tiles at a level.
pub struct TileIter<'a> {
    level: &'a Level,
    next_col: u64,
    next_row: u64,
    start_col: u64,
    end_col: u64,
    end_row: u64,
}

impl<'a> TileIter<'a> {
    pub(crate) fn new(
        level: &'a Level,
        start_col: u64,
        start_row: u64,
        end_col: u64,
        end_row: u64,
    ) -> Self {
        Self {
            level,
            next_col: start_col,
            next_row: start_row,
            start_col,
            end_col,
            end_row,
        }
    }
}

impl<'a> Iterator for TileIter<'a> {
    type Item = Tile<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.end_row || self.next_col >= self.end_col {
            return None;
        }
        let index = self.next_row * self.level.tiles_across + self.next_col;
        self.next_col += 1;
        if self.next_col >= self.end_col {
            self.next_col = self.start_col;
            self.next_row += 1;
        }
        Some(Tile {
            level: self.level,
            index,
        })
    }
}

fn classify(levels: &[Level]) -> ImageKind {
    if levels.len() <= 1 {
        return ImageKind::Pyramid;
    }
    let same = levels
        .iter()
        .all(|l| l.width == levels[0].width && l.height == levels[0].height);
    if same {
        return ImageKind::TimeSeries;
    }
    let pyramid = levels
        .windows(2)
        .all(|w| w[1].width == w[0].width.div_ceil(2) && w[1].height == w[0].height.div_ceil(2));
    if pyramid {
        ImageKind::Pyramid
    } else {
        ImageKind::Collection
    }
}
