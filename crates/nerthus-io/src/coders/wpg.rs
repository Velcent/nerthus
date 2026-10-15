//! WordPerfect Graphics as ImageMagick reads them (`coders/wpg.c`): the
//! records of a level 1 or level 2 file, the first bitmap of which is the
//! image — uncompressed or run-length rows of 1-, 2-, 4-, 8- or 24-bit
//! pixels, coloured by the file's palette, IM's default level 1 palette or
//! a grey ramp — flopped, flipped or turned as its record says. IM parses
//! the whole file, so a later broken record still fails the read, and so
//! does any embedded PostScript (IM's check for its format always fails).

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

/// `WPG1_Palette`, as written: IM's record is declared red, blue, green,
/// so the second number of each is the entry's blue.
const LEVEL1_PALETTE: [[u8; 3]; 256] = [
    [0, 0, 0], [0, 0, 168], [0, 168, 0], [0, 168, 168], [168, 0, 0], [168, 0, 168], [168, 84, 0], [168, 168, 168],
    [84, 84, 84], [84, 84, 252], [84, 252, 84], [84, 252, 252], [252, 84, 84], [252, 84, 252], [252, 252, 84], [252, 252, 252],
    [0, 0, 0], [20, 20, 20], [32, 32, 32], [44, 44, 44], [56, 56, 56], [68, 68, 68], [80, 80, 80], [96, 96, 96],
    [112, 112, 112], [128, 128, 128], [144, 144, 144], [160, 160, 160], [180, 180, 180], [200, 200, 200], [224, 224, 224], [252, 252, 252],
    [0, 0, 252], [64, 0, 252], [124, 0, 252], [188, 0, 252], [252, 0, 252], [252, 0, 188], [252, 0, 124], [252, 0, 64],
    [252, 0, 0], [252, 64, 0], [252, 124, 0], [252, 188, 0], [252, 252, 0], [188, 252, 0], [124, 252, 0], [64, 252, 0],
    [0, 252, 0], [0, 252, 64], [0, 252, 124], [0, 252, 188], [0, 252, 252], [0, 188, 252], [0, 124, 252], [0, 64, 252],
    [124, 124, 252], [156, 124, 252], [188, 124, 252], [220, 124, 252], [252, 124, 252], [252, 124, 220], [252, 124, 188], [252, 124, 156],
    [252, 124, 124], [252, 156, 124], [252, 188, 124], [252, 220, 124], [252, 252, 124], [220, 252, 124], [188, 252, 124], [156, 252, 124],
    [124, 252, 124], [124, 252, 156], [124, 252, 188], [124, 252, 220], [124, 252, 252], [124, 220, 252], [124, 188, 252], [124, 156, 252],
    [180, 180, 252], [196, 180, 252], [216, 180, 252], [232, 180, 252], [252, 180, 252], [252, 180, 232], [252, 180, 216], [252, 180, 196],
    [252, 180, 180], [252, 196, 180], [252, 216, 180], [252, 232, 180], [252, 252, 180], [232, 252, 180], [216, 252, 180], [196, 252, 180],
    [180, 220, 180], [180, 252, 196], [180, 252, 216], [180, 252, 232], [180, 252, 252], [180, 232, 252], [180, 216, 252], [180, 196, 252],
    [0, 0, 112], [28, 0, 112], [56, 0, 112], [84, 0, 112], [112, 0, 112], [112, 0, 84], [112, 0, 56], [112, 0, 28],
    [112, 0, 0], [112, 28, 0], [112, 56, 0], [112, 84, 0], [112, 112, 0], [84, 112, 0], [56, 112, 0], [28, 112, 0],
    [0, 112, 0], [0, 112, 28], [0, 112, 56], [0, 112, 84], [0, 112, 112], [0, 84, 112], [0, 56, 112], [0, 28, 112],
    [56, 56, 112], [68, 56, 112], [84, 56, 112], [96, 56, 112], [112, 56, 112], [112, 56, 96], [112, 56, 84], [112, 56, 68],
    [112, 56, 56], [112, 68, 56], [112, 84, 56], [112, 96, 56], [112, 112, 56], [96, 112, 56], [84, 112, 56], [68, 112, 56],
    [56, 112, 56], [56, 112, 69], [56, 112, 84], [56, 112, 96], [56, 112, 112], [56, 96, 112], [56, 84, 112], [56, 68, 112],
    [80, 80, 112], [88, 80, 112], [96, 80, 112], [104, 80, 112], [112, 80, 112], [112, 80, 104], [112, 80, 96], [112, 80, 88],
    [112, 80, 80], [112, 88, 80], [112, 96, 80], [112, 104, 80], [112, 112, 80], [104, 112, 80], [96, 112, 80], [88, 112, 80],
    [80, 112, 80], [80, 112, 88], [80, 112, 96], [80, 112, 104], [80, 112, 112], [80, 114, 112], [80, 96, 112], [80, 88, 112],
    [0, 0, 64], [16, 0, 64], [32, 0, 64], [48, 0, 64], [64, 0, 64], [64, 0, 48], [64, 0, 32], [64, 0, 16],
    [64, 0, 0], [64, 16, 0], [64, 32, 0], [64, 48, 0], [64, 64, 0], [48, 64, 0], [32, 64, 0], [16, 64, 0],
    [0, 64, 0], [0, 64, 16], [0, 64, 32], [0, 64, 48], [0, 64, 64], [0, 48, 64], [0, 32, 64], [0, 16, 64],
    [32, 32, 64], [40, 32, 64], [48, 32, 64], [56, 32, 64], [64, 32, 64], [64, 32, 56], [64, 32, 48], [64, 32, 40],
    [64, 32, 32], [64, 40, 32], [64, 48, 32], [64, 56, 32], [64, 64, 32], [56, 64, 32], [48, 64, 32], [40, 64, 32],
    [32, 64, 32], [32, 64, 40], [32, 64, 48], [32, 64, 56], [32, 64, 64], [32, 56, 64], [32, 48, 64], [32, 40, 64],
    [44, 44, 64], [48, 44, 64], [52, 44, 64], [60, 44, 64], [64, 44, 64], [64, 44, 60], [64, 44, 52], [64, 44, 48],
    [64, 44, 44], [64, 48, 44], [64, 52, 44], [64, 60, 44], [64, 64, 44], [60, 64, 44], [52, 64, 44], [48, 64, 44],
    [44, 64, 44], [44, 64, 48], [44, 64, 52], [44, 64, 60], [44, 64, 64], [44, 60, 64], [44, 55, 64], [44, 48, 64],
    [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0],
];

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// The image a bitmap record fills (IM's current image in the list).
struct Frame {
    columns: usize,
    rows: usize,
    /// `PseudoClass`: the pixels carry a colormap index after blue.
    pseudo: bool,
    /// `image->colors`, which may be fewer than the colormap's entries.
    colors: usize,
    colormap: Vec<[f64; 4]>,
    data: Vec<f32>,
    /// `image->page`: width, height and offsets.
    page: (u64, u64, i64, i64),
}

