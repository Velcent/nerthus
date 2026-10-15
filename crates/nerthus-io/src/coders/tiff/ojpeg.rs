//! libtiff's old-style JPEG codec (`tif_ojpeg.c`, TIFF 6.0's JPEG).
//!
//! The codec reads a JPEG stream of its own: the JPEGInterchangeFormat
//! block, then the strips or tiles. It takes the frame's tables and
//! parameters from the stream's markers, or from the JPEGQTables,
//! JPEGDCTables and JPEGACTables tags when the stream has none, writes a
//! clean header for libjpeg and passes the compressed data on after it,
//! with a restart marker between striles. Each plane is one libjpeg
//! session, read strile by strile. Subsampled YCbCr comes out raw, packed
//! into TIFF's clumps; anything else as scanlines without colour
//! conversion.
//!
//! libjpeg reads the stream as it goes; here the whole stream is built and
//! decoded when a session starts, and the striles are served from that.
//! Past the stream's end libjpeg's source gives EOI markers, as the JPEG
//! port's libtiff source does.

use anyhow::{anyhow, bail, Result};

/// A failure libtiff reports with no message (the input running out
/// while the header is read, a table too short): the strip read fails
/// quietly and the reader carries on.
#[derive(Debug)]
struct Silent;

impl std::fmt::Display for Silent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("old-style JPEG data could not be read")
    }
}

impl std::error::Error for Silent {}

fn silent() -> anyhow::Error {
    anyhow::Error::new(Silent)
}

use super::super::jpeg::{Strip, StripSamples, Tables, TiffOutput};
use super::directory::*;

/// `OJPEG_BUFFER`: the bytes the codec reads from the file at a time.
const BUFFER_SIZE: u64 = 2048;

const MARKER_SOF0: u8 = 0xC0;
const MARKER_SOF1: u8 = 0xC1;
const MARKER_SOF3: u8 = 0xC3;
const MARKER_DHT: u8 = 0xC4;
const MARKER_RST0: u8 = 0xD0;
const MARKER_SOI: u8 = 0xD8;
const MARKER_EOI: u8 = 0xD9;
const MARKER_SOS: u8 = 0xDA;
const MARKER_DQT: u8 = 0xDB;
const MARKER_DRI: u8 = 0xDD;
const MARKER_APP0: u8 = 0xE0;
const MARKER_COM: u8 = 0xFE;

/// Where the input comes from (`OJPEGStateInBufferSource`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    NotSetYet,
    InterchangeFormat,
    Strile,
    Eof,
}

/// The input's position (`in_buffer_*`): the block read last and what is
/// left of the current source in the file.
#[derive(Clone, Debug)]
struct Input {
    source: Source,
    next_strile: u32,
    file_pos: u64,
    file_togo: u64,
    buffer: Vec<u8>,
    /// The next unread byte of `buffer`.
    cursor: usize,
}

impl Input {
    fn new() -> Self {
        Self { source: Source::NotSetYet, next_strile: 0, file_pos: 0, file_togo: 0, buffer: Vec::new(), cursor: 0 }
    }

    fn togo(&self) -> usize {
        self.buffer.len() - self.cursor
    }
}

/// What the codec reads the file with: the data and the striles'
/// locations.
struct File<'a> {
    data: &'a [u8],
    interchange_format: u64,
    interchange_format_length: u64,
    offsets: Option<&'a [u64]>,
    byte_counts: Option<&'a [u64]>,
    strile_count: u32,
}

impl<'a> File<'a> {
    /// The file with the JPEGInterchangeFormat block made to fit it
    /// (`OJPEGReadHeaderInfoSec`'s first lines).
    fn new(tiff: &'a Tiff) -> Self {
        let dir = &tiff.dir;
        let tags = &dir.old_jpeg;
        let file_size = tiff.data.len() as u64;
        let mut interchange_format = tags.interchange_offset.unwrap_or(0);
        let mut interchange_format_length = tags.interchange_length.unwrap_or(0);
        if interchange_format != 0 {
            if interchange_format >= file_size {
                interchange_format = 0;
                interchange_format_length = 0;
            } else if interchange_format_length == 0
                || interchange_format > u64::MAX - interchange_format_length
                || interchange_format + interchange_format_length > file_size
            {
                interchange_format_length = file_size - interchange_format;
            }
        }
        let offsets = tags.strip_offsets_set.then_some(dir.strip_offsets.as_slice());
        let byte_counts = tags.strip_byte_counts_set.then_some(dir.strip_byte_counts.as_slice());
        Self { data: tiff.data, interchange_format, interchange_format_length, offsets, byte_counts, strile_count: dir.strip_count }
    }

    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    /// `TIFFReadFile` at `position`: up to `count` bytes.
    fn read(&self, position: u64, count: u64) -> &'a [u8] {
        let start = position.min(self.size()) as usize;
        let end = position.saturating_add(count).min(self.size()) as usize;
        &self.data[start..end]
    }
}

