//! Multiple-image Network Graphics as ImageMagick 7.1.2 reads them
//! (`ReadOneMNGImage` in `coders/png.c`) for the one scene Magick.NET asks
//! for. The chunk loop keeps the objects' offsets and clipping boxes, the
//! framing mode and delays, loops and magnification; each embedded PNG or
//! JNG becomes a layer (rebuilt as a file of its own for the PNG and JNG
//! readers), and reading stops after the second layer with a delay, whose
//! pixels are skipped. Unless the stream is MNG-VLC, more than one layer is
//! coalesced (`CoalesceImages`) and layers without a delay merge into the
//! next, so the image is the first frame with a delay, or the last.
//!
//! The chunks' 4-byte integers go through C `long`, which is 32 bits on
//! Windows: offsets and boxes past 2³¹ wrap to negative numbers here.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use super::jng;
use crate::pixel_cache::{read_png_bytes_with, CacheColorspace, ImageType, PixelCache, PixelChannel, DEFAULT_GAMMA};

const SIGNATURE: &[u8; 8] = b"\x8aMNG\r\n\x1a\n";
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const JNG_SIGNATURE: &[u8; 8] = b"\x8bJNG\r\n\x1a\n";
const MAX_OBJECTS: usize = 256;
const PNG_UINT_31_MAX: u64 = 0x7fff_ffff;
const QUANTUM_RANGE: f64 = 65535.0;
const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
/// `UndefinedTicksPerSecond`.
const IMAGE_TICKS_PER_SECOND: u64 = 100;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `mng_get_long`: a big-endian 32-bit value as a Windows `long`.
fn get_long(bytes: &[u8]) -> i32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i32
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// A box of `long`s (`MngBox`): right and bottom are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MngBox {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

impl MngBox {
    fn zero() -> MngBox {
        MngBox { left: 0, right: 0, top: 0, bottom: 0 }
    }

    fn unbounded() -> MngBox {
        MngBox { left: 0, right: PNG_UINT_31_MAX as i32, top: 0, bottom: PNG_UINT_31_MAX as i32 }
    }

    /// `mng_read_box`: absolute, or relative to `previous`.
    fn read(previous: MngBox, delta_type: u8, bytes: &[u8]) -> MngBox {
        let mut found = MngBox { left: get_long(bytes), right: get_long(&bytes[4..]), top: get_long(&bytes[8..]), bottom: get_long(&bytes[12..]) };
        if delta_type != 0 {
            found.left = found.left.wrapping_add(previous.left);
            found.right = found.right.wrapping_add(previous.right);
            found.top = found.top.wrapping_add(previous.top);
            found.bottom = found.bottom.wrapping_add(previous.bottom);
        }
        found
    }

    /// `mng_minimum_box`: the intersection.
    fn minimum(self, other: MngBox) -> MngBox {
        MngBox { left: self.left.max(other.left), right: self.right.min(other.right), top: self.top.max(other.top), bottom: self.bottom.min(other.bottom) }
    }
}

/// `image->page`.
#[derive(Clone, Copy, Debug, Default)]
struct Page {
    width: u64,
    height: u64,
    x: i64,
    y: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dispose {
    Undefined,
    None,
    Background,
    Previous,
}

/// `PixelInfo` for a background colour.
#[derive(Clone, Copy, Debug)]
struct Background {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
    alpha_trait: bool,
}

impl Background {
    /// `BackgroundColor`, white.
    fn white() -> Background {
        Background { red: QUANTUM_RANGE, green: QUANTUM_RANGE, blue: QUANTUM_RANGE, alpha: QUANTUM_RANGE, alpha_trait: false }
    }

    fn is_gray(&self) -> bool {
        (self.red - self.green).abs() < 1.0e-12 && (self.green - self.blue).abs() < 1.0e-12
    }
}

/// An image of the list `ReadOneMNGImage` builds.
#[derive(Clone)]
struct Layer {
    cache: PixelCache,
    page: Page,
    delay: u64,
    dispose: Dispose,
    background: Background,
    /// Whether the image's own pixel queue is set (`GetAuthenticPixelQueue`):
    /// true once a reader wrote pixels, never for a background layer, so
    /// the next image read reuses (overwrites) a background layer.
    has_pixel_queue: bool,
}

impl Layer {
    /// `AcquireImage`: no pixels, white background.
    fn new() -> Layer {
        let cache = PixelCache::new(0, 0, vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], Vec::new(), CacheColorspace::SRgb, DEFAULT_GAMMA);
        Layer { cache, page: Page::default(), delay: 0, dispose: Dispose::Undefined, background: Background::white(), has_pixel_queue: false }
    }

    /// `SetImageBackgroundColor` at a new size for an image whose
    /// `alpha_trait` was cleared: the colour's red, green and blue.
    fn fill_background(&mut self, columns: u64, rows: u64) {
        let background = self.background;
        let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
        let mut data = Vec::with_capacity((columns * rows * 3) as usize);
        for _ in 0..columns * rows {
            data.extend([background.red as f32, background.green as f32, background.blue as f32]);
        }
        let gamma = self.cache.gamma;
        self.cache = PixelCache::new(columns as usize, rows as usize, channels, data, CacheColorspace::SRgb, gamma);
    }
}

/// The parts of `MngReadInfo` the reader uses.
struct State {
    mng_width: u64,
    mng_height: u64,
    ticks_per_second: u64,
    frame: MngBox,
    clip: MngBox,
    image_box: MngBox,
    object_clip: [MngBox; MAX_OBJECTS],
    exists: [bool; MAX_OBJECTS],
    frozen: [bool; MAX_OBJECTS],
    invisible: [bool; MAX_OBJECTS],
    x_off: [i64; MAX_OBJECTS],
    y_off: [i64; MAX_OBJECTS],
    framing_mode: u8,
    global_plte: Option<Vec<u8>>,
    global_trns_length: usize,
    global_gamma: Option<f32>,
    global_chrm: Option<[u32; 8]>,
    global_bkgd: Option<[u16; 3]>,
    magn_mb: u16,
    magn_ml: u16,
    magn_mr: u16,
    magn_mt: u16,
    magn_mx: u16,
    magn_my: u16,
    magn_methx: u16,
    magn_methy: u16,
    loop_active: [bool; 256],
    loop_count: [i64; 256],
    loop_jump: [usize; 256],
    scenes_found: i64,
    image_found: i64,
}

impl State {
    fn new() -> State {
        State {
            mng_width: 0,
            mng_height: 0,
            ticks_per_second: 0,
            frame: MngBox::zero(),
            clip: MngBox::zero(),
            image_box: MngBox::zero(),
            object_clip: [MngBox::unbounded(); MAX_OBJECTS],
            exists: [false; MAX_OBJECTS],
            frozen: [false; MAX_OBJECTS],
            invisible: [false; MAX_OBJECTS],
            x_off: [0; MAX_OBJECTS],
            y_off: [0; MAX_OBJECTS],
            framing_mode: 1,
            global_plte: None,
            global_trns_length: 0,
            global_gamma: None,
            global_chrm: None,
            global_bkgd: None,
            magn_mb: 0,
            magn_ml: 0,
            magn_mr: 0,
            magn_mt: 0,
            magn_mx: 0,
            magn_my: 0,
            magn_methx: 0,
            magn_methy: 0,
            loop_active: [false; 256],
            loop_count: [0; 256],
            loop_jump: [0; 256],
            scenes_found: 0,
            image_found: 0,
        }
    }

    /// `MngReadInfoDiscardObject`.
    fn discard_object(&mut self, index: usize) {
        if index != 0 && index < MAX_OBJECTS && self.exists[index] && !self.frozen[index] {
            self.exists[index] = false;
            self.invisible[index] = false;
            self.frozen[index] = false;
            self.x_off[index] = 0;
            self.y_off[index] = 0;
            self.object_clip[index] = MngBox::unbounded();
        }
    }
}

