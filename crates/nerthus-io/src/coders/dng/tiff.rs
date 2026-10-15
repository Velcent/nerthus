//! LibRaw's TIFF parsing (`metadata/tiff.cpp`, `exif_gps.cpp`): every IFD
//! of the file (`parse_tiff_ifd`, with the PanasonicRaw tags of RW2 files,
//! the DNG tags and the maker-specific ones), the EXIF IFD with its maker
//! note, and `apply_tiff`, which picks the raw IFD and its decoder.

use anyhow::{bail, Result};

use super::identify::{Ifd, CBLACK_SIZE, D65_WHITE};
use super::identify::{cam_xyz_coeff_into, gamma_curve};
use super::identify::{DNGFM_ANALOGBALANCE, DNGFM_ASSHOTNEUTRAL, DNGFM_BLACK, DNGFM_CALIBRATION, DNGFM_COLORMATRIX, DNGFM_ILLUMINANT, DNGFM_LINTABLE, DNGFM_OPCODE2, DNGFM_WHITE};
use super::ljpeg::{ljpeg_start, BitReader};
use super::state::{contains_ignore_case, starts_with_ignore_case, LoadRaw, Raw};
use super::stream::{BIG_ENDIAN, LITTLE_ENDIAN, TYPE_IFD8, TYPE_LONG, TYPE_SHORT, TYPE_SIZES};

const IFD_MAXCOUNT: usize = 10;
/// `LIBRAW_EXIFTAG_TYPE_UNDEFINED`.
const TYPE_UNDEFINED: u32 = 7;
/// `LIBRAW_EXIFTAG_TYPE_ASCII`.
const TYPE_ASCII: u32 = 2;
/// `LIBRAW_MAX_METADATA_BLOCKS`.
pub(super) const MAX_METADATA_BLOCKS: u32 = 1024;

/// `GRBG_2_RGBG`, `RGGB_2_RGBG` and `GRGB_2_RGBG` index maps.
pub(super) const GRBG_2_RGBG: [usize; 4] = [1, 0, 2, 3];
pub(super) const RGGB_2_RGBG: [usize; 4] = [0, 1, 3, 2];
pub(super) const GRGB_2_RGBG: [usize; 4] = [1, 0, 3, 2];

/// One tag's header: `tiff_get`'s outputs.
pub(super) struct Tag {
    pub tag: u32,
    pub kind: u32,
    pub count: u32,
    /// Where the next tag starts.
    pub save: u64,
}