/// `OJPEGReadBufferFill`: the next block of input, moving on to the next
/// source when one is used up; false when there is no more (or a strile's
/// location can't be had).
fn fill(input: &mut Input, file: &File) -> bool {
    loop {
        if input.file_togo != 0 {
            let count = BUFFER_SIZE.min(input.file_togo);
            let block = file.read(input.file_pos, count);
            if block.is_empty() {
                return false;
            }
            input.buffer = block.to_vec();
            input.cursor = 0;
            input.file_togo -= block.len() as u64;
            input.file_pos += block.len() as u64;
            return true;
        }
        match input.source {
            Source::NotSetYet => {
                if file.interchange_format != 0 {
                    input.file_pos = file.interchange_format;
                    input.file_togo = file.interchange_format_length;
                }
                input.source = Source::InterchangeFormat;
            }
            Source::InterchangeFormat => input.source = Source::Strile,
            Source::Strile => {
                if input.next_strile == file.strile_count {
                    input.source = Source::Eof;
                } else {
                    let index = input.next_strile as usize;
                    let Some(&offset) = file.offsets.and_then(|offsets| offsets.get(index)) else {
                        return false;
                    };
                    input.file_pos = offset;
                    if input.file_pos != 0 {
                        let Some(&byte_count) = file.byte_counts.and_then(|counts| counts.get(index)) else {
                            return false;
                        };
                        if input.file_pos >= file.size() {
                            input.file_pos = 0;
                        } else if byte_count == 0 {
                            input.file_togo = file.size() - input.file_pos;
                        } else {
                            input.file_togo = byte_count;
                            if input.file_pos > u64::MAX - input.file_togo || input.file_pos + input.file_togo > file.size() {
                                input.file_togo = file.size() - input.file_pos;
                            }
                        }
                    }
                    input.next_strile += 1;
                }
            }
            Source::Eof => return false,
        }
    }
}

fn read_byte(input: &mut Input, file: &File) -> Option<u8> {
    if input.togo() == 0 && !fill(input, file) {
        return None;
    }
    let byte = input.buffer[input.cursor];
    input.cursor += 1;
    Some(byte)
}

fn peek_byte(input: &mut Input, file: &File) -> Option<u8> {
    if input.togo() == 0 && !fill(input, file) {
        return None;
    }
    Some(input.buffer[input.cursor])
}

fn read_word(input: &mut Input, file: &File) -> Option<u16> {
    let high = read_byte(input, file)?;
    let low = read_byte(input, file)?;
    Some((high as u16) << 8 | low as u16)
}

fn read_block(input: &mut Input, file: &File, length: usize) -> Option<Vec<u8>> {
    let mut block = Vec::with_capacity(length);
    while block.len() < length {
        if input.togo() == 0 && !fill(input, file) {
            return None;
        }
        let count = (length - block.len()).min(input.togo());
        block.extend_from_slice(&input.buffer[input.cursor..input.cursor + count]);
        input.cursor += count;
    }
    Some(block)
}

/// `OJPEGReadSkip`: past what is buffered it skips in the file, but never
/// beyond the current source.
fn skip(input: &mut Input, length: u16) {
    let mut remaining = length as u64;
    let buffered = remaining.min(input.togo() as u64);
    input.cursor += buffered as usize;
    remaining -= buffered;
    if remaining > 0 {
        let skipped = remaining.min(input.file_togo);
        input.file_pos += skipped;
        input.file_togo -= skipped;
    }
}

/// The subsampling the codec reports for YCbCrSubsampling
/// (`OJPEGSubsamplingCorrect`): that of the stream's frame header when
/// there is one, before the first strile is read. Sampling factors TIFF
/// can't express make the codec upsample inside libjpeg instead, and
/// report 1×1.
pub(super) fn correct_subsampling(tiff: &mut Tiff) {
    let dir = &tiff.dir;
    let [tag_horizontal, tag_vertical] = dir.ycbcr_subsampling;
    let ycbcr_like = matches!(dir.photometric, Some(PHOTOMETRIC_YCBCR) | Some(PHOTOMETRIC_ITULAB));
    if dir.samples_per_pixel != 3 || !ycbcr_like {
        tiff.dir.old_jpeg.subsampling = (1, 1);
        tiff.dir.old_jpeg.force_desubsampling = false;
        return;
    }
    let mut state = State::new(tiff);
    state.subsampling_hor = tag_horizontal as u8;
    state.subsampling_ver = tag_vertical as u8;
    state.correcting = true;
    let file = File::new(tiff);
    // Errors go unreported here; what was found stands.
    let _ = state.read_header_info_sec(tiff, &file);
    if state.force_desubsampling {
        state.subsampling_hor = 1;
        state.subsampling_ver = 1;
    }
    let corrected = (state.subsampling_hor as u16, state.subsampling_ver as u16);
    tiff.dir.old_jpeg.subsampling = corrected;
    tiff.dir.old_jpeg.force_desubsampling = state.force_desubsampling;
    tiff.dir.ycbcr_subsampling = [corrected.0, corrected.1];
}

/// The codec's state (`OJPEGState`) that header reading fills in.
#[derive(Clone)]
struct State {
    correcting: bool,
    subsampling_hor: u8,
    subsampling_ver: u8,
    force_desubsampling: bool,
    image_width: u32,
    image_length: u32,
    strile_width: u32,
    strile_length: u32,
    strile_length_total: u32,
    samples_per_pixel: u8,
    plane_sample_offset: u8,
    samples_per_pixel_per_plane: u8,
    qtable_offsets: [u64; 3],
    dctable_offsets: [u64; 3],
    actable_offsets: [u64; 3],
    /// Each table's marker segment as it goes into the stream.
    qtable: [Option<Vec<u8>>; 4],
    dctable: [Option<Vec<u8>>; 4],
    actable: [Option<Vec<u8>>; 4],
    restart_interval: u16,
    sof_log: bool,
    sof_marker_id: u8,
    sof_x: u32,
    sof_y: u32,
    sof_c: [u8; 3],
    sof_hv: [u8; 3],
    sof_tq: [u8; 3],
    sos_cs: [u8; 3],
    sos_tda: [u8; 3],
    input: Input,
}