/// One chunk of the MNG stream.
struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

/// The blob with its position (`ReadBlob`, `TellBlob`, `SeekBlob`).
struct Blob<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Blob<'a> {
    fn read(&mut self, count: usize) -> &'a [u8] {
        let start = self.position.min(self.bytes.len());
        let end = start.saturating_add(count).min(self.bytes.len());
        self.position = end;
        &self.bytes[start..end]
    }

    fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read(4);
        if bytes.len() < 4 {
            return None;
        }
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A chunk as the MNG loop reads it: length, type, data and CRC.
    fn chunk(&mut self) -> Result<Chunk<'a>> {
        let length = self.read_u32().unwrap_or(0) as u64;
        let kind_bytes = self.read(4);
        if length > PNG_UINT_31_MAX || length > self.bytes.len() as u64 || kind_bytes.len() < 4 {
            bail!("CorruptImage");
        }
        let kind = [kind_bytes[0], kind_bytes[1], kind_bytes[2], kind_bytes[3]];
        let data = self.read(length as usize);
        if data.len() != length as usize {
            bail!("CorruptImage");
        }
        self.read(4);
        Ok(Chunk { kind, data })
    }
}

/// A chunk of an embedded PNG or JNG: where it starts, its type and its
/// data length.
#[derive(Clone, Copy)]
struct EmbeddedChunk {
    position: usize,
    kind: [u8; 4],
    length: usize,
}

impl EmbeddedChunk {
    fn data<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.position + 8..self.position + 8 + self.length]
    }

    fn whole<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.position..self.position + 12 + self.length]
    }
}

/// The chunks of an embedded PNG or JNG, from its header to its IEND, with
/// the position after them.
fn embedded_chunks(bytes: &[u8], start: usize) -> Result<(Vec<EmbeddedChunk>, usize)> {
    let mut chunks = Vec::new();
    let mut position = start;
    loop {
        if position + 8 > bytes.len() {
            bail!("UnexpectedEndOfFile");
        }
        let length = u32::from_be_bytes([bytes[position], bytes[position + 1], bytes[position + 2], bytes[position + 3]]) as usize;
        let kind = [bytes[position + 4], bytes[position + 5], bytes[position + 6], bytes[position + 7]];
        let end = position.checked_add(12).and_then(|value| value.checked_add(length)).ok_or_else(|| anyhow!("UnexpectedEndOfFile"))?;
        if end > bytes.len() {
            bail!("UnexpectedEndOfFile");
        }
        chunks.push(EmbeddedChunk { position, kind, length });
        position = end;
        if &kind == b"IEND" {
            return Ok((chunks, position));
        }
    }
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    chunk.extend_from_slice(&hasher.finalize().to_be_bytes());
    chunk
}

/// A page a canvas chunk gives: its size and maybe its offsets.
#[derive(Clone, Copy)]
struct CanvasChunk {
    width: u64,
    height: u64,
    offsets: Option<(i64, i64)>,
}

/// What an embedded PNG's chunks say about its page and background.
#[derive(Default)]
struct PngExtras {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    filter_64: bool,
    /// The oFFs offsets when given in pixels (libpng gives 0 otherwise).
    offsets: Option<(i64, i64)>,
    /// caNv and vpAg before and after the image data.
    canvas_before: Vec<CanvasChunk>,
    canvas_after: Vec<CanvasChunk>,
    bkgd: Option<Vec<u8>>,
    palette: Option<Vec<u8>>,
    has_gamma: bool,
    has_chrm: bool,
}

/// The canvas chunks IM's chunk callback reads (`vpAg`, `caNv`); a wrong
/// size is an error.
fn canvas_chunk(kind: &[u8; 4], data: &[u8]) -> Result<Option<CanvasChunk>> {
    if kind == b"vpAg" {
        if data.len() != 9 {
            bail!("error in user chunk");
        }
        if data[8] != 0 {
            return Ok(None);
        }
        return Ok(Some(CanvasChunk { width: get_long(data) as i64 as u64, height: get_long(&data[4..]) as i64 as u64, offsets: None }));
    }
    if kind == b"caNv" {
        if data.len() != 16 {
            bail!("error in user chunk");
        }
        let offsets = (get_long(&data[8..]) as i64, get_long(&data[12..]) as i64);
        return Ok(Some(CanvasChunk { width: get_long(data) as i64 as u64, height: get_long(&data[4..]) as i64 as u64, offsets: Some(offsets) }));
    }
    Ok(None)
}

/// Rebuilds an embedded PNG as a file of its own: the global palette for an
/// empty PLTE, the global gamma, chromaticities and significant bits it
/// lacks, filter method 64 turned into 0 (undone after decoding).
fn standalone_png(bytes: &[u8], chunks: &[EmbeddedChunk], state: &State) -> Result<(Vec<u8>, PngExtras)> {
    let mut extras = PngExtras::default();
    let mut out = PNG_SIGNATURE.to_vec();
    let mut seen_data = false;
    for chunk in chunks {
        let data = chunk.data(bytes);
        let length = chunk.length;
        let kind = chunk.kind;
        match &kind {
            b"IHDR" => {
                if length < 13 {
                    bail!("IHDR: invalid");
                }
                extras.width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                extras.height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                extras.bit_depth = data[8];
                extras.color_type = data[9];
                let mut header = data.to_vec();
                if data[11] == 64 && matches!(data[9], 2 | 6) {
                    extras.filter_64 = true;
                    header[11] = 0;
                }
                out.extend(png_chunk(b"IHDR", &header));
                continue;
            }
            b"PLTE" if extras.color_type == 3 && length == 0 => {
                // An empty PLTE takes the global one; with no global PLTE
                // libpng is told to fail.
                let Some(global) = &state.global_plte else {
                    bail!("No global PLTE in file");
                };
                extras.palette = Some(global.clone());
                out.extend(png_chunk(b"PLTE", global));
                continue;
            }
            b"PLTE" if length == 0 => continue,
            b"PLTE" => extras.palette = Some(data.to_vec()),
            b"IDAT" => {
                if !seen_data {
                    // What libpng adds from the globals before the data.
                    if !extras.has_gamma
                        && let Some(gamma) = state.global_gamma
                    {
                        let fixed = ((gamma as f64) * 100000.0 + 0.5).floor() as u32;
                        out.extend(png_chunk(b"gAMA", &fixed.to_be_bytes()));
                    }
                    if !extras.has_chrm
                        && let Some(chrm) = state.global_chrm
                    {
                        let values: Vec<u8> = chrm.iter().flat_map(|value| value.to_be_bytes()).collect();
                        out.extend(png_chunk(b"cHRM", &values));
                    }
                }
                seen_data = true;
            }
            b"gAMA" => extras.has_gamma = true,
            b"cHRM" => extras.has_chrm = true,
            b"oFFs" if !seen_data && length >= 9 => {
                extras.offsets = Some(if data[8] == 0 { (get_long(data) as i64, get_long(&data[4..]) as i64) } else { (0, 0) });
            }
            b"bKGD" => extras.bkgd = Some(data.to_vec()),
            b"vpAg" | b"caNv" => {
                if let Some(canvas) = canvas_chunk(&kind, data)? {
                    if seen_data {
                        extras.canvas_after.push(canvas);
                    } else {
                        extras.canvas_before.push(canvas);
                    }
                }
                continue;
            }
            _ => {}
        }
            out.extend_from_slice(chunk.whole(bytes));
    }
    Ok((out, extras))
}

