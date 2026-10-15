//! ISOBMFF/HEIF boxes as libheif 1.21 reads them (`box.cc`,
//! `file_layout.cc`, `codecs/*_boxes.cc`): only the boxes the read path
//! looks at are parsed; the others are kept as known or unknown types,
//! which matters for libheif's essential-property check.

use anyhow::{anyhow, bail, Result};

use super::nclx::Nclx;

pub fn fourcc(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}

const MAX_BOX_NESTING_LEVEL: u32 = 20;
const MAX_LARGE_BOX_SIZE: u64 = 0x0FFF_FFFF_FFFF_FFFF;
const MAX_ITEMS: u32 = 1000;
const MAX_CHILDREN_PER_BOX: usize = 100;
const MAX_ILOC_EXTENTS_PER_ITEM: u16 = 32;
const MAX_NUMBER_OF_FILE_BRANDS: u64 = 1000;
const MAX_COLOR_PROFILE_SIZE: u64 = 100 * 1024 * 1024;
const MAX_FRACTION_VALUE: i32 = 0x10000;
const MAX_SEQUENCE_FRAMES: u32 = 18_000_000;

/// `BitstreamRange` over the file: reads past the end set the error flag
/// and give zeros.
#[derive(Clone, Copy)]
pub struct Range<'a> {
    pub data: &'a [u8],
    pub position: usize,
    pub end: usize,
    pub error: bool,
    pub nesting: u32,
}