impl Frame {
    fn empty() -> Frame {
        Frame { columns: 0, rows: 0, pseudo: false, colors: 0, colormap: Vec::new(), data: Vec::new(), page: (0, 0, 0, 0) }
    }

    fn stride(&self) -> usize {
        3 + usize::from(self.pseudo)
    }

    /// `SetImageExtent` and `ResetImagePixels`.
    fn reset(&mut self) {
        self.data = vec![0.0; self.columns * self.rows * self.stride()];
    }

    /// `AcquireImageColormap`, which also makes the image `PseudoClass`.
    fn acquire_colormap(&mut self, colors: u64) -> Result<()> {
        let colormap = usize::try_from(colors).ok().and_then(colormap::acquire).context("MemoryAllocationFailed")?;
        self.colors = colormap.len();
        self.colormap = colormap;
        if !self.pseudo {
            self.pseudo = true;
            self.reset();
        }
        Ok(())
    }

    /// `InsertRow`: one row of `bits`-bit pixels; false for a depth it
    /// doesn't know. A row past the image is IM's `PixelsAreNotAuthentic`
    /// error and an index past the colours `InvalidColormapIndex`.
    fn insert_row(&mut self, row: &[u8], y: usize, bits: u32) -> Result<bool> {
        if y >= self.rows {
            bail!("PixelsAreNotAuthentic");
        }
        let stride = self.stride();
        let start = y * self.columns * stride;
        for x in 0..self.columns {
            let index = match bits {
                1 => (row[x / 8] >> (7 - x % 8)) & 1,
                2 => (row[x / 4] >> (6 - 2 * (x % 4))) & 3,
                4 => (row[x / 2] >> if x % 2 == 0 { 4 } else { 0 }) & 0x0f,
                8 => row[x],
                24 => {
                    let pixel = &mut self.data[start + x * stride..start + x * stride + 3];
                    pixel[0] = char_quantum(row[3 * x]);
                    pixel[1] = char_quantum(row[3 * x + 1]);
                    pixel[2] = char_quantum(row[3 * x + 2]);
                    continue;
                }
                _ => return Ok(false),
            } as usize;
            if bits != 1 && index >= self.colors {
                bail!("InvalidColormapIndex");
            }
            let pixel = &mut self.data[start + x * stride..start + (x + 1) * stride];
            pixel[3] = index as f32;
            if index < self.colors {
                for (channel, value) in pixel[..3].iter_mut().enumerate() {
                    *value = self.colormap[index][channel] as f32;
                }
            }
        }
        Ok(true)
    }