/// The background colour a PNG's bKGD gives (`ReadOnePNGImage`): libpng's
/// colour (a palette entry for indexed images) scaled to 16 bits.
fn png_background(extras: &PngExtras) -> Option<Background> {
    let data = extras.bkgd.as_ref()?;
    let (red, green, blue) = match extras.color_type {
        3 => {
            let index = *data.first()? as usize;
            let palette = extras.palette.as_ref()?;
            if index >= palette.len() / 3 {
                return None;
            }
            (palette[index * 3] as u16, palette[index * 3 + 1] as u16, palette[index * 3 + 2] as u16)
        }
        0 | 4 => {
            if data.len() < 2 {
                return None;
            }
            let gray = be16(data);
            (gray, gray, gray)
        }
        _ => {
            if data.len() < 6 {
                return None;
            }
            (be16(data), be16(&data[2..]), be16(&data[4..]))
        }
    };
    // The depth was raised to 8 for packing by then, so only 16-bit
    // images skip the scaling by 257.
    let scale: u16 = if extras.bit_depth <= 8 { 257 } else { 1 };
    let scaled = |value: u16| value.wrapping_mul(scale) as f64;
    Some(Background { red: scaled(red), green: scaled(green), blue: scaled(blue), alpha: QUANTUM_RANGE, alpha_trait: false })
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob { bytes, position: 0 };
    if blob.read(8) != SIGNATURE {
        bail!("ImproperImageHeader");
    }
    let mut state = State::new();
    state.exists[0] = true;

    let mut layers = vec![Layer::new()];
    let mut skipping_loop: i32 = -1;
    let mut first_mng_object = true;
    let mut mng_type = 0u8;
    let mut insert_layers = false;
    let mut default_frame_delay: u64 = 0;
    let mut frame_delay: u64 = 0;
    let mut object_id = 0usize;
    let mut skip_to_iend = false;
    let mut mng_background = Background::white();
    let mut default_fb = state.frame;
    let mut previous_fb = state.frame;
    let mut simplicity = 0u32;
    let mut subframe_width: u64 = 0;
    let mut subframe_height: u64 = 0;

    loop {
        let chunk = blob.chunk()?;
        let kind = &chunk.kind;
        let data = chunk.data;
        if kind == b"DHDR" {
            bail!("delta-PNG is not supported");
        }
        if kind == b"MEND" {
            break;
        }
        if skip_to_iend {
            if kind == b"IEND" {
                skip_to_iend = false;
            }
            continue;
        }
        match kind {
            b"MHDR" => {
                if data.len() != 28 {
                    bail!("CorruptImage");
                }
                state.mng_width = get_long(data) as u32 as u64;
                state.mng_height = get_long(&data[4..]) as u32 as u64;
                state.ticks_per_second = get_long(&data[8..]) as i64 as u64 & 0xffff_ffff;
                default_frame_delay = IMAGE_TICKS_PER_SECOND.checked_div(state.ticks_per_second).unwrap_or(0);
                frame_delay = default_frame_delay;
                simplicity = get_long(&data[24..]) as u32;
                mng_type = 1;
                if simplicity != 0 && (simplicity | 11) == 11 {
                    mng_type = 2;
                }
                if simplicity != 0 && (simplicity | 9) == 9 {
                    mng_type = 3;
                }
                if mng_type != 3 {
                    insert_layers = true;
                }
                if layers.last().is_some_and(|layer| layer.has_pixel_queue) {
                    layers.push(Layer::new());
                }
                if state.mng_width > 65535 || state.mng_height > 65535 {
                    bail!("WidthOrHeightExceedsLimit");
                }
                state.frame = MngBox { left: 0, right: state.mng_width as i32, top: 0, bottom: state.mng_height as i32 };
                state.clip = state.frame;
                default_fb = state.frame;
                previous_fb = state.frame;
                state.object_clip = [state.frame; MAX_OBJECTS];
                continue;
            }
            // The final delay only reaches the last image, never the
            // frame read.
            b"TERM" => continue,
            b"DEFI" => {
                if mng_type == 3 {
                    bail!("DEFI chunk found in MNG-VLC datastream");
                }
                if data.len() < 2 {
                    bail!("CorruptImage");
                }
                object_id = be16(data) as usize;
                if mng_type == 2 && object_id != 0 {
                    bail!("Nonzero object_id in MNG-LC datastream");
                }
                if object_id >= MAX_OBJECTS {
                    bail!("object id too large");
                }
                if state.exists[object_id] && state.frozen[object_id] {
                    bail!("DEFI cannot redefine a frozen MNG object");
                }
                state.exists[object_id] = true;
                if data.len() > 2 {
                    state.invisible[object_id] = data[2] != 0;
                }
                if data.len() > 11 {
                    state.x_off[object_id] = get_long(&data[4..]) as i64;
                    state.y_off[object_id] = get_long(&data[8..]) as i64;
                }
                if data.len() > 27 {
                    state.object_clip[object_id] = MngBox::read(state.frame, 0, &data[12..]);
                }
                continue;
            }
            b"bKGD" => {
                state.global_bkgd = None;
                if data.len() > 5 {
                    state.global_bkgd = Some([be16(data), be16(&data[2..]), be16(&data[4..])]);
                }
                continue;
            }
            b"BACK" => {
                let mandatory_back = data.len() > 6 && data[6] != 0;
                if mandatory_back && data.len() > 5 {
                    mng_background.red = be16(data) as f64;
                    mng_background.green = be16(&data[2..]) as f64;
                    mng_background.blue = be16(&data[4..]) as f64;
                    mng_background.alpha = QUANTUM_RANGE;
                }
                continue;
            }
            b"PLTE" => {
                let entries = data.len() / 3;
                state.global_plte = if !data.is_empty() && data.len() < 769 && entries > 0 { Some(data[..entries * 3].to_vec()) } else { None };
                continue;
            }
            b"tRNS" => {
                state.global_trns_length = data.len();
                continue;
            }
            b"gAMA" => {
                state.global_gamma = if data.len() == 4 { Some(get_long(data) as i64 as f32 * 0.00001f32) } else { None };
                continue;
            }
            b"cHRM" => {
                // The values go through doubles and back to fixed point.
                state.global_chrm = if data.len() == 32 {
                    let mut values = [0u32; 8];
                    for (index, value) in values.iter_mut().enumerate() {
                        let real = 0.00001 * get_long(&data[index * 4..]) as f64;
                        *value = (real * 100000.0 + 0.5).floor() as u32;
                    }
                    Some(values)
                } else {
                    None
                };
                continue;
            }
            b"sRGB" | b"iCCP" => continue,
            b"FRAM" => {
                if mng_type == 3 {
                    bail!("FRAM chunk found in MNG-VLC datastream");
                }
                if (state.framing_mode == 2 || state.framing_mode == 4)
                    && let Some(layer) = layers.last_mut()
                {
                    layer.delay = frame_delay;
                }
                frame_delay = default_frame_delay;
                let mut fb = default_fb;
                if let Some(&mode) = data.first()
                    && mode != 0
                {
                    state.framing_mode = mode;
                }
                if data.len() > 6 {
                    let mut p = 1usize;
                    while p < data.len() && data[p] != 0 {
                        p += 1;
                    }
                    p += 1;
                    if (p as i64) < data.len() as i64 - 4 {
                        let change_delay = data[p];
                        let change_timeout = data[p + 1];
                        let change_clipping = data[p + 2];
                        p += 4;
                        if change_delay != 0 && p + 4 <= data.len() {
                            let ticks = IMAGE_TICKS_PER_SECOND.wrapping_mul(get_long(&data[p..]) as i64 as u64);
                            frame_delay = ticks.checked_div(state.ticks_per_second).unwrap_or(PNG_UINT_31_MAX);
                            if change_delay == 2 {
                                default_frame_delay = frame_delay;
                            }
                            p += 4;
                        }
                        if change_timeout != 0 && p + 4 <= data.len() {
                            p += 4;
                        }
                        if change_clipping != 0 && (p as i64) < data.len() as i64 - 16 {
                            fb = MngBox::read(previous_fb, data[p], &data[p + 1..]);
                            previous_fb = fb;
                            if change_clipping == 2 {
                                default_fb = fb;
                            }
                        }
                    }
                }
                state.clip = fb.minimum(state.frame);
                subframe_width = (state.clip.right as i64 - state.clip.left as i64) as u64;
                subframe_height = (state.clip.bottom as i64 - state.clip.top as i64) as u64;
                if insert_layers && state.framing_mode == 4 && subframe_width != 0 && subframe_height != 0 {
                    insert_background(&mut layers, subframe_width, subframe_height, state.clip, mng_background);
                }
                continue;
            }
            b"CLIP" => {
                if data.len() > 3 {
                    let first = be16(data) as usize;
                    let last = be16(&data[2..]) as usize;
                    let rest = &data[4..];
                    for index in first..=last {
                        if index >= MAX_OBJECTS {
                            continue;
                        }
                        if state.exists[index] && !state.frozen[index] && (4i64) < data.len() as i64 - 17 {
                            state.object_clip[index] = MngBox::read(state.object_clip[index], rest[0], &rest[1..]);
                        }
                    }
                }
                continue;
            }
            b"SAVE" => {
                for index in 1..MAX_OBJECTS {
                    if state.exists[index] {
                        state.frozen[index] = true;
                    }
                }
                continue;
            }
            b"DISC" | b"SEEK" => {
                if data.is_empty() || data.len() % 2 != 0 || kind == b"SEEK" {
                    for index in 1..MAX_OBJECTS {
                        state.discard_object(index);
                    }
                } else {
                    for pair in data.as_chunks::<2>().0 {
                        state.discard_object(be16(pair) as usize);
                    }
                }
                continue;
            }
            b"MOVE" => {
                if data.len() > 3 {
                    let first = be16(data) as usize;
                    let last = be16(&data[2..]) as usize;
                    let rest = &data[4..];
                    for index in first..=last {
                        if index >= MAX_OBJECTS {
                            continue;
                        }
                        if state.exists[index] && !state.frozen[index] && (4i64) < data.len() as i64 - 8 {
                            // `MngPair` holds `long`s.
                            let mut a = get_long(&rest[1..]);
                            let mut b = get_long(&rest[5..]);
                            if rest[0] != 0 {
                                a = a.wrapping_add(state.x_off[index] as i32);
                                b = b.wrapping_add(state.y_off[index] as i32);
                            }
                            state.x_off[index] = a as i64;
                            state.y_off[index] = b as i64;
                        }
                    }
                }
                continue;
            }
            b"LOOP" => {
                if data.len() > 4 {
                    let level = data[0] as usize;
                    state.loop_active[level] = true;
                    let mut iterations = get_long(&data[1..]) as i64;
                    if iterations <= 0 {
                        skipping_loop = level as i32;
                    } else {
                        // Magick.NET reads one scene.
                        iterations = iterations.min(1);
                        state.loop_jump[level] = blob.position;
                        state.loop_count[level] = iterations;
                    }
                }
                continue;
            }
            b"ENDL" => {
                if let Some(&level) = data.first() {
                    let level = level as usize;
                    if skipping_loop > 0 {
                        if skipping_loop == level as i32 {
                            skipping_loop = -1;
                            state.loop_active[level] = false;
                        }
                    } else if state.loop_active[level] {
                        state.loop_count[level] -= 1;
                        if state.loop_count[level] > 0 {
                            blob.position = state.loop_jump[level];
                        } else {
                            state.loop_active[level] = false;
                        }
                    }
                }
                continue;
            }
            b"CLON" => bail!("CLON is not implemented yet"),
            b"MAGN" => {
                read_magn(&mut state, data)?;
                continue;
            }
            b"PAST" => bail!("PAST is not implemented yet"),
            b"SHOW" => bail!("SHOW is not implemented yet"),
            // libpng is given the global sBIT, which IM never reads back.
            b"sBIT" => continue,
            b"pHYs" => continue,
            b"pHYg" => bail!("pHYg is not implemented."),
            b"BASI" => bail!("BASI is not implemented yet"),
            b"IHDR" | b"JHDR" => {}
            _ => continue,
        }

        // An IHDR or JHDR: the next image.
        state.exists[object_id] = true;
        if state.invisible[object_id] {
            skip_to_iend = true;
            continue;
        }
        if data.len() < 8 {
            bail!("ImproperImageHeader");
        }
        let image_width = get_long(data) as u32 as u64;
        let image_height = get_long(&data[4..]) as u32 as u64;

        if insert_layers && mng_type != 0 && first_mng_object {
            let clip = state.clip;
            let partial = clip.left > 0 || clip.top > 0 || image_width < state.mng_width || (clip.right as i64) < state.mng_width as i64 || image_height < state.mng_height || (clip.bottom as i64) < state.mng_height as i64;
            if partial {
                let whole = MngBox { left: 0, right: state.mng_width as i32, top: 0, bottom: state.mng_height as i32 };
                insert_background(&mut layers, state.mng_width, state.mng_height, whole, mng_background);
            }
        }
        if insert_layers && state.framing_mode == 3 && subframe_width != 0 && subframe_height != 0 && (simplicity == 0 || simplicity & 0x08 != 0) {
            insert_background(&mut layers, subframe_width, subframe_height, state.clip, mng_background);
        }
        first_mng_object = false;
        if layers.last().is_some_and(|layer| layer.has_pixel_queue) {
            layers.push(Layer::new());
        }
        let Some(layer) = layers.last_mut() else {
            bail!("no image");
        };
        if state.framing_mode == 1 || state.framing_mode == 3 {
            layer.delay = frame_delay;
            frame_delay = default_frame_delay;
        } else {
            layer.delay = 0;
        }
        layer.dispose = if state.framing_mode == 3 { Dispose::Background } else { Dispose::None };
        layer.page = Page { width: state.mng_width, height: state.mng_height, x: state.x_off[object_id], y: state.y_off[object_id] };

        // Back to the chunk's start for the PNG or JNG reader.
        let header_start = blob.position - data.len() - 12;
        let (chunks, end) = embedded_chunks(bytes, header_start)?;
        blob.position = end;

        let skipped = if kind == b"IHDR" { read_png_layer(bytes, &chunks, &mut state, layer, object_id, mng_type)? } else { read_jng_layer(bytes, &chunks, &mut state, layer, object_id, mng_type)? };
        if layer.cache.width == 0 || layer.cache.height == 0 {
            bail!("NegativeOrZeroImageSize");
        }
        if skipped {
            // The second scene with a delay: its pixels are skipped and the
            // reading ends. It stays in the list, but coalescing stops at
            // the scene before it.
            break;
        }

        if mng_type != 0 {
            magnify(&mut state, layer);
            crop_to_clip(&state, layer, object_id);
        }
    }

    finish(layers, &mut state, insert_layers, mng_background)
}

