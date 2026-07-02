use alloc::vec::Vec;
use core::ops::Range;

use crate::format::{ceil_div, checked_len};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Jpeg,
    Png,
    JpegXr,
    Jpeg2000,
}

impl Codec {
    pub(crate) fn from_code(code: u16) -> Result<Self> {
        match code {
            7 => Ok(Self::Jpeg),
            34933 => Ok(Self::Png),
            34934 => Ok(Self::JpegXr),
            34712 => Ok(Self::Jpeg2000),
            _ => Err(Error::MalformedFile("unsupported codec code")),
        }
    }

    pub(crate) fn code(self) -> u16 {
        match self {
            Self::Jpeg => 7,
            Self::Png => 34933,
            Self::JpegXr => 34934,
            Self::Jpeg2000 => 34712,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorModel {
    WhiteIsZero,
    BlackIsZero,
    Rgb,
    YCbCr,
}

impl ColorModel {
    pub(crate) fn from_code(code: u16) -> Result<Self> {
        match code {
            0 => Ok(Self::WhiteIsZero),
            1 => Ok(Self::BlackIsZero),
            2 => Ok(Self::Rgb),
            6 => Ok(Self::YCbCr),
            _ => Err(Error::MalformedFile("unsupported color model")),
        }
    }

    pub(crate) fn code(self) -> u16 {
        match self {
            Self::WhiteIsZero => 0,
            Self::BlackIsZero => 1,
            Self::Rgb => 2,
            Self::YCbCr => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainKind {
    Pyramid,
    TimeSeries,
    Collection,
}

/// A byte-range request made by the reader or by a tile.
///
/// ```
/// let req = zif_tiff::Request::new(10..20)?;
/// assert_eq!(req.range(), 10..20);
/// assert_eq!(req.start(), 10);
/// assert_eq!(req.end(), 20);
/// assert_eq!(req.len(), 10);
/// assert!(!req.is_empty());
/// assert!(zif_tiff::Request::new(4..4)?.is_empty());
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    range: Range<u64>,
}

impl Request {
    /// Creates a request for a half-open byte range.
    ///
    /// ```
    /// ```
    pub fn new(range: Range<u64>) -> Result<Self> {
        if range.start > range.end {
            return Err(Error::InvalidInput("request range start is after end"));
        }
        Ok(Self { range })
    }

    /// Returns the requested half-open byte range.
    ///
    /// ```
    /// ```
    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// Returns the first requested byte offset.
    ///
    /// ```
    /// ```
    pub fn start(&self) -> u64 {
        self.range.start
    }

    /// Returns the exclusive end offset.
    ///
    /// ```
    pub fn end(&self) -> u64 {
        self.range.end
    }

    /// Returns the requested byte count.
    ///
    /// ```
    /// ```
    pub fn len(&self) -> u64 {
        self.range.end - self.range.start
    }

    /// Returns true when the requested range is empty.
    ///
    /// ```
    /// let req = zif_tiff::Request::new(4..4)?;
    /// assert!(req.is_empty());
    /// # Ok::<(), zif_tiff::Error>(())
    /// ```
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

impl From<Request> for Range<u64> {
    fn from(value: Request) -> Self {
        value.range
    }
}

/// A coherent byte chunk returned by an IO layer.
///
/// ```
/// let chunk = zif_tiff::Chunk::new(10..13, vec![1, 2, 3])?;
/// assert_eq!(chunk.range(), 10..13);
/// assert_eq!(chunk.start(), 10);
/// assert_eq!(chunk.end(), 13);
/// assert_eq!(chunk.bytes(), &[1, 2, 3]);
///
/// let chunk = zif_tiff::Chunk::from_start(20, vec![4, 5])?;
/// assert_eq!(chunk.range(), 20..22);
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk<B = Vec<u8>> {
    range: Range<u64>,
    bytes: B,
}

impl Default for Chunk<Vec<u8>> {
    fn default() -> Self {
        Self {
            range: 0..0,
            bytes: Vec::new(),
        }
    }
}

impl<B: AsRef<[u8]>> Chunk<B> {
    /// Creates a coherent chunk for a half-open byte range.
    ///
    /// The range length must match the byte length.
    ///
    /// ```
    /// ```
    pub fn new(range: Range<u64>, bytes: B) -> Result<Self> {
        if range.start > range.end {
            return Err(Error::InvalidInput("chunk range start is after end"));
        }
        let expected = range.end - range.start;
        let actual = u64::try_from(bytes.as_ref().len())
            .map_err(|_| Error::InvalidInput("chunk length does not fit u64"))?;
        if expected != actual {
            return Err(Error::InvalidInput(
                "chunk range length differs from byte length",
            ));
        }
        Ok(Self { range, bytes })
    }

    /// Creates a coherent chunk from a start offset and byte buffer.
    ///
    /// ```
    /// ```
    pub fn from_start(start: u64, bytes: B) -> Result<Self> {
        let range = checked_len(start, bytes.as_ref().len())?;
        Self::new(range, bytes)
    }

    /// Returns the byte range covered by this chunk.
    ///
    /// ```
    /// ```
    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// Returns the bytes carried by this chunk.
    ///
    /// ```
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Returns the first byte offset covered by this chunk.
    ///
    /// ```
    /// ```
    pub fn start(&self) -> u64 {
        self.range.start
    }

    /// Returns the exclusive end offset covered by this chunk.
    ///
    /// ```
    /// ```
    pub fn end(&self) -> u64 {
        self.range.end
    }
}

/// Parsed ZIF metadata.
///
/// ```
/// let zif = zif_tiff::sample::zif();
/// assert_eq!(zif.dimensions(), (40, 40));
/// assert_eq!(zif.width(), 40);
/// assert_eq!(zif.height(), 40);
/// assert_eq!(zif.level_count(), 1);
/// assert_eq!(zif.levels().len(), 1);
/// assert_eq!(zif.level(0)?.dimensions(), (40, 40));
/// assert_eq!(zif.codec(), zif_tiff::Codec::Jpeg);
/// assert_eq!(zif.color_model(), zif_tiff::ColorModel::YCbCr);
/// assert_eq!(zif.channels(), 3);
/// assert_eq!(zif.chain_kind(), zif_tiff::ChainKind::Pyramid);
/// assert_eq!(zif.get_level_tiles(0)?.count(), 9);
/// assert_eq!(zif.get_cropped_level_tiles(0, (15..17, 0..16))?.count(), 2);
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Zif {
    pub(crate) levels: Vec<Level>,
    pub(crate) kind: ChainKind,
}

impl Zif {
    pub(crate) fn new(levels: Vec<Level>) -> Self {
        let kind = classify(&levels);
        Self { levels, kind }
    }

    /// Returns base-level dimensions as `(width, height)`.
    ///
    /// ```
    /// ```
    pub fn dimensions(&self) -> (u64, u64) {
        self.levels[0].dimensions()
    }

    /// Returns the base-level width in pixels.
    ///
    /// ```
    /// ```
    pub fn width(&self) -> u64 {
        self.levels[0].width
    }

    /// Returns the base-level height in pixels.
    ///
    /// ```
    /// ```
    pub fn height(&self) -> u64 {
        self.levels[0].height
    }

    /// Returns the number of image directories/levels.
    ///
    /// ```
    /// ```
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Returns all parsed levels.
    ///
    /// ```
    /// ```
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// Returns a level by index.
    ///
    /// ```
    /// ```
    pub fn level(&self, index: usize) -> Result<&Level> {
        self.levels
            .get(index)
            .ok_or(Error::InvalidInput("level index out of range"))
    }

    /// Returns the base-level codec.
    ///
    /// ```
    pub fn codec(&self) -> Codec {
        self.levels[0].codec
    }

    /// Returns the base-level color model.
    ///
    /// ```
    /// ```
    pub fn color_model(&self) -> ColorModel {
        self.levels[0].color_model
    }

    /// Returns the base-level channel count.
    ///
    /// ```
    /// ```
    pub fn channels(&self) -> u16 {
        self.levels[0].channels
    }

    /// Returns how the directory chain is classified.
    ///
    /// ```
    /// ```
    pub fn chain_kind(&self) -> ChainKind {
        self.kind
    }

    /// Iterates every tile in a level in row-major order.
    ///
    /// ```
    /// ```
    pub fn get_level_tiles(&self, level: usize) -> Result<LevelTiles<'_>> {
        let level = self.level(level)?;
        Ok(LevelTiles::new(
            level,
            0,
            0,
            level.tiles_across,
            level.tiles_down,
        ))
    }

    /// Iterates tiles intersecting a pixel region at a level.
    ///
    /// The region is `(x_range, y_range)` in pixels at that level.
    ///
    /// ```
    /// ```
    pub fn get_cropped_level_tiles(
        &self,
        level: usize,
        region: (Range<u64>, Range<u64>),
    ) -> Result<LevelTiles<'_>> {
        let level = self.level(level)?;
        Region::new(region.0, region.1).and_then(|r| level.tiles_in_region(r))
    }
}

/// Borrowed view of parsed ZIF metadata.
///
/// A view returned while reading is in progress may describe only the fully
/// parsed prefix of the directory chain. After the reader returns `Done`, the
/// same methods describe the complete file.
#[derive(Debug, Clone, Copy)]
pub struct ZifView<'a> {
    zif: &'a Zif,
}

impl<'a> ZifView<'a> {
    pub(crate) fn new(zif: &'a Zif) -> Self {
        Self { zif }
    }

    /// Returns the underlying owned metadata object.
    pub fn as_zif(&self) -> &'a Zif {
        self.zif
    }

    /// Returns base-level dimensions as `(width, height)`.
    pub fn dimensions(&self) -> (u64, u64) {
        self.zif.dimensions()
    }

    /// Returns the base-level width in pixels.
    pub fn width(&self) -> u64 {
        self.zif.width()
    }

    /// Returns the base-level height in pixels.
    pub fn height(&self) -> u64 {
        self.zif.height()
    }

    /// Returns the number of parsed image directories/levels.
    pub fn level_count(&self) -> usize {
        self.zif.level_count()
    }

    /// Returns all parsed levels.
    pub fn levels(&self) -> &'a [Level] {
        self.zif.levels()
    }

    /// Returns a level by index.
    pub fn level(&self, index: usize) -> Result<&'a Level> {
        self.zif.level(index)
    }