    /// `FlopImage`, which also moves the page offset across a set width.
    fn flop(&mut self) {
        let (page_width, _, page_x, _) = self.page;
        if page_width != 0 {
            self.page.2 = page_width as i64 - self.columns as i64 - page_x;
        }
        let stride = self.stride();
        for row in self.data.chunks_exact_mut(self.columns * stride) {
            for x in 0..self.columns / 2 {
                for channel in 0..stride {
                    row.swap(x * stride + channel, (self.columns - 1 - x) * stride + channel);
                }
            }
        }
    }

    /// `FlipImage`, which also moves the page offset across a set height.
    fn flip(&mut self) {
        let (_, page_height, _, page_y) = self.page;
        if page_height != 0 {
            self.page.3 = page_height as i64 - self.rows as i64 - page_y;
        }
        let row_length = self.columns * self.stride();
        for y in 0..self.rows / 2 {
            let (top, bottom) = self.data.split_at_mut((self.rows - 1 - y) * row_length);
            top[y * row_length..(y + 1) * row_length].swap_with_slice(&mut bottom[..row_length]);
        }
    }

    /// `RotateImage` by a whole number of degrees: right angles turn the
    /// pixels exactly (`IntegralRotateImage`); any other angle distorts
    /// the image about its centre on its page (`DistortImage` with
    /// ScaleRotateTranslate and best fit), the corners filled with the
    /// white background.
    fn rotate(&mut self, degrees: u32) -> Result<()> {
        let mut angle = (degrees % 360) as f64;
        let mut rotations = 0;
        while angle > 45.0 {
            angle -= 90.0;
            rotations += 1;
        }
        if angle != 0.0 {
            self.distort(degrees as f64);
            return Ok(());
        }
        let stride = self.stride();
        let (columns, rows) = (self.columns, self.rows);
        let turned_size = if rotations % 2 == 1 { (rows, columns) } else { (columns, rows) };
        let mut turned = vec![0.0f32; self.data.len()];
        for y in 0..rows {
            for x in 0..columns {
                let (target_x, target_y) = match rotations % 4 {
                    0 => (x, y),
                    1 => (rows - 1 - y, x),
                    2 => (columns - 1 - x, rows - 1 - y),
                    _ => (y, columns - 1 - x),
                };
                let from = (y * columns + x) * stride;
                let to = (target_y * turned_size.0 + target_x) * stride;
                turned[to..to + stride].copy_from_slice(&self.data[from..from + stride]);
            }
        }
        (self.columns, self.rows) = turned_size;
        self.data = turned;
        Ok(())
    }
}

impl Frame {
    /// `DistortImage` of the colour channels; the result is `DirectClass`.
    fn distort(&mut self, degrees: f64) {
        let stride = self.stride();
        let mut rgb = Vec::with_capacity(self.columns * self.rows * 3);
        for pixel in self.data.chunks_exact(stride) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        let mut source = nerthus_image::image::Image::from_data(self.columns, self.rows, 3, rgb);
        source.virtual_pixel = nerthus_image::image::VirtualPixel::Background;
        source.background = 65535.0;
        let (_, _, page_x, page_y) = self.page;
        let (turned, x, y) = nerthus_image::distort::rotate_best_fit(&source, page_x, page_y, degrees);
        self.columns = turned.width;
        self.rows = turned.height;
        self.data = turned.data;
        self.pseudo = false;
        self.colormap.clear();
        self.colors = 0;
        self.page.2 = x;
        self.page.3 = y;
    }
}