/// A background layer the size of the frame or subframe: on the current
/// image unless it already holds pixels. It never sets the pixel queue,
/// so the next image read overwrites it.
fn insert_background(layers: &mut Vec<Layer>, width: u64, height: u64, clip: MngBox, background: Background) {
    if layers.last().is_some_and(|layer| layer.has_pixel_queue) {
        layers.push(Layer::new());
    }
    let Some(layer) = layers.last_mut() else {
        return;
    };
    layer.delay = 0;
    layer.page = Page { width, height, x: clip.left as i64, y: clip.top as i64 };
    layer.background = background;
    layer.fill_background(width, height);
}

/// The MAGN chunk's factors for object 0.
fn read_magn(state: &mut State, data: &[u8]) -> Result<()> {
    let length = data.len();
    let magn_first = if length > 1 { be16(data) } else { 0 };
    let magn_last = if length > 3 { be16(&data[2..]) } else { magn_first };
    if magn_first != 0 || magn_last != 0 {
        bail!("MAGN is not implemented yet for nonzero objects");
    }
    let magn_methx = if length > 4 { data[4] as u16 } else { 0 };
    let mut magn_mx = if length > 6 { be16(&data[5..]) } else { 1 };
    if magn_mx == 0 {
        magn_mx = 1;
    }
    let mut magn_my = if length > 8 { be16(&data[7..]) } else { magn_mx };
    if magn_my == 0 {
        magn_my = 1;
    }
    let mut magn_ml = if length > 10 { be16(&data[9..]) } else { magn_mx };
    if magn_ml == 0 {
        magn_ml = 1;
    }
    let mut magn_mr = if length > 12 { be16(&data[11..]) } else { magn_mx };
    if magn_mr == 0 {
        magn_mr = 1;
    }
    let mut magn_mt = if length > 14 { be16(&data[13..]) } else { magn_my };
    if magn_mt == 0 {
        magn_mt = 1;
    }
    let mut magn_mb = if length > 16 { be16(&data[15..]) } else { magn_my };
    if magn_mb == 0 {
        magn_mb = 1;
    }
    let magn_methy = if length > 17 { data[17] as u16 } else { magn_methx };
    if magn_methx > 5 || magn_methy > 5 {
        bail!("Unknown MAGN method in MNG datastream");
    }
    state.magn_mb = magn_mb;
    state.magn_ml = magn_ml;
    state.magn_mr = magn_mr;
    state.magn_mt = magn_mt;
    state.magn_mx = magn_mx;
    state.magn_my = magn_my;
    state.magn_methx = magn_methx;
    state.magn_methy = magn_methy;
    Ok(())
}

