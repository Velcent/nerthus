//! An OpenEXR file's headers as OpenEXRCore 3.4.9 reads them without
//! strict header checking (`parse_header.c`, `validation.c`): the magic
//! number and version flags, each part's attributes up to its empty name,
//! the required ones checked and kept (later copies overwrite earlier ones,
//! channels accumulate), defaults for missing ones but the channel list,
//! then each part validated and its chunk count and tile levels worked
//! out. Unparseable attributes are skipped.

use anyhow::{anyhow, bail, Result};

const MAGIC: u32 = 20_000_630;
const VERSION_MASK: u32 = 0xFF;
const TILED_FLAG: u32 = 0x200;
const LONG_NAMES_FLAG: u32 = 0x400;
const NON_IMAGE_FLAG: u32 = 0x800;
const MULTI_PART_FLAG: u32 = 0x1000;
const VALID_FLAGS: u32 = TILED_FLAG | LONG_NAMES_FLAG | NON_IMAGE_FLAG | MULTI_PART_FLAG;
/// `EXR_LONGNAME_MAXLEN`: without strict checking every name may be long.
const MAX_NAME_LENGTH: usize = 255;

/// `exr_pixel_type_t`.
pub(super) const PIXEL_UINT: u8 = 0;
pub(super) const PIXEL_HALF: u8 = 1;
pub(super) const PIXEL_FLOAT: u8 = 2;

/// `exr_compression_t`.
pub(super) const COMPRESSION_NONE: u8 = 0;
pub(super) const COMPRESSION_RLE: u8 = 1;
pub(super) const COMPRESSION_ZIPS: u8 = 2;
pub(super) const COMPRESSION_ZIP: u8 = 3;
pub(super) const COMPRESSION_PIZ: u8 = 4;
pub(super) const COMPRESSION_PXR24: u8 = 5;
pub(super) const COMPRESSION_B44: u8 = 6;
pub(super) const COMPRESSION_B44A: u8 = 7;
pub(super) const COMPRESSION_DWAA: u8 = 8;
pub(super) const COMPRESSION_DWAB: u8 = 9;
pub(super) const COMPRESSION_HTJ2K256: u8 = 10;
pub(super) const COMPRESSION_HTJ2K32: u8 = 11;
const COMPRESSION_LAST_TYPE: u8 = 12;

/// `exr_lineorder_t`.
pub(super) const LINE_ORDER_DECREASING_Y: u8 = 1;
const LINE_ORDER_LAST_TYPE: u8 = 3;

/// `exr_storage_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Storage {
    Scanline,
    Tiled,
    DeepScanline,
    DeepTiled,
    Unknown,
    /// Not known yet (`EXR_STORAGE_LAST_TYPE`).
    Unset,
}

impl Storage {
    pub fn is_deep(self) -> bool {
        matches!(self, Storage::DeepScanline | Storage::DeepTiled)
    }

    pub fn is_scanline(self) -> bool {
        matches!(self, Storage::Scanline | Storage::DeepScanline)
    }
}

/// A channel list entry (`exr_attr_chlist_entry_t`).
#[derive(Clone, Debug)]
pub(super) struct ChannelInfo {
    pub name: Vec<u8>,
    pub pixel_type: u8,
    pub p_linear: bool,
    pub x_sampling: i32,
    pub y_sampling: i32,
}

impl ChannelInfo {
    pub fn bytes_per_element(&self) -> u64 {
        if self.pixel_type == PIXEL_HALF { 2 } else { 4 }
    }
}

/// `exr_attr_tiledesc_t`.
#[derive(Clone, Copy, Debug)]
pub(super) struct TileDescription {
    pub x_size: u32,
    pub y_size: u32,
    /// `EXR_TILE_ONE_LEVEL`, `…_MIPMAP_LEVELS` or `…_RIPMAP_LEVELS`.
    pub level_mode: u8,
    /// `EXR_TILE_ROUND_DOWN` or `…_UP`.
    pub round_mode: u8,
}

pub(super) const TILE_ONE_LEVEL: u8 = 0;
pub(super) const TILE_MIPMAP_LEVELS: u8 = 1;
pub(super) const TILE_RIPMAP_LEVELS: u8 = 2;
const TILE_ROUND_DOWN: u8 = 0;