impl<'a> Range<'a> {
    pub fn new(data: &'a [u8], start: usize, end: usize) -> Range<'a> {
        Range { data, position: start, end: end.min(data.len()).max(start), error: false, nesting: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.end - self.position
    }

    pub fn eof(&self) -> bool {
        self.position >= self.end
    }

    /// `prepare_read`.
    fn prepare(&mut self, count: usize) -> bool {
        if self.remaining() < count {
            self.position = self.end;
            self.error = true;
            return false;
        }
        true
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if !self.prepare(count) {
            return None;
        }
        let bytes = &self.data[self.position..self.position + count];
        self.position += count;
        Some(bytes)
    }

    pub fn read8(&mut self) -> u8 {
        self.take(1).map_or(0, |bytes| bytes[0])
    }

    pub fn read16(&mut self) -> u16 {
        self.take(2).map_or(0, |bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn read32(&mut self) -> u32 {
        self.take(4).map_or(0, |bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// `read_uint` of 8, 16, 24, 32 or 64 bits.
    pub fn read_uint(&mut self, bits: u32) -> u64 {
        match bits {
            8 => self.read8() as u64,
            16 => self.read16() as u64,
            24 => self.take(3).map_or(0, |bytes| u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) as u64),
            32 => self.read32() as u64,
            64 => {
                let high = self.read32() as u64;
                let low = self.read32() as u64;
                (high << 32) | low
            }
            _ => 0,
        }
    }

    pub fn read_bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        self.take(count)
    }

    /// `read_string`: up to a zero byte or the end of the box.
    pub fn read_string(&mut self) -> String {
        let mut text = Vec::new();
        if self.eof() {
            return String::new();
        }
        loop {
            if !self.prepare(1) {
                return String::new();
            }
            let byte = self.data[self.position];
            self.position += 1;
            // The box's last byte ends the string without joining it.
            if byte == 0 || self.remaining() == 0 {
                break;
            }
            text.push(byte);
        }
        String::from_utf8_lossy(&text).into_owned()
    }

    pub fn skip_to_end(&mut self) {
        self.position = self.end;
    }
}

/// How a box's parse error affects the file (`parse_error_fatality`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fatality {
    Fatal,
    Ignorable,
    Optional,
}

#[derive(Clone, Debug)]
pub struct Header {
    pub kind: u32,
    pub size: u64,
    pub header_size: u64,
    /// Where the box starts in the file.
    pub start: usize,
}

impl Header {
    pub fn fixed_size(&self) -> bool {
        self.size != 0
    }
}

#[derive(Clone, Debug)]
pub struct IlocExtent {
    pub offset: u64,
    pub length: u64,
}

#[derive(Clone, Debug)]
pub struct IlocItem {
    pub item_id: u32,
    pub construction_method: u8,
    pub base_offset: u64,
    pub extents: Vec<IlocExtent>,
}

#[derive(Clone, Debug)]
pub struct Infe {
    pub item_id: u32,
    pub hidden: bool,
    pub item_type: u32,
    pub content_type: String,
    pub content_encoding: String,
}

#[derive(Clone, Debug)]
pub struct IpmaEntry {
    pub item_id: u32,
    /// (essential, property index)
    pub associations: Vec<(bool, u16)>,
}

#[derive(Clone, Debug)]
pub struct Reference {
    pub kind: u32,
    pub from: u32,
    pub to: Vec<u32>,
}

/// libheif's `Fraction`.
#[derive(Clone, Copy, Debug)]
pub struct Fraction {
    pub numerator: i32,
    pub denominator: i32,
}

impl Fraction {
    pub fn new_i32(numerator: i32, denominator: i32) -> Fraction {
        let mut fraction = Fraction { numerator, denominator };
        while fraction.denominator > MAX_FRACTION_VALUE || fraction.denominator < -MAX_FRACTION_VALUE {
            fraction.numerator /= 2;
            fraction.denominator /= 2;
        }
        while fraction.denominator > 1 && (fraction.numerator > MAX_FRACTION_VALUE || fraction.numerator < -MAX_FRACTION_VALUE) {
            fraction.numerator /= 2;
            fraction.denominator /= 2;
        }
        fraction
    }

    pub fn new_i64(mut numerator: i64, mut denominator: i64) -> Fraction {
        let fits = |value: i64| value >= i32::MIN as i64 && value <= i32::MAX as i64;
        while !fits(numerator) || !fits(denominator) {
            numerator = (numerator + if numerator >= 0 { 1 } else { -1 }) / 2;
            denominator = (denominator + if denominator >= 0 { 1 } else { -1 }) / 2;
        }
        Fraction { numerator: numerator as i32, denominator: denominator as i32 }
    }

    pub fn add(self, other: Fraction) -> Fraction {
        if self.denominator == other.denominator {
            return Fraction::new_i64(self.numerator as i64 + other.numerator as i64, self.denominator as i64);
        }
        let numerator = self.numerator as i64 * other.denominator as i64 + other.numerator as i64 * self.denominator as i64;
        let denominator = self.denominator as i64 * other.denominator as i64;
        Fraction::new_i64(numerator, denominator)
    }

    pub fn sub(self, other: Fraction) -> Fraction {
        if self.denominator == other.denominator {
            return Fraction::new_i64(self.numerator as i64 - other.numerator as i64, self.denominator as i64);
        }
        let numerator = self.numerator as i64 * other.denominator as i64 - other.numerator as i64 * self.denominator as i64;
        let denominator = self.denominator as i64 * other.denominator as i64;
        Fraction::new_i64(numerator, denominator)
    }

    pub fn add_int(self, value: i32) -> Fraction {
        Fraction::new_i64(self.numerator as i64 + value as i64 * self.denominator as i64, self.denominator as i64)
    }

    pub fn sub_int(self, value: i32) -> Fraction {
        Fraction::new_i64(self.numerator as i64 - value as i64 * self.denominator as i64, self.denominator as i64)
    }

    pub fn div_int(self, value: i32) -> Fraction {
        Fraction::new_i64(self.numerator as i64, self.denominator as i64 * value as i64)
    }

    pub fn round_down(self) -> i32 {
        if self.denominator == 0 {
            return 0;
        }
        self.numerator.wrapping_div(self.denominator)
    }

    pub fn round(self) -> i32 {
        if self.denominator == 0 {
            return 0;
        }
        ((self.numerator as i64 + self.denominator as i64 / 2) / self.denominator as i64) as i32
    }
}

/// `Box_clap`.
#[derive(Clone, Copy, Debug)]
pub struct Clap {
    pub width: Fraction,
    pub height: Fraction,
    pub horizontal_offset: Fraction,
    pub vertical_offset: Fraction,
}

impl Clap {
    pub fn left_rounded(&self, image_width: u32) -> i32 {
        let centre = self.horizontal_offset.add(Fraction::new_i32(image_width.wrapping_sub(1) as i32, 2));
        let left = centre.sub(self.width.sub_int(1).div_int(2));
        left.round_down()
    }

    pub fn right_rounded(&self, image_width: u32) -> i32 {
        let right = self.width.sub_int(1).add_int(self.left_rounded(image_width));
        right.round()
    }

    pub fn top_rounded(&self, image_height: u32) -> i32 {
        let centre = self.vertical_offset.add(Fraction::new_i32(image_height.wrapping_sub(1) as i32, 2));
        let top = centre.sub(self.height.sub_int(1).div_int(2));
        top.round()
    }

    pub fn bottom_rounded(&self, image_height: u32) -> i32 {
        let bottom = self.height.sub_int(1).add_int(self.top_rounded(image_height));
        bottom.round()
    }
}

/// `HEVCDecoderConfigurationRecord`.
#[derive(Clone, Debug)]
pub struct HvcC {
    pub bit_depth_luma: u8,
    pub nal_units: Vec<Vec<u8>>,
}

/// `Box_av1C`.
#[derive(Clone, Debug)]
pub struct Av1C {
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub config_obus: Vec<u8>,
}

/// `Box_colr`'s profile.
#[derive(Clone, Debug)]
pub enum ColorProfile {
    Nclx(Nclx),
    Raw(Vec<u8>),
}

#[derive(Clone, Debug)]
pub enum Content {
    Ftyp { major: u32, brands: Vec<u32> },
    Meta(Vec<HeifBox>),
    Hdlr { handler: u32 },
    Pitm { item_id: u32 },
    Iloc(Vec<IlocItem>),
    Iinf(Vec<HeifBox>),
    Infe(Infe),
    Iprp(Vec<HeifBox>),
    Ipco(Vec<HeifBox>),
    Ipma(Vec<IpmaEntry>),
    Ispe { width: u32, height: u32 },
    AuxC { aux_type: String },
    Irot { rotation: i32 },
    Imir { horizontal: bool },
    Clap(Clap),
    Iref(Vec<Reference>),
    Idat { start: usize, size: u64 },
    Colr(ColorProfile),
    HvcC(HvcC),
    Av1C(Av1C),
    /// `moov`, `trak`, `mdia`, `minf`, `stbl`, `edts` and `dinf`.
    Container(Vec<HeifBox>),
    Mvhd { timescale: u32, duration: u64 },
    Tkhd { track_id: u32 },
    Mdhd { timescale: u32, duration: u64 },
    Stsd(Vec<HeifBox>),
    /// A visual sample entry (`hvc1`, `av01`, …) with its child boxes.
    VisualSampleEntry { width: u16, height: u16, children: Vec<HeifBox> },
    /// (sample count, sample delta)
    Stts(Vec<(u32, u32)>),
    /// (first chunk, samples per chunk, sample description index)
    Stsc(Vec<(u32, u32, u32)>),
    Stco(Vec<u32>),
    Stsz { fixed: u32, count: u32, sizes: Vec<u32> },
    /// The repeat flag and the (segment duration, media time) entries.
    Elst { repeat: bool, entries: Vec<(u64, i64)> },
    /// (reference type, track ids)
    Tref(Vec<(u32, Vec<u32>)>),
    Auxi(String),
    /// Sample auxiliary information sizes: a default size, or one size a
    /// sample when that is 0.
    Saiz { aux_type: u32, parameter: u32, default_size: u8, count: u32, sizes: Vec<u8> },
    Saio { aux_type: u32, parameter: u32, offsets: Vec<u64> },
    /// A type libheif has a class for that this reader does not model.
    Known,
    /// `Box_other`: a type libheif does not know.
    Other,
    /// `Box_Error`.
    Error { message: String, fatality: Fatality },
}

#[derive(Clone, Debug)]
pub struct HeifBox {
    pub header: Header,
    pub content: Content,
}

impl HeifBox {
    pub fn kind(&self) -> u32 {
        self.header.kind
    }

    pub fn children(&self) -> &[HeifBox] {
        match &self.content {
            Content::Meta(children) | Content::Iinf(children) | Content::Iprp(children) | Content::Ipco(children) | Content::Container(children) | Content::Stsd(children) => children,
            Content::VisualSampleEntry { children, .. } => children,
            _ => &[],
        }
    }

    pub fn child(&self, kind: &[u8; 4]) -> Option<&HeifBox> {
        let wanted = fourcc(kind);
        self.children().iter().find(|child| child.kind() == wanted)
    }
}

/// `BoxHeader::parse_header`.
fn parse_header(range: &mut Range) -> Result<Header> {
    let start = range.position;
    if range.remaining() < 8 {
        bail!("End of data");
    }
    let mut size = range.read32() as u64;
    let kind = range.read32();
    let mut header_size = 8u64;
    if size == 1 {
        if range.remaining() < 8 {
            bail!("End of data");
        }
        let high = range.read32() as u64;
        let low = range.read32() as u64;
        size = (high << 32) | low;
        header_size += 8;
        if size > MAX_LARGE_BOX_SIZE {
            bail!("Box size {size} exceeds security limit.");
        }
    }
    if kind == fourcc(b"uuid") {
        if range.remaining() < 16 {
            bail!("End of data");
        }
        range.read_bytes(16);
        header_size += 16;
    }
    if range.error {
        bail!("End of data");
    }
    Ok(Header { kind, size, header_size, start })
}

/// The fatality of a box type's parse errors.
fn fatality_of(kind: u32) -> Fatality {
    let ignorable = [b"irot", b"imir", b"clap", b"pymd", b"uuid"];
    let optional = [b"pixi", b"pasp", b"lsel", b"clli", b"mdcv", b"amve", b"cclv", b"cmin", b"cmex", b"udes", b"elng"];
    if ignorable.iter().any(|name| fourcc(name) == kind) {
        return Fatality::Ignorable;
    }
    if optional.iter().any(|name| fourcc(name) == kind) {
        return Fatality::Optional;
    }
    Fatality::Fatal
}

/// Types libheif has box classes for (anything else is `Box_other`).
fn is_known_type(kind: u32) -> bool {
    const KNOWN: [&[u8; 4]; 73] = [
        b"ftyp", b"free", b"skip", b"meta", b"hdlr", b"pitm", b"iloc", b"iinf", b"infe", b"iprp", b"ipco", b"ipma", b"ispe", b"auxC", b"irot", b"imir", b"clap", b"iref", b"hvcC", b"hvc1", b"av1C", b"av01", b"vvcC", b"vvc1", b"idat", b"grpl",
        b"pymd", b"altr", b"ster", b"dinf", b"dref", b"url ", b"colr", b"pixi", b"pasp", b"lsel", b"a1op", b"a1lx", b"clli", b"mdcv", b"amve", b"cmin", b"cmex", b"udes", b"jpgC", b"mjpg", b"elng", b"j2kH", b"cdef", b"cmap", b"pclr", b"j2kL",
        b"j2ki", b"mskC", b"itai", b"taic", b"avcC", b"avc1", b"mdat", b"moov", b"mvhd", b"trak", b"tkhd", b"mdia", b"mdhd", b"minf", b"vmhd", b"stbl", b"stsd", b"stts", b"ctts", b"stsc", b"stco",
    ];
    const MORE: [&[u8; 4]; 19] = [b"stsz", b"stss", b"ccst", b"auxi", b"edts", b"elst", b"sbgp", b"sgpd", b"btrt", b"saiz", b"saio", b"urim", b"uri ", b"nmhd", b"tref", b"sdtp", b"cclv", b"uuid", b"ster"];
    KNOWN.iter().chain(MORE.iter()).any(|name| fourcc(name) == kind)
}

/// `Box::read`: one box from `range`, advancing past it.
pub fn read_box(range: &mut Range) -> Result<HeifBox> {
    let header = parse_header(range)?;
    if range.nesting > MAX_BOX_NESTING_LEVEL {
        bail!("Security limit for maximum nesting of boxes has been exceeded");
    }
    let content_size = if header.fixed_size() {
        if header.size < header.header_size {
            bail!("Box size ({} bytes) smaller than header size ({} bytes)", header.size, header.header_size);
        }
        let size = header.size - header.header_size;
        if size > i64::MAX as u64 {
            bail!("Invalid box size");
        }
        if header.start as u64 + header.size > range.data.len() as u64 {
            bail!("End of data");
        }
        size
    } else {
        range.remaining() as u64
    };
    if (range.remaining() as u64) < content_size {
        bail!("Invalid box size");
    }
    let content_start = range.position;
    let content_end = content_start + content_size as usize;
    let mut child = Range { data: range.data, position: content_start, end: content_end, error: false, nesting: range.nesting + 1 };
    let parsed = parse_content(&header, &mut child);
    range.position = content_end;
    match parsed {
        Ok(content) => Ok(HeifBox { header, content }),
        Err(error) => {
            let fatality = fatality_of(header.kind);
            let content = Content::Error { message: error.to_string(), fatality };
            Err(BoxError { boxed: HeifBox { header, content }, message: error.to_string() }.into())
        }
    }
}

/// A box that failed to parse, with its `Box_Error` stand-in.
#[derive(Debug)]
pub struct BoxError {
    pub boxed: HeifBox,
    pub message: String,
}

impl std::fmt::Display for BoxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for BoxError {}

/// `read_children`: a fatal child error fails the parent; other errors
/// keep their `Box_Error`.
fn read_children(range: &mut Range, max_number: Option<u32>, parent: u32) -> Result<Vec<HeifBox>> {
    let mut children = Vec::new();
    let mut count = 0u32;
    while !range.eof() && !range.error {
        let child = match read_box(range) {
            Ok(child) => child,
            Err(error) => {
                let box_error = error.downcast::<BoxError>()?;
                if let Content::Error { fatality: Fatality::Fatal, .. } = box_error.boxed.content {
                    bail!("{}", box_error.message);
                }
                box_error.boxed
            },
        };
        if max_number.is_none() {
            let max_children = if parent == fourcc(b"iinf") { MAX_ITEMS as usize } else { MAX_CHILDREN_PER_BOX };
            if children.len() > max_children {
                bail!("Maximum number of child boxes ({max_children}) exceeded.");
            }
        }
        children.push(child);
        count += 1;
        if let Some(maximum) = max_number
            && count == maximum
        {
            break;
        }
    }
    if range.error {
        bail!("End of data");
    }
    Ok(children)
}

fn full_box_header(range: &mut Range) -> (u8, u32) {
    let value = range.read32();
    ((value >> 24) as u8, value & 0x00ff_ffff)
}

fn range_error(range: &Range) -> Result<()> {
    if range.error {
        bail!("End of data");
    }
    Ok(())
}

fn parse_content(header: &Header, range: &mut Range) -> Result<Content> {
    let kind = header.kind;
    let name = kind.to_be_bytes();
    let content = match &name {
        b"ftyp" => {
            let major = range.read32();
            let _minor = range.read32();
            if header.size < 8 || header.size - 8 < header.header_size {
                bail!("ftyp box too small (less than 8 bytes)");
            }
            let brand_count = (header.size - header.header_size - 8) / 4;
            if brand_count > MAX_NUMBER_OF_FILE_BRANDS {
                bail!("Number of minor brands in file exceeds security limit");
            }
            let mut brands = Vec::new();
            for _ in 0..brand_count {
                if range.error {
                    break;
                }
                brands.push(range.read32());
            }
            range_error(range)?;
            Content::Ftyp { major, brands }
        }
        b"meta" => {
            let (version, _) = full_box_header(range);
            if version != 0 {
                bail!("meta box data version {version} is not implemented yet");
            }
            Content::Meta(read_children(range, None, kind)?)
        }
        b"hdlr" => {
            let (version, _) = full_box_header(range);
            if version != 0 {
                bail!("hdlr box data version {version} is not implemented yet");
            }
            range.read32();
            let handler = range.read32();
            for _ in 0..3 {
                range.read32();
            }
            range.read_string();
            range_error(range)?;
            Content::Hdlr { handler }
        }
        b"pitm" => {
            let (version, _) = full_box_header(range);
            if version > 1 {
                bail!("pitm box data version {version} is not implemented yet");
            }
            let item_id = if version == 0 { range.read16() as u32 } else { range.read32() };
            range_error(range)?;
            Content::Pitm { item_id }
        }
        b"iloc" => parse_iloc(range)?,
        b"iinf" => {
            let (version, _) = full_box_header(range);
            let count = if version > 0 { range.read32() } else { range.read16() as u32 };
            if count == 0 {
                Content::Iinf(Vec::new())
            } else {
                Content::Iinf(read_children(range, Some(count), kind)?)
            }
        }
        b"infe" => parse_infe(range)?,
        b"iprp" => Content::Iprp(read_children(range, None, kind)?),
        b"ipco" => Content::Ipco(read_children(range, None, kind)?),
        b"ipma" => parse_ipma(range)?,
        b"ispe" => {
            let (version, _) = full_box_header(range);
            if version != 0 {
                bail!("ispe box data version {version} is not implemented yet");
            }
            let width = range.read32();
            let height = range.read32();
            range_error(range)?;
            Content::Ispe { width, height }
        }
        b"auxC" => {
            let (version, _) = full_box_header(range);
            if version != 0 {
                bail!("auxC box data version {version} is not implemented yet");
            }
            let aux_type = range.read_string();
            // the subtypes
            while !range.eof() {
                range.read8();
            }
            range_error(range)?;
            Content::AuxC { aux_type }
        }
        b"irot" => {
            let rotation = (range.read8() & 3) as i32 * 90;
            range_error(range)?;
            Content::Irot { rotation }
        }
        b"imir" => {
            let axis = range.read8();
            range_error(range)?;
            Content::Imir { horizontal: axis & 1 != 0 }
        }
        b"clap" => parse_clap(range)?,
        b"iref" => parse_iref(range)?,
        b"idat" => Content::Idat { start: range.position, size: header.size },
        b"colr" => parse_colr(header, range)?,
        b"hvcC" => parse_hvcc(range)?,
        b"av1C" => parse_av1c(header, range)?,
        b"pixi" => {
            let (version, _) = full_box_header(range);
            if version != 0 {
                bail!("pixi box data version {version} is not implemented yet");
            }
            let count = range.read8() as usize;
            if range.remaining() < count {
                bail!("End of data");
            }
            for _ in 0..count {
                range.read8();
            }
            range_error(range)?;
            Content::Known
        }
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" => Content::Container(read_children(range, None, kind)?),
        b"mvhd" | b"mdhd" => {
            let (version, _) = full_box_header(range);
            if version > 1 {
                bail!("{} box data version {version} is not implemented yet", String::from_utf8_lossy(&name));
            }
            let (timescale, duration) = if version == 1 {
                range.read_uint(64);
                range.read_uint(64);
                let timescale = range.read32();
                (timescale, range.read_uint(64))
            } else {
                range.read32();
                range.read32();
                let timescale = range.read32();
                (timescale, range.read32() as u64)
            };
            range.skip_to_end();
            if &name == b"mvhd" { Content::Mvhd { timescale, duration } } else { Content::Mdhd { timescale, duration } }
        }
        b"tkhd" => {
            let (version, _) = full_box_header(range);
            if version > 1 {
                bail!("tkhd box data version {version} is not implemented yet");
            }
            let id_offset = if version == 1 { 16 } else { 8 };
            range.read_bytes(id_offset);
            let track_id = range.read32();
            range.skip_to_end();
            range_error(range)?;
            Content::Tkhd { track_id }
        }
        b"stsd" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("stsd box data version {version} is not implemented yet");
            }
            let count = range.read32();
            if count > 1024 {
                bail!("Number of sample description entries exceeds the security limit");
            }
            let mut entries = Vec::new();
            for _ in 0..count {
                entries.push(read_box(range)?);
            }
            range_error(range)?;
            Content::Stsd(entries)
        }
        b"hvc1" | b"av01" | b"avc1" | b"vvc1" | b"j2ki" | b"mjpg" => {
            range.read_bytes(6);
            range.read16();
            range.read16();
            range.read16();
            range.read_bytes(12);
            let width = range.read16();
            let height = range.read16();
            range.read_bytes(4 + 4 + 4 + 2 + 32 + 2 + 2);
            let children = read_children(range, None, kind)?;
            Content::VisualSampleEntry { width, height, children }
        }
        b"stts" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("stts box data version {version} is not implemented yet");
            }
            let count = range.read32();
            if count > MAX_SEQUENCE_FRAMES {
                bail!("Security limit for maximum number of sequence frames exceeded");
            }
            let mut entries = Vec::new();
            for index in 0..count {
                if range.eof() {
                    bail!("stts box should contain {count} entries, but box only contained {index} entries");
                }
                let sample_count = range.read32();
                let delta = range.read32();
                entries.push((sample_count, delta));
            }
            range_error(range)?;
            Content::Stts(entries)
        }
        b"stsc" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("stsc box data version {version} is not implemented yet");
            }
            let count = range.read32();
            let mut entries = Vec::new();
            for _ in 0..count {
                let first_chunk = range.read32();
                let samples_per_chunk = range.read32();
                let description = range.read32();
                if description == 0 {
                    bail!("'sample_description_index' in 'stsc' must not be 0.");
                }
                if samples_per_chunk > MAX_SEQUENCE_FRAMES {
                    bail!("Number of chunk samples in `stsc` box exceeds security limits of maximum number of frames.");
                }
                entries.push((first_chunk, samples_per_chunk, description));
                if range.error {
                    break;
                }
            }
            range_error(range)?;
            Content::Stsc(entries)
        }
        b"stco" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("stco box data version {version} is not implemented yet");
            }
            let count = range.read32();
            let mut offsets = Vec::new();
            for _ in 0..count {
                offsets.push(range.read32());
                range_error(range)?;
            }
            Content::Stco(offsets)
        }
        b"stsz" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("stsz box data version {version} is not implemented yet");
            }
            let fixed = range.read32();
            let count = range.read32();
            let mut sizes = Vec::new();
            if fixed == 0 {
                if count > MAX_SEQUENCE_FRAMES {
                    bail!("Security limit for maximum number of sequence frames exceeded");
                }
                for index in 0..count {
                    if range.eof() {
                        bail!("stsz box should contain {count} entries, but box only contained {index} entries");
                    }
                    sizes.push(range.read32());
                    range_error(range)?;
                }
            }
            range_error(range)?;
            Content::Stsz { fixed, count, sizes }
        }
        b"elst" => {
            let (version, flags) = full_box_header(range);
            if version > 1 {
                bail!("edts box data version {version} is not implemented yet");
            }
            let count = range.read32();
            let mut entries = Vec::new();
            for index in 0..count {
                if range.eof() {
                    bail!("edit list should contain {count} entries, but box only contained {index} entries");
                }
                let (duration, media_time) = if version == 1 { (range.read_uint(64), range.read_uint(64) as i64) } else { (range.read32() as u64, range.read32() as i32 as i64) };
                range.read16();
                range.read16();
                entries.push((duration, media_time));
            }
            range_error(range)?;
            Content::Elst { repeat: flags & 1 != 0, entries }
        }
        b"tref" => {
            let mut references = Vec::new();
            while !range.eof() {
                let header = parse_header(range)?;
                if header.size < header.header_size {
                    bail!("Invalid box size (smaller than header)");
                }
                let data_size = header.size - header.header_size;
                if data_size % 4 != 0 || data_size < 4 {
                    bail!("Input file has a 'tref' TrackReferenceTypeBox with invalid size.");
                }
                let count = data_size / 4;
                if count > MAX_ITEMS as u64 {
                    bail!("Number of references in tref box exceeds the security limits");
                }
                let mut ids = Vec::new();
                for _ in 0..count {
                    ids.push(range.read32());
                }
                references.push((header.kind, ids));
            }
            range_error(range)?;
            Content::Tref(references)
        }
        b"auxi" => {
            let (version, _) = full_box_header(range);
            if version > 0 {
                bail!("auxi box data version {version} is not implemented yet");
            }
            let urn = range.read_string();
            range_error(range)?;
            Content::Auxi(urn)
        }
        b"saiz" | b"saio" => {
            let (version, flags) = full_box_header(range);
            let (aux_type, parameter) = if flags & 1 != 0 { (range.read32(), range.read32()) } else { (0, 0) };
            if &name == b"saiz" {
                let default_size = range.read8();
                let count = range.read32();
                if count > MAX_SEQUENCE_FRAMES {
                    bail!("Number of 'saiz' samples exceeds the maximum number of sequence frames.");
                }
                let mut sizes = Vec::new();
                if default_size == 0 {
                    sizes = range.read_bytes(count as usize).map(<[u8]>::to_vec).unwrap_or_else(|| vec![0; count as usize]);
                }
                range_error(range)?;
                Content::Saiz { aux_type, parameter, default_size, count, sizes }
            } else {
                let chunks = range.read32();
                if chunks > MAX_SEQUENCE_FRAMES {
                    bail!("Number of 'saio' chunks exceeds the maximum number of sequence frames.");
                }
                let entry_bits = if version == 1 { 64 } else { 32 };
                let mut offsets = Vec::new();
                for _ in 0..chunks {
                    offsets.push(range.read_uint(entry_bits));
                    range_error(range)?;
                }
                Content::Saio { aux_type, parameter, offsets }
            }
        }
        b"free" | b"skip" => {
            range.skip_to_end();
            Content::Known
        }
        _ => {
            if is_known_type(kind) {
                Content::Known
            } else {
                Content::Other
            }
        }
    };
    Ok(content)
}

