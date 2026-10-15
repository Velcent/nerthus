//! LibRaw's state while a raw file is opened, unpacked and processed: the
//! globals of `imgdata` (`idata`, `sizes`, `color`) and of the internal
//! structures (`unpacker_data`, `internal_output_params`, the TIFF IFDs and
//! the maker note fields processing depends on), with the values
//! `identify` starts from.

use super::identify::{DngLevels, Ifd, CBLACK_SIZE};
use super::stream::Stream;

/// The `load_raw` decoder `identify` picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoadRaw {
    None,
    /// `lossless_dng_load_raw`: lossless JPEG tiles (DNG compression 7).
    LosslessDng,
    /// `packed_dng_load_raw`: uncompressed DNG integers.
    PackedDng,
    /// `deflate_dng_load_raw`: zip-compressed DNG floats.
    DeflateDng,
    /// `uncompressed_fp_dng_load_raw`: uncompressed DNG floats.
    UncompressedFpDng,
    /// `lossy_dng_load_raw` (unsupported: LibRaw is built without JPEG).
    LossyDng,
    /// `vc5_dng_load_raw_placeholder`, `jxl_dng_load_raw_placeholder`.
    UnsupportedDng,
    /// `lossless_jpeg_load_raw`: Canon CR2 and other lossless JPEGs, in
    /// `cr2_slice` slices.
    LosslessJpeg,
    /// `canon_sraw_load_raw`: Canon sRAW/mRAW (YCbCr lossless JPEG).
    CanonSraw,
    /// `canon_load_raw`: Canon CRW (CIFF) compressed data.
    Canon,
    /// `packed_load_raw`: bit-packed integers (`load_flags` options).
    Packed,
    /// `unpacked_load_raw`: 16-bit words holding `tiff_bps` bits.
    Unpacked,
    /// `unpacked_load_raw_reversed`.
    UnpackedReversed,
    /// `eight_bit_load_raw`: 8-bit samples through `curve`.
    EightBit,
    /// Any other LibRaw decoder, by name (not ported).
    Other(&'static str),
}

/// A `libraw_area_t` (`get_CanonArea`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Area {
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

/// The Canon maker note fields (`imCanon`) processing reads.
#[derive(Clone, Debug)]
pub(super) struct CanonNotes {
    pub wbi: u16,
    pub color_data_ver: i32,
    pub color_data_sub_ver: i32,
    pub channel_black_level: [i16; 4],
    pub average_black_level: i32,
    pub normal_white_level: i32,
    pub specular_white_level: i32,
    pub sensor_width: u16,
    pub sensor_height: u16,
    pub default_crop_absolute: Area,
    pub left_optical_black: Area,
    pub sensor_info_parsed: bool,
    pub multishot: [u32; 4],
    pub record_mode: u16,
    pub sraw_quality: u16,
    pub makernotes_flip: i32,
}

impl Default for CanonNotes {
    fn default() -> Self {
        CanonNotes {
            wbi: 0,
            color_data_ver: 0,
            color_data_sub_ver: 0,
            channel_black_level: [0; 4],
            average_black_level: 0,
            normal_white_level: 0,
            specular_white_level: 0,
            sensor_width: 0,
            sensor_height: 0,
            default_crop_absolute: Area { left: -1, top: -1, right: -1, bottom: -1 },
            left_optical_black: Area::default(),
            sensor_info_parsed: false,
            multishot: [0; 4],
            record_mode: 0,
            sraw_quality: 0,
            makernotes_flip: 0,
        }
    }
}