/// A table tag's offsets, as `OJPEGVSetField` keeps them: none for more
/// than three.
fn table_offsets(values: &[u64]) -> [u64; 3] {
    let mut offsets = [0u64; 3];
    if values.len() <= 3 {
        offsets[..values.len()].copy_from_slice(values);
    }
    offsets
}

impl State {
    fn new(tiff: &Tiff) -> Self {
        let tags = &tiff.dir.old_jpeg;
        Self {
            correcting: false,
            subsampling_hor: 2,
            subsampling_ver: 2,
            force_desubsampling: false,
            image_width: 0,
            image_length: 0,
            strile_width: 0,
            strile_length: 0,
            strile_length_total: 0,
            samples_per_pixel: 0,
            plane_sample_offset: 0,
            samples_per_pixel_per_plane: 0,
            qtable_offsets: table_offsets(&tags.q_tables),
            dctable_offsets: table_offsets(&tags.dc_tables),
            actable_offsets: table_offsets(&tags.ac_tables),
            qtable: Default::default(),
            dctable: Default::default(),
            actable: Default::default(),
            restart_interval: tags.restart_interval.unwrap_or(0),
            sof_log: false,
            sof_marker_id: 0,
            sof_x: 0,
            sof_y: 0,
            sof_c: [0; 3],
            sof_hv: [0; 3],
            sof_tq: [0; 3],
            sos_cs: [0; 3],
            sos_tda: [0; 3],
            input: Input::new(),
        }
    }

    /// `OJPEGReadHeaderInfo`: the striles' layout, then the stream's header.
    fn read_header_info(&mut self, tiff: &Tiff, file: &File) -> Result<()> {
        let dir = &tiff.dir;
        self.image_width = dir.width;
        self.image_length = dir.length;
        if dir.tiled {
            self.strile_width = dir.tile_width;
            self.strile_length = dir.tile_length;
            self.strile_length_total = self.image_length.div_ceil(self.strile_length.max(1)) * self.strile_length;
        } else {
            self.strile_width = self.image_width;
            self.strile_length = dir.rows_per_strip;
            if self.strile_length == u32::MAX {
                self.strile_length = self.image_length;
            }
            self.strile_length_total = self.image_length;
        }
        if dir.samples_per_pixel == 1 {
            self.samples_per_pixel = 1;
            self.plane_sample_offset = 0;
            self.samples_per_pixel_per_plane = 1;
            self.subsampling_hor = 1;
            self.subsampling_ver = 1;
        } else {
            if dir.samples_per_pixel != 3 {
                bail!("SamplesPerPixel {} not supported for this compression scheme", self.samples_per_pixel);
            }
            self.samples_per_pixel = 3;
            self.plane_sample_offset = 0;
            self.samples_per_pixel_per_plane = if dir.planar_config == PLANARCONFIG_CONTIG { 3 } else { 1 };
        }
        if self.strile_length < self.image_length {
            let valid = |value: u8| matches!(value, 1 | 2 | 4);
            if !valid(self.subsampling_hor) || !valid(self.subsampling_ver) {
                bail!("Invalid subsampling values");
            }
            if !self.strile_length.is_multiple_of(self.subsampling_ver as u32 * 8) {
                bail!("Incompatible vertical subsampling and image strip/tile length");
            }
            let across = self.strile_width.div_ceil(self.subsampling_hor as u32 * 8);
            self.restart_interval = (across * (self.strile_length / (self.subsampling_ver as u32 * 8))) as u16;
        }
        self.read_header_info_sec(tiff, file)
    }

    /// `OJPEGReadHeaderInfoSec`: the stream's markers up to the start of
    /// the compressed data (or, when correcting the subsampling, up to the
    /// frame header); with no frame header, the tables from the tags.
    fn read_header_info_sec(&mut self, tiff: &Tiff, file: &File) -> Result<()> {
        self.input = Input::new();
        let input_error = silent;
        loop {
            let Some(byte) = peek_byte(&mut self.input, file) else {
                return Err(input_error());
            };
            if byte != 255 {
                break;
            }
            self.input.cursor += 1;
            let marker = loop {
                let Some(byte) = read_byte(&mut self.input, file) else {
                    return Err(input_error());
                };
                if byte != 255 {
                    break byte;
                }
            };
            match marker {
                MARKER_SOI => {}
                MARKER_COM | MARKER_APP0..=0xEF => {
                    let length = read_word(&mut self.input, file).ok_or_else(input_error)?;
                    if length < 2 {
                        bail!("Corrupt JPEG data");
                    }
                    if length > 2 {
                        skip(&mut self.input, length - 2);
                    }
                }
                MARKER_DRI => self.read_dri(file)?,
                MARKER_DQT => self.read_dqt(file)?,
                MARKER_DHT => self.read_dht(file)?,
                MARKER_SOF0 | MARKER_SOF1 | MARKER_SOF3 => {
                    self.read_sof(file, marker)?;
                    if self.correcting {
                        return Ok(());
                    }
                }
                MARKER_SOS => {
                    if self.correcting {
                        return Ok(());
                    }
                    self.read_sos(file)?;
                    break;
                }
                other => bail!("Unknown marker type {other} in JPEG data"),
            }
        }
        if self.correcting {
            return Ok(());
        }
        if !self.sof_log {
            self.read_qtables(tiff)?;
            self.sof_marker_id = MARKER_SOF0;
            for component in 0..self.samples_per_pixel as usize {
                self.sof_c[component] = component as u8;
            }
            self.sof_hv[0] = (self.subsampling_hor << 4) | self.subsampling_ver;
            for component in 1..self.samples_per_pixel as usize {
                self.sof_hv[component] = 17;
            }
            self.sof_x = self.strile_width;
            self.sof_y = self.strile_length_total;
            self.sof_log = true;
            self.read_dctables(tiff)?;
            self.read_actables(tiff)?;
            for component in 1..self.samples_per_pixel as usize {
                self.sos_cs[component] = component as u8;
            }
        }
        Ok(())
    }