fn parse_iloc(range: &mut Range) -> Result<Content> {
    let (version, _) = full_box_header(range);
    if version > 2 {
        bail!("iloc box data version {version} is not implemented yet");
    }
    let values = range.read16();
    let offset_size = (values >> 12) & 0xf;
    let length_size = (values >> 8) & 0xf;
    let base_offset_size = (values >> 4) & 0xf;
    let index_size = if version == 1 || version == 2 { values & 0xf } else { 0 };
    let item_count = if version < 2 { range.read16() as u32 } else { range.read32() };
    if item_count > MAX_ITEMS {
        bail!("iloc box contains {item_count} items, which exceeds the security limit of {MAX_ITEMS} items.");
    }
    let read_sized = |range: &mut Range, size: u16| -> u64 {
        match size {
            4 => range.read32() as u64,
            8 => {
                let high = range.read32() as u64;
                let low = range.read32() as u64;
                (high << 32) | low
            }
            _ => 0,
        }
    };
    let mut items = Vec::new();
    for index in 0..item_count {
        if range.eof() {
            bail!("iloc box should contain {item_count} items, but we can only read {index} items.");
        }
        let item_id = if version < 2 { range.read16() as u32 } else { range.read32() };
        let mut construction_method = 0;
        if version >= 1 {
            construction_method = (range.read16() & 0xf) as u8;
        }
        range.read16();
        let base_offset = read_sized(range, base_offset_size);
        let extent_count = range.read16();
        if extent_count > MAX_ILOC_EXTENTS_PER_ITEM {
            bail!("Number of extents in iloc box ({extent_count}) exceeds security limit ({MAX_ILOC_EXTENTS_PER_ITEM})");
        }
        let mut extents = Vec::new();
        for extent in 0..extent_count {
            if range.eof() {
                bail!("iloc item should contain {extent_count} extents, but we can only read {extent} extents.");
            }
            if index_size > 0 {
                read_sized(range, index_size);
            }
            let offset = read_sized(range, offset_size);
            let length = read_sized(range, length_size);
            extents.push(IlocExtent { offset, length });
        }
        if !range.error {
            items.push(IlocItem { item_id, construction_method, base_offset, extents });
        }
    }
    range_error(range)?;
    Ok(Content::Iloc(items))
}