/// An inclusive box (`exr_attr_box2i_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Box2i {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// One part's header and what is derived from it (`_priv_part`).
#[derive(Clone, Debug)]
pub(super) struct Part {
    pub storage: Storage,
    pub channels: Option<Vec<ChannelInfo>>,
    pub compression: Option<u8>,
    pub data_window: Option<Box2i>,
    pub display_window: Option<Box2i>,
    pub line_order: Option<u8>,
    pub pixel_aspect_ratio: Option<f32>,
    pub screen_window_center: bool,
    pub screen_window_width: Option<f32>,
    pub tiles: Option<TileDescription>,
    pub name: bool,
    pub part_type: Option<Vec<u8>>,
    pub version: Option<i32>,
    pub chunk_count_attribute: bool,
    pub chunk_count: i32,
    pub chunk_table_offset: u64,
    pub lines_per_chunk: i32,
    pub unpacked_size_per_chunk: u64,
    pub chan_has_line_sampling: bool,
    pub tile_level_counts: (Vec<i32>, Vec<i32>),
    pub tile_level_sizes: (Vec<i32>, Vec<i32>),
}

impl Part {
    fn new(storage: Storage) -> Self {
        Self {
            storage,
            channels: None,
            compression: None,
            data_window: None,
            display_window: None,
            line_order: None,
            pixel_aspect_ratio: None,
            screen_window_center: false,
            screen_window_width: None,
            tiles: None,
            name: false,
            part_type: None,
            version: None,
            chunk_count_attribute: false,
            chunk_count: -1,
            chunk_table_offset: 0,
            lines_per_chunk: 0,
            unpacked_size_per_chunk: 0,
            chan_has_line_sampling: false,
            tile_level_counts: (Vec::new(), Vec::new()),
            tile_level_sizes: (Vec::new(), Vec::new()),
        }
    }

    pub fn channels(&self) -> &[ChannelInfo] {
        self.channels.as_deref().unwrap_or(&[])
    }

    pub fn compression(&self) -> u8 {
        self.compression.unwrap_or(COMPRESSION_ZIP)
    }

    pub fn data_window(&self) -> Box2i {
        self.data_window.unwrap_or(Box2i { min_x: 0, min_y: 0, max_x: 63, max_y: 63 })
    }

    pub fn line_order(&self) -> u8 {
        self.line_order.unwrap_or(0)
    }
}

/// The parsed file: its flags, parts and data.
pub(super) struct Headers {
    pub multipart: bool,
    pub parts: Vec<Part>,
}

/// `compute_sampled_height`: how many of `height` lines from `start_y`
/// fall on a multiple of `y_sampling`.
pub(super) fn sampled_height(height: i32, y_sampling: i32, start_y: i32) -> i32 {
    if y_sampling <= 1 {
        return height;
    }
    if height == 1 {
        return i32::from(start_y % y_sampling == 0);
    }
    let remainder = start_y % y_sampling;
    let start = if remainder != 0 { start_y + (y_sampling - remainder) } else { start_y };
    let mut end = start_y + height - 1;
    end -= end.abs() % y_sampling;
    if start > end {
        i32::from(start == start_y)
    } else {
        (end - start) / y_sampling + 1
    }
}

/// `compute_sampled_width`.
pub(super) fn sampled_width(width: i32, x_sampling: i32, _start_x: i32) -> i32 {
    if x_sampling <= 1 {
        return width;
    }
    if width == 1 { 1 } else { width / x_sampling }
}

/// `exr_compression_lines_per_chunk`.
pub(super) fn lines_per_chunk(compression: u8) -> i32 {
    match compression {
        COMPRESSION_NONE | COMPRESSION_RLE | COMPRESSION_ZIPS => 1,
        COMPRESSION_ZIP | COMPRESSION_PXR24 => 16,
        COMPRESSION_PIZ | COMPRESSION_B44 | COMPRESSION_B44A | COMPRESSION_HTJ2K32 | COMPRESSION_DWAA => 32,
        COMPRESSION_DWAB | COMPRESSION_HTJ2K256 => 256,
        _ => -1,
    }
}