    /// `OJPEGReadHeaderInfoSecStreamDri`.
    fn read_dri(&mut self, file: &File) -> Result<()> {
        let length = read_word(&mut self.input, file).ok_or_else(silent)?;
        if length != 4 {
            bail!("Corrupt DRI marker in JPEG data");
        }
        self.restart_interval = read_word(&mut self.input, file).ok_or_else(silent)?;
        Ok(())
    }

    /// `OJPEGReadHeaderInfoSecStreamDqt`: each 8-bit table kept whole.
    fn read_dqt(&mut self, file: &File) -> Result<()> {
        let length = read_word(&mut self.input, file).ok_or_else(silent)?;
        if length <= 2 {
            bail!("Corrupt DQT marker in JPEG data");
        }
        if self.correcting {
            skip(&mut self.input, length - 2);
            return Ok(());
        }
        let mut remaining = length - 2;
        loop {
            if remaining < 65 {
                bail!("Corrupt DQT marker in JPEG data");
            }
            let body = read_block(&mut self.input, file, 65).ok_or_else(silent)?;
            let index = (body[0] & 15) as usize;
            if index > 3 {
                bail!("Corrupt DQT marker in JPEG data");
            }
            let mut segment = vec![255, MARKER_DQT, 0, 67];
            segment.extend_from_slice(&body);
            self.qtable[index] = Some(segment);
            remaining -= 65;
            if remaining == 0 {
                return Ok(());
            }
        }
    }

    /// `OJPEGReadHeaderInfoSecStreamDht`: the marker kept whole, filed by
    /// its first table's class and index.
    fn read_dht(&mut self, file: &File) -> Result<()> {
        let length = read_word(&mut self.input, file).ok_or_else(silent)?;
        if length <= 2 {
            bail!("Corrupt DHT marker in JPEG data");
        }
        if self.correcting {
            skip(&mut self.input, length - 2);
            return Ok(());
        }
        let body = read_block(&mut self.input, file, length as usize - 2).ok_or_else(silent)?;
        let mut segment = vec![255, MARKER_DHT, (length >> 8) as u8, (length & 255) as u8];
        segment.extend_from_slice(&body);
        let selector = body[0];
        if selector & 240 == 0 {
            if selector > 3 {
                bail!("Corrupt DHT marker in JPEG data");
            }
            self.dctable[selector as usize] = Some(segment);
        } else {
            if selector & 240 != 16 {
                bail!("Corrupt DHT marker in JPEG data");
            }
            let index = (selector & 15) as usize;
            if index > 3 {
                bail!("Corrupt DHT marker in JPEG data");
            }
            self.actable[index] = Some(segment);
        }
        Ok(())
    }

    /// `OJPEGReadHeaderInfoSecStreamSof`: checked against the image; when
    /// correcting, only the sampling factors are taken.
    fn read_sof(&mut self, file: &File, marker: u8) -> Result<()> {
        let premature = silent;
        if self.sof_log {
            bail!("Corrupt JPEG data");
        }
        if !self.correcting {
            self.sof_marker_id = marker;
        }
        let mut length = read_word(&mut self.input, file).ok_or_else(premature)?;
        if length < 11 {
            bail!("Corrupt SOF marker in JPEG data");
        }
        length -= 8;
        if !length.is_multiple_of(3) {
            bail!("Corrupt SOF marker in JPEG data");
        }
        let count = length / 3;
        if !self.correcting && count != self.samples_per_pixel as u16 {
            bail!("JPEG compressed data indicates unexpected number of samples");
        }
        let precision = read_byte(&mut self.input, file).ok_or_else(premature)?;
        if precision != 8 {
            bail!("JPEG compressed data indicates unexpected number of bits per sample");
        }
        if self.correcting {
            skip(&mut self.input, 4);
        } else {
            let height = read_word(&mut self.input, file).ok_or_else(premature)? as u32;
            if height < self.image_length && height < self.strile_length_total {
                bail!("JPEG compressed data indicates unexpected height");
            }
            self.sof_y = height;
            let width = read_word(&mut self.input, file).ok_or_else(premature)? as u32;
            if width < self.image_width && width < self.strile_width {
                bail!("JPEG compressed data indicates unexpected width");
            }
            if width > self.strile_width {
                bail!("JPEG compressed data image width exceeds expected image width");
            }
            self.sof_x = width;
        }
        let components = read_byte(&mut self.input, file).ok_or_else(premature)?;
        if components as u16 != count {
            bail!("Corrupt SOF marker in JPEG data");
        }
        for index in 0..count as usize {
            let identifier = read_byte(&mut self.input, file).ok_or_else(premature)?;
            if !self.correcting {
                self.sof_c[index] = identifier;
            }
            let sampling = read_byte(&mut self.input, file).ok_or_else(premature)?;
            if self.correcting {
                if index == 0 {
                    self.subsampling_hor = sampling >> 4;
                    self.subsampling_ver = sampling & 15;
                    let valid = |value: u8| matches!(value, 1 | 2 | 4);
                    if !valid(self.subsampling_hor) || !valid(self.subsampling_ver) {
                        self.force_desubsampling = true;
                    }
                } else if sampling != 17 {
                    self.force_desubsampling = true;
                }
            } else {
                self.sof_hv[index] = sampling;
                if !self.force_desubsampling {
                    let expected = if index == 0 { (self.subsampling_hor << 4) | self.subsampling_ver } else { 17 };
                    if sampling != expected {
                        bail!("JPEG compressed data indicates unexpected subsampling values");
                    }
                }
            }
            let table = read_byte(&mut self.input, file).ok_or_else(premature)?;
            if !self.correcting {
                self.sof_tq[index] = table;
            }
        }
        if !self.correcting {
            self.sof_log = true;
        }
        Ok(())
    }