fn parse_infe(range: &mut Range) -> Result<Content> {
    let (version, flags) = full_box_header(range);
    if version > 3 {
        bail!("infe box data version {version} is not implemented yet");
    }
    let mut infe = Infe { item_id: 0, hidden: false, item_type: 0, content_type: String::new(), content_encoding: String::new() };
    if version <= 1 {
        infe.item_id = range.read16() as u32;
        range.read16();
        range.read_string();
        infe.content_type = range.read_string();
        infe.content_encoding = range.read_string();
    }
    if version >= 2 {
        infe.hidden = flags & 1 != 0;
        infe.item_id = if version == 2 { range.read16() as u32 } else { range.read32() };
        range.read16();
        infe.item_type = range.read32();
        range.read_string();
        if infe.item_type == fourcc(b"mime") {
            infe.content_type = range.read_string();
            infe.content_encoding = range.read_string();
        } else if infe.item_type == fourcc(b"uri ") {
            range.read_string();
        }
    }
    range_error(range)?;
    Ok(Content::Infe(infe))
}

fn parse_ipma(range: &mut Range) -> Result<Content> {
    let (version, flags) = full_box_header(range);
    if version > 1 {
        bail!("ipma box data version {version} is not implemented yet");
    }
    let count = range.read32();
    if count > MAX_ITEMS {
        bail!("ipma box wants to define properties for {count} items, but the security limit has been set to {MAX_ITEMS} items");
    }
    let mut entries = Vec::new();
    let mut index = 0;
    while index < count && !range.error && !range.eof() {
        let item_id = if version < 1 { range.read16() as u32 } else { range.read32() };
        let association_count = range.read8();
        let mut associations = Vec::new();
        for _ in 0..association_count {
            if flags & 1 != 0 {
                let value = range.read16();
                associations.push((value & 0x8000 != 0, value & 0x7fff));
            } else {
                let value = range.read8() as u16;
                associations.push((value & 0x80 != 0, value & 0x7f));
            }
        }
        entries.push(IpmaEntry { item_id, associations });
        index += 1;
    }
    range_error(range)?;
    Ok(Content::Ipma(entries))
}