/// Reads the header bytes in order (`_internal_exr_seq_scratch`).
struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let byte = *self.data.get(self.position).ok_or_else(|| anyhow!("Unable to extract header byte"))?;
        self.position += 1;
        Ok(byte)
    }

    fn bytes(&mut self, count: usize) -> Result<&[u8]> {
        let end = self.position.checked_add(count).filter(|&end| end <= self.data.len()).ok_or_else(|| anyhow!("Unable to read attribute data"))?;
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn skip(&mut self, count: i64) {
        if count > 0 {
            self.position = (self.position + count as usize).min(self.data.len());
        }
    }

    fn i32(&mut self) -> Result<i32> {
        let bytes = self.bytes(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// `read_text`: a zero-terminated name after `start` (its first byte,
    /// already read, when given).
    fn text(&mut self, start: Option<u8>) -> Result<Vec<u8>> {
        let mut text = Vec::new();
        if let Some(byte) = start {
            text.push(byte);
        }
        loop {
            if text.len() > MAX_NAME_LENGTH {
                bail!("Invalid name encountered (max {MAX_NAME_LENGTH})");
            }
            let byte = self.byte()?;
            if byte == 0 {
                return Ok(text);
            }
            text.push(byte);
        }
    }

    /// `scratch_attr_too_big`.
    fn too_big(&self, size: i32) -> bool {
        size as i64 > (self.data.len() - self.position) as i64
    }
}

fn box2i(bytes: &[u8]) -> Box2i {
    let value = |index: usize| i32::from_le_bytes([bytes[4 * index], bytes[4 * index + 1], bytes[4 * index + 2], bytes[4 * index + 3]]);
    Box2i { min_x: value(0), min_y: value(1), max_x: value(2), max_y: value(3) }
}

/// `extract_attr_chlist`: entries of a name, the pixel type, the pLinear
/// byte and three reserved, and the two sampling factors, until an empty
/// name; each added in name order.
fn read_channel_list(reader: &mut Reader, size: i32, list: &mut Vec<ChannelInfo>) -> Result<()> {
    if size < 0 || reader.too_big(size) {
        bail!("Attribute 'channels': Invalid size {size}");
    }
    let mut remaining = size as i64;
    let mut result = Ok(());
    while remaining > 0 {
        let name = match reader.text(None) {
            Ok(name) => name,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        remaining -= name.len() as i64 + 1;
        if name.is_empty() {
            break;
        }
        if remaining < 16 {
            reader.skip(remaining);
            bail!("Out of data parsing 'channels'");
        }
        let pixel_type = reader.i32()?;
        let flags = reader.bytes(4)?[0];
        let x_sampling = reader.i32()?;
        let y_sampling = reader.i32()?;
        remaining -= 16;
        let pixel_type = if !(0..=3).contains(&pixel_type) { 3 } else { pixel_type };
        if let Err(error) = add_channel(list, name, pixel_type, flags, x_sampling, y_sampling) {
            result = Err(error);
            break;
        }
    }
    reader.skip(remaining);
    result
}

/// `exr_attr_chlist_add_with_length`.
fn add_channel(list: &mut Vec<ChannelInfo>, name: Vec<u8>, pixel_type: i32, flags: u8, x_sampling: i32, y_sampling: i32) -> Result<()> {
    if name.len() > MAX_NAME_LENGTH {
        bail!("Channel name too long");
    }
    if !matches!(pixel_type, 0..=2) {
        bail!("Invalid pixel type specified ({pixel_type})");
    }
    if flags > 1 {
        bail!("Invalid perceptual linear flag value ({flags})");
    }
    if x_sampling <= 0 || y_sampling <= 0 {
        bail!("Invalid pixel sampling (x {x_sampling} y {y_sampling})");
    }
    let mut position = 0;
    for (index, existing) in list.iter().enumerate() {
        match name.as_slice().cmp(existing.name.as_slice()) {
            std::cmp::Ordering::Less => {
                position = index;
                break;
            }
            std::cmp::Ordering::Equal => bail!("Attempt to add duplicate channel"),
            std::cmp::Ordering::Greater => position = index + 1,
        }
    }
    let channel = ChannelInfo { name, pixel_type: pixel_type as u8, p_linear: flags == 1, x_sampling, y_sampling };
    list.insert(position, channel);
    Ok(())
}

/// One attribute (`pull_attr`): the required ones kept, others skipped.
fn read_attribute(reader: &mut Reader, part: &mut Part, first: u8, flags: (bool, bool)) -> Result<()> {
    let (multipart, non_image) = flags;
    let name = reader.text(Some(first))?;
    let type_name = reader.text(None)?;
    if name.is_empty() || type_name.is_empty() {
        bail!("Invalid empty string encountered parsing attribute");
    }
    let size = reader.i32()?;
    let type_is = |wanted: &str| type_name.as_slice() == wanted.as_bytes();
    let mismatch = |reader: &mut Reader| -> Result<()> {
        reader.skip(size as i64);
        bail!("Required attribute has an invalid type")
    };
    match name.as_slice() {
        b"channels" => {
            if !type_is("chlist") {
                return mismatch(reader);
            }
            if let Some(list) = part.channels.as_mut() {
                return read_channel_list(reader, size, list);
            }
            let mut list = Vec::new();
            read_channel_list(reader, size, &mut list)?;
            part.channels = Some(list);
            Ok(())
        }
        b"compression" => {
            if !type_is("compression") {
                return mismatch(reader);
            }
            let value = read_uint8(reader, size, COMPRESSION_LAST_TYPE)?;
            part.compression = Some(value);
            Ok(())
        }
        b"dataWindow" | b"displayWindow" => {
            if !type_is("box2i") {
                return mismatch(reader);
            }
            if size != 16 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            let value = box2i(reader.bytes(16)?);
            if name.as_slice() == b"dataWindow" {
                part.data_window = Some(value);
            } else {
                part.display_window = Some(value);
            }
            Ok(())
        }
        b"lineOrder" => {
            if !type_is("lineOrder") {
                return mismatch(reader);
            }
            part.line_order = Some(read_uint8(reader, size, LINE_ORDER_LAST_TYPE)?);
            Ok(())
        }
        b"pixelAspectRatio" | b"screenWindowWidth" => {
            if !type_is("float") {
                return mismatch(reader);
            }
            if size != 4 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            let bytes = reader.bytes(4)?;
            let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            if name.as_slice() == b"pixelAspectRatio" {
                part.pixel_aspect_ratio = Some(value);
            } else {
                part.screen_window_width = Some(value);
            }
            Ok(())
        }
        b"screenWindowCenter" => {
            if !type_is("v2f") {
                return mismatch(reader);
            }
            if size != 8 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            reader.bytes(8)?;
            part.screen_window_center = true;
            Ok(())
        }
        b"tiles" => {
            if !type_is("tiledesc") {
                return mismatch(reader);
            }
            if size != 9 {
                reader.skip(size as i64);
                bail!("Invalid size {size} for tiles");
            }
            let bytes = reader.bytes(9)?;
            let x_size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let y_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let mode = bytes[8];
            part.tiles = Some(TileDescription { x_size, y_size, level_mode: mode & 0xF, round_mode: mode >> 4 });
            Ok(())
        }
        b"name" => {
            if !type_is("string") {
                return mismatch(reader);
            }
            if size < 0 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            reader.bytes(size as usize)?;
            part.name = true;
            Ok(())
        }
        b"type" => {
            if size < 0 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            if part.part_type.is_some() {
                reader.skip(size as i64);
                bail!("Duplicate copy of required attribute 'type' encountered");
            }
            if !type_is("string") {
                return mismatch(reader);
            }
            let value = reader.bytes(size as usize)?.to_vec();
            let storage = match value.as_slice() {
                b"scanlineimage" => Some(Storage::Scanline),
                b"tiledimage" => Some(Storage::Tiled),
                b"deepscanline" => Some(Storage::DeepScanline),
                b"deeptile" => Some(Storage::DeepTiled),
                _ => None,
            };
            match storage {
                Some(storage) if multipart || non_image => part.storage = storage,
                Some(_) => {}
                None if part.storage == Storage::Unset => part.storage = Storage::Unknown,
                None => {}
            }
            part.part_type = Some(value);
            Ok(())
        }
        b"version" => {
            if !type_is("int") || size != 4 {
                return mismatch(reader);
            }
            part.version = Some(reader.i32()?);
            Ok(())
        }
        b"chunkCount" => {
            if part.chunk_count_attribute || !type_is("int") || size != 4 {
                return mismatch(reader);
            }
            part.chunk_count = reader.i32()?;
            part.chunk_count_attribute = true;
            Ok(())
        }
        _ => {
            if size < 0 || reader.too_big(size) {
                bail!("Invalid size {size}");
            }
            reader.skip(size as i64);
            Ok(())
        }
    }
}

/// `extract_attr_uint8`.
fn read_uint8(reader: &mut Reader, size: i32, limit: u8) -> Result<u8> {
    if size != 1 {
        bail!("Invalid size {size} (expected 1)");
    }
    let value = reader.byte()?;
    if value >= limit {
        bail!("Invalid value {value} (max allowed {limit})");
    }
    Ok(value)
}

/// `internal_exr_validate_read_part`, defaults filled in as without strict
/// checking.
fn validate_part(part: &mut Part, singlepart_tiled: bool, multipart: bool, part_count: usize) -> Result<()> {
    part.compression.get_or_insert(COMPRESSION_ZIP);
    part.data_window.get_or_insert(Box2i { min_x: 0, min_y: 0, max_x: 63, max_y: 63 });
    part.display_window.get_or_insert(Box2i { min_x: 0, min_y: 0, max_x: 63, max_y: 63 });
    part.line_order.get_or_insert(0);
    part.pixel_aspect_ratio.get_or_insert(1.0);
    part.screen_window_width.get_or_insert(1.0);
    if multipart && !part.name {
        bail!("'name' attribute for multipart file not found");
    }

    // `validate_image_dimensions`.
    let large = (i32::MAX / 2) as i64;
    let data_window = part.data_window();
    let display_window = part.display_window.expect("defaulted");
    let invalid = |window: Box2i| {
        window.min_x > window.max_x || window.min_y > window.max_y || window.min_x as i64 <= -large || window.min_y as i64 <= -large || window.max_x as i64 >= large || window.max_y as i64 >= large
    };
    if invalid(display_window) {
        bail!("Invalid display window");
    }
    if invalid(data_window) {
        bail!("Invalid data window");
    }
    let aspect = part.pixel_aspect_ratio.expect("defaulted");
    if !aspect.is_normal() || aspect < 1e-6 || aspect > 1e6 {
        bail!("Invalid pixel aspect ratio {aspect}");
    }
    if part.screen_window_width.expect("defaulted") < 0.0 {
        bail!("Invalid screen window width");
    }

    // `validate_channels`.
    let Some(channels) = &part.channels else {
        bail!("'channels' attribute not found");
    };
    if channels.is_empty() {
        bail!("At least one channel required");
    }
    let width = data_window.max_x as i64 - data_window.min_x as i64 + 1;
    let height = data_window.max_y as i64 - data_window.min_y as i64 + 1;
    for channel in channels {
        let (xs, ys) = (channel.x_sampling, channel.y_sampling);
        if xs < 1 || ys < 1 {
            bail!("invalid subsampling factor");
        }
        if data_window.min_x % xs != 0 || data_window.min_y % ys != 0 || width % xs as i64 != 0 || height % ys as i64 != 0 {
            bail!("channel subsampling does not divide the data window");
        }
    }

    // `validate_part_type`.
    if part.part_type.is_some() && singlepart_tiled {
        if multipart || part_count > 1 {
            bail!("Multipart files cannot have the tiled bit set");
        }
        part.storage = Storage::Tiled;
    }
    if part.storage == Storage::Unset {
        bail!("Unable to determine data storage type for part");
    }

    // `validate_tile_data`.
    if matches!(part.storage, Storage::Tiled | Storage::DeepTiled) {
        let Some(tiles) = part.tiles else {
            bail!("'tiles' attribute for tiled file not found");
        };
        if tiles.x_size == 0 || tiles.y_size == 0 || tiles.x_size > (i32::MAX / 4) as u32 || tiles.y_size > (i32::MAX / 4) as u32 {
            bail!("Invalid tile description size");
        }
        if tiles.level_mode > TILE_RIPMAP_LEVELS || tiles.round_mode > 1 {
            bail!("Invalid tile level or rounding mode");
        }
        if channels.iter().any(|channel| channel.x_sampling != 1 || channel.y_sampling != 1) {
            bail!("subsampling in a tiled image");
        }
    }

    // `validate_deep_data`.
    if part.storage.is_deep() {
        if !matches!(part.compression(), COMPRESSION_NONE | COMPRESSION_RLE | COMPRESSION_ZIPS) {
            bail!("Invalid compression for deep data");
        }
        if channels.iter().any(|channel| channel.x_sampling != 1 || channel.y_sampling != 1) {
            bail!("subsampling in a deep image");
        }
    }
    Ok(())
}

/// `floor_log2` and `ceil_log2`.
fn floor_log2(mut x: i64) -> i32 {
    let mut y = 0;
    while x > 1 {
        y += 1;
        x >>= 1;
    }
    y
}

fn ceil_log2(mut x: i64) -> i32 {
    let (mut y, mut r) = (0, 0);
    while x > 1 {
        if x & 1 == 1 {
            r = 1;
        }
        y += 1;
        x >>= 1;
    }
    y + r
}

/// `calc_level_size`.
fn level_size(min: i32, max: i32, level: i32, round_down: bool) -> i64 {
    let size = max as i64 - min as i64 + 1;
    let factor = 1i64 << level;
    let mut result = size / factor;
    if !round_down && result * factor < size {
        result += 1;
    }
    result.max(1)
}

/// `internal_exr_compute_tile_information`.
fn compute_tile_information(part: &mut Part) -> Result<()> {
    if !matches!(part.storage, Storage::Tiled | Storage::DeepTiled) {
        return Ok(());
    }
    let Some(tiles) = part.tiles else {
        bail!("missing required attribute 'tiles'");
    };
    let window = part.data_window();
    let width = window.max_x as i64 - window.min_x as i64 + 1;
    let height = window.max_y as i64 - window.min_y as i64 + 1;
    let round_down = tiles.round_mode == TILE_ROUND_DOWN;
    let log2 = |value: i64| if round_down { floor_log2(value) } else { ceil_log2(value) };
    let (levels_x, levels_y) = match tiles.level_mode {
        TILE_ONE_LEVEL => (1, 1),
        TILE_MIPMAP_LEVELS => {
            let count = log2(width.max(height)) + 1;
            (count, count)
        }
        TILE_RIPMAP_LEVELS => (log2(width) + 1, log2(height) + 1),
        _ => bail!("invalid tile level mode"),
    };
    let mut counts = (Vec::new(), Vec::new());
    let mut sizes = (Vec::new(), Vec::new());
    for level in 0..levels_x {
        let size = level_size(window.min_x, window.max_x, level, round_down);
        if size > i32::MAX as i64 {
            bail!("Invalid data window x dims");
        }
        counts.0.push((size as u64).div_ceil(tiles.x_size as u64) as i32);
        sizes.0.push(size as i32);
    }
    for level in 0..levels_y {
        let size = level_size(window.min_y, window.max_y, level, round_down);
        if size > i32::MAX as i64 {
            bail!("Invalid data window y dims");
        }
        counts.1.push((size as u64).div_ceil(tiles.y_size as u64) as i32);
        sizes.1.push(size as i32);
    }
    part.tile_level_counts = counts;
    part.tile_level_sizes = sizes;
    Ok(())
}

/// `internal_exr_compute_chunk_offset_size`: the chunk count, and the size
/// of a full chunk's pixels.
fn compute_chunk_count(part: &mut Part) -> i64 {
    let window = part.data_window();
    let width = (window.max_x as i64 - window.min_x as i64 + 1) as i32;
    let channels = part.channels().to_vec();
    if part.storage == Storage::Unknown {
        return part.chunk_count.max(0) as i64;
    }
    if matches!(part.storage, Storage::Tiled | Storage::DeepTiled) {
        let tiles = part.tiles.expect("tiled parts have tiles");
        let (counts_x, counts_y) = &part.tile_level_counts;
        let mut count = 0i64;
        match tiles.level_mode {
            TILE_ONE_LEVEL | TILE_MIPMAP_LEVELS => {
                for level in 0..counts_x.len() {
                    count += counts_x[level] as i64 * counts_y[level] as i64;
                }
            }
            _ => {
                for &x in counts_x {
                    for &y in counts_y {
                        count += x as i64 * y as i64;
                        if count > i32::MAX as i64 {
                            return -1;
                        }
                    }
                }
            }
        }
        if count > i32::MAX as i64 {
            return -1;
        }
        part.unpacked_size_per_chunk = channels.iter().map(|channel| channel.bytes_per_element() * tiles.x_size as u64 * tiles.y_size as u64).sum();
        part.chan_has_line_sampling = false;
        return count;
    }
    let lines = lines_per_chunk(part.compression());
    part.lines_per_chunk = lines;
    if lines < 0 {
        return -1;
    }
    let mut size = 0u64;
    let mut line_sampling = false;
    for channel in &channels {
        let samples = sampled_width(width, channel.x_sampling, window.min_x) as u64 * sampled_height(lines, channel.y_sampling, window.min_y) as u64;
        if channel.y_sampling > 1 {
            line_sampling = true;
        }
        size += channel.bytes_per_element() * samples;
    }
    part.unpacked_size_per_chunk = size;
    part.chan_has_line_sampling = line_sampling;
    let height = window.max_y as i64 - window.min_y as i64 + 1;
    ((height + lines as i64 - 1) / lines as i64) as i32 as i64
}

/// `internal_exr_parse_header`.
pub(super) fn parse(data: &[u8]) -> Result<Headers> {
    if data.len() < 8 {
        bail!("Unable to read magic and version flags");
    }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let flags = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if magic != MAGIC {
        bail!("File is not an OpenEXR file");
    }
    if flags & VERSION_MASK != 2 {
        bail!("File is of an unsupported version: {}", flags & VERSION_MASK);
    }
    let flags = flags & !VERSION_MASK;
    if flags & !VALID_FLAGS != 0 {
        bail!("File has an unsupported flags");
    }
    let singlepart_tiled = flags & TILED_FLAG != 0;
    let non_image = flags & NON_IMAGE_FLAG != 0;
    let multipart = flags & MULTI_PART_FLAG != 0;
    if singlepart_tiled && (non_image || multipart) {
        bail!("Invalid combination of version flags");
    }
    let first_storage = if !non_image && !multipart {
        if singlepart_tiled { Storage::Tiled } else { Storage::Scanline }
    } else {
        Storage::Unset
    };
    let mut parts = vec![Part::new(first_storage)];
    let mut reader = Reader { data, position: 8 };
    loop {
        let mut byte = reader.byte()?;
        if byte == 0 {
            let count = parts.len();
            let part = parts.last_mut().expect("there is a part");
            validate_part(part, singlepart_tiled, multipart, count)?;
            if !multipart {
                break;
            }
            byte = reader.byte().map_err(|_| anyhow!("Unable to go to next part definition"))?;
            if byte == 0 {
                break;
            }
            parts.push(Part::new(Storage::Unset));
        }
        let part = parts.last_mut().expect("there is a part");
        // Without strict checking a bad attribute is passed over.
        let _ = read_attribute(&mut reader, part, byte, (multipart, non_image));
    }
    // `update_chunk_offsets`.
    let mut offset = reader.position as u64;
    for part in &mut parts {
        compute_tile_information(part)?;
        let count = compute_chunk_count(part);
        if count < 0 {
            bail!("Invalid chunk count ({count})");
        }
        if part.chunk_count < 0 || part.chunk_count as i64 != count {
            part.chunk_count = count as i32;
        }
        part.chunk_table_offset = offset;
        offset += 8 * part.chunk_count as u64;
    }
    Ok(Headers { multipart, parts })
}