    /// `OJPEGReadHeaderInfoSecStreamSos`: the plane's components and their
    /// table selectors.
    fn read_sos(&mut self, file: &File) -> Result<()> {
        let premature = silent;
        if !self.sof_log {
            bail!("Corrupt SOS marker in JPEG data");
        }
        let per_plane = self.samples_per_pixel_per_plane;
        let length = read_word(&mut self.input, file).ok_or_else(premature)?;
        if length != 6 + per_plane as u16 * 2 {
            bail!("Corrupt SOS marker in JPEG data");
        }
        let count = read_byte(&mut self.input, file).ok_or_else(premature)?;
        if count != per_plane {
            bail!("Corrupt SOS marker in JPEG data");
        }
        for component in 0..per_plane as usize {
            let index = self.plane_sample_offset as usize + component;
            let selector = read_byte(&mut self.input, file).ok_or_else(premature)?;
            self.sos_cs[index] = selector;
            let tables = read_byte(&mut self.input, file).ok_or_else(premature)?;
            self.sos_tda[index] = tables;
        }
        skip(&mut self.input, 3);
        Ok(())
    }

    /// The offsets of a table tag that hold a new table for component
    /// `index`: set, not the previous component's, and (from the third on)
    /// not any earlier one's but the first's.
    fn new_table(offsets: &[u64; 3], index: usize, name: &str) -> Result<bool> {
        let fresh = offsets[index] != 0 && (index == 0 || offsets[index] != offsets[index - 1]);
        if fresh {
            for earlier in 0..index.saturating_sub(1) {
                if offsets[index] == offsets[earlier] {
                    bail!("Corrupt {name} tag value");
                }
            }
        }
        Ok(fresh)
    }

    /// `OJPEGReadHeaderInfoSecTablesQTable`: 64-byte tables at the
    /// JPEGQTables offsets.
    fn read_qtables(&mut self, tiff: &Tiff) -> Result<()> {
        if self.qtable_offsets[0] == 0 {
            bail!("Missing JPEG tables");
        }
        for index in 0..self.samples_per_pixel as usize {
            if Self::new_table(&self.qtable_offsets, index, "JpegQTables")? {
                let values = read_at(tiff, self.qtable_offsets[index], 64);
                if values.len() != 64 {
                    return Err(silent());
                }
                let mut segment = vec![255, MARKER_DQT, 0, 67, index as u8];
                segment.extend_from_slice(values);
                self.qtable[index] = Some(segment);
                self.sof_tq[index] = index as u8;
            } else {
                self.sof_tq[index] = self.sof_tq[index - 1];
            }
        }
        Ok(())
    }

    /// A Huffman table at `offset`: 16 counts, then their values, as a DHT
    /// marker for `selector`.
    fn huffman_table(tiff: &Tiff, offset: u64, selector: u8) -> Result<Vec<u8>> {
        let counts = read_at(tiff, offset, 16);
        if counts.len() != 16 {
            return Err(silent());
        }
        let total: u32 = counts.iter().map(|&count| count as u32).sum();
        let values = read_at(tiff, offset + 16, total as u64);
        if values.len() != total as usize {
            return Err(silent());
        }
        let length = 19 + total;
        let mut segment = vec![255, MARKER_DHT, (length >> 8) as u8, (length & 255) as u8, selector];
        segment.extend_from_slice(counts);
        segment.extend_from_slice(values);
        Ok(segment)
    }

    /// `OJPEGReadHeaderInfoSecTablesDcTable`.
    fn read_dctables(&mut self, tiff: &Tiff) -> Result<()> {
        if self.dctable_offsets[0] == 0 {
            bail!("Missing JPEG tables");
        }
        for index in 0..self.samples_per_pixel as usize {
            if Self::new_table(&self.dctable_offsets, index, "JpegDcTables")? {
                self.dctable[index] = Some(Self::huffman_table(tiff, self.dctable_offsets[index], index as u8)?);
                self.sos_tda[index] = (index as u8) << 4;
            } else {
                self.sos_tda[index] = self.sos_tda[index - 1];
            }
        }
        Ok(())
    }

    /// `OJPEGReadHeaderInfoSecTablesAcTable`.
    fn read_actables(&mut self, tiff: &Tiff) -> Result<()> {
        if self.actable_offsets[0] == 0 {
            bail!("Missing JPEG tables");
        }
        for index in 0..self.samples_per_pixel as usize {
            if Self::new_table(&self.actable_offsets, index, "JpegAcTables")? {
                self.actable[index] = Some(Self::huffman_table(tiff, self.actable_offsets[index], 16 | index as u8)?);
                self.sos_tda[index] |= index as u8;
            } else {
                self.sos_tda[index] |= self.sos_tda[index - 1] & 15;
            }
        }
        Ok(())
    }