fn parse_clap(range: &mut Range) -> Result<Content> {
    let width_numerator = range.read32();
    let width_denominator = range.read32();
    let height_numerator = range.read32();
    let height_denominator = range.read32();
    let horizontal_numerator = range.read32() as i32;
    let horizontal_denominator = range.read32();
    let vertical_numerator = range.read32() as i32;
    let vertical_denominator = range.read32();
    let limit = i32::MAX as u32;
    if width_numerator > limit || width_denominator > limit || height_numerator > limit || height_denominator > limit || horizontal_denominator > limit || vertical_denominator > limit {
        bail!("Exceeded supported value range.");
    }
    let clap = Clap {
        width: Fraction::new_i32(width_numerator as i32, width_denominator as i32),
        height: Fraction::new_i32(height_numerator as i32, height_denominator as i32),
        horizontal_offset: Fraction::new_i32(horizontal_numerator, horizontal_denominator as i32),
        vertical_offset: Fraction::new_i32(vertical_numerator, vertical_denominator as i32),
    };
    if clap.width.denominator == 0 || clap.height.denominator == 0 || clap.horizontal_offset.denominator == 0 || clap.vertical_offset.denominator == 0 {
        bail!("Invalid fractional number");
    }
    range_error(range)?;
    Ok(Content::Clap(clap))
}