impl Raw<'_> {
    /// `tiff_get`: the stream at the tag's value. Fewer than 12 bytes
    /// left is LibRaw's end-of-file exception.
    pub(super) fn tiff_get(&mut self, base: i64) -> Result<Tag> {
        let position = self.stream.tell();
        let size = self.stream.size();
        if size < 12 || size.saturating_sub(position) < 12 {
            bail!("unexpected end of file");
        }
        let tag = self.stream.get2() as u32;
        let kind = self.stream.get2() as u32;
        let count = self.stream.get4();
        let save = self.stream.tell() + 4;
        let unit = TYPE_SIZES[if kind <= TYPE_IFD8 { kind as usize } else { 0 }];
        if count.wrapping_mul(unit) > 4 {
            let offset = self.stream.get4() as i64 + base;
            self.seek(offset);
        }
        Ok(Tag { tag, kind, count, save })
    }

    /// `fseek` to an absolute position (negative ones stay at 0).
    pub(super) fn seek(&mut self, position: i64) {
        self.stream.seek(position.max(0) as u64);
    }

    /// `parse_tiff`: the IFD chain of a TIFF structure at `base`.
    pub(super) fn parse_tiff(&mut self, base: i64) -> Result<bool> {
        self.seek(base);
        let order = self.stream.get2();
        if order != LITTLE_ENDIAN && order != BIG_ENDIAN {
            return Ok(false);
        }
        self.stream.order = order;
        self.stream.get2();
        loop {
            let offset = self.stream.get4() as i32;
            if offset == 0 {
                break;
            }
            let position = offset as i64 + base;
            if position > self.stream.size() as i64 || position < 0 {
                break;
            }
            self.seek(position);
            if self.parse_tiff_ifd(base)? {
                break;
            }
        }
        Ok(true)
    }

    /// `ljpeg_start(&jh, 1)` at the stream's position.
    fn jpeg_info(&mut self) -> Option<(i32, i32, i32, i32, i32)> {
        let mut bits = BitReader::new();
        let head = ljpeg_start(&mut self.stream, &mut bits, self.dng_version, true)?;
        Some((head.bits, head.high, head.wide, head.clrs, head.sraw))
    }

    /// The JPEG at an IFD's data offset: its sizes become the IFD's.
    fn inspect_jpeg_offset(&mut self, ifd: usize, sony_special: bool) -> Result<()> {
        if self.ifds[ifd].bps != 0 || self.ifds[ifd].offset <= 0 {
            return Ok(());
        }
        let offset = self.ifds[ifd].offset;
        self.seek(offset);
        let Some((bits, high, wide, clrs, sraw)) = self.jpeg_info() else {
            return Ok(());
        };
        let sony = self.dng_version == 0 && self.make.eq_ignore_ascii_case("SONY");
        let current = &mut self.ifds[ifd];
        if sony_special && sony && current.phint == 32803 && current.comp == 7 {
            current.comp = 6;
            current.bps = bits;
            current.samples = 1;
        } else if sony_special && sony && current.phint == 6 && current.comp == 7 && current.samples == 3 {
            current.comp = 6;
            current.bps = bits;
        } else {
            current.comp = 6;
            current.bps = bits;
            current.width = wide;
            current.height = high;
            current.samples = clrs;
            if !(sraw != 0 || clrs & 1 != 0) {
                current.width *= clrs;
            }
            if (current.width > 4 * current.height) & (!clrs != 0) {
                current.width /= 2;
                current.height *= 2;
            }
            let order = self.stream.order;
            self.parse_tiff(offset + 12)?;
            self.stream.order = order;
        }
        Ok(())
    }

    /// StripOffsets: the strips' offsets (more than one) and the first.
    fn strip_offsets(&mut self, ifd: usize, count: u32, base: i64) {
        if count > 1 && count < 16384 {
            let position = self.stream.tell();
            let offsets: Vec<i64> = (0..count).map(|_| self.stream.get4() as i64 + base).collect();
            self.ifds[ifd].strip_offsets = offsets;
            self.stream.seek(position);
        }
    }

    /// `parse_tiff_ifd`: returns true to stop reading IFDs.
    pub(super) fn parse_tiff_ifd(&mut self, mut base: i64) -> Result<bool> {
        if self.tiff_nifds >= IFD_MAXCOUNT {
            return Ok(true);
        }
        let ifd = self.tiff_nifds;
        self.tiff_nifds += 1;
        while self.ifds.len() < self.tiff_nifds {
            self.ifds.push(Ifd::default());
        }
        if self.ifd0_offset == -1 {
            self.ifd0_offset = base;
        }
        let entries = self.stream.get2();
        if entries > 512 {
            return Ok(true);
        }
        let file_size = self.stream.size() as i64;

        let mut cc = [[0.0f64; 4]; 4];
        for (row, values) in cc.iter_mut().enumerate() {
            values[row] = 1.0;
        }
        let mut cm = [[0.0f64; 3]; 4];
        let mut ab = [1.0f64; 4];
        let mut asn = [0.0f64; 4];
        let mut xyz = [1.0f64; 3];
        let mut use_cm = false;
        let mut cfa_pattern = [0u8; 16];
        let mut pattern_length = 16usize;
        let mut plane_colors = [0u8, 1, 2, 3];
        let mut sony_curve = [0u32, 0, 0, 0, 0, 4095];
        let mut sony_offset = 0u32;
        let mut sony_length = 0u32;
        let mut sony_key = 0u32;

        for _ in 0..entries {
            let Tag { tag, kind, count, save } = self.tiff_get(base)?;
            let value_position = self.stream.tell() as i64;
            if count > 8 && value_position + count as i64 > 2 * file_size {
                self.stream.seek(save);
                continue;
            }

            if !self.is_pana_raw {
                // EXIF tags that collide with PanasonicRaw ones.
                match tag {
                    0x0001 if count == 4 => self.is_pana_raw = self.stream.get4() != 0,
                    0x000b => {
                        self.software = self.stream.read_string(64);
                        if self.software_is_converter() {
                            self.is_raw = 0;
                        }
                    }
                    0x001c..=0x001e => {
                        self.cblack[(tag - 0x001c) as usize] = self.stream.get2() as u32;
                        self.cblack[3] = self.cblack[1];
                    }
                    0x0111 | 0x0201 | 0xf007 => {
                        if tag == 0x0111 {
                            self.strip_offsets(ifd, count, base);
                        }
                        self.ifds[ifd].offset = self.stream.get4() as i64 + base;
                        self.inspect_jpeg_offset(ifd, true)?;
                    }
                    _ => {}
                }
            } else {
                self.panasonic_raw_tag(ifd, tag, kind, count, base)?;
            }

            match tag {
                0xf000 => {
                    let offset = self.stream.get4() as i64 + base;
                    self.seek(offset);
                    self.parse_tiff_ifd(base)?;
                }
                0x00fe => self.ifds[ifd].newsubfiletype = self.stream.getreal(kind) as i32,
                0x0100 | 0xf001 => self.ifds[ifd].width = self.stream.getint(kind) as i32,
                0x0101 | 0xf002 => self.ifds[ifd].height = self.stream.getint(kind) as i32,
                0x0102 | 0xf003 => {
                    if self.ifds[ifd].samples == 0 || tag != 0x0102 {
                        self.ifds[ifd].samples = (count & 7) as i32;
                    }
                    self.ifds[ifd].bps = self.stream.getint(kind) as i32;
                    if (self.tiff_bps as i32) < self.ifds[ifd].bps {
                        self.tiff_bps = self.ifds[ifd].bps as u32;
                    }
                }
                0xf006 => {
                    self.raw_height = 0;
                    if self.ifds[ifd].bps <= 12 {
                        self.load_raw = LoadRaw::Packed;
                        self.load_flags = if self.stream.get4() != 0 { 24 } else { 80 };
                    }
                }
                0x0103 => self.ifds[ifd].comp = self.stream.getint(kind) as i32,
                0x0106 => self.ifds[ifd].phint = self.stream.get2() as i32,
                0x010f => self.make = self.stream.read_string(64),
                0x0110 => {
                    let hasselblad = self.make.starts_with("Hasselblad");
                    if !(hasselblad && !self.model.is_empty()) {
                        self.model = self.stream.read_string(64);
                    }
                }
                0x0116 => self.ifds[ifd].rows_per_strip = self.stream.getint(kind) as i32,
                0x0112 => {
                    let flips = b"50132467";
                    self.ifds[ifd].flip = (flips[(self.stream.get2() & 7) as usize] - b'0') as i32;
                }
                0x0115 => self.ifds[ifd].samples = (self.stream.getint(kind) & 7) as i32,
                0x0152 => self.ifds[ifd].extrasamples = (self.stream.getint(kind) & 0xff) as i32 + 1024,
                0x0117 | 0x0202 | 0xf008 => {
                    if tag == 0x0117 && count > 1 && count < 16384 {
                        let position = self.stream.tell();
                        let counts: Vec<i64> = (0..count).map(|_| self.stream.get4() as i64).collect();
                        self.ifds[ifd].strip_byte_counts = counts;
                        self.stream.seek(position);
                    }
                    self.ifds[ifd].bytes = self.stream.get4() as i64;
                }
                0xf00e => {
                    for color in 0..3 {
                        self.cam_mul[GRBG_2_RGBG[color]] = self.stream.getint(kind) as f32;
                    }
                }
                0x0131 => {
                    self.software = self.stream.read_string(64);
                    if self.software_is_converter() {
                        self.is_raw = 0;
                    }
                }
                0x0132 => self.get_timestamp(false),
                0x013d => self.ifds[ifd].predictor = self.stream.getint(kind) as i32,
                0x0142 => self.ifds[ifd].tile_width = self.stream.getint(kind) as i32,
                0x0143 => self.ifds[ifd].tile_length = self.stream.getint(kind) as i32,
                0x0144 => {
                    self.ifds[ifd].offset = if count > 1 { self.stream.tell() as i64 } else { self.stream.get4() as i64 };
                    if count == 1 {
                        self.ifds[ifd].tile_width = 0;
                        self.ifds[ifd].tile_length = 0;
                    }
                    if count == 4 {
                        self.load_raw = LoadRaw::Other("sinar_4shot_load_raw");
                        self.is_raw = 5;
                    }
                }
                0x0145 => self.ifds[ifd].bytes = if count > 1 { self.stream.tell() as i64 } else { self.stream.get4() as i64 },
                0x014a => {
                    if self.model == "DSLR-A100" && self.ifds[ifd].width == 3872 {
                        self.load_raw = LoadRaw::Other("sony_arw_load_raw");
                        self.data_offset = self.stream.get4() as i64 + base;
                        if ifd + 1 >= IFD_MAXCOUNT {
                            bail!("corrupt raw file");
                        }
                    } else if self.make.starts_with("Hasselblad") && self.hasselblad_parser_flag {
                        let position = self.stream.tell() as i64 + 4;
                        self.seek(position);
                        let offset = self.stream.get4() as i64 + base;
                        self.seek(offset);
                        self.parse_tiff_ifd(base)?;
                    } else {
                        for _ in 0..count.min(1000) {
                            let position = self.stream.tell();
                            let offset = self.stream.get4() as i64 + base;
                            self.seek(offset);
                            if self.parse_tiff_ifd(base)? {
                                break;
                            }
                            self.stream.seek(position + 4);
                        }
                    }
                }
                0x0153 => self.ifds[ifd].sample_format = self.stream.getint(kind) as i32,
                0x0190 => {
                    self.make = "Sarnoff".to_string();
                    self.maximum = 0xfff;
                }
                0x7010 => {
                    for color in 0..4 {
                        sony_curve[color + 1] = (self.stream.get2() as u32 >> 2) & 0xfff;
                    }
                    for index in 0..5 {
                        let first = sony_curve[index] as usize + 1;
                        let last = sony_curve[index + 1] as usize;
                        for position in first..=last {
                            self.curve[position] = self.curve[position - 1].wrapping_add(1 << index);
                        }
                    }
                }
                0x7200 => sony_offset = self.stream.get4(),
                0x7201 => sony_length = self.stream.get4(),
                0x7221 => sony_key = self.stream.get4(),
                0x7250 => {
                    let position = self.stream.tell() as i64;
                    self.parse_minolta(position)?;
                    self.raw_width = 0;
                }
                0x7303 => {
                    for color in 0..4 {
                        self.cam_mul[GRBG_2_RGBG[color]] = self.stream.get2() as f32;
                    }
                }
                0x7313 => {
                    for color in 0..4 {
                        self.cam_mul[RGGB_2_RGBG[color]] = self.stream.get2() as f32;
                    }
                }
                0x7310 => {
                    for color in 0..4 {
                        self.cblack[RGGB_2_RGBG[color]] = self.stream.get2() as u32;
                    }
                    let mut lowest = self.cblack[3];
                    for color in 0..3 {
                        if lowest > self.cblack[color] {
                            lowest = self.cblack[color];
                        }
                    }
                    for color in 0..4 {
                        self.cblack[color] -= lowest;
                    }
                    self.black = lowest;
                }
                0x827d => self.model2 = self.stream.read_string(64),
                0x828d => {
                    if self.stream.get2() == 6 && self.stream.get2() == 6 {
                        self.ifds[ifd].filters = 9;
                        self.filters = 9;
                    }
                }
                0x828e | 0xfd09 => {
                    if tag == 0x828e && self.filters == 9 {
                        self.read_xtrans();
                    } else if count == 36 {
                        self.ifds[ifd].filters = 9;
                        self.filters = 9;
                        self.colors = 3;
                        self.read_xtrans();
                    } else if count > 0 {
                        pattern_length = (count as usize).min(16);
                        self.stream.read(&mut cfa_pattern[..pattern_length]);
                        let mut cfa = 0u32;
                        self.colors = 0;
                        for &color in cfa_pattern.iter().take(pattern_length) {
                            if self.colors >= 4 {
                                break;
                            }
                            if color > 31 {
                                continue;
                            }
                            if cfa & (1 << color) == 0 {
                                self.colors += 1;
                            }
                            cfa |= 1 << color;
                        }
                        if cfa == 0o70 {
                            plane_colors[..3].copy_from_slice(&[3, 4, 5]);
                        }
                        if cfa == 0o72 {
                            plane_colors = [5, 3, 4, 1];
                        }
                        self.guess_cfa(ifd, &cfa_pattern, pattern_length, &plane_colors);
                    }
                }
                0x8290 | 0xfe00 => {
                    let offset = self.stream.get4() as i64 + base;
                    self.seek(offset);
                    self.unported("Kodak IFD");
                }
                0x8602 => {
                    for color in 0..4 {
                        let value = self.stream.get2();
                        if value != 0 {
                            self.cam_mul[GRGB_2_RGBG[color]] = 4096.0 / value as f32;
                        }
                    }
                }
                0x8603 => self.unported("Leaf CatchLight matrix"),
                0x8606 => {
                    self.unported("Leaf metadata");
                    self.make = "Leaf".to_string();
                }
                0x85ff => self.make = "Leaf".to_string(),
                0x8769 => {
                    let offset = self.stream.get4() as i64 + base;
                    self.seek(offset);
                    self.parse_exif(base)?;
                }
                0x9102 => self.kodak_cbpp = self.stream.get4(),
                0x9218 => {
                    self.raw_color = false;
                    for row in 0..3 {
                        self.stream.getreal(kind);
                        for color in 0..3 {
                            self.rgb_cam[row][color] = self.stream.getrealf(kind);
                        }
                    }
                }
                0xa010 => {
                    self.strip_offset = self.stream.get4() as i64;
                    match self.ifds[ifd].comp {
                        0x8002 => self.load_raw = LoadRaw::Other("samsung_load_raw"),
                        0x8004 => self.load_raw = LoadRaw::Other("samsung2_load_raw"),
                        0x8005 => self.load_raw = LoadRaw::Other("samsung3_load_raw"),
                        _ => {}
                    }
                }
                0xb4c3 | 0xb4c7 | 0xb4c2 => self.unported("Imacon"),
                0xc516 | 0xc517 => {
                    if count >= 1 && count <= 2_560_000 {
                        let mut buffer = vec![0u8; count as usize];
                        if self.stream.read(&mut buffer) != count as usize {
                            bail!("corrupt raw file");
                        }
                        buffer[count as usize - 1] = 0;
                        let text = String::from_utf8_lossy(&buffer).into_owned();
                        for line in text.split('\n') {
                            if let Some(rest) = line.strip_prefix("Neutral ") {
                                let values: Vec<f32> = rest.split_whitespace().take(3).filter_map(|value| value.parse().ok()).collect();
                                for (index, value) in values.into_iter().enumerate() {
                                    self.cam_mul[index] = value;
                                }
                            }
                        }
                    }
                }
                0xc51a => {
                    if self.make.is_empty() {
                        self.make = "Hasselblad".to_string();
                    }
                }
                0xc51b if !self.hasselblad_parser_flag => {
                    self.hasselblad_parser_flag = true;
                    let order = self.stream.order;
                    let position = self.stream.tell() as i64;
                    let saved_count = self.tiff_nifds;
                    self.stream.order = self.stream.get2();
                    self.stream.get2();
                    let offset = self.stream.get4() as i64;
                    self.seek(position + offset);
                    self.parse_tiff_ifd(position)?;
                    self.maximum = 0xffff;
                    self.tiff_nifds = saved_count;
                    self.stream.order = order;
                }
                0xc51b | 0xc612 => {
                    for _ in 0..4 {
                        let byte = self.stream.getc().map_or(u32::MAX, |byte| byte as u32);
                        self.dng_version = (self.dng_version << 8).wrapping_add(byte);
                    }
                    if self.make.is_empty() {
                        self.make = "DNG".to_string();
                    }
                    self.is_raw = 1;
                }
                0xc614 => {
                    let unique = self.read_text(count as usize, 64);
                    if self.model.is_empty() {
                        let limited: String = unique.chars().take((count as usize).min(64)).collect();
                        match limited.split_once(' ') {
                            Some((make, model)) => {
                                self.make = make.to_string();
                                self.model = model.to_string();
                            }
                            None => self.make = limited,
                        }
                    }
                }
                0xc616 => {
                    if self.filters != 9 {
                        let length = (count as usize).min(4);
                        self.colors = length;
                        self.stream.read(&mut plane_colors[..length]);
                        self.guess_cfa(ifd, &cfa_pattern, pattern_length, &plane_colors);
                    }
                }
                0xc617 => {
                    if self.stream.get2() == 2 {
                        self.ifds[ifd].fuji_width = 1;
                        self.fuji_width = 1;
                    }
                }
                0x0123 | 0xc618 => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_LINTABLE;
                    self.ifds[ifd].lineartable_offset = self.stream.tell() as i64;
                    self.ifds[ifd].lineartable_len = count;
                    self.linear_table(count);
                }
                0xc619 => {
                    let levels = &mut self.ifds[ifd].dng_levels;
                    levels.parsed |= DNGFM_BLACK;
                    self.cblack[4] = self.stream.get2() as u32;
                    levels.dng_cblack[4] = self.cblack[4];
                    levels.dng_fcblack[4] = self.cblack[4] as f32;
                    self.cblack[5] = self.stream.get2() as u32;
                    levels.dng_cblack[5] = self.cblack[5];
                    levels.dng_fcblack[5] = self.cblack[5] as f32;
                    if self.cblack[4] * self.cblack[5] > (CBLACK_SIZE - 7) as u32 {
                        levels.dng_fcblack[4] = 1.0;
                        levels.dng_fcblack[5] = 1.0;
                        levels.dng_cblack[4] = 1;
                        levels.dng_cblack[5] = 1;
                        self.cblack[4] = 1;
                        self.cblack[5] = 1;
                    }
                }
                0xf00c | 0xf00d => {}
                0xf00a | 0xc61a => {
                    if tag == 0xf00a {
                        let side = ((count as f32).sqrt()).min(64.0) as u32;
                        self.cblack[4] = side;
                        self.cblack[5] = side;
                    }
                    self.black_level(ifd, tag, kind, count);
                }
                0xc61b | 0xc61c => {
                    let mut sum = 0.0f64;
                    for _ in 0..count.min(65536) {
                        sum += self.stream.getreal(kind);
                    }
                    if count > 0 {
                        let mean = sum / count as f64;
                        self.black = self.black.wrapping_add((mean + 0.5) as u32);
                        let levels = &mut self.ifds[ifd].dng_levels;
                        levels.dng_fblack += mean as f32;
                        levels.dng_black = levels.dng_black.wrapping_add((mean + 0.5) as u32);
                        levels.parsed |= DNGFM_BLACK;
                    }
                }
                0xc61d => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_WHITE;
                    self.maximum = self.stream.getint(kind);
                    self.ifds[ifd].dng_levels.dng_whitelevel[0] = self.maximum;
                    if self.ifds[ifd].samples > 1 {
                        for index in 1..(count as usize).min(4) {
                            self.ifds[ifd].dng_levels.dng_whitelevel[index] = self.stream.getint(kind);
                        }
                    }
                }
                0xc61e => {
                    let horizontal = self.stream.getrealf(kind);
                    let vertical = self.stream.getrealf(kind);
                    if horizontal > 0.00001 && vertical > 0.00001 {
                        let aspect = horizontal / vertical;
                        self.pixel_aspect = if aspect > 0.995 && aspect < 1.005 { 1.0 } else { aspect as f64 };
                    }
                }
                0xc65a | 0xc65b => {
                    let index = (tag - 0xc65a) as usize;
                    self.ifds[ifd].dng_color[index].illuminant = self.stream.get2();
                    self.ifds[ifd].dng_color[index].parsed |= DNGFM_ILLUMINANT;
                }
                0xc621 | 0xc622 => {
                    let channels = match count {
                        9 => 3,
                        12 => 4,
                        _ => 0,
                    };
                    let index = (tag - 0xc621) as usize;
                    if channels > 0 {
                        self.ifds[ifd].dng_color[index].parsed |= DNGFM_COLORMATRIX;
                    }
                    for row in 0..channels {
                        for column in 0..3 {
                            let value = self.stream.getreal(kind);
                            cm[row][column] = value;
                            self.ifds[ifd].dng_color[index].colormatrix[row][column] = value as f32;
                        }
                    }
                    use_cm = true;
                }
                0xc623 | 0xc624 => {
                    let channels = match count {
                        9 => 3,
                        16 => 4,
                        _ => 0,
                    };
                    let index = (tag - 0xc623) as usize;
                    if channels > 0 {
                        self.ifds[ifd].dng_color[index].parsed |= DNGFM_CALIBRATION;
                    }
                    for row in 0..channels {
                        for column in 0..channels {
                            let value = self.stream.getreal(kind);
                            cc[row][column] = value;
                            self.ifds[ifd].dng_color[index].calibration[row][column] = value as f32;
                        }
                    }
                }
                0xc627 => {
                    if count >= 3 {
                        self.ifds[ifd].dng_levels.parsed |= DNGFM_ANALOGBALANCE;
                    }
                    for index in 0..(count as usize).min(4) {
                        ab[index] = self.stream.getreal(kind);
                        self.ifds[ifd].dng_levels.analogbalance[index] = ab[index] as f32;
                    }
                }
                0xc628 => {
                    if count >= 3 {
                        self.ifds[ifd].dng_levels.parsed |= DNGFM_ASSHOTNEUTRAL;
                    }
                    for index in 0..(count as usize).min(4) {
                        asn[index] = self.stream.getreal(kind);
                        self.ifds[ifd].dng_levels.asshotneutral[index] = asn[index] as f32;
                    }
                }
                0xc629 => {
                    xyz[0] = self.stream.getreal(kind);
                    xyz[1] = self.stream.getreal(kind);
                    xyz[2] = 1.0 - xyz[0] - xyz[1];
                    for (value, white) in xyz.iter_mut().zip(D65_WHITE) {
                        *value /= white as f64;
                    }
                }
                0xc634 => self.dng_private_data(tag, count, &mut base)?,
                0xc640 => {
                    let mut slices = [0u16; 3];
                    self.stream.read_shorts(&mut slices);
                    self.cr2_slice = slices;
                }
                0xc68d => {
                    let top = self.stream.getint(kind);
                    let left = self.stream.getint(kind);
                    let bottom = self.stream.getint(kind);
                    let right = self.stream.getint(kind);
                    self.top_margin = top & 0xffff;
                    self.left_margin = left & 0xffff;
                    self.height = bottom.wrapping_sub(self.top_margin) & 0xffff;
                    self.width = right.wrapping_sub(self.left_margin) & 0xffff;
                    let current = &mut self.ifds[ifd];
                    current.top_margin = self.top_margin as i32;
                    current.left_margin = self.left_margin as i32;
                    current.vheight = self.height as i32;
                    current.vwidth = self.width as i32;
                }
                0xc68e => {
                    for index in 0..(count as usize).min(32) {
                        self.mask[index / 4][index % 4] = self.stream.getint(kind) as i32;
                    }
                    self.black = 0;
                }
                0xc741 => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_OPCODE2;
                    self.meta_offset = self.stream.tell() as i64;
                    self.ifds[ifd].opcode2_offset = self.meta_offset;
                }
                0xfd04 => {
                    if count >= 13 {
                        self.stream.skip(16);
                        self.data_offset = self.stream.get4() as i64;
                        self.stream.skip(28);
                        self.data_offset += self.stream.get4() as i64;
                        self.load_raw = LoadRaw::Packed;
                    }
                }
                0xfe02 => {
                    if kind == TYPE_ASCII {
                        self.model2 = self.stream.read_string(64);
                    }
                }
                _ => {}
            }
            self.stream.seek(save);
        }

        if sony_length != 0 && sony_length < 10_240_000 {
            self.sony_sr2_private(sony_offset, sony_length, sony_key)?;
        }

        // The colour data of this IFD.
        let colors = self.colors.min(4);
        for (row, balance) in cc.iter_mut().zip(ab).take(colors) {
            for value in row.iter_mut().take(colors) {
                *value *= balance;
            }
        }
        if use_cm {
            let mut cam_xyz = [[0.0f64; 3]; 4];
            for (cam_row, cc_row) in cam_xyz.iter_mut().zip(cc.iter()).take(colors) {
                for (column, value) in cam_row.iter_mut().enumerate() {
                    for (calibration, cm_row) in cc_row.iter().zip(cm.iter()).take(colors) {
                        *value += calibration * cm_row[column] * xyz[column];
                    }
                }
            }
            let mut cmatrix = self.cmatrix;
            cam_xyz_coeff_into(&mut cmatrix, &cam_xyz, self.colors, &mut self.pre_mul);
            self.cmatrix = cmatrix;
        }
        if asn[0] != 0.0 {
            self.cam_mul[3] = 0.0;
            for (multiplier, neutral) in self.cam_mul.iter_mut().zip(asn).take(colors) {
                if neutral.abs() > 0.0001 {
                    *multiplier = (1.0 / neutral) as f32;
                }
            }
        }
        if !use_cm {
            for (color, (multiplier, cc_row)) in self.pre_mul.iter_mut().zip(cc.iter()).enumerate().take(colors) {
                if cc_row[color].abs() > 0.0001 {
                    *multiplier /= cc_row[color] as f32;
                }
            }
        }
        Ok(false)
    }

    /// What `software` names a raw converter, whose output isn't raw.
    fn software_is_converter(&self) -> bool {
        let software = &self.software;
        software.starts_with("Adobe") || software.starts_with("dcraw") || software.starts_with("UFRaw") || software.starts_with("Bibble") || software == "Digital Photo Professional"
    }

    /// `stmread`: up to `limit - 1` bytes of `count`, as a string.
    pub(super) fn read_text(&mut self, count: usize, limit: usize) -> String {
        let length = count.min(limit - 1);
        let mut buffer = vec![0u8; length];
        let read = self.stream.read(&mut buffer);
        buffer.truncate(read);
        if let Some(end) = buffer.iter().position(|&byte| byte == 0) {
            buffer.truncate(end);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// The 6×6 X-Trans pattern.
    fn read_xtrans(&mut self) {
        for row in 0..6 {
            for column in 0..6 {
                self.xtrans[row][column] = self.stream.getc().map_or(3, |byte| byte & 3);
            }
        }
    }

    /// Records a feature LibRaw has and this port does not: the file
    /// then fails to open.
    pub(super) fn unported(&mut self, what: &'static str) {
        if self.unported.is_none() {
            self.unported = Some(what);
        }
    }

    /// `get_timestamp`: whether the date/time string parses (only its
    /// being set matters here).
    pub(super) fn get_timestamp(&mut self, reversed: bool) {
        let mut text = [0u8; 20];
        if reversed {
            for index in (0..19).rev() {
                text[index] = self.stream.getc().unwrap_or(0xff);
            }
        } else {
            self.stream.read(&mut text[..20]);
        }
        let string = String::from_utf8_lossy(&text[..19]).into_owned();
        let fields: Vec<&str> = string.split([':', ' ']).collect();
        let parsed: Vec<i64> = fields.iter().filter_map(|field| field.trim().parse::<i64>().ok()).collect();
        if parsed.len() >= 6 {
            self.timestamp = 1;
        }
    }

    /// The PanasonicRaw tags of an RW2 IFD.
    fn panasonic_raw_tag(&mut self, ifd: usize, tag: u32, kind: u32, count: u32, base: i64) -> Result<()> {
        match tag {
            0x000a => self.pana_bpp = (self.stream.get2() as u32).clamp(8, 16),
            0x000b => self.panasonic.compression = self.stream.get2(),
            0x0013 => {
                let entries = self.stream.get2() as u32;
                if entries <= 0x100 {
                    for _ in 0..entries {
                        if self.stream.get2() < 0x100 {
                            self.stream.get2();
                            self.stream.get2();
                        } else {
                            self.stream.get4();
                        }
                    }
                }
            }
            0x001c..=0x001e => self.pana_black[(tag - 0x001c) as usize] = self.stream.get2() as u32,
            0x002d => self.pana_encoding = self.stream.get2() as u32,
            0x0039 if kind == TYPE_UNDEFINED && count == 26 => {
                let entries = (self.stream.get2() as usize).min(6);
                for index in 0..entries {
                    self.pana8.tag39[index] = self.stream.get4();
                }
            }
            0x003a if kind == TYPE_UNDEFINED && count == 26 => {
                let entries = (self.stream.get2() as usize).min(6);
                for index in 0..entries {
                    self.stream.get2();
                    self.pana8.tag3a[index] = self.stream.get2();
                }
            }
            0x003b if kind == TYPE_SHORT && count == 1 => self.pana8.tag3b = self.stream.get2(),
            0x003c..=0x003f if kind == TYPE_SHORT && count == 1 => self.pana8.initial[(tag - 0x3c) as usize] = self.stream.get2(),
            0x0040 if kind == TYPE_UNDEFINED && count == 70 => {
                let entries = (self.stream.get2() as usize).min(17);
                for index in 0..entries {
                    self.pana8.tag40a[index] = self.stream.get2().min(16);
                    self.pana8.tag40b[index] = self.stream.get2().min(0xfff);
                }
            }
            0x0041 if kind == TYPE_UNDEFINED && count == 36 => {
                let entries = (self.stream.get2() as usize).min(17);
                for index in 0..entries {
                    let value = self.stream.get2();
                    self.pana8.tag41[index] = if value > 0x40 { 64 } else { value };
                }
            }
            0x0042 if kind == TYPE_SHORT && count == 1 => self.pana8.stripe_count = self.stream.get2().min(5),
            0x0043 if kind == TYPE_SHORT && count == 1 => self.pana8.tag43 = self.stream.get2().min(5),
            0x0044..=0x0046 if kind == TYPE_UNDEFINED && count == 50 => {
                let entries = (self.stream.get2() as usize).min(5);
                for index in 0..entries {
                    let value = self.stream.get4();
                    match tag {
                        0x0044 => self.pana8.stripe_offsets[index] = value,
                        0x0045 => self.pana8.stripe_left[index] = value,
                        _ => self.pana8.stripe_compressed_size[index] = value,
                    }
                }
            }
            0x0047 | 0x0048 if kind == TYPE_UNDEFINED && count == 26 => {
                let entries = (self.stream.get2() as usize).min(5);
                for index in 0..entries {
                    let value = self.stream.get2();
                    if tag == 0x0047 {
                        self.pana8.stripe_width[index] = value;
                    } else {
                        self.pana8.stripe_height[index] = value;
                    }
                }
            }
            0x0120 => {
                let order = self.stream.order;
                let new_base = self.stream.tell() as i64;
                self.stream.order = self.stream.get2();
                self.stream.skip(2);
                let offset = self.stream.get4() as i64;
                self.stream.skip(offset - 8);
                self.parse_tiff_ifd(new_base)?;
                self.stream.order = order;
            }
            0x2009 => {
                if self.pana_encoding == 4 || self.pana_encoding == 5 {
                    let entries = (count as usize).min(8);
                    let permutation = [3usize, 2, 1, 0, 7, 6, 5, 4];
                    for &target in permutation.iter().take(entries) {
                        let value = self.stream.get2() as f32 / 2f32.powf(14.0 - self.pana_bpp as f32);
                        self.panasonic.black_level[target] = value;
                    }
                }
            }
            0x0002 => self.ifds[ifd].width = self.stream.getint(kind) as i32,
            0x0003 => self.ifds[ifd].height = self.stream.getint(kind) as i32,
            0x0005 => self.width = self.stream.get2() as u32,
            0x0006 => self.height = self.stream.get2() as u32,
            0x0007 => {
                let right = self.stream.get2() as u32;
                self.width = self.width.wrapping_add(right);
            }
            0x0009 => {
                let pattern = self.stream.get2() as u32;
                if pattern != 0 {
                    self.filters = pattern;
                }
            }
            0x0011 | 0x0012 => {
                if kind == TYPE_SHORT && count == 1 {
                    self.cam_mul[((tag - 0x0011) * 2) as usize] = self.stream.get2() as f32 / 256.0;
                }
            }
            0x0024..=0x0026 => self.cam_mul[(tag - 0x0024) as usize] = self.stream.get2() as f32,
            0x0027 => {
                let entries = self.stream.get2() as u32;
                if entries <= 0x100 {
                    for entry in 0..entries {
                        let light = self.stream.get2() as usize;
                        if light < 0x100 {
                            let red = self.stream.get2() as f32;
                            let green = self.stream.get2() as f32;
                            let blue = self.stream.get2() as f32;
                            if entry == 1 && entries > 6 && self.cam_mul[0] <= 0.001 {
                                self.cam_mul = [red, green, blue, green];
                            }
                        } else {
                            self.stream.skip(6);
                        }
                    }
                }
            }
            0x0118 | 0x0111 => {
                if tag == 0x0118 {
                    if kind != TYPE_LONG {
                        return Ok(());
                    }
                    self.load_raw = LoadRaw::Panasonic;
                    self.load_flags = 0x2008;
                }
                self.strip_offsets(ifd, count, base);
                self.ifds[ifd].offset = self.stream.get4() as i64 + base;
                self.inspect_jpeg_offset(ifd, false)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// DNGPrivateData (50740): Adobe's copy of the maker note, or a
    /// camera's own DNG maker note; in other files a Minolta block and an
    /// IFD.
    fn dng_private_data(&mut self, tag: u32, count: u32, base: &mut i64) -> Result<()> {
        let _ = tag;
        let start = self.stream.tell();
        let order = self.stream.order;
        let mut signature = [0u8; 8];
        self.stream.read(&mut signature[..6]);
        if &signature[..6] == b"Adobe\0" {
            let _ = count;
            self.unported_if_dng_maker_notes();
        } else {
            self.stream.read(&mut signature[6..8]);
            let text_end = signature.iter().position(|&byte| byte == 0).unwrap_or(8);
            let text = &signature[..text_end];
            let ricoh = text == b"RICOH" && {
                let marker = u16::from_le_bytes([signature[6], signature[7]]);
                marker == 0x4949 || marker == 0x4d4d
            };
            if text == b"PENTAX " || text == b"SAMSUNG" || ricoh {
                self.unported_if_dng_maker_notes();
            }
        }
        self.stream.seek(start);
        self.stream.order = order;
        if self.dng_version != 0 {
            return Ok(());
        }
        let offset = self.stream.get4() as i64 + *base;
        self.parse_minolta(offset)?;
        self.seek(offset);
        self.parse_tiff_ifd(*base)?;
        Ok(())
    }

    /// DNG maker notes (Adobe's MakN copy, Pentax, Samsung, Ricoh): only a
    /// few camera-specific DNG crops depend on them, which `identify`
    /// takes from the camera ids instead.
    fn unported_if_dng_maker_notes(&mut self) {}

    /// The `guess_cfa_pc` tail of CFAPattern and CFAPlaneColor.
    fn guess_cfa(&mut self, ifd: usize, pattern: &[u8; 16], length: usize, plane_colors: &[u8; 4]) {
        let mut table = [0u8; 256];
        for (color, &plane) in plane_colors.iter().enumerate().take(self.colors.min(4)) {
            table[plane as usize] = color as u8;
        }
        let mut filters = self.filters;
        for index in (0..16).rev() {
            filters = (filters << 2) | table[pattern[index % length] as usize] as u32;
        }
        if filters == 0 {
            filters = filters.wrapping_sub(1);
        }
        self.filters = filters;
        self.ifds[ifd].filters = filters;
    }

    /// BlackLevel (50714, and Fuji's 61450).
    fn black_level(&mut self, ifd: usize, tag: u32, kind: u32, count: u32) {
        let samples = self.ifds[ifd].samples;
        if samples > 1 && samples == count as i32 {
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            for index in 0..(count as usize).min(4) {
                levels.dng_fcblack[index] = self.stream.getrealf(kind);
                let value = (levels.dng_fcblack[index] + 0.5) as u32;
                levels.dng_cblack[index] = value;
                self.cblack[index] = value;
            }
            levels.dng_cblack[CBLACK_SIZE - 1] = count;
            levels.dng_fblack = 0.0;
            levels.dng_black = 0;
            self.black = 0;
        } else if samples > 1 && samples as u32 * self.cblack[4] * self.cblack[5] == count {
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            levels.dng_cblack[CBLACK_SIZE - 1] = count;
            self.cblack[CBLACK_SIZE - 1] = count;
            for index in 0..(count as usize).min(CBLACK_SIZE - 7) {
                levels.dng_fcblack[index + 6] = self.stream.getrealf(kind);
                let value = (levels.dng_fcblack[index + 6] + 0.5) as u32;
                levels.dng_cblack[index + 6] = value;
                self.cblack[index + 6] = value;
            }
        } else if self.cblack[4] * self.cblack[5] < 2 && count == 1 {
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            levels.dng_fblack = self.stream.getrealf(kind);
            levels.dng_black = levels.dng_fblack as u32;
            self.black = levels.dng_black;
        } else if self.cblack[4] * self.cblack[5] <= count {
            let pattern = (self.cblack[4] * self.cblack[5]) as usize;
            for index in 0..pattern {
                let value = self.stream.getrealf(kind);
                self.ifds[ifd].dng_levels.dng_fcblack[6 + index] = value;
                self.cblack[6 + index] = value as u32;
            }
            self.black = 0;
            self.cblack[..4].fill(0);
            if tag == 0xc61a {
                let levels = &mut self.ifds[ifd].dng_levels;
                levels.parsed |= DNGFM_BLACK;
                levels.dng_cblack[6..6 + pattern].copy_from_slice(&self.cblack[6..6 + pattern]);
                levels.dng_fblack = 0.0;
                levels.dng_black = 0;
                levels.dng_fcblack[..4].fill(0.0);
                levels.dng_cblack[..4].fill(0);
            }
        }
    }

    /// `linear_table`: a linearisation table into `curve`.
    pub(super) fn linear_table(&mut self, length: u32) {
        let length = length.min(0x10000) as usize;
        if length < 1 {
            return;
        }
        self.stream.read_shorts(&mut self.curve[..length]);
        for index in length..0x10000 {
            self.curve[index] = self.curve[index - 1];
        }
        self.maximum = self.curve[if length < 0x1000 { 0xfff } else { length - 1 }] as u32;
    }

    /// `parse_exif`.
    pub(super) fn parse_exif(&mut self, base: i64) -> Result<()> {
        let kodak = self.make.starts_with("EASTMAN") && self.tiff_nifds < 3;
        if self.exif_subdir_offset == 0 {
            self.exif_offset = base;
            self.exif_subdir_offset = self.stream.tell() as i64;
        }
        let entries = self.stream.get2();
        if self.make.starts_with("Hasselblad") && self.tiff_nifds > 3 && entries > 512 {
            return Ok(());
        }
        let file_size = self.stream.size() as i64;
        for _ in 0..entries {
            let Tag { tag, kind, count, save } = self.tiff_get(base)?;
            let value_position = self.stream.tell() as i64;
            if count > 8 && value_position + count as i64 > file_size * 2 {
                self.stream.seek(save);
                continue;
            }
            match tag {
                0x9209 => self.flash_used = self.stream.getrealf(kind),
                0x927c => {
                    let raspberry = (self.make.is_empty() && self.model.starts_with("ov5647")) || (self.make.starts_with("RaspberryPi") && (self.model.starts_with("RP_OV5647") || self.model.starts_with("RP_imx219")));
                    if raspberry {
                        self.unported("Raspberry Pi maker note");
                    } else if self.make.starts_with("SONY") && (self.model.starts_with("DSC-V3") || self.model.starts_with("DSC-F828")) {
                        self.unported("Sony SRF maker note");
                    } else {
                        if count == 1 && self.make.starts_with("NIKON") {
                            let offset = self.stream.get4();
                            if offset != 0 {
                                self.seek(offset as i64);
                            }
                            self.is_nikon_transfer = 1;
                        }
                        self.parse_makernote(base, 0)?;
                    }
                }
                0xa002 if kodak => self.raw_width = self.stream.get4(),
                0xa003 if kodak => self.raw_height = self.stream.get4(),
                0xa302 => {
                    if self.stream.get4() == 0x20002 {
                        self.exif_cfa = 0;
                        for shift in (0..8).step_by(2) {
                            let byte = self.stream.getc().map_or(u32::MAX, |byte| byte as u32);
                            self.exif_cfa |= byte.wrapping_mul(0x01010101) << shift;
                        }
                    }
                }
                _ => {}
            }
            self.stream.seek(save);
        }
        Ok(())
    }

    /// `apply_tiff`.
    pub(super) fn apply_tiff(&mut self) -> Result<()> {
        let mut raw: i64 = -1;
        let mut ties = 0u32;
        if self.dng_version != 0 {
            let mut frames = Vec::new();
            for (index, ifd) in self.ifds.iter().enumerate().take(self.tiff_nifds) {
                if ifd.width < 1 || ifd.width > 65535 || ifd.height < 1 || ifd.height > 65535 {
                    continue;
                }
                let subfile = ifd.newsubfiletype;
                if (subfile == 16 || subfile == 0 || (subfile & 0xffff) == 1) && ifd.phint != 32803 && ifd.phint != 34892 {
                    continue;
                }
                if subfile == 0 {
                    frames.push(index);
                    if ifd.samples == 2 {
                        frames.push(index);
                    }
                }
            }
            if let Some(&index) = frames.first() {
                let selected = self.ifds[index].clone();
                self.raw_width = selected.width as u32;
                self.raw_height = selected.height as u32;
                self.tiff_bps = selected.bps as u32;
                self.tiff_compress = selected.comp as u32;
                self.tiff_sampleformat = selected.sample_format as u32;
                self.data_offset = selected.offset;
                self.data_size = selected.bytes;
                self.tiff_flip = selected.flip;
                self.tiff_samples = selected.samples as u32;
                self.tile_width = selected.tile_width as u32;
                self.tile_length = selected.tile_length as u32;
                self.fuji_width = selected.fuji_width as u32;
                if self.tiff_samples != 2 {
                    self.filters = if selected.phint == 34892 {
                        0
                    } else if index > 0 && selected.phint == 32803 && self.ifds[0].phint == 32803 && selected.filters == 0 && self.ifds[0].filters != 0 {
                        self.ifds[0].filters
                    } else {
                        selected.filters
                    };
                    self.width = selected.vwidth as u32;
                    self.height = selected.vheight as u32;
                    self.top_margin = selected.top_margin as u32;
                    self.left_margin = selected.left_margin as u32;
                    if selected.dng_levels.dng_whitelevel[0] != 0 {
                        self.maximum = selected.dng_levels.dng_whitelevel[0];
                    } else if selected.sample_format <= 2 && self.tiff_bps > 0 && self.tiff_bps < 32 {
                        self.maximum = (1 << self.tiff_bps) - 1;
                    } else if selected.sample_format == 3 {
                        self.maximum = 1;
                    }
                }
                raw = index as i64;
                self.is_raw = frames.len() as u32;
            } else {
                self.is_raw = 0;
            }
        } else {
            if starts_with_ignore_case(&self.make, "Sony") {
                for ifd in self.ifds.iter_mut().take(self.tiff_nifds) {
                    if ifd.bps > 33 && ifd.samples == 1 {
                        let mut bps = 14;
                        if ifd.dng_levels.dng_whitelevel[0] > 0 {
                            for bit in 0..16 {
                                if (1i32 << bit) > ifd.dng_levels.dng_whitelevel[0] as i32 {
                                    bps = bit;
                                    break;
                                }
                            }
                        }
                        ifd.bps = bps;
                    }
                }
            }
            for index in 0..self.tiff_nifds {
                let ifd = self.ifds[index].clone();
                if ifd.width < 1 || ifd.width > 65535 || ifd.height < 1 || ifd.height > 65535 {
                    continue;
                }
                let mut old_size = self.raw_width as u64 * self.raw_height as u64;
                let mut new_size = (ifd.width as u32) as u64 * (ifd.height as u32) as u64;
                if self.tiff_bps != 0 {
                    old_size *= self.tiff_bps as u64;
                    new_size = new_size.wrapping_mul(ifd.bps as u32 as u64);
                }
                if ifd.phint == 2 && ifd.extrasamples > 0 && ifd.samples > 3 {
                    continue;
                }
                let sony = starts_with_ignore_case(&self.make, "Sony");
                if sony && ifd.bps == 8 && ifd.phint == 6 && ifd.comp == 7 && ifd.samples == 3 && ifd.newsubfiletype == 1 {
                    continue;
                }
                let not_jpeg_rgb = ifd.comp != 6 || ifd.samples != 3 || (ifd.comp == 6 && ifd.samples == 3 && sony && ifd.bps >= 12);
                if not_jpeg_rgb && ((ifd.width | ifd.height) as u32) < 0x10000 && (ifd.bps as u32) < 33 && (ifd.samples as u32) < 13 && new_size != 0 {
                    let larger = new_size > old_size;
                    let tie = new_size == old_size && {
                        let matched = self.shot_select == ties;
                        ties += 1;
                        matched
                    };
                    if larger {
                        ties = 1;
                    }
                    if larger || tie {
                        self.raw_width = ifd.width as u32;
                        self.raw_height = ifd.height as u32;
                        self.tiff_bps = ifd.bps as u32;
                        self.tiff_compress = ifd.comp as u32;
                        self.tiff_sampleformat = ifd.sample_format as u32;
                        self.data_offset = ifd.offset;
                        self.data_size = ifd.bytes;
                        self.tiff_flip = ifd.flip;
                        self.tiff_samples = ifd.samples as u32;
                        self.tile_width = ifd.tile_width as u32;
                        self.tile_length = ifd.tile_length as u32;
                        raw = index as i64;
                    }
                }
            }
            if self.is_raw == 1 && ties != 0 {
                self.is_raw = ties;
            }
        }
        if self.is_nikon_transfer != 0 && raw >= 0 {
            self.unported("Nikon Transfer files");
        }
        if self.tile_width == 0 {
            self.tile_width = i32::MAX as u32;
        }
        if self.tile_length == 0 {
            self.tile_length = i32::MAX as u32;
        }
        for index in (0..self.tiff_nifds).rev() {
            if self.ifds[index].flip != 0 {
                self.tiff_flip = self.ifds[index].flip;
            }
        }
        if raw >= 0 {
            self.raw_ifd = raw as usize;
        }

        if raw >= 0 && self.load_raw == LoadRaw::None {
            let raw_bytes = self.ifds[raw as usize].bytes;
            let raw_samples = self.ifds[raw as usize].samples;
            let raw_phint = self.ifds[raw as usize].phint;
            let pixels = self.raw_width as i64 * self.raw_height as i64;
            let sony = starts_with_ignore_case(&self.make, "Sony");
            let olympus = self.make.starts_with("OLYMPUS") || self.make.starts_with("OM Digi") || (starts_with_ignore_case(&self.make, "CLAUSS") && starts_with_ignore_case(&self.model, "piX 5oo"));
            let mut goto_slr = false;
            match self.tiff_compress {
                32767 => {
                    if self.dng_version == 0 && raw_bytes == pixels {
                        self.tiff_bps = 14;
                        self.load_raw = LoadRaw::Other("sony_arw2_load_raw");
                    } else if self.dng_version == 0 && sony && raw_bytes == pixels * 2 {
                        self.tiff_bps = 14;
                        self.load_raw = LoadRaw::Unpacked;
                    } else if raw_bytes * 8 != pixels * self.tiff_bps as i64 {
                        self.raw_height += 8;
                        self.load_raw = LoadRaw::Other("sony_arw_load_raw");
                    } else {
                        self.load_flags = 79 + 1;
                        goto_slr = true;
                    }
                }
                32769 => {
                    self.load_flags += 1;
                    goto_slr = true;
                }
                32770 | 32773 => goto_slr = true,
                0 | 1 => {
                    let float_dng = self.dng_version != 0 && self.tiff_sampleformat == 3 && self.tiff_bps > 8 && self.tiff_bps.is_multiple_of(8) && self.tiff_bps <= 32;
                    if float_dng {
                        self.load_raw = LoadRaw::UncompressedFpDng;
                    } else if self.dng_version == 0 && sony && raw_bytes == pixels * 2 {
                        self.tiff_bps = 14;
                        self.load_raw = LoadRaw::Unpacked;
                    } else if self.dng_version == 0 && sony && raw_samples == 4 && raw_bytes == pixels * 8 {
                        self.tiff_bps = 14;
                        self.tiff_samples = 4;
                        self.load_raw = LoadRaw::Other("sony_arq_load_raw");
                        self.filters = 0;
                        self.cdesc = "RGBG".to_string();
                    } else if starts_with_ignore_case(&self.make, "Nikon") && (self.software.starts_with("Nikon Scan") || self.is_nikon_transfer == 2 || contains_ignore_case(&self.model, "COOLSCAN")) {
                        self.load_raw = LoadRaw::Other("nikon_coolscan_load_raw");
                        self.raw_color = true;
                        self.colors = if self.tiff_samples == 3 { 3 } else { 1 };
                        self.filters = 0;
                    } else {
                        if olympus && raw_bytes * 2 == pixels * 3 {
                            self.load_flags = 24;
                        }
                        if self.dng_version == 0 && raw_bytes * 5 == pixels * 8 {
                            self.load_flags = 81;
                            self.tiff_bps = 12;
                        }
                        goto_slr = true;
                    }
                }
                6 | 7 | 99 => {
                    let sony_exact = self.make.eq_ignore_ascii_case("SONY");
                    if self.dng_version == 0 && self.tiff_compress == 6 && sony_exact {
                        if raw_phint == 6 && self.ifds[raw as usize].bps >= 12 && raw_samples == 3 {
                            self.load_raw = LoadRaw::Other("sony_ycbcr_load_raw");
                            self.filters = 0;
                            self.colors = 3;
                        } else {
                            self.load_raw = LoadRaw::Other("sony_ljpeg_load_raw");
                        }
                    } else if self.dng_version == 0 && self.tiff_compress == 7 && sony_exact && (self.model.eq_ignore_ascii_case("ILCE-1") || starts_with_ignore_case(&self.model, "ILCE-7") || starts_with_ignore_case(&self.model, "ILCE-9")) && self.tiff_bps == 14 && self.tiff_samples == 1 {
                        self.load_raw = LoadRaw::Other("sony_ljpeg_load_raw");
                    } else {
                        self.load_raw = LoadRaw::LosslessJpeg;
                    }
                }
                262 => self.load_raw = LoadRaw::Other("kodak_262_load_raw"),
                34713 => self.nikon_compression(raw as usize)?,
                65535 => self.load_raw = LoadRaw::Other("pentax_load_raw"),
                65000 => match raw_phint {
                    2 => {
                        self.load_raw = LoadRaw::Other("kodak_rgb_load_raw");
                        self.filters = 0;
                    }
                    6 => {
                        self.load_raw = LoadRaw::Other("kodak_ycbcr_load_raw");
                        self.filters = 0;
                    }
                    32803 => self.load_raw = LoadRaw::Other("kodak_65000_load_raw"),
                    _ => {}
                },
                32867 | 34892 | 8 => {}
                52546 | 9 if self.dng_version != 0 => {}
                _ => self.is_raw = 0,
            }
            if goto_slr {
                match self.tiff_bps {
                    8 => self.load_raw = LoadRaw::EightBit,
                    12 => {
                        if raw_phint == 2 {
                            self.load_flags = 6;
                        }
                        if starts_with_ignore_case(&self.make, "NIKON") && starts_with_ignore_case(&self.model, "COOLPIX A1000") && self.data_size == pixels * 2 {
                            self.load_raw = LoadRaw::Unpacked;
                        } else {
                            self.load_raw = LoadRaw::Packed;
                        }
                    }
                    14 | 16 => {
                        if self.tiff_bps == 14 {
                            self.load_flags = 0;
                        }
                        self.load_raw = LoadRaw::Unpacked;
                        if olympus && raw_bytes * 7 > pixels {
                            self.load_raw = LoadRaw::Other("olympus_load_raw");
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.dng_version == 0 {
            let raw_index = raw.max(0) as usize;
            let raw_ifd = self.ifds.get(raw_index).cloned().unwrap_or_default();
            let three_samples = self.tiff_samples == 3
                && raw_ifd.bytes != 0
                && !(self.tiff_bps == 16 && self.make.starts_with("Leaf"))
                && !(raw_ifd.comp == 6 && raw_ifd.phint == 6 && self.tiff_bps >= 12 && starts_with_ignore_case(&self.make, "Sony"))
                && self.tiff_bps != 14
                && (self.tiff_compress & !15) != 32768;
            let eight_bit = self.tiff_bps == 8 && !self.make.starts_with("Phase") && !self.make.starts_with("Leaf") && !contains_ignore_case(&self.make, "Kodak") && !self.model2.contains("DEBUG RAW");
            if (three_samples || eight_bit) && !contains_ignore_case(&self.model, "COOLSCAN") && !self.software.starts_with("Nikon Scan") && self.is_nikon_transfer != 2 {
                self.is_raw = 0;
            }
            if self.is_raw != 0 && raw >= 0 && raw_ifd.phint == 2 && raw_ifd.extrasamples > 0 && raw_ifd.samples > 3 {
                self.is_raw = 0;
            }
        }
        Ok(())
    }

    /// `apply_tiff` for Nikon's compression 34713.
    fn nikon_compression(&mut self, raw: usize) -> Result<()> {
        let bytes = self.ifds[raw].bytes;
        let width = self.raw_width as i64;
        let height = self.raw_height as i64;
        if (width + 9) / 10 * 16 * height == bytes {
            self.load_raw = LoadRaw::Packed;
            self.load_flags = 1;
        } else if width * height * 3 == bytes * 2 {
            self.load_raw = LoadRaw::Packed;
            if self.model.starts_with('N') {
                self.load_flags = 80;
            }
        } else if width * height * 3 == bytes {
            self.load_raw = LoadRaw::Other("nikon_yuv_load_raw");
            gamma_curve(&mut self.curve, 1.0 / 2.4, 12.92, 1, 4095);
            self.cblack.fill(0);
            self.filters = 0;
        } else if width * height * 2 == bytes {
            self.load_raw = LoadRaw::Unpacked;
            self.load_flags = 4;
            self.stream.order = BIG_ENDIAN;
        } else if self.ifds[raw].rows_per_strip != 0 && !self.ifds[raw].strip_offsets.is_empty() && self.ifds[raw].strip_offsets.len() == self.ifds[raw].strip_byte_counts.len() {
            let rows = self.ifds[raw].rows_per_strip as i64;
            let counts = &self.ifds[raw].strip_byte_counts;
            let fit = counts[..counts.len() - 1].iter().all(|&count| count * 2 == rows * width * 3);
            self.load_raw = if fit { LoadRaw::Other("nikon_load_striped_packed_raw") } else { LoadRaw::Nikon };
        } else if ((width * 3 / 2) + 15) / 16 * 16 * height == bytes {
            self.load_raw = LoadRaw::Other("nikon_load_padded_packed_raw");
            self.load_flags = (((width * 3 / 2) + 15) / 16 * 16) as u32;
        } else if (self.model.starts_with("NIKON Z 9") || self.model.starts_with("NIKON Z 8") || self.model == "NIKON Z f" || self.model == "NIKON Z6_3") && self.ifds[raw].offset != 0 {
            let position = self.stream.tell();
            let offset = self.ifds[raw].offset;
            self.seek(offset);
            let mut marker = [0u8; 4];
            self.stream.read(&mut marker);
            self.stream.seek(position);
            self.load_raw = if marker == [0xff, 0x10, 0xff, 0x50] { LoadRaw::Other("nikon_he_load_raw") } else { LoadRaw::Nikon };
        } else {
            self.load_raw = LoadRaw::Nikon;
        }
        Ok(())
    }
}