/// Reads an embedded PNG into `layer` (`ReadOnePNGImage` in an MNG):
/// returns whether its pixels were skipped as a scene past the one read.
fn read_png_layer(bytes: &[u8], chunks: &[EmbeddedChunk], state: &mut State, layer: &mut Layer, object_id: usize, mng_type: u8) -> Result<bool> {
    state.image_found += 1;
    let (png, extras) = standalone_png(bytes, chunks, state)?;

    // The page: pre-data canvas chunks, then oFFs, then the object's y.
    for canvas in &extras.canvas_before {
        layer.page.width = canvas.width;
        layer.page.height = canvas.height;
        if let Some((x, y)) = canvas.offsets {
            layer.page.x = x;
            layer.page.y = y;
        }
    }
    if let Some((x, y)) = extras.offsets {
        layer.page.x = x;
        layer.page.y = y;
    }
    if state.global_bkgd.is_some() && extras.bkgd.is_none()
        && let Some([red, green, blue]) = state.global_bkgd
    {
        layer.background = Background { red: red as f64, green: green as f64, blue: blue as f64, alpha: 0.0, alpha_trait: false };
    }
    if let Some(background) = png_background(&extras) {
        layer.background = background;
    }
    state.image_box = MngBox { left: 0, right: extras.width as i32, top: 0, bottom: extras.height as i32 };
    if mng_type == 0 {
        state.mng_width = extras.width as u64;
        state.mng_height = extras.height as u64;
        state.frame = state.image_box;
        state.clip = state.image_box;
    } else {
        layer.page.y = state.y_off[object_id];
    }

    if layer.delay != 0 {
        state.scenes_found += 1;
    }
    if state.scenes_found > 1 {
        // Only the header is read.
        layer.cache = PixelCache::new(extras.width as usize, extras.height as usize, Vec::new(), Vec::new(), CacheColorspace::SRgb, DEFAULT_GAMMA);
        return Ok(true);
    }
    let mut cache = read_png_bytes_with(&png, extras.filter_64)?;
    for canvas in &extras.canvas_after {
        layer.page.width = canvas.width;
        layer.page.height = canvas.height;
        if let Some((x, y)) = canvas.offsets {
            layer.page.x = x;
            layer.page.y = y;
        }
    }
    if cache.image_type == ImageType::Undefined {
        cache.image_type = ImageType::TrueColor;
    }
    layer.cache = cache;
    layer.has_pixel_queue = true;
    Ok(false)
}

/// Reads an embedded JNG into `layer` (`ReadOneJNGImage` in an MNG).
fn read_jng_layer(bytes: &[u8], chunks: &[EmbeddedChunk], state: &mut State, layer: &mut Layer, object_id: usize, mng_type: u8) -> Result<bool> {
    let mut jng = JNG_SIGNATURE.to_vec();
    let mut offsets = None;
    for chunk in chunks {
        let data = chunk.data(bytes);
        if &chunk.kind == b"oFFs" && chunk.length > 8 {
            let mut x = get_long(data) as i64;
            let mut y = get_long(&data[4..]) as i64;
            if data[8] != 0 {
                x /= 10000;
                y /= 10000;
            }
            offsets = Some((x, y));
        }
        jng.extend_from_slice(chunk.whole(bytes));
    }
    let cache = jng::decode(&jng)?;
    if mng_type == 0 {
        state.mng_width = cache.width as u64;
        state.mng_height = cache.height as u64;
    }
    if let Some((x, y)) = offsets {
        layer.page.x = x;
        layer.page.y = y;
    }
    if layer.page.x == 0 && layer.page.y == 0 {
        layer.page.x = state.x_off[object_id];
        layer.page.y = state.y_off[object_id];
    } else {
        layer.page.y = state.y_off[object_id];
    }
    state.image_found += 1;
    layer.cache = cache;
    layer.has_pixel_queue = true;
    Ok(false)
}

/// Channel offsets of a cache: red, green, blue (all the grey channel for
/// a grey image) and alpha.
struct Offsets {
    red: usize,
    green: usize,
    blue: usize,
    alpha: Option<usize>,
    stride: usize,
}

fn offsets(cache: &PixelCache) -> Offsets {
    let find = |wanted: PixelChannel| cache.channels.iter().position(|&channel| channel == wanted);
    let gray = find(PixelChannel::Gray);
    let red = gray.or_else(|| find(PixelChannel::Red)).unwrap_or(0);
    let green = gray.or_else(|| find(PixelChannel::Green)).unwrap_or(0);
    let blue = gray.or_else(|| find(PixelChannel::Blue)).unwrap_or(0);
    Offsets { red, green, blue, alpha: find(PixelChannel::Alpha), stride: cache.channels.len() }
}