    /// Returns the base-level codec.
    pub fn codec(&self) -> Codec {
        self.zif.codec()
    }

    /// Returns the base-level color model.
    pub fn color_model(&self) -> ColorModel {
        self.zif.color_model()
    }

    /// Returns the base-level channel count.
    pub fn channels(&self) -> u16 {
        self.zif.channels()
    }

    /// Returns how the parsed directory chain prefix is classified.
    pub fn chain_kind(&self) -> ChainKind {
        self.zif.chain_kind()
    }

    /// Iterates every tile in a level in row-major order.
    pub fn get_level_tiles(&self, level: usize) -> Result<LevelTiles<'a>> {
        self.zif.get_level_tiles(level)
    }

    /// Iterates tiles intersecting a pixel region at a level.
    pub fn get_cropped_level_tiles(
        &self,
        level: usize,
        region: (Range<u64>, Range<u64>),
    ) -> Result<LevelTiles<'a>> {
        self.zif.get_cropped_level_tiles(level, region)
    }
}

/// Metadata for one image level.
///
/// ```
/// let zif = zif_tiff::sample::zif();
/// let level = zif.level(0)?;
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
            crate::format::tile_count(width, height, tile_width, tile_height)?;
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
    ///
    /// ```
    /// ```
    pub fn dimensions(&self) -> (u64, u64) {
        (self.width, self.height)
    }

    /// Returns this level's width in pixels.
    ///
    /// ```
    /// ```
    pub fn width(&self) -> u64 {
        self.width
    }
    /// Returns this level's height in pixels.
    ///
    /// ```
    /// ```
    pub fn height(&self) -> u64 {
        self.height
    }
    /// Returns tile dimensions as `(width, height)`.
    ///
    /// ```
    /// ```
    pub fn tile_size(&self) -> (u64, u64) {
        (self.tile_width, self.tile_height)
    }
    /// Returns tile grid dimensions as `(tiles_across, tiles_down)`.
    ///
    /// ```
    /// ```
    pub fn tile_grid(&self) -> (u64, u64) {
        (self.tiles_across, self.tiles_down)
    }
    /// Returns the total tile count for this level.
    ///
    /// ```
    pub fn tile_count(&self) -> u64 {
        self.tiles_across * self.tiles_down
    }
    /// Returns this level's codec.
    ///
    /// ```
    /// ```
    pub fn codec(&self) -> Codec {
        self.codec
    }
    /// Returns this level's color model.
    ///
    /// ```
    /// ```
    pub fn color_model(&self) -> ColorModel {
        self.color_model
    }
    /// Returns this level's channel count.
    ///
    /// ```
    /// ```
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns the YCbCr subsampling factors recorded for JPEG YCbCr tiles.
    ///
    /// ```
    /// let zif = zif_tiff::sample::zif();
    /// let level = zif.level(0)?;
    /// assert_eq!(level.ycbcr_subsampling(), Some((2, 2)));
    /// # Ok::<(), zif_tiff::Error>(())
    /// ```
    pub fn ycbcr_subsampling(&self) -> Option<(u16, u16)> {
        self.ycbcr_subsampling
    }

    /// Returns a tile by column and row.
    ///
    /// ```
    /// ```
    pub fn tile(&self, col: u64, row: u64) -> Result<Tile<'_>> {
        if col >= self.tiles_across || row >= self.tiles_down {
            return Err(Error::InvalidInput("tile coordinate out of range"));
        }
        Ok(Tile {
            level: self,
            index: row * self.tiles_across + col,
        })
    }

    fn tiles_in_region(&self, region: Region) -> Result<LevelTiles<'_>> {
        let x0 = region.x.start.min(self.width);
        let x1 = region.x.end.min(self.width);
        let y0 = region.y.start.min(self.height);
        let y1 = region.y.end.min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return Ok(LevelTiles::new(self, 0, 0, 0, 0));
        }
        let start_col = x0 / self.tile_width;
        let end_col = ceil_div(x1, self.tile_width)?.min(self.tiles_across);
        let start_row = y0 / self.tile_height;
        let end_row = ceil_div(y1, self.tile_height)?.min(self.tiles_down);
        Ok(LevelTiles::new(
            self, start_col, start_row, end_col, end_row,
        ))
    }
}