/// The row buffer the run-length readers fill (`InsertByte`).
struct Rows {
    buffer: Vec<u8>,
    row_bytes: usize,
    x: usize,
    y: usize,
    bits: u32,
    xor: bool,
}

impl Rows {
    /// `None` when IM can't allocate the buffer (a zero-byte row).
    fn new(frame: &Frame, bits: u32) -> Option<Rows> {
        let row_bytes = (bits as usize * frame.columns).div_ceil(8);
        if row_bytes == 0 {
            return None;
        }
        Some(Rows { buffer: vec![0u8; row_bytes * 8], row_bytes, x: 0, y: 0, bits, xor: false })
    }

    fn insert(&mut self, frame: &mut Frame, byte: u8) -> Result<()> {
        self.buffer[self.x] = if self.xor { !byte } else { byte };
        self.x += 1;
        if self.x >= self.row_bytes {
            if frame.insert_row(&self.buffer[..self.row_bytes], self.y, self.bits)? {
                self.y += 1;
            }
            self.x = 0;
        }
        Ok(())
    }
}

/// `Rd_WP_DWORD`: a byte, or after 0xff a little-endian word, or after
/// that a word with its top bit set and another word below it. IM's
/// size_t sums wrap when a byte comes back as EOF (-1).
fn read_dword(blob: &mut Blob) -> u64 {
    let mut byte = || blob.byte().map_or(-1i64, i64::from) as u64;
    let first = byte();
    if (first as i64) < 0xff {
        return first;
    }
    let mut value = byte();
    value = value.wrapping_add(byte().wrapping_mul(256));
    if value < 0x8000 {
        return value;
    }
    value = (value & 0x7fff) << 16;
    value = value.wrapping_add(byte());
    value.wrapping_add(byte().wrapping_mul(256))
}

/// `UnpackWPGRaster`: level 1 run-length rows; negative when it fails.
fn unpack_level1(blob: &mut Blob, frame: &mut Frame, bits: u32) -> Result<i32> {
    let Some(mut rows) = Rows::new(frame, bits) else {
        return Ok(-2);
    };
    while rows.y < frame.rows {
        let Some(c) = blob.byte() else {
            break;
        };
        let run = c & 0x7f;
        if c & 0x80 != 0 {
            if run != 0 {
                let value = blob.byte().unwrap_or(0xff);
                for _ in 0..run {
                    rows.insert(frame, value)?;
                }
            } else {
                let Some(count) = blob.byte() else {
                    break;
                };
                for _ in 0..count {
                    rows.insert(frame, 0xff)?;
                }
            }
        } else if run != 0 {
            for _ in 0..run {
                let Some(value) = blob.byte() else {
                    break;
                };
                rows.insert(frame, value)?;
            }
        } else {
            // Repeat the previous row.
            let Some(count) = blob.byte() else {
                return Ok(-7);
            };
            if rows.x != 0 {
                frame.insert_row(&rows.buffer[..rows.row_bytes], rows.y, bits)?;
                return Ok(-3);
            }
            for _ in 0..count {
                if rows.y >= frame.rows {
                    return Ok(-4);
                }
                if !frame.insert_row(&rows.buffer[..rows.row_bytes], rows.y, bits)? {
                    return Ok(-6);
                }
                rows.y += 1;
            }
        }
        if blob.eof {
            break;
        }
    }
    Ok(if rows.y < frame.rows { -5 } else { 0 })
}