fn parse_iref(range: &mut Range) -> Result<Content> {
    let (version, _) = full_box_header(range);
    if version > 1 {
        bail!("iref box data version {version} is not implemented yet");
    }
    let id_bits = if version == 0 { 16 } else { 32 };
    let mut references = Vec::new();
    while !range.eof() {
        let header = parse_header(range)?;
        let from = range.read_uint(id_bits) as u32;
        let count = range.read16();
        if count == 0 {
            bail!("Input file has an 'iref' box with no references.");
        }
        if count as u32 > MAX_ITEMS {
            bail!("Number of references in iref box ({count}) exceeds the security limits of {MAX_ITEMS} references.");
        }
        let mut to = Vec::new();
        for index in 0..count {
            if range.eof() {
                bail!("iref box should contain {count} references, but we can only read {index} references.");
            }
            to.push(range.read_uint(id_bits) as u32);
        }
        references.push(Reference { kind: header.kind, from, to });
    }
    for reference in &references {
        let mut seen = std::collections::HashSet::new();
        for id in &reference.to {
            if !seen.insert(*id) {
                bail!("'iref' has double references");
            }
        }
    }
    range_error(range)?;
    Ok(Content::Iref(references))
}

fn parse_colr(header: &Header, range: &mut Range) -> Result<Content> {
    let colour_type = range.read32();
    if colour_type == fourcc(b"nclx") {
        if range.remaining() < 7 {
            bail!("End of data");
        }
        let primaries = range.read16();
        let transfer = range.read16();
        let matrix = range.read16();
        let full_range = range.read8() & 0x80 != 0;
        return Ok(Content::Colr(ColorProfile::Nclx(Nclx { colour_primaries: primaries, transfer_characteristics: transfer, matrix_coefficients: matrix, full_range })));
    }
    if colour_type == fourcc(b"nclc") {
        if range.remaining() < 6 {
            bail!("End of data");
        }
        let primaries = range.read16();
        let transfer = range.read16();
        let matrix = range.read16();
        return Ok(Content::Colr(ColorProfile::Nclx(Nclx { colour_primaries: primaries, transfer_characteristics: transfer, matrix_coefficients: matrix, full_range: matrix == 0 })));
    }
    if colour_type == fourcc(b"prof") || colour_type == fourcc(b"rICC") {
        if !header.fixed_size() {
            bail!("colr boxes with undefined box size are not supported");
        }
        let size = header.size.wrapping_sub(header.header_size).wrapping_sub(4);
        if size > MAX_COLOR_PROFILE_SIZE {
            bail!("Color profile exceeds maximum supported size");
        }
        let size = size as usize;
        if range.remaining() < size {
            bail!("End of data");
        }
        let bytes = range.read_bytes(size).ok_or_else(|| anyhow!("End of data"))?.to_vec();
        range_error(range)?;
        return Ok(Content::Colr(ColorProfile::Raw(bytes)));
    }
    bail!("Unknown color profile type");
}