/// LibRaw's globals.
pub(super) struct Raw<'a> {
    /// `ifp`, with the byte `order` of what is being read.
    pub stream: Stream<'a>,

    // imgdata.idata
    pub make: String,
    pub model: String,
    pub model2: String,
    pub software: String,
    pub normalized_make: String,
    pub normalized_model: String,
    /// `maker_index` as its `CorpTable` name ("" when unknown).
    pub maker: &'static str,
    pub dng_version: u32,
    /// `is_raw` (`raw_count`).
    pub is_raw: u32,
    pub colors: usize,
    pub filters: u32,
    pub xtrans: [[u8; 6]; 6],
    pub xtrans_abs: [[u8; 6]; 6],
    pub cdesc: String,
    pub unique_id: u64,

    // imgdata.sizes
    pub raw_width: u32,
    pub raw_height: u32,
    pub width: u32,
    pub height: u32,
    pub top_margin: u32,
    pub left_margin: u32,
    pub pixel_aspect: f64,
    pub flip: i32,
    pub mask: [[i32; 4]; 8],

    // imgdata.color
    pub black: u32,
    pub cblack: Vec<u32>,
    pub maximum: u32,
    pub cam_mul: [f32; 4],
    pub pre_mul: [f32; 4],
    pub cmatrix: [[f32; 4]; 3],
    pub rgb_cam: [[f32; 4]; 3],
    pub cam_xyz: [[f32; 3]; 4],
    pub curve: Vec<u16>,
    pub white: [[u16; 8]; 8],
    pub raw_color: bool,
    pub flash_used: f32,
    pub canon_ev: f32,
    pub as_shot_wb_applied: u32,
    pub dng_levels: DngLevels,

    // unpacker_data and internal_output_params
    pub tiff_nifds: usize,
    pub ifds: Vec<Ifd>,
    pub tiff_bps: u32,
    pub tiff_compress: u32,
    pub tiff_samples: u32,
    pub tiff_sampleformat: u32,
    pub tiff_flip: i32,
    pub data_offset: i64,
    pub data_size: i64,
    pub meta_offset: i64,
    pub thumb_offset: i64,
    pub load_raw: LoadRaw,
    pub load_flags: u32,
    pub zero_is_bad: bool,
    pub fuji_width: u32,
    pub fuji_layout: u32,
    pub shot_select: u32,
    pub timestamp: i64,
    pub kodak_cbpp: u32,
    pub tile_width: u32,
    pub tile_length: u32,
    pub cr2_slice: [u16; 3],
    pub sraw_mul: [u16; 4],
    pub exif_cfa: u32,
    pub is_pana_raw: bool,
    pub is_sony: bool,
    pub is_olympus: bool,
    pub is_nikon_transfer: u32,
    pub cm_found: bool,
    pub metadata_blocks: u32,
    pub ifd0_offset: i64,
    pub exif_offset: i64,
    pub exif_subdir_offset: i64,
    pub zero_after_ff: bool,
    /// The raw IFD `apply_tiff` picked.
    pub raw_ifd: usize,

    // Maker notes
    pub canon: CanonNotes,
}

impl<'a> Raw<'a> {
    /// The state `identify` starts from.
    pub fn new(data: &'a [u8]) -> Raw<'a> {
        let mut cam_mul = [0.0f32; 4];
        let mut pre_mul = [0.0f32; 4];
        let mut rgb_cam = [[0.0f32; 4]; 3];
        for index in 0..4 {
            cam_mul[index] = if index == 1 { 1.0 } else { 0.0 };
            pre_mul[index] = if index < 3 { 1.0 } else { 0.0 };
            for (row, values) in rgb_cam.iter_mut().enumerate() {
                values[index] = if row == index { 1.0 } else { 0.0 };
            }
        }
        Raw {
            stream: Stream::new(data),
            make: String::new(),
            model: String::new(),
            model2: String::new(),
            software: String::new(),
            normalized_make: String::new(),
            normalized_model: String::new(),
            maker: "",
            dng_version: 0,
            is_raw: 1,
            colors: 3,
            filters: u32::MAX,
            xtrans: [[0; 6]; 6],
            xtrans_abs: [[0; 6]; 6],
            cdesc: String::new(),
            unique_id: 0,
            raw_width: 0,
            raw_height: 0,
            width: 0,
            height: 0,
            top_margin: 0,
            left_margin: 0,
            pixel_aspect: 1.0,
            flip: u32::MAX as i32,
            mask: [[0; 4]; 8],
            black: 0,
            cblack: vec![0; CBLACK_SIZE],
            maximum: 0,
            cam_mul,
            pre_mul,
            cmatrix: [[0.0; 4]; 3],
            rgb_cam,
            cam_xyz: [[0.0; 3]; 4],
            curve: (0..=u16::MAX).collect(),
            white: [[0; 8]; 8],
            raw_color: true,
            flash_used: 0.0,
            canon_ev: 0.0,
            as_shot_wb_applied: 0,
            dng_levels: DngLevels::default(),
            tiff_nifds: 0,
            ifds: Vec::new(),
            tiff_bps: 0,
            tiff_compress: 0,
            tiff_samples: 0,
            tiff_sampleformat: 0,
            tiff_flip: u32::MAX as i32,
            data_offset: 0,
            data_size: 0,
            meta_offset: 0,
            thumb_offset: 0,
            load_raw: LoadRaw::None,
            load_flags: 0,
            zero_is_bad: false,
            fuji_width: 0,
            fuji_layout: 0,
            shot_select: 0,
            timestamp: 0,
            kodak_cbpp: 0,
            tile_width: 0,
            tile_length: 0,
            cr2_slice: [0; 3],
            sraw_mul: [0; 4],
            exif_cfa: 0,
            is_pana_raw: false,
            is_sony: false,
            is_olympus: false,
            is_nikon_transfer: 0,
            cm_found: false,
            metadata_blocks: 0,
            ifd0_offset: -1,
            exif_offset: 0,
            exif_subdir_offset: 0,
            zero_after_ff: false,
            raw_ifd: 0,
            canon: CanonNotes::default(),
        }
    }

    /// `makeIs`.
    pub fn make_is(&self, maker: &str) -> bool {
        self.maker == maker
    }
}

/// `strncasecmp(text, prefix, prefix.len()) == 0`.
pub(super) fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// `strcasestr(text, needle) != NULL`.
pub(super) fn contains_ignore_case(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}