/// A pixel region represented as `x_range` and `y_range`.
///
/// ```
/// let region = zif_tiff::Region::new(10..20, 30..40)?;
/// let _ = region;
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Region {
    x: Range<u64>,
    y: Range<u64>,
}

impl Region {
    /// Creates a pixel region from `(x_range, y_range)`.
    ///
    /// ```
    pub fn new(x: Range<u64>, y: Range<u64>) -> Result<Self> {
        if x.start > x.end || y.start > y.end {
            return Err(Error::InvalidInput("region range start is after end"));
        }
        Ok(Self { x, y })
    }
}

/// Metadata for one encoded tile.
///
/// ```
/// let zif = zif_tiff::sample::zif();
/// let tile = zif.level(0)?.tile(2, 2)?;
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
/// assert_eq!(tile.req().range(), tile.bytes());
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
    ///
    /// ```
    /// ```
    pub fn level(&self) -> usize {
        self.level.index
    }
    /// Returns this tile's row-major index.
    ///
    /// ```
    /// ```
    pub fn index(&self) -> u64 {
        self.index
    }
    /// Returns the tile column.
    ///
    /// ```
    /// ```
    pub fn col(&self) -> u64 {
        self.index % self.level.tiles_across
    }
    /// Returns the tile row.
    ///
    /// ```
    /// ```
    pub fn row(&self) -> u64 {
        self.index / self.level.tiles_across
    }
    /// Returns the tile's left pixel coordinate.
    ///
    /// ```
    /// ```
    pub fn x(&self) -> u64 {
        self.col() * self.level.tile_width
    }
    /// Returns the tile's top pixel coordinate.
    ///
    /// ```
    /// ```
    pub fn y(&self) -> u64 {
        self.row() * self.level.tile_height
    }
    /// Returns the tile's clipped pixel width.
    ///
    /// ```
    pub fn width(&self) -> u64 {
        self.level.tile_width.min(self.level.width - self.x())
    }
    /// Returns the tile's clipped pixel height.
    ///
    /// ```
    /// ```
    pub fn height(&self) -> u64 {
        self.level.tile_height.min(self.level.height - self.y())
    }
    /// Returns the tile's `(x, y)` pixel position.
    ///
    /// ```
    /// ```
    pub fn position(&self) -> (u64, u64) {
        (self.x(), self.y())
    }
    /// Returns the tile's clipped `(width, height)`.
    ///
    /// ```
    /// ```
    pub fn size(&self) -> (u64, u64) {
        (self.width(), self.height())
    }

    /// Returns the encoded tile byte range in the ZIF file.
    ///
    /// ```
    /// let zif = zif_tiff::sample::zif();
    /// let tile = zif.level(0)?.tile(0, 0)?;
    /// assert!(!tile.bytes().is_empty());
    /// # Ok::<(), zif_tiff::Error>(())
    /// ```
    pub fn bytes(&self) -> Range<u64> {
        let i = usize::try_from(self.index)
            .expect("tile index fits usize because arrays are indexed by it");
        let start = self.level.offsets[i];
        start..start + u64::from(self.level.counts[i])
    }

    /// Returns a byte-range request for the encoded tile.
    ///
    /// ```
    /// ```
    pub fn req(&self) -> Request {
        Request::new(self.bytes()).expect("tile ranges are validated when levels are built")
    }

    /// Returns the codec used by this tile's level.
    ///
    /// ```
    /// ```
    pub fn codec(&self) -> Codec {
        self.level.codec
    }
}

pub struct LevelTiles<'a> {
    level: &'a Level,
    next_col: u64,
    next_row: u64,
    start_col: u64,
    end_col: u64,
    end_row: u64,
}

impl<'a> LevelTiles<'a> {
    fn new(level: &'a Level, start_col: u64, start_row: u64, end_col: u64, end_row: u64) -> Self {
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

impl<'a> Iterator for LevelTiles<'a> {
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

fn classify(levels: &[Level]) -> ChainKind {
    if levels.len() <= 1 {
        return ChainKind::Pyramid;
    }
    let same = levels
        .iter()
        .all(|l| l.width == levels[0].width && l.height == levels[0].height);
    if same {
        return ChainKind::TimeSeries;
    }
    let pyramid = levels
        .windows(2)
        .all(|w| w[1].width == w[0].width.div_ceil(2) && w[1].height == w[0].height.div_ceil(2));
    if pyramid {
        ChainKind::Pyramid
    } else {
        ChainKind::Collection
    }
}