    /// `OJPEGReadSecondarySos`: the input after plane `plane`'s SOS marker,
    /// found by scanning on from the last plane whose start is known.
    fn read_secondary_sos(&mut self, file: &File, plane: u8, sos_end: &mut [Option<Input>; 3]) -> Result<()> {
        let mut known = plane - 1;
        while sos_end[known as usize].is_none() {
            known -= 1;
        }
        self.plane_sample_offset = known;
        self.input = sos_end[known as usize].clone().expect("a plane's start is known");
        while self.plane_sample_offset < plane {
            loop {
                let byte = read_byte(&mut self.input, file).ok_or_else(silent)?;
                if byte != 255 {
                    continue;
                }
                let marker = loop {
                    let next = read_byte(&mut self.input, file).ok_or_else(silent)?;
                    if next != 255 {
                        break next;
                    }
                };
                if marker == MARKER_SOS {
                    break;
                }
            }
            self.plane_sample_offset += 1;
            self.read_sos(file)?;
            sos_end[self.plane_sample_offset as usize] = Some(self.input.clone());
        }
        Ok(())
    }

    /// The stream libjpeg reads for plane `plane` (`OJPEGWriteStream`): a
    /// header of the tables, restart interval, frame and scan, then the
    /// input from the plane's scan on, a restart marker after each strile
    /// but the last and EOI after it.
    fn write_stream(&self, file: &File, start: &Input) -> Vec<u8> {
        let per_plane = self.samples_per_pixel_per_plane as usize;
        let offset = self.plane_sample_offset as usize;
        let mut stream = vec![255, MARKER_SOI];
        for table in self.qtable.iter().chain(self.dctable.iter()).chain(self.actable.iter()).flatten() {
            stream.extend_from_slice(table);
        }
        if self.restart_interval != 0 {
            stream.extend_from_slice(&[255, MARKER_DRI, 0, 4, (self.restart_interval >> 8) as u8, (self.restart_interval & 255) as u8]);
        }
        stream.extend_from_slice(&[255, self.sof_marker_id, 0, (8 + per_plane * 3) as u8, 8]);
        stream.extend_from_slice(&[(self.sof_y >> 8) as u8, (self.sof_y & 255) as u8, (self.sof_x >> 8) as u8, (self.sof_x & 255) as u8]);
        stream.push(per_plane as u8);
        for component in 0..per_plane {
            stream.extend_from_slice(&[self.sof_c[offset + component], self.sof_hv[offset + component], self.sof_tq[offset + component]]);
        }
        stream.extend_from_slice(&[255, MARKER_SOS, 0, (6 + per_plane * 2) as u8, per_plane as u8]);
        for component in 0..per_plane {
            stream.extend_from_slice(&[self.sos_cs[offset + component], self.sos_tda[offset + component]]);
        }
        stream.extend_from_slice(&[0, 63, 0]);

        // `OJPEGWriteStreamCompressed`, `…Rst` and `…Eoi`.
        let mut input = start.clone();
        let mut restart_index = 0u8;
        loop {
            if input.togo() == 0 && !fill(&mut input, file) {
                // libjpeg would stop with "Premature end of JPEG data" if
                // it needed more; the decoder ends the data instead.
                break;
            }
            stream.extend_from_slice(&input.buffer[input.cursor..]);
            input.cursor = input.buffer.len();
            if input.file_togo != 0 {
                continue;
            }
            match input.source {
                Source::Strile if input.next_strile < file.strile_count => {
                    stream.extend_from_slice(&[255, MARKER_RST0 + restart_index]);
                    restart_index = (restart_index + 1) % 8;
                }
                Source::Strile | Source::Eof => {
                    stream.extend_from_slice(&[255, MARKER_EOI]);
                    break;
                }
                _ => {}
            }
        }
        stream
    }
}

/// The file's bytes at `offset` (`TIFFSeekFile` and `TIFFReadFile`).
fn read_at<'a>(tiff: &'a Tiff, offset: u64, count: u64) -> &'a [u8] {
    let size = tiff.data.len() as u64;
    let start = offset.min(size) as usize;
    let end = offset.saturating_add(count).min(size) as usize;
    &tiff.data[start..end]
}

/// One plane's libjpeg session: the decoded samples and how far the
/// striles have read them.
struct Session {
    raw: bool,
    bytes_per_line: usize,
    lines_per_strile: u32,
    /// Raw: each component's samples (whole blocks) and row stride, block
    /// rows, sampling.
    planes: Vec<(Vec<u16>, usize)>,
    block_rows: Vec<usize>,
    v_sampling: Vec<usize>,
    total_imcu_rows: usize,
    imcu_rows_read: usize,
    /// Scanlines: interleaved samples, a row's length, rows read.
    samples: Vec<u16>,
    line_length: usize,
    height: usize,
    lines_read: usize,
}

/// `subsampling_convert_*`: the buffer libjpeg's raw output lands in,
/// kept for good once made. libjpeg leaves the rows of an iMCU row past
/// the image alone, so they keep what an earlier iMCU row left.
struct RawBuffer {
    y_line_length: usize,
    c_line_length: usize,
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    clumps_per_line: usize,
}

/// The codec's state between strips.
pub(super) struct OldJpeg {
    state: Option<State>,
    sos_end: [Option<Input>; 3],
    session: Option<Session>,
    /// A session whose start failed stays in the way of any new one.
    session_stuck: bool,
    write_cursample: u16,
    write_curstrile: u32,
    raw_buffer: Option<RawBuffer>,
    convert_state: u32,
    raw_error: bool,
}