/// MAGN's magnification of an image read (`ReadOneMNGImage`). The
/// magnified image is a new one (`AcquireNextImage`): sRGB without alpha,
/// a white background and the default delay and page, which replaces the
/// image read. Its arithmetic is in floats (Q16 HDRI quanta).
fn magnify(state: &mut State, layer: &mut Layer) {
    let methods_valid = (1..=5).contains(&state.magn_methx) && (1..=5).contains(&state.magn_methy);
    if !methods_valid {
        return;
    }
    let columns = layer.cache.width;
    let rows = layer.cache.height;
    if columns == 1 {
        state.magn_methx = 1;
    }
    if rows == 1 {
        state.magn_methy = 1;
    }
    let mx = state.magn_mx as usize;
    let my = state.magn_my as usize;
    let ml = state.magn_ml as usize;
    let mr = state.magn_mr as usize;
    let mt = state.magn_mt as usize;
    let mb = state.magn_mb as usize;
    let magnified_width = if state.magn_methx == 1 {
        let mut width = ml;
        if columns > 1 {
            width += mr;
        }
        if columns > 2 {
            width += (columns - 2) * mx;
        }
        width
    } else {
        let mut width = columns;
        if columns > 1 {
            width += ml - 1;
        }
        if columns > 2 {
            width += mr - 1;
        }
        if columns > 3 {
            width += (columns - 3) * (mx - 1);
        }
        width
    };
    let magnified_height = if state.magn_methy == 1 {
        let mut height = mt;
        if rows > 1 {
            height += mb;
        }
        if rows > 2 {
            height += (rows - 2) * my;
        }
        height
    } else {
        let mut height = rows;
        if rows > 1 {
            height += mt - 1;
        }
        if rows > 2 {
            height += mb - 1;
        }
        if rows > 3 {
            height += (rows - 3) * (my - 1);
        }
        height
    };
    if magnified_height <= rows && magnified_width <= columns {
        return;
    }

    let mut magn_methx = state.magn_methx;
    let mut magn_methy = state.magn_methy;
    if !layer.cache.alpha_trait {
        if magn_methx == 4 {
            magn_methx = 2;
        }
        if magn_methx == 5 {
            magn_methx = 3;
        }
        if magn_methy == 4 {
            magn_methy = 2;
        }
        if magn_methy == 5 {
            magn_methy = 3;
        }
    }

    // The row buffers: `memcpy(next, row, length)` with `length` counting
    // quanta, so only that many bytes of each row arrive and the rest of
    // the buffer keeps its (fresh, zero) memory.
    let off = offsets(&layer.cache);
    let stride = off.stride;
    let quanta = stride * columns;
    let row_buffer = |y: usize| -> Vec<f32> {
        let row = &layer.cache.data[y * quanta..(y + 1) * quanta];
        let row_bytes: Vec<u8> = row.iter().flat_map(|value| value.to_le_bytes()).collect();
        let mut buffer_bytes = vec![0u8; quanta * 4];
        buffer_bytes[..quanta].copy_from_slice(&row_bytes[..quanta]);
        buffer_bytes.as_chunks::<4>().0.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect()
    };
    let rgb = |buffer: &[f32], x: usize| -> [f32; 3] {
        let base = x * stride;
        [buffer[base + off.red], buffer[base + off.green], buffer[base + off.blue]]
    };
    let white = QUANTUM_RANGE as f32;
    let mut large = vec![white; magnified_width * magnified_height * 3];

    // Methods 2 and 4 interpolate, `(QM) (((ssize_t) (2*i*(n-p+m)))/
    // ((ssize_t) (m*2))+p)` down rows and `(QM) ((2*i*(n-p)+m)/
    // ((ssize_t) (m*2))+p)` along them; 1 replicates and 3 and 5 take the
    // previous sample (their "nearest" test is always true).
    let interpolate_rows = |p: f32, n: f32, i: usize, m: usize| -> f32 {
        let numerator = ((2 * i) as f32 * (n - p + m as f32)) as i64;
        (numerator / (m as i64 * 2)) as f32 + p
    };
    let interpolate_columns = |p: f32, n: f32, i: usize, m: usize| -> f32 {
        ((2 * i) as f32 * (n - p) + m as f32) / (m * 2) as f32 + p
    };

    // Rows, into the right side of the large image.
    let mut yy = 0usize;
    let mut next = row_buffer(0);
    for y in 0..rows {
        let m = if y == 0 {
            mt
        } else if (magn_methy > 1 && y == rows - 2) || (magn_methy <= 1 && y == rows - 1) {
            mb
        } else if magn_methy > 1 && y == rows - 1 {
            1
        } else {
            my
        };
        let previous = next.clone();
        if y < rows - 1 {
            next = row_buffer(y + 1);
        }
        for i in 0..m {
            if yy >= magnified_height {
                break;
            }
            let row_start = (yy * magnified_width + (magnified_width - columns)) * 3;
            for x in 0..columns {
                let p = rgb(&previous, x);
                let n = rgb(&next, x);
                let q = &mut large[row_start + x * 3..row_start + x * 3 + 3];
                for channel in 0..3 {
                    q[channel] = if (magn_methy == 2 || magn_methy == 4) && i != 0 { interpolate_rows(p[channel], n[channel], i, m) } else { p[channel] };
                }
            }
            yy += 1;
        }
    }

    // Columns, in place: the source pointer never moves past the first
    // sample of the right side, while the next one does.
    let length = columns;
    for y in 0..magnified_height {
        let row = &mut large[y * magnified_width * 3..(y + 1) * magnified_width * 3];
        let pixels = magnified_width - length;
        let mut q = 0usize;
        for x in magnified_width - length..magnified_width {
            let n = x + 1;
            let m = if x == magnified_width - length {
                ml
            } else if (magn_methx > 1 && x == magnified_width - 2) || (magn_methx <= 1 && x == magnified_width - 1) {
                mr
            } else if magn_methx > 1 && x == magnified_width - 1 {
                1
            } else {
                mx
            };
            for i in 0..m {
                for channel in 0..3 {
                    let p = row[pixels * 3 + channel];
                    let value = if (magn_methx == 2 || magn_methx == 4) && i != 0 {
                        let n_value = row.get(n * 3 + channel).copied().unwrap_or(0.0);
                        interpolate_columns(p, n_value, i, m)
                    } else {
                        p
                    };
                    row[q * 3 + channel] = value;
                }
                q += 1;
            }
        }
    }

    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    let mut magnified = Layer::new();
    magnified.cache = PixelCache::new(magnified_width, magnified_height, channels, large, CacheColorspace::SRgb, DEFAULT_GAMMA);
    magnified.has_pixel_queue = true;
    *layer = magnified;
}

/// The image without its colormap index (`SetImageStorageClass(DirectClass)`).
fn to_direct(cache: &PixelCache) -> PixelCache {
    let Some(index) = cache.channels.iter().position(|&channel| channel == PixelChannel::Index) else {
        return cache.clone();
    };
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * (stride - 1));
    for pixel in cache.data.chunks_exact(stride) {
        for (channel, &value) in pixel.iter().enumerate() {
            if channel != index {
                data.push(value);
            }
        }
    }
    let mut channels = cache.channels.clone();
    channels.remove(index);
    let mut direct = PixelCache::new(cache.width, cache.height, channels, data, cache.colorspace, cache.gamma);
    direct.alpha_trait = cache.alpha_trait;
    direct.image_type = cache.image_type;
    direct
}