/// `UnpackWPG2Raster`: level 2 run-length rows of `SampleSize`-byte
/// samples; negative when it fails.
fn unpack_level2(blob: &mut Blob, frame: &mut Frame, bits: u32) -> Result<i32> {
    let Some(mut rows) = Rows::new(frame, bits) else {
        return Ok(-2);
    };
    let mut sample_size = 1usize;
    let mut samples = [0u8; 8];
    while rows.y < frame.rows {
        let token = blob.byte().unwrap_or(0xff);
        match token {
            0x7d => {
                let size = blob.byte().map_or(u32::MAX, u32::from);
                if !(1..=8).contains(&size) {
                    return Ok(-2);
                }
                sample_size = size as usize;
            }
            0x7e => rows.xor = !rows.xor,
            0x7f | 0xff => {
                // Runs of black (0x7f) or white (0xff) bytes.
                if let Some(count) = blob.byte() {
                    let value = if token == 0x7f { 0 } else { 0xff };
                    for _ in 0..sample_size * (count as usize + 1) {
                        rows.insert(frame, value)?;
                    }
                }
            }
            0xfd => {
                if let Some(count) = blob.byte() {
                    for _ in 0..=count {
                        for &sample in &samples[..sample_size] {
                            rows.insert(frame, sample)?;
                        }
                    }
                }
            }
            0xfe => {
                if let Some(count) = blob.byte() {
                    if rows.x != 0 {
                        return Ok(-3);
                    }
                    for _ in 0..=count {
                        let y = rows.y.min(frame.rows - 1);
                        if !frame.insert_row(&rows.buffer[..rows.row_bytes], y, bits)? {
                            return Ok(-3);
                        }
                        rows.y += 1;
                    }
                }
            }
            _ => {
                let run = (token & 0x7f) as usize;
                if token & 0x80 != 0 {
                    for sample in samples[..sample_size].iter_mut() {
                        *sample = blob.byte().unwrap_or(0xff);
                    }
                    for _ in 0..=run {
                        for &sample in &samples[..sample_size] {
                            rows.insert(frame, sample)?;
                        }
                    }
                } else {
                    for _ in 0..sample_size * (run + 1) {
                        let value = blob.byte().unwrap_or(0xff);
                        rows.insert(frame, value)?;
                    }
                }
            }
        }
        if blob.eof {
            break;
        }
    }
    Ok(if rows.y < frame.rows { -5 } else { 0 })
}

/// `SeekBlob` to a record: false when IM's seek doesn't land there.
fn seek_to(blob: &mut Blob, offset: i64) -> bool {
    let landed = blob.seek(offset).map_or(-1, |position| position as i64);
    landed == offset
}

/// A palette record's checks (`InvalidColormapIndex`), in IM's unsigned
/// arithmetic.
fn check_palette(start: u32, entries: u32, record_length: u64, level2: bool) -> Result<()> {
    let span = entries.wrapping_sub(start) as u64;
    let room = record_length.wrapping_sub(4) / 3;
    let backwards = if level2 { start >= entries } else { start > entries };
    if span > room || backwards {
        bail!("InvalidColormapIndex");
    }
    Ok(())
}

/// `LoadWPG2Flags`: whether the bitmap's transform flops and flips it
/// (negative x and y scales).
fn load_level2_flags(blob: &mut Blob, precision: u8) -> (bool, bool) {
    const TPR: u16 = 1;
    const TRN: u16 = 2;
    const SKW: u16 = 4;
    const SCL: u16 = 8;
    const ROT: u16 = 0x10;
    const OID: u16 = 0x20;
    const LCK: u16 = 0x80;
    let mut scale_x = 1.0f32;
    let mut scale_y = 1.0f32;
    let flags = blob.lsb_u16();
    if flags & LCK != 0 {
        blob.lsb_u32();
    }
    if flags & OID != 0 {
        if precision == 0 {
            if blob.lsb_u16() >= 0x8000 {
                blob.lsb_u16();
            }
        } else {
            blob.lsb_u32();
        }
    }
    if flags & ROT != 0 {
        blob.lsb_u32();
    }
    if flags & (ROT | SCL) != 0 {
        scale_x = blob.lsb_u32() as i32 as f32 / 65536.0;
        scale_y = blob.lsb_u32() as i32 as f32 / 65536.0;
    }
    if flags & (ROT | SKW) != 0 {
        blob.read(8);
    }
    if flags & TRN != 0 {
        blob.read(12);
    }
    if flags & TPR != 0 {
        blob.read(8);
    }
    (scale_x < 0.0, scale_y < 0.0)
}