impl OldJpeg {
    /// `OJPEGSetupDecode`.
    pub fn setup(_tiff: &Tiff) -> Result<Self> {
        Ok(Self { state: None, sos_end: Default::default(), session: None, session_stuck: false, write_cursample: 0, write_curstrile: 0, raw_buffer: None, convert_state: 0, raw_error: false })
    }

    /// `OJPEGPreDecode`, `OJPEGDecode` and `OJPEGPostDecode` for strile
    /// `strile` of `plane`, into `output`: false for a read that fails
    /// without an error. libtiff clears the buffer either way.
    pub fn decode(&mut self, tiff: &Tiff, output: &mut [u8], plane: u16, strile: u32) -> Result<bool> {
        let result = self.pre_decode(tiff, plane, strile).and_then(|()| self.decode_strile(output));
        if let Err(error) = result {
            output.fill(0);
            if error.is::<Silent>() {
                return Ok(false);
            }
            return Err(error);
        }
        // `OJPEGPostDecode`: a plane's last strile ends its session.
        self.write_curstrile += 1;
        if self.write_curstrile.is_multiple_of(tiff.dir.strips_per_image.max(1)) {
            self.session = None;
        }
        Ok(true)
    }

    fn pre_decode(&mut self, tiff: &Tiff, plane: u16, strile: u32) -> Result<()> {
        let file = File::new(tiff);
        if self.state.is_none() {
            let mut state = State::new(tiff);
            let (horizontal, vertical) = tiff.dir.old_jpeg.subsampling;
            state.subsampling_hor = horizontal as u8;
            state.subsampling_ver = vertical as u8;
            state.force_desubsampling = tiff.dir.old_jpeg.force_desubsampling;
            state.read_header_info(tiff, &file)?;
            self.sos_end[0] = Some(state.input.clone());
            self.state = Some(state);
        }
        if self.sos_end[plane as usize].is_none() {
            let state = self.state.as_mut().expect("the header was read");
            state.read_secondary_sos(&file, plane as u8, &mut self.sos_end)?;
        }
        if self.session.is_some() && (self.write_cursample != plane || self.write_curstrile > strile) {
            self.session = None;
        }
        if self.session.is_none() {
            let state = self.state.as_mut().expect("the header was read");
            state.plane_sample_offset = plane as u8;
            self.write_cursample = plane;
            self.write_curstrile = plane as u32 * tiff.dir.strips_per_image;
            let start = self.sos_end[plane as usize].clone().expect("the plane's start is known");
            self.session = Some(self.start_session(&file, &start)?);
        }
        self.convert_state = 0;
        while self.write_curstrile < strile {
            self.skip_strile();
            self.write_curstrile += 1;
        }
        Ok(())
    }

    /// `OJPEGWriteHeaderInfo`: libjpeg reads the plane's stream, raw for
    /// subsampled colour, scanlines otherwise.
    fn start_session(&mut self, file: &File, start: &Input) -> Result<Session> {
        if self.session_stuck {
            bail!("the JPEG decoder could not start");
        }
        self.session_stuck = true;
        let state = self.state.as_ref().expect("the header was read");
        let stream = state.write_stream(file, start);
        let strip = Strip::open(&stream, &Tables::default()).map_err(|error| anyhow!("{error}"))?;
        let info = strip.info();
        let raw = !state.force_desubsampling && state.samples_per_pixel_per_plane > 1;
        let mode = if raw { TiffOutput::Raw } else { TiffOutput::Unconverted };
        let mut tables = Tables::default();
        let samples = strip.decode(mode, false, &mut tables).map_err(|error| anyhow!("{error}"))?;
        if info.width != state.strile_width as usize {
            bail!("jpeg_start_decompress() returned image_width = {}, expected {}", info.width, state.strile_width);
        }
        let max_h = info.sampling.iter().map(|&(h, _, _)| h).max().unwrap_or(1);
        let max_v = info.sampling.iter().map(|&(_, v, _)| v).max().unwrap_or(1);
        if max_h != state.subsampling_hor as usize || max_v != state.subsampling_ver as usize {
            bail!(
                "jpeg_start_decompress() returned max_h_samp_factor = {max_h} and max_v_samp_factor = {max_v}, expected {} and {}",
                state.subsampling_hor,
                state.subsampling_ver
            );
        }
        self.session_stuck = false;

        let hor = state.subsampling_hor as usize;
        let ver = state.subsampling_ver as usize;
        let mut session = Session {
            raw,
            bytes_per_line: state.samples_per_pixel_per_plane as usize * state.strile_width as usize,
            lines_per_strile: state.strile_length,
            planes: Vec::new(),
            block_rows: Vec::new(),
            v_sampling: Vec::new(),
            total_imcu_rows: info.height.div_ceil(8 * max_v),
            imcu_rows_read: 0,
            samples: Vec::new(),
            line_length: 0,
            height: info.height,
            lines_read: 0,
        };
        match samples {
            StripSamples::Raw { planes } => {
                if self.raw_buffer.is_none() {
                    let y_line_length = (state.strile_width as usize).div_ceil(hor * 8) * hor * 8;
                    let c_line_length = y_line_length / hor;
                    self.raw_buffer = Some(RawBuffer {
                        y_line_length,
                        c_line_length,
                        y: vec![0; y_line_length * ver * 8],
                        cb: vec![0; c_line_length * 8],
                        cr: vec![0; c_line_length * 8],
                        clumps_per_line: (state.strile_width as usize).div_ceil(hor),
                    });
                    self.raw_error = false;
                }
                let buffer = self.raw_buffer.as_ref().expect("the raw buffer was made");
                session.bytes_per_line = buffer.clumps_per_line * (ver * hor + 2);
                session.lines_per_strile = state.strile_length.div_ceil(ver as u32);
                session.block_rows = planes.iter().map(|(samples, stride)| samples.len() / (*stride).max(1) / 8).collect();
                session.v_sampling = info.sampling.iter().map(|&(_, v, _)| v).collect();
                session.planes = planes;
            }
            StripSamples::Interleaved { components, samples } => {
                session.line_length = info.width * components;
                session.samples = samples;
            }
        }
        Ok(session)
    }