/// The clipping of an image read to the clip, frame and object boxes. IM
/// keeps only the crop's size: the image becomes the top-left region of
/// that size. With nothing left, the image is one background pixel.
fn crop_to_clip(state: &State, layer: &mut Layer, object_id: usize) {
    let x_off = state.x_off[object_id];
    let y_off = state.y_off[object_id];
    let image_box = state.image_box;
    let mut crop_box = MngBox {
        left: image_box.left.wrapping_add(x_off as i32),
        right: image_box.right.wrapping_add(x_off as i32),
        top: image_box.top.wrapping_add(y_off as i32),
        bottom: image_box.bottom.wrapping_add(y_off as i32),
    };
    crop_box = crop_box.minimum(state.clip);
    crop_box = crop_box.minimum(state.frame);
    crop_box = crop_box.minimum(state.object_clip[object_id]);
    let unchanged = crop_box.left as i64 == image_box.left as i64 + x_off
        && crop_box.right as i64 == image_box.right as i64 + x_off
        && crop_box.top as i64 == image_box.top as i64 + y_off
        && crop_box.bottom as i64 == image_box.bottom as i64 + y_off;
    if unchanged {
        return;
    }
    if crop_box.left < crop_box.right && crop_box.top < crop_box.bottom {
        let crop_x = crop_box.left as i64 - x_off;
        let crop_y = crop_box.top as i64 - y_off;
        let crop_width = (crop_box.right as i64 - crop_box.left as i64) as u64;
        let crop_height = (crop_box.bottom as i64 - crop_box.top as i64) as u64;
        let columns = layer.cache.width;
        let rows = layer.cache.height;
        layer.page = Page { width: columns as u64, height: rows as u64, x: 0, y: 0 };
        if let Some((new_columns, new_rows)) = crop_size(columns, rows, crop_x, crop_y, crop_width, crop_height) {
            layer.cache = top_left_region(&layer.cache, new_columns, new_rows);
            layer.page = Page { width: new_columns as u64, height: new_rows as u64, x: crop_box.left as i64, y: crop_box.top as i64 };
        }
    } else {
        let background = layer.background;
        let mut cache = top_left_region(&to_direct(&layer.cache), 1, 1);
        let off = offsets(&cache);
        cache.data[off.red] = background.red as f32;
        cache.data[off.green] = background.green as f32;
        cache.data[off.blue] = background.blue as f32;
        if let Some(alpha) = off.alpha {
            cache.data[alpha] = if background.alpha_trait { background.alpha as f32 } else { QUANTUM_RANGE as f32 };
        }
        layer.cache = cache;
        layer.page = Page { width: 1, height: 1, x: 0, y: 0 };
    }
}

/// The size `CropImage` gives an image with page `columns`x`rows`+0+0:
/// one pixel when the region misses it, `None` when it is empty.
fn crop_size(columns: usize, rows: usize, x: i64, y: i64, width: u64, height: u64) -> Option<(usize, usize)> {
    let to_size = |value: f64| if value < 0.0 { 0 } else { value as u64 };
    if -(x as f64) >= width as f64 || -(y as f64) >= height as f64 || x as f64 > columns as f64 || y as f64 > rows as f64 {
        return Some((1, 1));
    }
    let (mut page_width, page_x) = if x < 0 { (to_size(width as f64 + x as f64), 0) } else { (to_size(width as f64 + x as f64), x) };
    let (mut page_height, page_y) = if y < 0 { (to_size(height as f64 + y as f64), 0) } else { (to_size(height as f64 + y as f64), y) };
    if page_x + page_width as i64 > columns as i64 {
        page_width = (columns as i64 - page_x) as u64;
    }
    if page_width > width {
        page_width = width;
    }
    if page_y + page_height as i64 > rows as i64 {
        page_height = to_size(rows as f64 - page_y as f64);
    }
    if page_height > height {
        page_height = height;
    }
    if page_width == 0 || page_height == 0 {
        return None;
    }
    Some((page_width as usize, page_height as usize))
}

/// The pixels a resized cache keeps (`ClonePixelCacheRepository`): rows
/// and columns from the top left, zeros beyond the old size.
fn top_left_region(cache: &PixelCache, columns: usize, rows: usize) -> PixelCache {
    let stride = cache.channels.len();
    let mut data = vec![0f32; columns * rows * stride];
    for y in 0..rows.min(cache.height) {
        let count = columns.min(cache.width) * stride;
        let from = y * cache.width * stride;
        let to = y * columns * stride;
        data[to..to + count].copy_from_slice(&cache.data[from..from + count]);
    }
    PixelCache { width: columns, height: rows, data, ..cache.clone() }
}

/// After the chunk loop: a background layer when nothing was read, the
/// list checks, then the first frame Magick.NET takes.
fn finish(mut layers: Vec<Layer>, state: &mut State, insert_layers: bool, mng_background: Background) -> Result<PixelCache> {
    if insert_layers && state.image_found == 0 && state.mng_width != 0 && state.mng_height != 0 {
        if layers.last().is_some_and(|layer| layer.has_pixel_queue) {
            layers.push(Layer::new());
        }
        if let Some(layer) = layers.last_mut() {
            layer.page = Page { width: state.mng_width, height: state.mng_height, x: 0, y: 0 };
            layer.background = mng_background;
            layer.fill_background(state.mng_width, state.mng_height);
        }
        state.image_found += 1;
    }
    if layers.len() as i64 - 1 > 10 * state.image_found {
        bail!("Linked list is corrupted, beginning of list not found");
    }
    if state.ticks_per_second != 0 && state.image_found > 1 && layers.len() == 1 {
        bail!("image->next for first image is NULL but shouldn't be.");
    }
    if state.image_found == 0 {
        bail!("No visible images in file");
    }
    if !insert_layers || layers.len() == 1 {
        return Ok(layers.swap_remove(0).cache);
    }
    // The coalesced frames up to the first with a delay (later ones merge
    // into it), or the last.
    let target = layers.iter().position(|layer| layer.delay != 0).unwrap_or(layers.len() - 1);
    coalesce(&layers[..=target])
}

/// A canvas the frames are drawn on.
#[derive(Clone)]
struct Canvas {
    cache: PixelCache,
    page: Page,
}

/// `CoalesceImages` up to the last of `layers`.
fn coalesce(layers: &[Layer]) -> Result<PixelCache> {
    let first = &layers[0];
    let mut bounds = first.page;
    if bounds.width == 0 {
        bounds.width = first.cache.width as u64;
        if bounds.x > 0 {
            bounds.width += bounds.x as u64;
        }
    }
    if bounds.height == 0 {
        bounds.height = first.cache.height as u64;
        if bounds.y > 0 {
            bounds.height += bounds.y as u64;
        }
    }
    bounds.x = 0;
    bounds.y = 0;

    // A clone of the first image at the bounds' size, filled with its
    // background made transparent, then keeping only the first image's
    // alpha trait.
    let mut canvas = blank_like(&first.cache, bounds.width as usize, bounds.height as usize, first.background);
    if !first.cache.alpha_trait {
        canvas = drop_alpha(&canvas);
    }
    let mut dispose_image = Canvas { cache: canvas.clone(), page: bounds };
    let mut current = Canvas { cache: canvas, page: bounds };
    composite(&mut current.cache, &first.cache, first.page.x, first.page.y, false)?;
    current.cache.gamma = first.cache.gamma;

    for index in 1..layers.len() {
        let next = &layers[index];
        let previous = &layers[index - 1];
        let mut area = previous.page;
        area.width = previous.cache.width as u64;
        area.height = previous.cache.height as u64;
        let mut area_x = area.x;
        let mut area_width = area.width as i64;
        if area_x < 0 {
            area_width += area_x;
            area_x = 0;
        }
        if area_x + area_width > current.cache.width as i64 {
            area_width = current.cache.width as i64 - area_x;
        }
        let mut area_y = area.y;
        let mut area_height = area.height as i64;
        if area_y < 0 {
            area_height += area_y;
            area_y = 0;
        }
        if area_y + area_height > current.cache.height as i64 {
            area_height = current.cache.height as i64 - area_y;
        }
        if previous.dispose != Dispose::Previous {
            dispose_image = current.clone();
        }
        if previous.dispose == Dispose::Background {
            clear_bounds(&mut dispose_image.cache, area_x, area_y, area_width, area_height);
        }
        let mut frame = dispose_image.clone();
        let over = next.cache.alpha_trait;
        composite(&mut frame.cache, &next.cache, next.page.x, next.page.y, over)?;
        frame.cache.gamma = next.cache.gamma;
        frame.page = current.page;
        current = frame;
    }
    let mut result = current.cache;
    result.image_type = ImageType::Undefined;
    Ok(result)
}