/// A level 1 bitmap record after its header: the colormap IM sets up, the
/// rows, then its flop, flip and turn.
fn read_level1_bitmap(blob: &mut Blob, frame: &mut Frame, bits: u32, angle: u32) -> Result<()> {
    frame.reset();
    if !frame.pseudo && bits < 24 {
        let colors = 1u64 << (bits & 63);
        if colors > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        frame.acquire_colormap(colors)?;
        for (entry, rgb) in frame.colormap.iter_mut().zip(LEVEL1_PALETTE.iter()) {
            *entry = [char_quantum(rgb[0]) as f64, char_quantum(rgb[2]) as f64, char_quantum(rgb[1]) as f64, 65535.0];
        }
    } else if bits < 24 && (frame.colors as u64) < (1u64 << (bits & 63)) {
        let old = std::mem::take(&mut frame.colormap);
        let old_colors = frame.colors;
        frame.acquire_colormap(1u64 << (bits & 63))?;
        let kept = frame.colors.min(old_colors).min(old.len());
        frame.colormap[..kept].copy_from_slice(&old[..kept]);
    }
    if bits == 1 {
        frame.colormap[0] = [0.0, 0.0, 0.0, 65535.0];
        frame.colormap[1] = [65535.0, 65535.0, 65535.0, 65535.0];
    }
    if unpack_level1(blob, frame, bits)? < 0 {
        bail!("UnableToDecompressImage");
    }
    if angle != 0 {
        if angle & 0x8000 != 0 {
            frame.flop();
        }
        if angle & 0x2000 != 0 {
            frame.flip();
        }
        if angle & 0x0fff != 0 {
            frame.rotate(angle & 0x0fff)?;
        }
    }
    Ok(())
}

fn read_level1(blob: &mut Blob, mut data_offset: i64, frames: &mut Vec<Frame>) -> Result<()> {
    let mut frame = Frame::empty();
    while !blob.eof {
        if !seek_to(blob, data_offset) || blob.eof {
            break;
        }
        let Some(record_type) = blob.byte() else {
            break;
        };
        let record_length = read_dword(blob);
        if record_length.wrapping_add(4) >= blob.size() {
            bail!("ImproperImageHeader");
        }
        if blob.eof {
            break;
        }
        data_offset = (blob.tell() as i64).wrapping_add(record_length as i64);
        if data_offset > blob.size() as i64 {
            bail!("InsufficientImageDataInFile");
        }
        match record_type {
            0x0b | 0x14 => {
                let mut angle = 0;
                if record_type == 0x14 {
                    angle = blob.lsb_u16() as u32;
                    // The page comes from the lower left x alone (IM's slip).
                    let low_left_x = blob.lsb_u16() as u32;
                    blob.lsb_u16();
                    let up_right_x = blob.lsb_u16() as u32;
                    let up_right_y = blob.lsb_u16() as u32;
                    let page_width = (low_left_x.wrapping_sub(up_right_x) as f64 / 470.0) as u32 as u64;
                    let page_height = (low_left_x.wrapping_sub(up_right_y) as f64 / 470.0) as u32 as u64;
                    let offset = (low_left_x as f64 / 470.0) as i32 as i64;
                    frame.page = (page_width, page_height, offset, offset);
                }
                let width = blob.lsb_u16() as usize;
                let height = blob.lsb_u16() as usize;
                if width == 0 || height == 0 {
                    bail!("ImproperImageHeader");
                }
                let bits = blob.lsb_u16() as u32;
                blob.read(4);
                frame.columns = width;
                frame.rows = height;
                if record_type == 0x0b && bits == 1 {
                    frame.acquire_colormap(2)?;
                }
                read_level1_bitmap(blob, &mut frame, bits, angle)?;
                frames.push(std::mem::replace(&mut frame, Frame::empty()));
            }
            0x0e => {
                let start = blob.lsb_u16() as u32;
                let entries = blob.lsb_u16() as u32;
                check_palette(start, entries, record_length, false)?;
                frame.acquire_colormap(entries as u64)?;
                for index in start as usize..entries as usize {
                    for channel in 0..3 {
                        frame.colormap[index][channel] = char_quantum(blob.byte().unwrap_or(0xff)) as f64;
                    }
                }
            }
            0x11 if record_length > 8 => bail!("ImproperImageHeader"),
            0x1b if record_length > 0x3c => bail!("ImproperImageHeader"),
            _ => {}
        }
    }
    Ok(())
}