fn parse_hvcc(range: &mut Range) -> Result<Content> {
    range.read8();
    range.read8();
    range.read32();
    for _ in 0..6 {
        range.read8();
    }
    range.read8();
    range.read16();
    range.read8();
    // chroma_format_idc
    range.read8();
    let bit_depth_luma = (range.read8() & 7) + 8;
    // bit_depth_chroma
    range.read8();
    range.read16();
    range.read8();
    let array_count = range.read8();
    let mut nal_units = Vec::new();
    let mut array = 0;
    while array < array_count && !range.error {
        range.read8();
        let unit_count = range.read16();
        let mut unit = 0;
        while unit < unit_count && !range.error {
            let size = range.read16() as usize;
            unit += 1;
            if size == 0 {
                continue;
            }
            let bytes = range.read_bytes(size).map(|bytes| bytes.to_vec()).unwrap_or_default();
            nal_units.push(bytes);
        }
        array += 1;
    }
    range.skip_to_end();
    range_error(range)?;
    Ok(Content::HvcC(HvcC { bit_depth_luma, nal_units }))
}

fn parse_av1c(header: &Header, range: &mut Range) -> Result<Content> {
    if !header.fixed_size() {
        bail!("av1C with unspecified box size");
    }
    // marker and version, profile and level
    range.read8();
    range.read8();
    let flags = range.read8();
    // presentation delay
    range.read8();
    let high_bitdepth = flags & 0x40 != 0;
    let twelve_bit = flags & 0x20 != 0;
    let config_obus = range.read_bytes(range.remaining()).map(|bytes| bytes.to_vec()).unwrap_or_default();
    range_error(range)?;
    Ok(Content::Av1C(Av1C { high_bitdepth, twelve_bit, config_obus }))
}