/// A new cache like `like` (its channels without the index, alpha added
/// for the transparent background) filled with `background`.
fn blank_like(like: &PixelCache, width: usize, height: usize, background: Background) -> PixelCache {
    let mut colorspace = like.colorspace;
    let mut channels: Vec<PixelChannel> = like.channels.iter().copied().filter(|&channel| channel != PixelChannel::Index && channel != PixelChannel::Alpha).collect();
    if colorspace.is_gray() && !background.is_gray() {
        // `ConformPixelInfo` turns the canvas to sRGB for a colour background.
        colorspace = CacheColorspace::SRgb;
        channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    }
    channels.push(PixelChannel::Alpha);
    let gray = channels.contains(&PixelChannel::Gray);
    let mut data = Vec::with_capacity(width * height * channels.len());
    for _ in 0..width * height {
        if gray {
            // The red, green and blue writes all land on the grey channel.
            data.push(background.blue as f32);
        } else {
            data.extend([background.red as f32, background.green as f32, background.blue as f32]);
        }
        data.push(0.0);
    }
    let mut cache = PixelCache::new(width, height, channels, data, colorspace, like.gamma);
    cache.alpha_trait = true;
    cache.image_type = like.image_type;
    cache
}

fn drop_alpha(cache: &PixelCache) -> PixelCache {
    let Some(alpha) = cache.channels.iter().position(|&channel| channel == PixelChannel::Alpha) else {
        return cache.clone();
    };
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * (stride - 1));
    for pixel in cache.data.chunks_exact(stride) {
        for (channel, &value) in pixel.iter().enumerate() {
            if channel != alpha {
                data.push(value);
            }
        }
    }
    let mut channels = cache.channels.clone();
    channels.remove(alpha);
    let mut result = PixelCache::new(cache.width, cache.height, channels, data, cache.colorspace, cache.gamma);
    result.alpha_trait = false;
    result.image_type = cache.image_type;
    result
}

/// `ClearBounds`: the area made transparent (an opaque alpha channel added
/// first when there is none).
fn clear_bounds(cache: &mut PixelCache, x: i64, y: i64, width: i64, height: i64) {
    if x < 0 {
        return;
    }
    if !cache.alpha_trait {
        let stride = cache.channels.len();
        let mut data = Vec::with_capacity(cache.width * cache.height * (stride + 1));
        for pixel in cache.data.chunks_exact(stride) {
            data.extend_from_slice(pixel);
            data.push(QUANTUM_RANGE as f32);
        }
        cache.channels.push(PixelChannel::Alpha);
        cache.data = data;
        cache.alpha_trait = true;
    }
    let off = offsets(cache);
    let Some(alpha) = off.alpha else {
        return;
    };
    for row in y..y + height.max(0) {
        if row < 0 || row >= cache.height as i64 {
            continue;
        }
        for column in x..x + width.max(0) {
            if column >= cache.width as i64 {
                break;
            }
            let index = (row as usize * cache.width + column as usize) * off.stride + alpha;
            cache.data[index] = 0.0;
        }
    }
}

/// The source in the canvas's colourspace (`SetImageColorspace` without a
/// transform): the cache's channels are remapped, and a grey cache maps
/// red, green and blue to its grey.
fn conform_source(canvas: &PixelCache, source: &PixelCache) -> PixelCache {
    let source = to_direct(source);
    let canvas_gray = canvas.colorspace.is_gray();
    let source_gray = source.channels.contains(&PixelChannel::Gray);
    if canvas_gray == source_gray {
        return source;
    }
    let off = offsets(&source);
    let mut channels = if canvas_gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if off.alpha.is_some() {
        channels.push(PixelChannel::Alpha);
    }
    let mut data = Vec::with_capacity(source.width * source.height * channels.len());
    for pixel in source.data.chunks_exact(off.stride) {
        if canvas_gray {
            data.push(pixel[off.red]);
        } else {
            data.extend([pixel[off.red], pixel[off.green], pixel[off.blue]]);
        }
        if let Some(alpha) = off.alpha {
            data.push(pixel[alpha]);
        }
    }
    let mut conformed = PixelCache::new(source.width, source.height, channels, data, canvas.colorspace, source.gamma);
    conformed.alpha_trait = source.alpha_trait;
    conformed
}

/// `ClampPixel`.
fn clamp_pixel(value: f64) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value >= QUANTUM_RANGE {
        return QUANTUM_RANGE as f32;
    }
    value as f32
}

/// `MagickSafeReciprocal`.
fn safe_reciprocal(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    if sign * value >= 1.0e-12 {
        return 1.0 / value;
    }
    sign / 1.0e-12
}

/// `CompositeImage` with clipping to the source: `Over` (when the source
/// has alpha) or `Copy`, whose fast path copies the samples when the
/// source lies inside the canvas and otherwise premultiplies them.
fn composite(canvas: &mut PixelCache, source: &PixelCache, x_offset: i64, y_offset: i64, over: bool) -> Result<()> {
    let source = conform_source(canvas, source);
    let canvas_off = offsets(canvas);
    let source_off = offsets(&source);
    let gray = canvas.channels.contains(&PixelChannel::Gray);
    let colour_channels: Vec<(usize, usize)> = if gray { vec![(canvas_off.red, source_off.red)] } else { vec![(canvas_off.red, source_off.red), (canvas_off.green, source_off.green), (canvas_off.blue, source_off.blue)] };
    let columns = canvas.width as i64;
    let rows = canvas.height as i64;
    let source_columns = source.width as i64;
    let source_rows = source.height as i64;
    let inside = x_offset >= 0 && y_offset >= 0 && x_offset + source_columns <= columns && y_offset + source_rows <= rows;
    let copy_fast = !over && inside;
    for y in 0..rows {
        if y < y_offset || y - y_offset >= source_rows {
            continue;
        }
        let source_y = (y - y_offset) as usize;
        for x in 0..columns {
            if x < x_offset {
                continue;
            }
            if x - x_offset >= source_columns {
                break;
            }
            let source_x = (x - x_offset) as usize;
            let p = (source_y * source.width + source_x) * source_off.stride;
            let q = (y as usize * canvas.width + x as usize) * canvas_off.stride;
            let source_alpha = source_off.alpha.map_or(QUANTUM_RANGE as f32, |alpha| source.data[p + alpha]);
            if copy_fast {
                for &(to, from) in &colour_channels {
                    canvas.data[q + to] = source.data[p + from];
                }
                if let Some(alpha) = canvas_off.alpha {
                    canvas.data[q + alpha] = source_alpha;
                }
                continue;
            }
            let sa = QUANTUM_SCALE * source_alpha as f64;
            let canvas_alpha = canvas_off.alpha.map_or(QUANTUM_RANGE as f32, |alpha| canvas.data[q + alpha]);
            let da = QUANTUM_SCALE * canvas_alpha as f64;
            if over {
                let alpha = sa + da - sa * da;
                let gamma = safe_reciprocal(alpha);
                for &(to, from) in &colour_channels {
                    let sc = source.data[p + from] as f64;
                    let dc = canvas.data[q + to] as f64;
                    let sca = QUANTUM_SCALE * sa * sc;
                    let dca = QUANTUM_SCALE * da * dc;
                    canvas.data[q + to] = clamp_pixel(QUANTUM_RANGE * gamma * (sca + dca * (1.0 - sa)));
                }
                if let Some(alpha_channel) = canvas_off.alpha {
                    canvas.data[q + alpha_channel] = clamp_pixel(QUANTUM_RANGE * alpha);
                }
            } else {
                for &(to, from) in &colour_channels {
                    let sc = source.data[p + from] as f64;
                    let sca = QUANTUM_SCALE * sa * sc;
                    canvas.data[q + to] = clamp_pixel(QUANTUM_RANGE * sca);
                }
                if let Some(alpha_channel) = canvas_off.alpha {
                    canvas.data[q + alpha_channel] = clamp_pixel(QUANTUM_RANGE * sa);
                }
            }
        }
    }
    Ok(())
}