    /// `jpeg_read_raw_data` for one iMCU row into the raw buffer; past the
    /// last one libjpeg only warns, and the buffer stays as it was.
    fn read_imcu_row(&mut self) {
        let session = self.session.as_mut().expect("a session is running");
        let buffer = self.raw_buffer.as_mut().expect("the raw buffer was made");
        let row = session.imcu_rows_read;
        if row >= session.total_imcu_rows {
            return;
        }
        session.imcu_rows_read += 1;
        let targets = [(&mut buffer.y, buffer.y_line_length), (&mut buffer.cb, buffer.c_line_length), (&mut buffer.cr, buffer.c_line_length)];
        for (component, (target, line_length)) in targets.into_iter().enumerate() {
            let (samples, stride) = &session.planes[component];
            let v = session.v_sampling[component];
            let first_block_row = row * v;
            let block_rows = session.block_rows[component].saturating_sub(first_block_row).min(v);
            for line in 0..block_rows * 8 {
                let source = &samples[(first_block_row * 8 + line) * stride..][..*stride];
                let destination = &mut target[line * line_length..][..(*stride).min(line_length)];
                for (byte, &sample) in destination.iter_mut().zip(source.iter()) {
                    *byte = sample as u8;
                }
            }
        }
    }

    /// `OJPEGPreDecodeSkipRaw` or `OJPEGPreDecodeSkipScanlines`: a strile
    /// read and dropped.
    fn skip_strile(&mut self) {
        let session = self.session.as_mut().expect("a session is running");
        if !session.raw {
            session.lines_read += session.lines_per_strile as usize;
            return;
        }
        let mut lines = session.lines_per_strile;
        let clines = 8;
        if self.convert_state != 0 {
            if clines - self.convert_state >= lines {
                self.convert_state += lines;
                if self.convert_state == clines {
                    self.convert_state = 0;
                }
                return;
            }
            lines -= clines - self.convert_state;
            self.convert_state = 0;
            self.raw_error = false;
        }
        while lines >= clines {
            self.read_imcu_row();
            lines -= clines;
        }
        if lines > 0 {
            self.read_imcu_row();
            self.convert_state = lines;
        }
    }

    /// `OJPEGDecode`: the strile's lines.
    fn decode_strile(&mut self, output: &mut [u8]) -> Result<()> {
        let session = self.session.as_ref().expect("a session is running");
        if session.raw && self.raw_error {
            return Err(silent());
        }
        let bytes_per_line = session.bytes_per_line;
        if bytes_per_line == 0 || !output.len().is_multiple_of(bytes_per_line) {
            bail!("Fractional scanline not read");
        }
        if session.raw {
            self.decode_raw(output);
        } else {
            self.decode_scanlines(output);
        }
        Ok(())
    }

    /// `OJPEGDecodeRaw`: each line one row of clumps — a block of Y
    /// samples, then Cb and Cr — from the raw buffer, a new iMCU row every
    /// eight lines.
    fn decode_raw(&mut self, output: &mut [u8]) {
        let (hor, ver) = {
            let state = self.state.as_ref().expect("the header was read");
            (state.subsampling_hor as usize, state.subsampling_ver as usize)
        };
        let bytes_per_line = self.session.as_ref().expect("a session is running").bytes_per_line;
        for line in output.chunks_exact_mut(bytes_per_line) {
            if self.convert_state == 0 {
                self.read_imcu_row();
            }
            let buffer = self.raw_buffer.as_ref().expect("the raw buffer was made");
            let state = self.convert_state as usize;
            let mut y_start = state * ver * buffer.y_line_length;
            let c_start = state * buffer.c_line_length;
            let mut position = 0;
            for clump in 0..buffer.clumps_per_line {
                for sy in 0..ver {
                    let row = y_start + sy * buffer.y_line_length;
                    line[position..position + hor].copy_from_slice(&buffer.y[row..row + hor]);
                    position += hor;
                }
                y_start += hor;
                line[position] = buffer.cb[c_start + clump];
                line[position + 1] = buffer.cr[c_start + clump];
                position += 2;
            }
            self.convert_state += 1;
            if self.convert_state == 8 {
                self.convert_state = 0;
            }
        }
    }

    /// `OJPEGDecodeScanlines`: a scanline per line; past the image libjpeg
    /// returns nothing and the line keeps what it had.
    fn decode_scanlines(&mut self, output: &mut [u8]) {
        let session = self.session.as_mut().expect("a session is running");
        for line in output.chunks_exact_mut(session.bytes_per_line) {
            if session.lines_read < session.height {
                let start = session.lines_read * session.line_length;
                let source = &session.samples[start..start + session.line_length];
                for (byte, &sample) in line.iter_mut().zip(source.iter()) {
                    *byte = sample as u8;
                }
            }
            session.lines_read += 1;
        }
    }
}