fn read_level2(blob: &mut Blob, mut data_offset: i64, frames: &mut Vec<Frame>) -> Result<()> {
    let mut frame = Frame::empty();
    let mut precision = 0u8;
    let mut flop = false;
    let mut flip = false;
    while !blob.eof {
        if !seek_to(blob, data_offset) || blob.eof {
            break;
        }
        if blob.byte().is_none() {
            break;
        }
        let Some(record_type) = blob.byte() else {
            break;
        };
        read_dword(blob);
        let record_length = read_dword(blob);
        if blob.eof {
            break;
        }
        data_offset = (blob.tell() as i64).wrapping_add(record_length as i64);
        match record_type {
            0x01 => {
                blob.read(4);
                precision = blob.byte().unwrap_or(0xff);
            }
            0x0c => {
                let start = blob.lsb_u16() as u32;
                let entries = blob.lsb_u16() as u32;
                check_palette(start, entries, record_length, true)?;
                frame.acquire_colormap(entries as u64)?;
                for index in start as usize..entries as usize {
                    for channel in 0..3 {
                        frame.colormap[index][channel] = char_quantum(blob.byte().unwrap_or(0xff)) as f64;
                    }
                    blob.byte();
                }
            }
            0x0e => {
                let width = blob.lsb_u16() as usize;
                let height = blob.lsb_u16() as usize;
                if width == 0 || height == 0 {
                    bail!("ImproperImageHeader");
                }
                let depth = blob.byte().unwrap_or(0xff);
                let compression = blob.byte().unwrap_or(0xff);
                if compression > 1 {
                    continue;
                }
                let bits = match depth {
                    1 => 1,
                    2 => 2,
                    3 => 4,
                    4 => 8,
                    8 => 24,
                    _ => continue,
                };
                frame.columns = width;
                frame.rows = height;
                frame.reset();
                if frame.colors == 0 && bits != 24 {
                    frame.acquire_colormap(1u64 << bits)?;
                } else if bits < 24 && frame.colors < (1usize << bits) {
                    // The colormap grows; the colour count doesn't.
                    frame.colormap.resize(1usize << bits, [0.0; 4]);
                }
                if compression == 0 {
                    let row_bytes = (bits as usize * width).div_ceil(8);
                    for y in 0..height {
                        let row = blob.read(row_bytes);
                        if row.len() != row_bytes || !frame.insert_row(row, y, bits)? {
                            bail!("UnableToDecompressImage");
                        }
                    }
                } else if unpack_level2(blob, &mut frame, bits)? < 0 {
                    bail!("UnableToDecompressImage");
                }
                if flop {
                    frame.flop();
                }
                if flip {
                    frame.flip();
                }
                frames.push(std::mem::replace(&mut frame, Frame::empty()));
            }
            0x12 => {
                let header_size = blob.lsb_u16() as u64;
                if record_length > header_size {
                    bail!("ImproperImageHeader");
                }
            }
            0x1b => (flop, flip) = load_level2_flags(blob, precision),
            _ => {}
        }
    }
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let file_id = blob.lsb_u32();
    let data_offset = blob.lsb_u32() as i64;
    let _product = blob.byte();
    let file_type = blob.byte().unwrap_or(0xff);
    let major_version = blob.byte().unwrap_or(0xff);
    let _minor_version = blob.byte();
    let encrypt_key = blob.lsb_u16();
    blob.lsb_u16();
    if file_id != 0x4350_57ff || file_type != 0x16 {
        bail!("ImproperImageHeader");
    }
    if encrypt_key != 0 {
        bail!("EncryptedWPGImageFileNotSupported");
    }
    let mut frames = Vec::new();
    match major_version {
        1 => read_level1(&mut blob, data_offset, &mut frames)?,
        2 => read_level2(&mut blob, data_offset, &mut frames)?,
        _ => bail!("DataEncodingSchemeIsNotSupported"),
    }
    let Some(frame) = frames.into_iter().next() else {
        bail!("ImageFileDoesNotContainAnyImageData");
    };
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if frame.pseudo {
        channels.push(PixelChannel::Index);
    }
    let mut cache = PixelCache::new(frame.columns, frame.rows, channels, frame.data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if frame.pseudo {
        cache.colormap = frame.colormap;
        cache.colormap.truncate(frame.colors);
    }
    Ok(cache)
}
