//! The camera raw metadata pass for non-DNG files, Canon first. Walks each
//! container (CRW/CIFF, CR2/TIFF, CR3) the way the reference engine does, then
//! resolves dimensions, CFA layout, black and white levels, white balance and
//! colour matrices, so the resulting [`CameraMeta`] matches the oracle probe
//! field for field. Sensor pixels are still unpacked by rawler at this stage.

use super::identify::{CBLACK_SIZE, Ifd, cam_xyz_coeff};
use super::stream::{
    BIG_ENDIAN, LITTLE_ENDIAN, Stream, TYPE_IFD, TYPE_LONG, TYPE_RATIONAL, TYPE_SHORT,
    TYPE_UNDEFINED,
};

/// TIFF type byte counts (`tagtype_dataunit_bytes`), one entry per
/// `LIBRAW_EXIFTAG_TYPE_*` up to IFD8; wider types read as type 0.
const TYPE_UNIT_BYTES: [u32; 20] = [1, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 4, 2, 8, 8, 8, 8, 8];

/// `LIBRAW_MAX_IFD_COUNT`, the per-file IFD records cap.
const IFD_MAXCOUNT: usize = 10;

/// `LIBRAW_MAX_METADATA_BLOCKS`, the makernote-block guard.
const METADATA_MAXBLOCKS: usize = 1024;

/// `imgdata.params.use_camera_matrix` at open — the engine sets 1, not 3.
const USE_CAMERA_MATRIX_OPEN: u32 = 1;

/// `raw_aspect` when no aspect marker (Canon tag 0x009a) was seen.
const ASPECT_UNKNOWN: u32 = 0;
const ASPECT_OTHER: u32 = 1;
const ASPECT_3TO2: u32 = 1500;
const ASPECT_1TO1: u32 = 1000;
const ASPECT_4TO3: u32 = 1333;
const ASPECT_16TO9: u32 = 1777;
const ASPECT_5TO4: u32 = 1250;

/// Maker index returned when no colour-matrix row exists for a camera, in the
/// sense of `CM_found`: the camera table already supplied one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SensorKind {
    /// The generic pixel unpackers the engine's `load_raw` switch picks, named
    /// after their loaders so later phases can grow into per-maker loaders.
    /// Makers with native loaders this pass does not model (Sony, Nikon,
    /// Kodak, ...) stay `None` in the walker, which the engine treats as no
    /// loader and rejects the same files.
    CanonLosslessJpeg,
    CanonSraw,
    CanonCrx,
    /// `canon_load_raw`: the CRW heap's block-coded rows, with the body's
    /// pair of JPEG trees and its low-order-bits tail.
    CanonCiff,
    Unpacked,
    Packed,
    EightBit,
    /// The engine's `nikon_load_raw`: the 12-bit compressed NEF fallback.
    NikonPlain,
    /// `nikon_14bit_load_raw`: the Z-mount and D6 bodies' 14-bit packed rows.
    Nikon14Bit,
    /// `nikon_load_striped_packed_raw`: multi-strip packed 12-bit NEFs.
    NikonStripedPacked,
    /// `nikon_load_padded_packed_raw`, with `load_flags` = bytes per row.
    NikonPaddedPacked,
    /// The uncompressed YUV stream, kept only to name the loader the open
    /// stage rewrites; it never decodes pixels itself.
    NikonYuv,
    /// `nikon_load_sraw`: the uncompressed YUV bodies after the open stage's
    /// rewrite, with its own curve and the CC interpolation.
    NikonSraw,
    /// `nikon_he_load_raw`: the JPEG-XS container the Z 8/9 bodies write.
    NikonHe,
    /// `sony_arw2_load_raw`: ARW v2's LZ77-style 12-bit stream, ARW 2.0+.
    SonyArw2,
    /// `sony_arw_load_raw`: the packed 12-bit layout of the earliest ARWs,
    /// with the extra eight rows the metadata stage adds to `raw_height`.
    SonyArw,
    /// `sony_ljpeg_load_raw`: the tiled lossless-JPEG ARW stream.
    SonyLjpeg,
    /// `sony_arq_load_raw`: the four-sample pixel-shift composites.
    SonyArq,
    /// `fuji_compressed_load_raw`: the block-compressed RAF rows.
    FujiCompressed,
    /// `fuji_14bit_load_raw`: the packed fourteen-bit RAF rows.
    Fuji14Bit,
    /// `unpacked_load_raw_FujiDBP`: the DBP super-sensor's unpacked rows.
    FujiDbp,
    /// `olympus_load_raw`: the ORF loader whose row split the body's own bit
    /// plan spells out.
    Olympus,
    /// `panasonic_load_raw`: the RW2 rows packed as 8/7 bytes per pixel,
    /// selected by the 0x0118 offset tag or the file-size rule in the maker's
    /// arm.
    Panasonic,
    /// `panasonicC6_load_raw`: encoding 6's per-block 14/12-bit rows.
    PanasonicC6,
    /// `panasonicC7_load_raw`: encoding 7's plain 14/12-bit packed rows.
    PanasonicC7,
    /// `pentax_load_raw`: the PEF Huffman stream, seeded by the huff table
    /// the 0x0220 record points at.
    Pentax,
    /// `samsung_load_raw`: the compressed SRW rows of the first generation,
    /// picked by the body's own 0xa010 record.
    Samsung,
    /// `samsung2_load_raw`: the second SRW generation's rows.
    Samsung2,
    /// `samsung3_load_raw`: the third SRW generation's rows.
    Samsung3,
    /// `unpacked_load_raw_fuji_f700s20`: the oldest SuperCCD bodies' rows,
    /// read past their half-width record.
    FujiF700S20,
}

/// The white-balance index values Canon and Fuji records use, spelled out
/// with the same numbers the engine's enumeration gives them.
mod wbi {
    pub const UNKNOWN: usize = 0;
    pub const DAYLIGHT: usize = 1;
    pub const FLUORESCENT: usize = 2;
    pub const TUNGSTEN: usize = 3;
    pub const FLASH: usize = 4;
    pub const FINE_WEATHER: usize = 9;
    pub const CLOUDY: usize = 10;
    pub const SHADE: usize = 11;
    pub const FL_D: usize = 12;
    pub const FL_N: usize = 13;
    pub const FL_W: usize = 14;
    pub const FL_WW: usize = 15;
    pub const FL_L: usize = 16;
    pub const ILL_A: usize = 17;
    pub const D65: usize = 21;
    pub const UNDERWATER: usize = 65;
    pub const FLUORESCENT_HIGH: usize = 66;
    pub const AS_SHOT: usize = 81;
    pub const CUSTOM: usize = 83;
    pub const CUSTOM1: usize = 90;
    pub const CUSTOM2: usize = 91;
    pub const CUSTOM3: usize = 92;
    pub const CUSTOM4: usize = 93;
    pub const PC_SET1: usize = 96;
    pub const PC_SET2: usize = 97;
    pub const PC_SET3: usize = 98;
    pub const PC_SET4: usize = 99;
    pub const PC_SET5: usize = 100;
    pub const MEASURED: usize = 110;
    pub const BW: usize = 120;
    pub const KELVIN: usize = 254;
    pub const AUTO: usize = 82;
    pub const AUTO1: usize = 85;
    pub const OTHER: usize = 255;
}

/// Sony's white-balance index values the Minolta private block and the SR2
/// notes record; the shared Canon rows carry the same numbers.
mod sony_wbi {
    pub const UNKNOWN: usize = 0;
    pub const DAYLIGHT: usize = 1;
    pub const FLUORESCENT: usize = 2;
    pub const FLASH: usize = 4;
    pub const TUNGSTEN: usize = 3;
    pub const CLOUDY: usize = 10;
    pub const SHADE: usize = 11;
    pub const FL_D: usize = 12;
    pub const FL_N: usize = 13;
    pub const FL_W: usize = 14;
    pub const FL_WW: usize = 15;
    pub const FL_L: usize = 16;
    pub const CUSTOM: usize = 83;
    pub const AUTO: usize = 82;
    pub const STUDIO_TUNGSTEN: usize = 24;
}

/// Sony SR2 white-balance names for tags 0x7480-0x7486; rows above 255 are
/// colour-temperature entries the metadata pass does not carry.
const SONY_SR2_WB_LIST: [usize; 7] = [
    sony_wbi::DAYLIGHT,
    sony_wbi::CLOUDY,
    sony_wbi::TUNGSTEN,
    sony_wbi::FLASH,
    4500,
    sony_wbi::UNKNOWN,
    sony_wbi::FLUORESCENT,
];

/// Sony SR2 white-balance names for tags 0x7820-0x782d.
const SONY_SR2_WB_LIST1: [usize; 14] = [
    sony_wbi::DAYLIGHT,
    sony_wbi::CLOUDY,
    sony_wbi::TUNGSTEN,
    sony_wbi::FLASH,
    4500,
    sony_wbi::SHADE,
    sony_wbi::FL_W,
    sony_wbi::FL_N,
    sony_wbi::FL_D,
    sony_wbi::FL_L,
    8500,
    6000,
    3200,
    2500,
];

/// Fuji's body records name their preset white-balance rows by tag value;
/// this pair list is the engine's `tag2wbtable`.
const FUJI_TAG_2_WB_TABLE: [(u16, usize); 10] = [
    (0x2000, wbi::AUTO),
    (0x2100, wbi::FINE_WEATHER),
    (0x2200, wbi::SHADE),
    (0x2300, wbi::FL_D),
    (0x2301, wbi::FL_N),
    (0x2302, wbi::FL_W),
    (0x2310, wbi::FL_WW),
    (0x2311, wbi::FL_L),
    (0x2400, wbi::TUNGSTEN),
    (0x2410, wbi::FLASH),
];

/// `Fuji_wb_list1`: the preset rows the body's CCT table follows, fine
/// weather first.
const FUJI_WB_LIST1: [usize; 6] = [
    wbi::FINE_WEATHER,
    wbi::SHADE,
    wbi::FL_D,
    wbi::FL_N,
    wbi::FL_W,
    wbi::TUNGSTEN,
];

/// `Fuji_wb_list2`: body record to white-balance row pairs, keyed by the
/// `nWB` code of each 4096-record entry.
const FUJI_WB_LIST2: [(usize, u32); 11] = [
    (wbi::AUTO, 0),
    (wbi::CUSTOM, 6),
    (wbi::FINE_WEATHER, 1),
    (wbi::SHADE, 8),
    (wbi::FL_D, 10),
    (wbi::FL_N, 11),
    (wbi::FL_W, 12),
    (wbi::TUNGSTEN, 2),
    (wbi::UNDERWATER, 35),
    (wbi::ILL_A, 82),
    (wbi::D65, 83),
];

/// `FujiCCT_K`: the colour temperatures the body's WB table aligns to.
const FUJI_CCT_K: [f32; 31] = [
    2500.0, 2550.0, 2650.0, 2700.0, 2800.0, 2850.0, 2950.0, 3000.0, 3100.0, 3200.0, 3300.0, 3400.0,
    3600.0, 3700.0, 3800.0, 4000.0, 4200.0, 4300.0, 4500.0, 4800.0, 5000.0, 5300.0, 5600.0, 5900.0,
    6300.0, 6700.0, 7100.0, 7700.0, 8300.0, 9100.0, 10000.0,
];

/// The body record values that mark the newer RAFData layouts with 34 CCT
/// values instead of 31 (`imFuji.RAFDataVersion`); the models that imply the
/// same layout are below.
const FUJI_IS34_MODELS: [&str; 21] = [
    "X-Pro3",
    "GFX100S II",
    "GFX 100S II",
    "GFX100S",
    "GFX 100S",
    "GFX100 II",
    "GFX 100 II",
    "GFX50S II",
    "GFX 50S II",
    "X100VI",
    "X100V",
    "X-T4",
    "X-H2S",
    "X-H2",
    "X-E4",
    "X-T50",
    "X-T5",
    "X-M5",
    "X-T30 II",
    "X-S10",
    "X-S20",
];

/// Canon records store R, G1, B, G2 entries in RGGB order; these tables put
/// each entry on the engine's RGBG layout where G2 sits at index 2.
const RGGB_2_RGBG: [usize; 4] = [0, 1, 3, 2];
const GRBG_2_RGBG: [usize; 4] = [1, 0, 2, 3];

/// The byte-order maps above serve the Canon records; the Sony notes spell the
/// same mappings as bit formulas, so the Sony code calls these instead.
fn rggb_to_rgbg_index(index: usize) -> usize {
    index ^ (index >> 1)
}

fn grbg_to_rgbg_index(index: usize) -> usize {
    index ^ (index >> 1) ^ 1
}

/// `GRGB_2_RGBG(q)`: the layout the Fuji records spell plain — G1, R, G2, B.
fn grgb_to_rgbg_index(index: usize) -> usize {
    index ^ 1
}

/// `G2BRG1_2_RGBG(q)`: the A100's Minolta-private Bayer plan.
fn g2brg1_to_rgbg_index(index: usize) -> usize {
    index ^ (index >> 1) ^ 3
}

/// A checked-buffer u16: `None` past the buffer end, which the engine turns
/// into an `IO_EOF` throw that abandons the whole SR2 parse.
fn sony_buf2(buffer: &[u8], order: u16, offset: i64) -> Option<u16> {
    // The engine's checkoffset throws when `offset + 2` itself reaches the
    // length, one byte stricter than the bytes actually read.
    if offset < 0 || offset + 2 >= buffer.len() as i64 {
        return None;
    }
    let bytes = [buffer[offset as usize], buffer[offset as usize + 1]];
    Some(if order == LITTLE_ENDIAN {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}

/// A checked-buffer u32: the engine checks the offset and `offset + 4`
/// against the length, one byte stricter than the four bytes actually read.
fn sony_buf4(buffer: &[u8], order: u16, offset: i64) -> Option<u32> {
    if offset < 0 || offset + 4 >= buffer.len() as i64 {
        return None;
    }
    let start = offset as usize;
    let bytes = [
        buffer[start],
        buffer[start + 1],
        buffer[start + 2],
        buffer[start + 3],
    ];
    Some(if order == LITTLE_ENDIAN {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    })
}

/// `sony_decrypt`: the Sony SR2 stream cipher, plain u32 arithmetic. The
/// engine keeps the pad and the word cursor as persistent thread-locals, but
/// every call site passes `start = 1`, so each call re-derives them from the
/// key; the setup loops leave the cursor at 127, where the stream begins.
fn sony_decrypt(data: &mut [u8], words: usize, key: u32) {
    let mut pad = [0u32; 128];
    let mut key_next = key;
    for slot in pad[..4].iter_mut() {
        key_next = key_next.wrapping_mul(48828125).wrapping_add(1);
        *slot = key_next;
    }
    pad[3] = pad[3] << 1 | (pad[0] ^ pad[2]) >> 31;
    for p in 4..127usize {
        pad[p] = (pad[p - 4] ^ pad[p - 2]) << 1 | (pad[p - 3] ^ pad[p - 1]) >> 31;
    }
    for value in pad[..127].iter_mut() {
        *value = value.swap_bytes();
    }
    let mut cursor = 127usize;
    for block in data.chunks_exact_mut(4).take(words) {
        let update = pad[(cursor + 1) & 127] ^ pad[(cursor + 65) & 127];
        pad[cursor & 127] = update;
        let mut word = [0u8; 4];
        word.copy_from_slice(block);
        let decrypted = u32::from_le_bytes(word) ^ update;
        block.copy_from_slice(&decrypted.to_le_bytes());
        cursor = cursor.wrapping_add(1);
    }
}

/// `BG2RG1_2_RGBG`: the engine defines it as `q ^ 2`.
fn bg2rg1_2rgbg(index: usize) -> usize {
    index ^ 2
}

/// `sgetn`: fixed big-endian `n`-byte value, for CR3 headers.
fn sgetn(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |value, &byte| (value << 8) | byte as u64)
}

/// Canon's `Canon_wbi2std`: makernote white-balance index to engine index.
const CANON_WBI2STD: [usize; 24] = [
    wbi::AUTO,
    wbi::DAYLIGHT,
    wbi::CLOUDY,
    wbi::TUNGSTEN,
    wbi::FLUORESCENT,
    wbi::FLASH,
    wbi::CUSTOM,
    wbi::BW,
    wbi::SHADE,
    wbi::KELVIN,
    wbi::PC_SET1,
    wbi::PC_SET2,
    wbi::PC_SET3,
    wbi::UNKNOWN,
    wbi::FLUORESCENT_HIGH,
    wbi::CUSTOM1,
    wbi::CUSTOM2,
    wbi::UNDERWATER,
    wbi::CUSTOM3,
    wbi::CUSTOM4,
    wbi::PC_SET4,
    wbi::PC_SET5,
    wbi::UNKNOWN,
    wbi::AUTO1,
];

/// The engine's `canon[57][11]` dimensions table: raw width and height, left
/// and top margins, the width and height decrements, four edge-mask entries
/// and the CFA byte for `filters = value * 0x01010101`.
static CANON_DIMENSIONS: [[u16; 11]; 57] = [
    [1944, 1416, 0, 0, 48, 0, 0, 0, 0, 0, 0],
    [2144, 1560, 4, 8, 52, 2, 0, 0, 0, 25, 0],
    [2224, 1456, 48, 6, 0, 2, 0, 0, 0, 0, 0],
    [2376, 1728, 12, 6, 52, 2, 0, 0, 0, 0, 0],
    [2672, 1968, 12, 6, 44, 2, 0, 0, 0, 0, 0],
    [3152, 2068, 64, 12, 0, 0, 16, 0, 0, 0, 0],
    [3160, 2344, 44, 12, 4, 4, 0, 0, 0, 0, 0],
    [3344, 2484, 4, 6, 52, 6, 0, 0, 0, 0, 0],
    [3516, 2328, 42, 14, 0, 0, 0, 0, 0, 0, 0],
    [3596, 2360, 74, 12, 0, 0, 0, 0, 0, 0, 0],
    [3744, 2784, 52, 12, 8, 12, 0, 0, 0, 0, 0],
    [3944, 2622, 30, 18, 6, 2, 0, 0, 0, 0, 0],
    [3948, 2622, 42, 18, 0, 2, 0, 0, 0, 0, 0],
    [3984, 2622, 76, 20, 0, 2, 14, 0, 0, 0, 0],
    [4032, 2656, 112, 44, 10, 0, 0, 0, 0, 0, 0],
    [4104, 3048, 48, 12, 24, 12, 0, 0, 0, 0, 0],
    [4116, 2178, 4, 2, 0, 0, 0, 0, 0, 0, 0],
    [4152, 2772, 192, 12, 0, 0, 0, 0, 0, 0, 0],
    [4160, 3124, 104, 11, 8, 65, 0, 0, 0, 0, 0],
    [4176, 3062, 96, 17, 8, 0, 0, 16, 0, 7, 73],
    [4192, 3062, 96, 17, 24, 0, 0, 16, 0, 0, 73],
    [4312, 2876, 22, 18, 0, 2, 0, 0, 0, 0, 0],
    [4352, 2850, 144, 46, 0, 0, 0, 0, 0, 0, 0],
    [4352, 2874, 62, 18, 0, 0, 0, 0, 0, 0, 0],
    [4476, 2954, 90, 34, 0, 0, 0, 0, 0, 0, 0],
    [4480, 3348, 12, 10, 36, 12, 0, 0, 0, 18, 73],
    [4480, 3366, 80, 50, 0, 0, 0, 0, 0, 0, 0],
    [4496, 3366, 80, 50, 12, 0, 0, 0, 0, 0, 0],
    [4768, 3516, 96, 16, 0, 0, 0, 16, 0, 0, 0],
    [4832, 3204, 62, 26, 0, 0, 0, 0, 0, 0, 0],
    [4832, 3228, 62, 51, 0, 0, 0, 0, 0, 0, 0],
    [5108, 3349, 98, 13, 0, 0, 0, 0, 0, 0, 0],
    [5120, 3318, 142, 45, 62, 0, 0, 0, 0, 0, 0],
    [5280, 3528, 72, 52, 0, 0, 0, 0, 0, 0, 0],
    [5344, 3516, 142, 51, 0, 0, 0, 0, 0, 0, 0],
    [5344, 3584, 126, 100, 0, 2, 0, 0, 0, 0, 0],
    [5344, 3950, 98, 18, 0, 0, 0, 24, 0, 0, 0],
    [5360, 3516, 158, 51, 0, 0, 0, 0, 0, 0, 0],
    [5568, 3708, 72, 38, 0, 0, 0, 0, 0, 0, 0],
    [5632, 3710, 96, 17, 0, 0, 0, 16, 0, 0, 73],
    [5712, 3774, 62, 20, 10, 2, 0, 0, 0, 0, 0],
    [5792, 3804, 158, 51, 0, 0, 0, 0, 0, 0, 0],
    [5920, 3950, 122, 80, 2, 0, 0, 0, 0, 0, 0],
    [6096, 4051, 76, 35, 0, 0, 0, 0, 0, 0, 0],
    [6096, 4056, 72, 34, 0, 0, 0, 0, 0, 0, 0],
    [6288, 4056, 264, 36, 0, 0, 0, 0, 0, 0, 0],
    [6384, 4224, 120, 44, 0, 0, 0, 0, 0, 0, 0],
    [6880, 4544, 136, 42, 0, 0, 0, 0, 0, 0, 0],
    [6888, 4546, 146, 48, 0, 0, 0, 0, 0, 0, 0],
    [7128, 4732, 144, 72, 0, 0, 0, 0, 0, 0, 0],
    [8896, 5920, 160, 64, 0, 0, 0, 0, 0, 0, 0],
    // Rows of the second table half; entries 51-56 sit after the 8960-wide
    // row on purpose, matching the engine's order.
    [6192, 4152, 160, 120, 0, 0, 0, 0, 0, 0, 0],
    [6192, 4060, 168, 52, 24, 8, 16, 48, 32, 0, 0],
    [6188, 4120, 154, 96, 12, 0, 16, 48, 32, 0, 0],
    [8480, 5650, 258, 162, 0, 0, 128, 2, 128, 0, 0],
    [5376, 3574, 258, 162, 0, 0, 128, 2, 128, 0, 0],
    [6160, 4144, 128, 120, 0, 0, 0, 0, 0, 0, 0],
];

/// `get_CanonArea`: four `get2` values, left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CanonArea {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
}

impl CanonArea {
    const fn never_set() -> Self {
        CanonArea { l: -1, t: -1, r: -1, b: -1 }
    }

    fn width(&self) -> i32 {
        self.r - self.l + 1
    }

    fn height(&self) -> i32 {
        self.b - self.t + 1
    }
}

/// `raw_inset_crops[0]`'s own shape: width and height first, then the corner.
#[derive(Debug, Clone, Copy, Default)]
struct CanonInset {
    width: i32,
    height: i32,
    left: i32,
    top: i32,
}

/// The Canon makernote facts that feed metadata, in `imCanon` order.
#[derive(Debug, Clone, Default)]
struct CanonState {
    /// `MN.canon.DefaultCropAbsolute`, tag 0x00e0 / CIFF 0x1031.
    default_crop_absolute: Option<CanonArea>,
    left_optical_black: Option<CanonArea>,
    sensor_width: i32,
    sensor_height: i32,
    channel_black: [u32; 4],
    average_black: u32,
    normal_white: u32,
    specular_white: u32,
    sraw_mul: [u16; 4],
    color_data_ver: u32,
    color_data_sub_ver: u32,
    /// Canon white-balance index, unknown (zero) until ShotInfo sets it.
    wbi: i32,
    makernotes_flip: u32,
    /// `raw_aspect`, tag 0x009a.
    raw_aspect: u32,
    /// `raw_inset_crops[0]`, the in-camera crop as width, height and origin.
    inset_crops: Option<CanonInset>,
    /// `CR3_CTMDtag`: the makernote came from a CR3 CTMD item.
    cr3_ctmd: bool,
    /// CanonCameraInfo body id from makernote tag 0x0010 / CIFF 0x5834.
    unique_id: u64,
}

/// `raw_inset_crops[k]`: the in-camera crop a RAF body records. An unset
/// origin is the engine's `0xffff` mark and an unset size is zero.
#[derive(Debug, Clone, Copy)]
struct FujiInset {
    cleft: u16,
    ctop: u16,
    cwidth: u16,
    cheight: u16,
}

impl Default for FujiInset {
    fn default() -> Self {
        FujiInset {
            cleft: 0xffff,
            ctop: 0xffff,
            cwidth: 0,
            cheight: 0,
        }
    }
}

/// The Fuji body's RAF facts that feed the metadata pass, in `imFuji` order.
#[derive(Debug, Clone, Default)]
struct FujiState {
    /// `RAFDataGeneration`: the RAFData table's layout; the `0x4953` header
    /// scan sets 1-4 and the 4096-byte layout sets `4096`.
    raf_data_generation: u32,
    /// `RAFDataVersion`, the generation 2/3/4 record version.
    raf_data_version: u32,
    /// `RAFData_ImageSizeTable`: the body's size table, width first.
    raf_data_image_size_table: [u32; 32],
    /// `BlackLevel`: the body's 0x4000 record, four plus four channel values
    /// after the row-width mark.
    black_level: [u16; 10],
    /// `raw_inset_crops[2]`: the body's crop records.
    inset_crops: [FujiInset; 2],
    /// `imgdata.sizes.raw_aspect`, from the 0x0115 aspect record.
    raw_aspect: u32,
    /// The 0xc000 RAFData record's position and half-length; the TIFF 0xf00c
    /// scan reads that table.
    raf_data_pos: u64,
    raf_data_len: u32,
    /// `fuji_total_lines`: the block-compressed stream's row count, from the
    /// 0x4953 header `finish_open` validates.
    total_lines: u32,
    /// `fuji_total_blocks`, the blocks that share one row group.
    total_blocks: u32,
    /// `fuji_block_width`, the samples per block.
    block_width: u32,
    /// `fuji_bits`, the compressed stream's bit depth.
    compr_bits: u32,
    /// `fuji_raw_type`, which the header spells as 16 or 0.
    compr_raw_type: u32,
    /// `fuji_lossless`.
    compr_lossless: u32,
}

/// The compressed-stream facts the RAF pixel decoder consumes.
#[derive(Debug, Clone, Copy, Default)]
pub struct FujiComprState {
    /// `fuji_total_lines`, the row groups per block stream.
    pub total_lines: u32,
    /// `fuji_total_blocks`, the blocks sharing one row group width.
    pub total_blocks: u32,
    /// `fuji_block_width`, the samples per block.
    pub block_width: u32,
    /// `fuji_bits`.
    pub compr_bits: u32,
    /// `fuji_raw_type`, 16 or 0.
    pub compr_raw_type: u32,
    /// `fuji_lossless`.
    pub compr_lossless: u32,
}

/// The Olympus note's facts this pass carries, in `imOly` order.
#[derive(Debug, Clone, Default)]
struct OlyState {
    /// `is_Olympus` — true when the maker is Olympus (or the CLAUSS clone),
    /// which routes the note's entries to the body-arm block.
    is_olympus: bool,
    /// `OlyID`, the five-character body code as packed bits; the body
    /// fix-ups branch on it.
    oly_id: u64,
    /// `OlympusColorSpace`, CameraSettings tag 0x0507.
    color_space: u32,
    /// `OlympusValidBits`, from ImageProcessing tag 0x102c.
    valid_bits: u32,
    /// `imgdata.sizes.raw_aspect`, ImageProcessing tag 0x1112.
    raw_aspect: u32,
    /// The compression plan's status shorts, 0x0640 through 0x0653, the
    /// maker loader's Huffman and prediction tuning. Their defaults keep
    /// their names: the loader names the shorts after the tags.
    tag_x640: u32,
    tag_x641: u32,
    tag_x642: u32,
    tag_x643: u32,
    tag_x644: u32,
    tag_x645: u32,
    tag_x646: u32,
    tag_x647: u32,
    tag_x648: u32,
    tag_x649: u32,
    tag_x650: u32,
    tag_x651: u32,
    tag_x652: u32,
    tag_x653: u32,
}

/// `OlyID_str2hex`: the body code's five characters, from a packed
/// twenty-four-bit number whose low bytes repeat the last one.
const fn oly_id_value(chars: &[u8]) -> u64 {
    ((chars[0] as u64) << 32) | ((chars[1] as u64) << 24) | ((chars[2] as u64) << 16)
        | ((chars[3] as u64) << 8) | (chars[4] as u64)
}

/// The camera-id numbers Olympus spells in its CameraType2 note tag, named for
/// the body each identifies.
pub(super) mod oly_id {
    use super::oly_id_value;

    pub const E_1: u64 = oly_id_value(b"D4040");
    pub const E_300: u64 = oly_id_value(b"D4041");
    pub const E_500: u64 = oly_id_value(b"S0004");
    pub const E_330: u64 = oly_id_value(b"S0003");
    pub const SP_550UZ: u64 = oly_id_value(b"D4321");
    pub const SP_510UZ: u64 = oly_id_value(b"D4322");
    pub const STYLUS_1: u64 = oly_id_value(b"D4572");
    pub const TG_4: u64 = oly_id_value(b"D4586");
    pub const TG_5: u64 = oly_id_value(b"D4593");
    pub const TG_6: u64 = oly_id_value(b"D4603");
    pub const TG_7: u64 = oly_id_value(b"D4605");
    pub const NORMA: u64 = oly_id_value(b"NORMA");
    pub const C_740UZ: u64 = oly_id_value(b"SX754");
    pub const C_770UZ: u64 = oly_id_value(b"SX772");
    pub const E_PL5: u64 = oly_id_value(b"S0042");
}

/// The Panasonic container's facts this pass carries, in the engine's
/// `is_pana_raw` / `pana_*` order.
#[derive(Debug, Clone, Default)]
struct PanaState {
    /// `is_pana_raw`: the body's own layout flag, read from the first tag
    /// 0x0001 carrying a long value. From then on the tags the EXIF and
    /// PanasonicRaw layouts share read the Panasonic records, like the
    /// engine's split of the tag walk.
    raw: u32,
    /// `pana_bpp`, BitsPerSample clamped to 8..16.
    bits: u32,
    /// `pana_encoding`, the RawFormat record: 4 is the ordinary RW2 layout
    /// whose black levels gain 15, 5 is the pixel-shift one.
    encoding: u32,
    /// `pana_black`, the Black{Red,Green,Blue}Level records.
    black: [u32; 3],
    /// `linear_max`, the LinearityLimit{Red,Green,Blue} records, with the
    /// engine's clipping: 16383 loses 64 and 4095 loses 16.
    linear_max: [u32; 4],
}

/// `pana`, the sensor records keyed to a body's exact raw size. A row match
/// adds its margins to the frame and trims its inset counts.
const PANA_ROWS: [[i32; 6]; 25] = [
    [3130, 1743, 4, 0, -6, 0],
    [3130, 2055, 4, 0, -6, 0],
    [3130, 2319, 4, 0, -6, 0],
    [3170, 2103, 18, 0, -42, 20],
    [3170, 2367, 18, 13, -42, -21],
    [3177, 2367, 0, 0, -1, 0],
    [3304, 2458, 0, 0, -1, 0],
    [3330, 2463, 9, 0, -5, 0],
    [3330, 2479, 9, 0, -17, 4],
    [3370, 1899, 15, 0, -44, 20],
    [3370, 2235, 15, 0, -44, 20],
    [3370, 2511, 15, 10, -44, -21],
    [3690, 2751, 3, 0, -8, -3],
    [3710, 2751, 0, 0, -3, 0],
    [3724, 2450, 0, 0, 0, -2],
    [3770, 2487, 17, 0, -44, 19],
    [3770, 2799, 17, 15, -44, -19],
    [3880, 2170, 6, 0, -6, 0],
    [4060, 3018, 0, 0, 0, -2],
    [4290, 2391, 3, 0, -8, -1],
    [4330, 2439, 17, 15, -44, -19],
    [4508, 2962, 0, 0, -3, -4],
    [4508, 3330, 0, 0, -3, -6],
    // The DC-G9 and DC-GH7 high-resolution sizes.
    [10480, 7794, 0, 0, -2, 0],
    [11560, 8680, 8, 8, -16, -16],
];

/// The CFA byte each combination of flag and margin parity folds into, the
/// engine's `"\x94\x61\x49\x16"` body code.
const PANA_FOLD: [u32; 4] = [0x94, 0x61, 0x49, 0x16];

/// The body ids the Pentax records carry, the engine's `PentaxID_*` values
/// this pass keys the frame and balance rules on.
const PENTAX_ID_OPTIO_S_V101: u64 = 0x12971;
const PENTAX_ID_OPTIO_33WR: u64 = 0x129c6;
const PENTAX_ID_STARIST_D: u64 = 0x12994;
const PENTAX_ID_STARIST_DS: u64 = 0x12aa2;
const PENTAX_ID_K_7: u64 = 0x12db8;
const PENTAX_ID_K_X: u64 = 0x12dfe;
const PENTAX_ID_645D: u64 = 0x12e08;
const PENTAX_ID_K_R: u64 = 0x12e6c;
const PENTAX_ID_K_5: u64 = 0x12e76;
const PENTAX_ID_K_5_II: u64 = 0x12f70;
const PENTAX_ID_K_5_II_S: u64 = 0x12f71;
const PENTAX_ID_K_3: u64 = 0x12fc0;
const PENTAX_ID_K_1: u64 = 0x13092;
const PENTAX_ID_K_3_II: u64 = 0x1309c;
const PENTAX_ID_K_70: u64 = 0x13222;
const PENTAX_ID_KP: u64 = 0x1322c;
const PENTAX_ID_K_1_MARK_II: u64 = 0x13240;
const PENTAX_ID_K_3_III: u64 = 0x13254;
const PENTAX_ID_KF: u64 = 0x132b8;
const PENTAX_ID_K_3_III_MONO: u64 = 0x132d6;

/// The Olympus compression plan the maker loader tunes through: the body's
/// `ValidBits` mark and the raw-image record's `tagX640`-`tagX653` status
/// shorts. Their defaults keep their names, the loader naming each after
/// its tag.
#[derive(Debug, Clone, Default)]
/// The fields the striped loader reads off the directory it matches by its
/// raw-data offset, one record per directory the walker walked.
pub(super) struct StripIfdFacts {
    pub offset: i64,
    pub rows_per_strip: i32,
    pub strip_offsets: Vec<i64>,
}

/// The Olympus compression plan, consumed by the maker's pixel loader.
#[derive(Debug, Clone)]
pub(super) struct OlympusState {
    pub valid_bits: u32,
    pub tag_x640: u32,
    pub tag_x641: u32,
    pub tag_x642: u32,
    pub tag_x643: u32,
    pub tag_x644: u32,
    pub tag_x645: u32,
    pub tag_x646: u32,
    pub tag_x647: u32,
    pub tag_x648: u32,
    pub tag_x649: u32,
    pub tag_x650: u32,
    pub tag_x651: u32,
    pub tag_x652: u32,
    pub tag_x653: u32,
}

/// The Panasonic packing facts the maker's pixel loader branches on: the
/// body's `BitsPerSample` short, clamped 8..16 when read, and the
/// `RawFormat` record — 5 is the block layout, anything else the
/// bit-packed rows.
#[derive(Debug, Clone, Default)]
pub(super) struct PanasonicState {
    pub bits: u32,
    pub encoding: u32,
}

/// The CRX image header (`crx_data_header_t`) of the `stsd/CRAW` record the
/// CR3 track selection picked: the whole CRX decode plan — plane count, bit
/// depth, wavelet levels, tiling, quantization — comes from it. `f_width`/
/// `f_height` are the coded plane extents, generally larger than the active
/// crop the ActiveArea record resolves.
#[derive(Debug, Clone, Default)]
pub(super) struct CameraCrxHeader {
    pub version: u32,
    pub f_width: u32,
    pub f_height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub n_planes: u32,
    pub cfa_layout: u32,
    pub enc_type: u32,
    pub image_levels: u32,
    pub n_bits: u32,
    pub median_bits: u32,
    pub mdat_header_size: u32,
}

/// The resolved metadata for a non-DNG camera raw, ready for the decoder's
/// `Raw` state. Field names follow the metadata engine's.
#[derive(Debug, Clone)]
pub(super) struct CameraMeta {
    pub make: String,
    pub model: String,
    pub colors: usize,
    pub filters: u32,
    pub xtrans: [[u8; 6]; 6],
    /// `xtrans_abs`: the X-Trans pattern in sensor order; the compressed-RAF
    /// pixel loader copies lines out by it, while `xtrans` carries the
    /// margin-rotated form.
    pub xtrans_abs: [[u8; 6]; 6],
    pub raw_width: u32,
    pub raw_height: u32,
    pub width: u32,
    pub height: u32,
    pub left_margin: u32,
    pub top_margin: u32,
    pub pixel_aspect: f64,
    pub fuji_width: u32,
    pub tiff_bps: u32,
    pub tiff_samples: u32,
    pub data_offset: i64,
    pub data_size: i64,
    /// `strip_offset`: the Samsung strip-table offset, `unpacker_data`'s
    /// record of tag 0xa010; the other loaders never read it.
    pub strip_offset: i64,
    /// `fuji_layout`: the SuperCCD interleave mark the body's 0x0130
    /// layout record sets; the processing path keys its pixel placement on
    /// it.
    pub fuji_layout: u32,
    /// `meta_offset`: where a maker record the pixel loader needs sits —
    /// Pentax's Huffman table, Nikon's tree or curve, the raw-IFD's
    /// opcode2 base — as its walker left it.
    pub meta_offset: i64,
    /// The byte order the parse ends with; the Pentax loader reads its
    /// table with it, while the Samsung body forces its own little-endian.
    pub order: u16,
    pub tile_width: u32,
    pub tile_length: u32,
    pub curve: Vec<u16>,
    pub black: u32,
    pub cblack: [u32; CBLACK_SIZE],
    pub maximum: u32,
    pub pre_mul: [f32; 4],
    pub cam_mul: [f32; 4],
    pub rgb_cam: [[f32; 4]; 3],
    /// `raw_color`: true when the colour matrix is the camera's own identity
    /// and the camera table may replace it.
    pub raw_color: bool,
    pub mask: [[i32; 4]; 8],
    pub loader: SensorKind,
    /// CR2 `cr2_slice` and sRAW multipliers, carried for the unpacking phase.
    pub cr2_slice: [u16; 3],
    pub sraw_mul: [u16; 4],
    /// `load_flags`, needed by the sRAW and CRX unpackers later.
    pub load_flags: u32,
    /// `tiff_compress`, the compression record: the raw IFD's for files
    /// without one, and Canon's 0x1835 decoder-table pick for CRW.
    pub tiff_compress: u32,
    /// `zero_is_bad`, the loader's zero-trim mark.
    pub zero_is_bad: bool,
    /// `raw_pitch` for sRAW (8 bytes per raw column).
    pub raw_pitch: u32,
    /// `canon_unique_id`, the body id record the Canon makernote 0x0010
    /// carries; the sRAW loader's hue flip and its RGB pass key on it.
    pub canon_unique_id: u64,
    /// `model2`, the firmware string the Canon makernote 0x0007 fgets
    /// reads; the sRAW loader sniffs a three-part version out of it.
    pub model2: String,
    /// The Olympus compression plan, consumed by the maker's pixel loader.
    pub olympus: OlympusState,
    /// The Panasonic packing facts, consumed by the maker's pixel loader.
    pub panasonic: PanasonicState,
    /// The CRX image header of the chosen CR3 raw track, consumed by the
    /// CRX pixel loader.
    pub crx: CameraCrxHeader,
    /// The Fuji compressed-stream facts, consumed by the RAF pixel loader.
    pub fuji: FujiComprState,
    /// Every directory's strip facts in walk order, for the striped loader's
    /// own directory search by raw-data offset.
    pub strip_ifds: Vec<StripIfdFacts>,
    /// The open-stage refusal reason: when set, the file may not decode at
    /// all — the reference engine fails its open on such records.
    pub refusal: Option<&'static str>,
}

/// The container walker. Holds the engine's identify-stage globals while the
/// container is parsed, then resolves them in `finish_identify` and applies
/// the open-stage rules in `finish_open`.
struct Walker<'a> {
    stream: Stream<'a>,
    makes_seen: usize,
    make: String,
    model: String,
    /// `filters` before resolution: `u32::MAX` means "not seen".
    filters: u32,
    colors: usize,
    width: i32,
    height: i32,
    raw_width: i32,
    raw_height: i32,
    top_margin: i32,
    left_margin: i32,
    flip: u32,
    tiff_flip: u32,
    pixel_aspect: f64,
    fuji_width: u32,
    black: u32,
    cblack: [u32; CBLACK_SIZE],
    maximum: u32,
    pre_mul: [f32; 4],
    cam_mul: [f32; 4],
    cam_xyz: [[f32; 3]; 4],
    cmatrix: [[f32; 4]; 3],
    rgb_cam: [[f32; 4]; 3],
    /// `raw_color` and `CM_found`.
    raw_color: bool,
    cm_found: bool,
    tiff_bps: u32,
    tiff_samples: u32,
    tiff_compress: u32,
    tiff_sampleformat: u32,
    data_offset: i64,
    data_size: i64,
    tile_width: u32,
    tile_length: u32,
    meta_offset: i64,
    /// `strip_offset`: the Samsung bodies' strip-table offset, the 0xa010
    /// record's first read.
    strip_offset: i64,
    mask: [[i32; 4]; 8],
    xtrans: [[u8; 6]; 6],
    ifds: Vec<Ifd>,
    /// `tiff_nifds`: how many slots the parse has handed out. A leaked index
    /// may grow the vector past this count; those extra slots never take part
    /// in the raw pick, and the next directory parse re-zeroes the first of
    /// them for its own frame.
    ifd_count: usize,
    /// `is_raw` with the engine's integer meaning: 0 rejects the file, 1 is a
    /// single raw, higher values count frames or multi-shot records.
    is_raw: i32,
    dng_version: u32,
    /// `software`, tag 0x0131: fgets keeps the trailing newline, which makes
    /// the engine's exact-equality check against "Digital Photo Professional"
    /// never fire, so the port keeps the newline and the same behaviour.
    software: String,
    /// `model2`, the Canon makernote 0x0007 firmware string: the sRAW
    /// loader's hue flip keys on the version it spells.
    model2: String,
    /// `is_NikonTransfer`: 2 when an 8-bit debayed TIFF was detected.
    nikon_transfer: i32,
    /// `shot_select`: which raw frame or track to open; always 0 here.
    shot_select: i32,
    /// `use_camera_wb`, the open default: white balance from the camera, not
    /// the file's colour-space records.
    use_camera_wb: bool,
    /// The IFD selected by `apply_tiff` (`raw`), or -1.
    raw_ifd: i32,
    /// `cr2_slice`, tag 0xc640, for the sRAW column plan.
    cr2_slice: [u16; 3],
    /// `extrasamples` per IFD (tag 0x0152), for the RGB-alpha sample skip.
    extrasamples: Vec<u32>,
    loader: Option<SensorKind>,
    /// The open-stage refusal: the engine fails the whole open when the
    /// record's size contradicts its layout; neither engine nor rawler may
    /// decode such a file.
    refusal: Option<&'static str>,
    load_flags: u32,
    load_flags2: u32,
    /// `raw_pitch`, the sRAW row stride in bytes.
    raw_pitch: u32,
    canon: CanonState,
    /// `metadata_blocks`.
    metadata_blocks: usize,
    /// CR3 container state: the chosen track, chunk offsets, sample tables.
    cr3: Cr3State,
    /// `imgdata.color.icWBC`: the CIFF white-balance rows Canon records fill,
    /// shared across the container walk like the engine's colour state.
    ciff_icwbc: [[u16; 4]; 256],
    /// `imCommon.firmware`: CIFF record 0x080b, read by the 0x0032 gate.
    firmware: String,
    /// `zero_after_ff` for JPEG-marker parsing.
    zero_after_ff: bool,
    /// `zero_is_bad`: the loader trims a zero tail, set by the maker arms.
    zero_is_bad: bool,
    /// `unique_id`, the body id record the Pentax notes carry; the frame-size
    /// arm and the loader table both key on it.
    pentax_id: u64,
    /// `imSamsung.key`: the hidden offsets the body's balance, black and
    /// colour words carry.
    samsung_key: [u32; 11],
    /// `exif_cfa`, Exif tag 0xa302: an Olympus-over-replacement CFA word.
    exif_cfa: u32,
    sony: SonyState,
    fuji: FujiState,
    /// `is_Olympus` plus the Olympus note's facts.
    oly: OlyState,
    /// `is_pana_raw` plus the Panasonic container's facts.
    pana: PanaState,
    /// `fuji_layout`: the SuperCCD sensor's row-interleave flag, set by the
    /// body's 0x0130 layout record.
    fuji_layout: i32,
    /// `xtrans_abs`: the X-Trans pattern the body's 0x0131 record spells, in
    /// sensor order; `xtrans` carries it rotated to the margins.
    xtrans_abs: [[u8; 6]; 6],
    /// `process_warnings`' parse-fuji mark, which keeps a `fuji_width` value
    /// set by the body.
    parsefuji_processed: bool,
    /// Set when a tag arm slips the frame's directory index forward, the
    /// engine's A100 `ifd++`; consumed by `parse_tiff_ifd` after the tag.
    leaked_ifd: Option<usize>,
    /// The tone curve head, identity by default; Sony tag 0x7010 rewrites the
    /// first four kilometres of it and the maximum consult reads it back.
    curve_head: [u16; 4096],
    /// The compression record's tone curve the plain Nikon loader's read
    /// supplies at open, replacing the identity over its own entries.
    nikon_curve: Option<Vec<u16>>,
    /// The YUV bodies' mark that the compressed arm ran its s-curve build
    /// before the open stage's own table rebuild.
    yuv_gamma: bool,
}

/// Sony metadata state: the SR2 private-data stash, the Minolta-private block
/// facts and the note-parse body id.
#[derive(Debug, Clone, Default)]
struct SonyState {
    /// Tags 0x7200 / 0x7201 / 0x7221: the encrypted SR2 private data, decoded
    /// at the end of every IFD the container walks.
    sr2_offset: i64,
    sr2_length: i64,
    sr2_key: u32,
    /// `imSony.prd_*` from the Minolta private block; only the Bayer plan
    /// shapes the white-balance channel order this pass carries.
    prd_bayer_pattern: u16,
    /// `is_Sony`, shared across maker-note parses like the engine's
    /// unpacker-data field, so a nested note inherits the mark.
    is_sony: bool,
    /// The engine's `unique_id`: Sony tag 0xb001, read as a short.
    body_id: u64,
}

/// CR3 walk state: the atom-name stack, the shared track walk variables and
/// the CMT record lengths.
#[derive(Debug, Clone, Default)]
struct Cr3State {
    nesting: i32,
    track: i32,
    track_type: usize,
    /// Per-tracked-track media descriptions.
    tracks: Vec<Cr3Track>,
    /// The growing atom-name stack, four bytes per nesting level.
    name_stack: Vec<u8>,
    /// Each parse level keeps its own `MediaFormatID`, as the engine's
    /// per-call local does.
    media_format_ids: Vec<[u8; 4]>,
    /// `cr3_ifd0_length`, the first CMT1 record length.
    ifd0_length: u32,
    /// `cr3_exif_length`, the first CMT2 record length.
    exif_length: u32,
    /// The raw track `selectCRXTrack` picked; the CRX loader reads its record.
    chosen_track: usize,
}

#[derive(Debug, Clone, Default)]
struct StscEntry {
    first: i64,
    count: i64,
    id: i64,
}

/// One tracked track: `TrackData` plus the decoded CRX image header
/// (`CrxImageHeader`) of its `stsd/CRAW` record.
#[derive(Debug, Clone, Default)]
struct Cr3Track {
    media_type: i32,
    media_offset: i64,
    media_size: i64,
    handler_type: [u8; 4],
    /// The CRX image header bits from `stsd/CRAW`.
    version: i32,
    n_bits: u32,
    f_width: i32,
    f_height: i32,
    tile_width: i32,
    tile_height: i32,
    n_planes: u32,
    cfa_layout: u32,
    enc_type: u32,
    image_levels: u32,
    has_tile_cols: u32,
    has_tile_rows: u32,
    mdat_header_size: u32,
    median_bits: u32,
    /// Chunk tables for this track.
    chunk_count: i64,
    chunk_offsets: Vec<i64>,
    stsc: Vec<StscEntry>,
    sample_sizes: Vec<i64>,
    sample_size: i64,
    sample_count: i64,
}

impl<'a> Walker<'a> {
    fn new(data: &'a [u8]) -> Self {
        Walker {
            stream: Stream::new(data),
            makes_seen: 0,
            make: String::new(),
            model: String::new(),
            filters: u32::MAX,
            colors: 3,
            width: 0,
            height: 0,
            raw_width: 0,
            raw_height: 0,
            top_margin: 0,
            left_margin: 0,
            flip: u32::MAX,
            tiff_flip: u32::MAX,
            pixel_aspect: 1.0,
            fuji_width: 0,
            black: 0,
            cblack: [0; CBLACK_SIZE],
            maximum: 0,
            pre_mul: [1.0, 1.0, 1.0, 0.0],
            cam_mul: [0.0, 1.0, 0.0, 0.0],
            cam_xyz: [[0.0; 3]; 4],
            cmatrix: [[0.0; 4]; 3],
            rgb_cam: [[0.0; 4]; 3],
            raw_color: true,
            cm_found: false,
            tiff_bps: 0,
            tiff_samples: 0,
            tiff_compress: 0,
            tiff_sampleformat: 0,
            data_offset: 0,
            data_size: 0,
            tile_width: 0,
            tile_length: 0,
            meta_offset: 0,
            strip_offset: 0,
            mask: [[0; 4]; 8],
            xtrans: [[0; 6]; 6],
            ifds: Vec::new(),
            ifd_count: 0,
            is_raw: 1,
            dng_version: 0,
            software: String::new(),
            model2: String::new(),
            nikon_transfer: 0,
            shot_select: 0,
            use_camera_wb: false,
            raw_ifd: -1,
            cr2_slice: [0; 3],
            extrasamples: Vec::new(),
            loader: None,
            refusal: None,
            load_flags: 0,
            load_flags2: 0,
            canon: CanonState::default(),
            raw_pitch: 0,
            metadata_blocks: 0,
            cr3: Cr3State {
                tracks: vec![Cr3Track::default(); CRX_TRACKS_MAXCOUNT],
                name_stack: vec![0; CR3_STACK_BYTES],
                media_format_ids: vec![[0; 4]; 32],
                ..Cr3State::default()
            },
            ciff_icwbc: [[0; 4]; 256],
            firmware: String::new(),
            zero_after_ff: false,
            zero_is_bad: false,
            pentax_id: 0,
            samsung_key: [0; 11],
            exif_cfa: 0,
            sony: SonyState::default(),
            fuji: FujiState::default(),
            oly: OlyState::default(),
            pana: PanaState::default(),
            fuji_layout: 0,
            xtrans_abs: [[0; 6]; 6],
            parsefuji_processed: false,
            leaked_ifd: None,
            curve_head: std::array::from_fn(|index| index as u16),
            nikon_curve: None,
            yuv_gamma: false,
        }
    }

    fn fsize(&self) -> i64 {
        self.stream.size() as i64
    }

    /// `get4` followed by a signed reinterpretation, as the engine does for
    /// offsets that may be negative.
    fn get4_signed(&mut self) -> i32 {
        self.stream.get4() as i32
    }
}

// ===== container dispatch ===================================================

/// Tracing for the metadata convergence loop (NERTHUS_RAW_TRACE=1).
fn trace() -> bool {
    std::env::var("NERTHUS_RAW_TRACE").is_ok()
}

/// The metadata entry point: walk the container and resolve the metadata, or
/// `None` when the container or maker is not handled yet (the caller then
/// keeps rawler's own view, as before).
pub(super) fn parse_camera_meta(data: &[u8]) -> Option<CameraMeta> {
    let trace = std::env::var("NERTHUS_RAW_TRACE").is_ok();
    if trace {
        eprintln!("trace: meta entry {} bytes head {:02x?}", data.len(), &data[..8.min(data.len())]);
    }
    let mut walker = Walker::new(data);
    if !walker.dispatch() {
        if trace {
            eprintln!("trace: dispatch rejected, make {:?} model {:?}", walker.make, walker.model);
        }
        return None;
    }
    if trace {
        eprintln!(
            "trace: after dispatch make {:?} model {:?} is_raw {} dng {} loader {:?} data_offset {} data_size {}",
            walker.make, walker.model, walker.is_raw, walker.dng_version, walker.loader, walker.data_offset, walker.data_size
        );
        for (index, record) in walker.ifds.iter().take(walker.ifd_count).enumerate() {
            eprintln!(
                "trace:   ifd {} {}x{} bps {} samples {} comp {} phint {} w {} h {} margins {} {} sft {} bytes {}",
                index, record.width, record.height, record.bps, record.samples, record.comp,
                record.phint, record.vwidth, record.vheight, record.top_margin, record.left_margin,
                record.newsubfiletype, record.bytes
            );
        }
    }
    // DNG containers are decoded by the dedicated DNG pass; files that turn
    // out to be DNGs (or DNG-in-TIFF) keep rawler's metadata instead.
    if walker.dng_version != 0 {
        return None;
    }
    walker.finish_identify();
    if trace {
        eprintln!(
            "trace: after identify is_raw {} raw {}x{} w {} h {} margins {} {} bps {} samples {} loader {:?}",
            walker.is_raw, walker.raw_width, walker.raw_height, walker.width, walker.height,
            walker.top_margin, walker.left_margin, walker.tiff_bps, walker.tiff_samples, walker.loader
        );
    }
    if walker.is_raw == 0 {
        return None;
    }
    // Per-maker metadata lands one maker at a time; until then those
    // files keep rawler's metadata.
    if walker.make != "Canon"
        && walker.make != "Nikon"
        && walker.make != "Sony"
        && walker.make != "Fujifilm"
        && walker.make != "Olympus"
        && walker.make != "Panasonic"
        && walker.make != "Pentax"
        && walker.make != "Samsung"
    {
        return None;
    }
    walker.finish_open();
    // A maker whose pixel loader this port does not run keeps rawler's
    // decode: the metadata walker carried everything else, but the pixels
    // must come from one engine. The open stage can also refuse the whole
    // file instead; that refusal flows through for the decode to fail on.
    if walker.loader.is_none() && walker.refusal.is_none() {
        return None;
    }
    Some(walker.into_meta())
}

impl<'a> Walker<'a> {
    /// The engine's container dispatch: phase-one scans, CRW, TIFF, CR3.
    fn dispatch(&mut self) -> bool {
        let head = self.read_head();
        // The first two bytes give the byte order for the header itself; as
        // they repeat the same byte, either reading order returns them.
        self.stream.order = self.stream.get2() as u16;
        let header_len = self.stream.get4() as i64;

        // Only the first 32 bytes are searched, as `memmem(head, 32, ...)`.
        let phase_one = head[..32]
            .windows(4)
            .any(|window| window == b"IIII" || window == b"MMMM");
        if std::env::var("NERTHUS_RAW_TRACE").is_ok() {
            eprintln!(
                "trace: dispatch order {:04x} header_len {} phase_one {} head {:02x?}",
                self.stream.order, header_len, phase_one, &head[..16]
            );
        }
        if phase_one {
            // Phase One containers are their own metadata format; not ported
            // yet, so rawler keeps handling them.
            return false;
        }

        if self.stream.order == LITTLE_ENDIAN || self.stream.order == BIG_ENDIAN {
            if &head[6..14] == b"HEAPCCDR" {
                let data_offset = header_len;
                let ciff_length = self.fsize() - data_offset;
                self.data_offset = data_offset;
                self.parse_ciff(data_offset, ciff_length as i32, 0);
                self.loader = Some(SensorKind::CanonCiff);
            } else if !self.parse_tiff(0) {
                return false;
            } else {
                self.apply_tiff();
            }
        } else if &head[..8] == b"FUJIFILM" {
            self.parse_raf_head(&head);
        } else if &head[4..12] == b"ftypcrx " {
            // `strcpy(make, "Canon")` happens before the parse, so a broken
            // CR3 still reports the maker; we return false instead and let
            // rawler deal with the file.
            self.make = "Canon".to_string();
            let atom_list_size = self.fsize();
            // The engine starts its walk one below the top level, so the top
            // level's names land at the stack's base.
            self.cr3.nesting = -1;
            let err = self.parse_cr3(0, atom_list_size);
            if trace() {
                eprintln!(
                    "trace:   cr3 err {} track {} tracks {} chosen {}",
                    err, self.cr3.track, self.cr3.tracks.len(), self.cr3.chosen_track
                );
            }
            // err -14 is nesting overflow; either it or success with a found
            // track goes on to the track selection.
            if (err == 0 || err == -14) && self.cr3.track >= 0 {
                self.select_crx_track();
            } else {
                return false;
            }
        }

        if self.make.is_empty() {
            // The engine has a by-filesize fallback table for headerless
            // bodies; without it the maker stays unknown and rawler keeps
            // handling the file.
            return false;
        }
        true
    }

    fn read_head(&mut self) -> [u8; 64] {
        let mut head = [0u8; 64];
        self.stream.read(&mut head);
        self.stream.seek(0);
        head
    }

    /// The RAF container's head facts, as `identify`'s `"FUJIFILM"` arm.
    fn parse_raf_head(&mut self, head: &[u8; 64]) {
        // Model: the RAF header spells it plainly; make stays unset until the
        // metadata TIFF's 0x010f tag supplies it. The serial-signature scan
        // the engine runs here only reaches the unpacker's `ilm.CamID`, which
        // the metadata pass does not carry.
        let mut model_bytes = [0u8; 32];
        model_bytes.copy_from_slice(&head[0x1c..0x3c]);
        let trimmed = model_bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(model_bytes.len());
        self.model = String::from_utf8_lossy(&model_bytes[..trimmed]).into_owned();

        // The header offsets are read with the mark's byte order, which not
        // being `II` or `MM` means big-endian, as the engine's sget does.
        self.stream.seek(84);
        let thumb_offset = self.stream.get4() as i64;
        let thumb_length = self.stream.get4() as i64;
        if trace() {
            eprintln!("trace: raf head thumb_offset {} thumb_length {}", thumb_offset, thumb_length);
        }
        self.stream.seek(92);
        let fuji_offset = self.stream.get4() as i64;
        self.parse_fuji(fuji_offset);
        if thumb_offset > 120 {
            self.stream.seek(120);
            let second_offset = self.stream.get4();
            if second_offset != 0 {
                self.is_raw += 1;
                if self.is_raw == 2 && self.shot_select != 0 {
                    self.parse_fuji(second_offset as i64);
                }
            }
        }
        self.loader = Some(SensorKind::Unpacked);
        self.stream.seek(100);
        self.data_offset = self.stream.get4() as i64;
        self.parse_tiff(self.data_offset);
        self.parse_tiff(thumb_offset + 12);
        // The engine also walks the thumbnail JPEG for its XMP block here;
        // that packet only feeds the lens fields this pass does not carry.
        self.apply_tiff();
        if trace() {
            eprintln!(
                "trace: raf after apply_tiff rw {} rh {} w {} h {} tm {} lm {} bps {} comp {} samples {} fuji_width {} layout {} filters {:08x} cam_mul {:?}",
                self.raw_width, self.raw_height, self.width, self.height,
                self.top_margin, self.left_margin, self.tiff_bps, self.tiff_compress,
                self.tiff_samples, self.fuji_width, self.fuji_layout, self.filters, self.cam_mul
            );
        }
    }

    /// `fgetc` with the engine's end-of-file value.
    fn getc_i(&mut self) -> i32 {
        self.stream.getc().map_or(-1, i32::from)
    }

    /// `get_average_WB`: one of `parse_fuji`'s white-balance records. The body
    /// spells the four channel shorts in G1, R, G2, B order, which the
    /// engine's `GRGB_2_RGBG` permutation undoes onto the shared rows.
    fn get_average_wb(
        &mut self,
        row: usize,
        len: u32,
        average_wb_data: bool,
        use_wbcorr_coeffs: bool,
        wb_r_corr: f64,
        wb_b_corr: f64,
    ) {
        for c in 0..4usize {
            let slot = [1usize, 0usize, 3usize, 2usize][c];
            self.ciff_icwbc[row][slot] = self.stream.get2();
        }
        if len == 16 && average_wb_data {
            for c in 0..4usize {
                let slot = [1usize, 0usize, 3usize, 2usize][c];
                let average =
                    (u32::from(self.ciff_icwbc[row][slot]) + u32::from(self.stream.get2())) / 2;
                self.ciff_icwbc[row][slot] = average as u16;
            }
        }
        if use_wbcorr_coeffs {
            // The engine's int() truncation, then the u16 store.
            self.ciff_icwbc[row][0] = (self.ciff_icwbc[row][0] as f64 * wb_r_corr) as i32 as u16;
            self.ciff_icwbc[row][2] = (self.ciff_icwbc[row][2] as f64 * wb_b_corr) as i32 as u16;
        }
    }

    /// `parse_fuji`: the RAF body's own record list.
    fn parse_fuji(&mut self, offset: i64) {
        self.stream.seek(offset.max(0) as u64);
        let entries = self.stream.get4();
        if trace() {
            eprintln!("trace: parse_fuji off {} entries {} model {:?}", offset, entries, self.model);
        }
        if entries > 255 {
            return;
        }
        self.parsefuji_processed = true;

        // The SuperCCD bodies' records carry corrected levels; these models'
        // rows keep the correction factors.
        let use_wbcorr_coeffs;
        let wb_r_corr;
        let wb_b_corr;
        if ["S2Pro", "S20Pro", "F700", "S5000", "S7000"]
            .iter()
            .any(|name| self.model.contains(name))
        {
            use_wbcorr_coeffs = true;
            wb_r_corr = 10.0 / 17.0 / 0.652941;
            wb_b_corr = 2.0 / 3.0 / (3.0 / 4.0 + 1.0 / 300.0);
        } else if self.model.contains("DBP") || self.model.contains("DX-2000") {
            use_wbcorr_coeffs = true;
            wb_r_corr = 0.7632653061;
            wb_b_corr = 0.8591549296;
        } else {
            use_wbcorr_coeffs = false;
            wb_r_corr = 1.0;
            wb_b_corr = 1.0;
        }

        let mut raw_inset_present = false;
        for _ in 0..entries as u64 {
            let tag = self.stream.get2() as u32;
            let len = self.stream.get2() as u32;
            let save = self.stream.tell();
            if trace() {
                eprintln!("trace:   fuji tag {:04x} len {} @{}", tag, len, save);
            }
            match tag {
                0x0100 => {
                    // RawImageFullSize: height first.
                    let height = self.stream.get2() as i32;
                    let width = self.stream.get2() as i32;
                    self.raw_height = height;
                    self.raw_width = width;
                    raw_inset_present = true;
                    if trace() {
                        eprintln!("trace:     0100 rh {} rw {}", height, width);
                    }
                }
                0x0110 if raw_inset_present => {
                    let ctop = self.stream.get2();
                    let cleft = self.stream.get2();
                    self.fuji.inset_crops[0].ctop = ctop;
                    self.fuji.inset_crops[0].cleft = cleft;
                }
                0x0111 if raw_inset_present => {
                    let cheight = self.stream.get2();
                    let cwidth = self.stream.get2();
                    self.fuji.inset_crops[0].cheight = cheight;
                    self.fuji.inset_crops[0].cwidth = cwidth;
                }
                0x0115 if raw_inset_present => {
                    let a = self.stream.get2() as i32;
                    let b = self.stream.get2() as i32;
                    if a * b == 6 {
                        self.fuji.raw_aspect = 1500;
                    } else if a * b == 12 {
                        self.fuji.raw_aspect = 1333;
                    } else if a * b == 144 {
                        self.fuji.raw_aspect = 1777;
                    } else if a * b == 1 {
                        self.fuji.raw_aspect = 1000;
                    }
                }
                0x0121 => {
                    let height = self.stream.get2() as i32;
                    let width = self.stream.get2() as i32;
                    let width = if width == 4284 { width + 3 } else { width };
                    self.height = height;
                    self.width = width;
                }
                0x0130 => {
                    // Two separate reads, as the engine's pair of fgetc calls;
                    // an end-of-file read is -1, which is no layout flag and a
                    // zero width.
                    self.fuji_layout = self.getc_i() >> 7;
                    let second = self.getc_i();
                    self.fuji_width = u32::from(second & 8 == 0);
                }
                0x0131 => {
                    // The X-Trans plan, spelled reversed row-major, clamped.
                    self.filters = 9;
                    let mut flat = [0u8; 36];
                    for c in 0..36usize {
                        let q = self.getc_i();
                        flat[35 - c] = q.clamp(0, 2) as u8;
                    }
                    let mut pattern = [[0u8; 6]; 6];
                    for row in 0..6usize {
                        pattern[row].copy_from_slice(&flat[row * 6..row * 6 + 6]);
                    }
                    self.xtrans_abs = pattern;
                }
                0x2ff0 => {
                    self.get_average_wb(
                        wbi::AS_SHOT,
                        len,
                        true,
                        use_wbcorr_coeffs,
                        wb_r_corr,
                        wb_b_corr,
                    );
                    for c in 0..4 {
                        let level = self.ciff_icwbc[wbi::AS_SHOT][c] as f32;
                        self.cam_mul[c] = level;
                    }
                    if trace() {
                        eprintln!("trace:     2ff0 cam_mul {:?}", self.cam_mul);
                    }
                }
                0x4000 if len == 8 || len == 16 => {
                    self.fuji.black_level[0] = (len / 2) as u16;
                    for c in 0..4usize {
                        let slot = 1 + grgb_to_rgbg_index(c);
                        let level = self.stream.get2();
                        self.fuji.black_level[slot] = level;
                    }
                    if self.fuji.black_level[0] == 8 {
                        for c in 0..4usize {
                            let slot = 5 + grgb_to_rgbg_index(c);
                            let level = self.stream.get2();
                            self.fuji.black_level[slot] = level;
                        }
                    }
                    if self.fuji.black_level[0] == 4 {
                        for c in 0..4usize {
                            let level = u32::from(self.fuji.black_level[c + 1]);
                            self.cblack[c] = level;
                        }
                    } else if self.fuji.black_level[0] == 8 {
                        for c in 0..4usize {
                            let sum = u32::from(self.fuji.black_level[c + 1])
                                + u32::from(self.fuji.black_level[c + 5]);
                            let merged = sum / 2;
                            self.cblack[c] = merged;
                        }
                    }
                }
                _ if (0x2000..=0x2410).contains(&tag) => {
                    for entry in &FUJI_TAG_2_WB_TABLE {
                        if u32::from(entry.0) == tag {
                            self.get_average_wb(
                                entry.1,
                                len,
                                true,
                                use_wbcorr_coeffs,
                                wb_r_corr,
                                wb_b_corr,
                            );
                            break;
                        }
                    }
                }
                0x2f00 => {
                    // MIN(get4(), 6), applied after the read, so a negative
                    // count stays negative and fills nothing.
                    let n_wbs = (self.stream.get4() as i32).min(6);
                    for i in 0..n_wbs.max(0) {
                        let row = wbi::CUSTOM1 + i as usize;
                        for c in 0..4usize {
                            let slot = [1usize, 0usize, 3usize, 2usize][c];
                            let level = self.stream.get2();
                            self.ciff_icwbc[row][slot] = level;
                        }
                        if len >= 4 + 16 * n_wbs.max(0) as u32 {
                            for c in 0..4usize {
                                let slot = [1usize, 0usize, 3usize, 2usize][c];
                                let average = (u32::from(self.ciff_icwbc[row][slot])
                                    + u32::from(self.stream.get2()))
                                    / 2;
                                self.ciff_icwbc[row][slot] = average as u16;
                            }
                        }
                        if use_wbcorr_coeffs {
                            // The engine's loop index is itself already offset
                            // by the first Custom row here, so this lands on
                            // rows past the table's Custom block.
                            let doubled = wbi::CUSTOM1 * 2 + i as usize;
                            let red = self.ciff_icwbc[doubled][0];
                            let blue = self.ciff_icwbc[doubled][2];
                            self.ciff_icwbc[doubled][0] =
                                (f64::from(red) * wb_r_corr) as i32 as u16;
                            self.ciff_icwbc[doubled][2] =
                                (f64::from(blue) * wb_b_corr) as i32 as u16;
                        }
                    }
                }
                0xc000 => {
                    // The RAFData record's numbers are little-endian, however
                    // the container's own records read.
                    let saved_order = self.stream.order;
                    self.stream.order = LITTLE_ENDIAN;
                    if len > 20000 {
                        self.fuji.raf_data_pos = save;
                        self.fuji.raf_data_len = len >> 1;
                        let mut header = [0u8; 16];
                        self.stream.read(&mut header);
                        let offset_wh = self.guess_raf_data_generation(&header);
                        // The table starts `offset_wh` bytes past the header.
                        let target = self.stream.tell() as i64 + i64::from(offset_wh) - 16;
                        self.stream.seek(target.max(0) as u64);
                        for slot in self.fuji.raf_data_image_size_table.iter_mut() {
                            let size = self.stream.get4();
                            *slot = size;
                        }
                    } else if len == 4096 {
                        self.fuji.raf_data_generation = 4096;
                        self.stream.seek((save + 0x200).max(0) as u64);
                        for _ in 0..42 {
                            let n_wb = self.stream.get4();
                            // The engine also carries the paired CCT value to
                            // its CCT table here; that table reaches neither
                            // cam_mul nor the black levels.
                            let _t_wb = self.stream.get4();
                            let mut wb = [0u32; 4];
                            wb[0] = self.stream.get4() << 1;
                            wb[1] = self.stream.get4();
                            wb[3] = self.stream.get4();
                            wb[2] = self.stream.get4() << 1;
                            if n_wb != 70 {
                                for entry in &FUJI_WB_LIST2 {
                                    if entry.1 == n_wb {
                                        for c in 0..4 {
                                            let level = wb[c] as u16;
                                            self.ciff_icwbc[entry.0][c] = level;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    self.stream.order = saved_order;
                }
                _ => {}
            }
            self.stream.seek(save + u64::from(len));
        }

        if trace() {
            eprintln!(
                "trace: fuji tail layout {} fw {} rh {} rw {} w {} h {} gen {}",
                self.fuji_layout, self.fuji_width, self.raw_height, self.raw_width,
                self.width, self.height, self.fuji.raf_data_generation
            );
        }
        if self.fuji.raf_data_generation == 0 {
            self.height <<= self.fuji_layout;
            self.width >>= self.fuji_layout;
        }
    }

    /// `guess_RAFDataGeneration`: the RAFData header's generation code, and the
    /// offset to its first width/height pair. The record is little-endian,
    /// like everything the 0xc000 arm reads.
    fn guess_raf_data_generation(&mut self, header: &[u8; 16]) -> i32 {
        let b01 = u16::from_le_bytes([header[0], header[1]]);
        let b23 = u16::from_le_bytes([header[2], header[3]]);
        let wrts = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let mut offset_wh = 0;
        if b01 != 0 && b23 == 0 && b01 < 10000 {
            self.fuji.raf_data_generation = 1;
        } else if b01 > 10000 && b23 == 0 {
            self.fuji.raf_data_generation = 2;
            self.fuji.raf_data_version = u32::from(b01);
            offset_wh = 4;
        } else if b01 == 0 {
            if wrts == 0x5354_5257 {
                self.fuji.raf_data_generation = 4;
                offset_wh = 8;
            } else {
                self.fuji.raf_data_generation = 3;
                offset_wh = 4;
            }
            self.fuji.raf_data_version = u32::from(b23);
        }
        offset_wh
    }
}

// ===== TIFF container =======================================================

/// `tiff_get`: tag, type and count, and on to the value area when it does not
/// fit inline. `TYPE_UNIT_BYTES` is the engine's per-type byte width table,
/// indexed with the clamp `((unsigned)type < 20) ? type : 0`.
#[derive(Debug, Clone, Copy)]
struct TiffTag {
    tag: u32,
    kind: u32,
    len: u32,
    save: u64,
}

impl<'a> Walker<'a> {
    fn tiff_get(&mut self, base: i64) -> TiffTag {
        let tag = self.stream.get2() as u32;
        let kind = self.stream.get2() as u32;
        let len = self.stream.get4();
        // The resume point is always just past the value field, inline or not.
        let save = self.stream.tell() + 4;
        let unit = TYPE_UNIT_BYTES[usize::try_from(kind).map_or(0, |kind| if kind < 20 { kind } else { 0 })];
        if len.wrapping_mul(unit) > 4 {
            let offset = self.stream.get4() as i64;
            self.stream.seek((offset + base).max(0) as u64);
        }
        TiffTag { tag, kind, len, save }
    }

    /// `parse_tiff`: walk the IFD chain. The byte order mark at `base`
    /// refreshes `order`, as the engine re-derives it for embedded headers.
    fn parse_tiff(&mut self, base: i64) -> bool {
        self.stream.seek(base.max(0) as u64);
        let order = self.stream.get2() as u16;
        if order != LITTLE_ENDIAN && order != BIG_ENDIAN {
            return false;
        }
        self.stream.order = order;
        self.stream.get2();
        loop {
            let next_offset = self.stream.get4() as i64;
            if next_offset == 0 {
                break;
            }
            if next_offset + base > self.fsize() {
                break;
            }
            self.stream.seek((next_offset + base).max(0) as u64);
            if self.parse_tiff_ifd(base) {
                break;
            }
        }
        true
    }

    /// `parse_tiff_ifd`: one directory; every tag re-seeks to the value area
    /// start afterwards, so the next tag header follows the entry table.
    fn parse_tiff_ifd(&mut self, base: i64) -> bool {
        // The engine allots the slot before the entries check, so a broken
        // directory still consumes one.
        if self.ifd_count >= IFD_MAXCOUNT {
            return true;
        }
        let mut ifd = self.ifd_count;
        self.ifd_count += 1;
        if ifd < self.ifds.len() {
            // A leaked index filled this slot on credit earlier; the frame
            // takes it over with a fresh record, so the leaked tags never
            // reach the raw pick.
            self.ifds[ifd] = Ifd::default();
        } else {
            self.ifds.push(Ifd::default());
        }
        let entries = self.stream.get2() as usize;
        let trace = std::env::var("NERTHUS_RAW_TRACE").is_ok();
        if trace {
            eprintln!("trace: ifd {} at {} entries {}", ifd, self.stream.tell() - 2, entries);
        }
        if entries > 512 {
            // The engine increments its IFD count before the check, so the
            // zero record stays in the table.
            return true;
        }

        for _ in 0..entries {
            let parsed = self.tiff_get(base);
            if trace {
                eprintln!(
                    "trace:   tag {:06x} kind {} len {} at {}",
                    parsed.tag, parsed.kind, parsed.len, self.stream.tell()
                );
            }
            // A value that reaches past twice the file size recovers to the
            // entry table, as the `len > 8 && savepos + len > 2 * fsize`
            // check does.
            if parsed.len > 8
                && self.stream.tell() as i64 + parsed.len as i64 > 2 * self.fsize()
            {
                self.stream.seek(parsed.save);
                continue;
            }
            self.parse_tiff_tag(ifd, base, &parsed);
            if let Some(leaked) = self.leaked_ifd.take() {
                // The engine's DSLR-A100 arm slips its `ifd++` into the frame:
                // the remaining tags of this directory land in the next slot,
                // on credit past the IFD count. The next directory parse
                // re-zeroes that slot for its own frame, so the leaked tags
                // never reach the raw pick; until then the slot exists.
                if self.ifds.len() <= leaked {
                    self.ifds.push(Ifd::default());
                }
                ifd = leaked;
            }
            self.stream.seek(parsed.save);
        }
        // The SR2 private data stashed while this directory was read is
        // decrypted here, as the engine does at the end of every directory.
        self.decrypt_sony_sr2();

        // The engine's normal exit is `return 0`; the chain loop keeps
        // following the next-IFD pointers until one of the early exits.
        false
    }

    /// The end-of-directory SR2 decrypt: assemble the stashed offset, length
    /// and key, then hand the buffer to the SR2 note parser.
    fn decrypt_sony_sr2(&mut self) {
        let SonyState {
            sr2_offset,
            sr2_length,
            sr2_key,
            ..
        } = self.sony;
        self.decrypt_sony_sr2_rows(sr2_offset, sr2_length, sr2_key, sr2_offset);
    }

    /// Decrypt and parse an SR2 block; `save` is the base the block's value
    /// pointers are stored against (the Adobe tail stores them against the
    /// SR2 pointer itself, the directory tails against the same offset).
    fn decrypt_sony_sr2_rows(&mut self, offset: i64, length: i64, key: u32, save: i64) {
        if length == 0 || length >= 10_240_000 {
            return;
        }
        self.stream.seek(offset.max(0) as u64);
        let mut buffer = vec![0u8; length as usize];
        let read = self.stream.read(&mut buffer) as i64;
        if read != length {
            return;
        }
        let words = length as usize / 4;
        sony_decrypt(&mut buffer, words, key);
        self.parse_sony_sr2(&buffer, save);
    }

    /// `parseSonySR2`: the decrypted SR2 private directory. The buffer keeps
    /// the stream order it was read under, as the engine's checked buffer
    /// carries its own order; a read past the end abandons the whole parse.
    fn parse_sony_sr2(&mut self, buffer: &[u8], save: i64) {
        let order = self.stream.order;
        let length = buffer.len() as i64;
        let mut entries = i64::from(match sony_buf2(buffer, order, 0) {
            Some(value) => value,
            None => return,
        });
        if entries <= 1000 {
            let mut tag_offset = 2i64;
            while entries != 0 {
                entries -= 1;
                let Some(tag) = sony_buf2(buffer, order, tag_offset) else {
                    return;
                };
                let Some(kind) = sony_buf2(buffer, order, tag_offset + 2) else {
                    return;
                };
                let Some(data_len) = sony_buf4(buffer, order, tag_offset + 4) else {
                    return;
                };
                let unit = TYPE_UNIT_BYTES
                    [usize::try_from(kind).map_or(0, |kind| if kind < 20 { kind } else { 0 })];
                let data_offset = if u64::from(data_len) * u64::from(unit) > 4 {
                    let Some(pointer) = sony_buf4(buffer, order, tag_offset + 8) else {
                        return;
                    };
                    // An out-of-range pointer skips the tag but keeps the
                    // walk, as the `-2` return does.
                    let offset = i64::from(pointer) - save;
                    if offset + i64::from(data_len) > length {
                        tag_offset += 12;
                        continue;
                    }
                    offset
                } else {
                    tag_offset + 8
                };
                tag_offset += 12;
                self.parse_sony_sr2_tag(buffer, order, u32::from(tag), data_offset, i64::from(unit));
            }
        }
    }

    /// One SR2 entry: the four core records first, then the white-balance
    /// families that run for every tag.
    fn parse_sony_sr2_tag(&mut self, buffer: &[u8], order: u16, tag: u32, base: i64, unit: i64) {
        let mut processed = true;
        match tag {
            0x7300 => {
                for cell in 0..4usize {
                    if let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) {
                        self.cblack[cell] = u32::from(value);
                    } else {
                        return;
                    }
                }
            }
            0x7303 => {
                for cell in 0..4usize {
                    let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) else {
                        return;
                    };
                    self.cam_mul[grbg_to_rgbg_index(cell)] = f32::from(value);
                }
            }
            0x7310 => {
                let mut written = [0u32; 4];
                for cell in 0..4usize {
                    let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) else {
                        return;
                    };
                    written[rggb_to_rgbg_index(cell)] = u32::from(value);
                }
                self.cblack[..4].copy_from_slice(&written);
                // The floor comes from the third entry, sitting at index 3.
                let mut floor = self.cblack[3];
                for cell in 0..3 {
                    let value = self.cblack[cell];
                    if floor > value {
                        floor = value;
                    }
                }
                for cell in 0..4 {
                    self.cblack[cell] -= floor;
                }
                self.black = floor;
            }
            0x7313 => {
                for cell in 0..4usize {
                    let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) else {
                        return;
                    };
                    self.cam_mul[rggb_to_rgbg_index(cell)] = f32::from(value);
                }
            }
            // Aperture and focal-length limits, the camera matrix and the
            // linear-maximum plan are lens or colour state this pass does
            // not carry.
            0x74a0..=0x74a3 | 0x7800 | 0x787f => {}
            _ => {
                processed = false;
            }
        }
        if processed {
            return;
        }
        if (0x7480..=0x7486).contains(&tag) {
            let row = SONY_SR2_WB_LIST[(tag - 0x7480) as usize];
            if row <= 255 {
                self.sony_sr2_wb_row(buffer, order, row, base, unit, 3);
            }
        } else if (0x7820..=0x782d).contains(&tag) {
            let row = SONY_SR2_WB_LIST1[(tag - 0x7820) as usize];
            if row <= 255 {
                self.sony_sr2_wb_row(buffer, order, row, base, unit, 3);
            }
            // The 3200 name also seeds the studio-tungsten row, but only
            // from the colour-temperature records this pass does not carry.
        } else if tag == 0x7302 {
            for cell in 0..4usize {
                let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) else {
                    return;
                };
                self.ciff_icwbc[sony_wbi::AUTO][grbg_to_rgbg_index(cell)] = value;
            }
        } else if tag == 0x7312 {
            for cell in 0..4usize {
                let Some(value) = sony_buf2(buffer, order, base + unit * cell as i64) else {
                    return;
                };
                self.ciff_icwbc[sony_wbi::AUTO][rggb_to_rgbg_index(cell)] = value;
            }
        }
    }

    /// A white-balance triple into one `icWBC` row, greens at the second
    /// channel.
    fn sony_sr2_wb_row(
        &mut self,
        buffer: &[u8],
        order: u16,
        row: usize,
        base: i64,
        unit: i64,
        count: i64,
    ) {
        for channel in 0..count {
            let Some(value) = sony_buf2(buffer, order, base + unit * channel) else {
                return;
            };
            self.ciff_icwbc[row][channel as usize] = value;
        }
        self.ciff_icwbc[row][3] = self.ciff_icwbc[row][1];
    }

    /// One TIFF tag.
    fn parse_tiff_tag(&mut self, ifd: usize, base: i64, parsed: &TiffTag) {
        let TiffTag { tag, kind, len, .. } = *parsed;

        // The EXIF and PanasonicRaw layouts share tag numbers: the engine
        // splits their shared tags ahead of the general switch on the body's
        // `is_pana_raw` flag, which tag 0x0001 carries, and both layouts run
        // the general switch afterwards.
        if self.pana.raw == 0 {
            // The EXIF meanings of the tags the Panasonic layout shares.
            match tag {
                0x0001 => {
                    if len == 4 {
                        self.pana.raw = self.stream.get4();
                    }
                }
                0x000b => {
                    // The engine's fgets keeps the newline in the string it
                    // compares, and the software row takes it too.
                    let mut software_bytes = [0u8; 64];
                    let read = self.stream.read(&mut software_bytes);
                    let newline_end = software_bytes[..read]
                        .iter()
                        .position(|&byte| byte == b'\n')
                        .map_or(read, |position| position + 1);
                    let software = &software_bytes[..newline_end];
                    let software = software.split(|&byte| byte == 0).next().unwrap_or(software);
                    self.software = String::from_utf8_lossy(software).into_owned();
                    const BLOCKED: [&[u8]; 4] = [b"Adobe", b"dcraw", b"UFRaw", b"Bibble"];
                    if BLOCKED.iter().any(|name| software.starts_with(name))
                        || software == b"Digital Photo Professional"
                    {
                        self.is_raw = 0;
                    }
                }
                0x001c | 0x001d | 0x001e => {
                    let value = self.stream.get2() as u32;
                    self.cblack[(tag - 0x001c) as usize] = value;
                    self.cblack[3] = self.cblack[1];
                }
                0x0111 | 0x0201 | 0xf007 => {
                    self.handle_strip_offsets(ifd, base, len);
                }
                _ => {}
            }
        } else {
            // The Panasonic-specific records, at their own meanings for the
            // tags the EXIF layout shares. The engine's remaining arms —
            // SensorTopBorder (0x0004), Compression (0x000b), the
            // LinearityLimit records (0x000e to 0x0010), WBInfo (0x0013), the
            // HighISOMultiplier records (0x0018 to 0x001a), the crop records
            // (0x002f to 0x0032), the extended ISO (0x0037), ISO (0x0017),
            // the striped-row records (0x0039 to 0x0048), Gamma (0x011c), the
            // CameraIFD walk (0x0120, whose lens, multishot and white-balance
            // records 0x1001 to 0x3421 lead to), JpgFromRaw (0x002e) — all
            // feed the pixel loader later, or facts the metadata record does
            // not carry. The inset records the borders keep are the same
            // side note.
            match tag {
                0x0002 => {
                    let value = self.stream.getint(kind) as i32;
                    self.ifds[ifd].width = value;
                }
                0x0003 => {
                    let value = self.stream.getint(kind) as i32;
                    self.ifds[ifd].height = value;
                }
                0x0005 => {
                    self.width = self.stream.get2() as i32;
                }
                0x0006 => {
                    self.height = self.stream.get2() as i32;
                }
                0x0007 => {
                    let right_border = self.stream.get2() as i32;
                    self.width += right_border;
                }
                0x0009 => {
                    let value = self.stream.get2() as u32;
                    if value != 0 {
                        self.filters = value;
                    }
                }
                0x000a => {
                    self.pana.bits = (self.stream.get2() as u32).clamp(8, 16);
                }
                0x0011 => {
                    if kind == TYPE_SHORT && len == 1 {
                        self.cam_mul[0] = self.stream.get2() as f32 / 256.0;
                    }
                }
                0x0012 => {
                    if kind == TYPE_SHORT && len == 1 {
                        self.cam_mul[2] = self.stream.get2() as f32 / 256.0;
                    }
                }
                0x001c | 0x001d | 0x001e => {
                    let value = self.stream.get2() as u32;
                    self.pana.black[(tag - 0x001c) as usize] = value;
                }
                0x000e | 0x000f | 0x0010 => {
                    // LinearityLimit{Red,Green,Blue}; the clipped values then
                    // feed the later bodies' maximum.
                    let mut limit = self.stream.get2() as u32;
                    if limit == 16383 {
                        limit -= 64;
                    }
                    if limit == 4095 {
                        limit -= 16;
                    }
                    let channel = (tag - 0x000e) as usize;
                    self.pana.linear_max[channel] = limit;
                    if tag == 0x000f {
                        self.pana.linear_max[3] = self.pana.linear_max[1];
                    }
                }
                0x002d => {
                    self.pana.encoding = self.stream.get2() as u32;
                }
                0x0024 => {
                    self.cam_mul[0] = self.stream.get2() as f32;
                }
                0x0025 => {
                    self.cam_mul[1] = self.stream.get2() as f32;
                }
                0x0026 => {
                    self.cam_mul[2] = self.stream.get2() as f32;
                }
                0x0027 => {
                    // WBInfo2's source-numbered white-balance rows land on
                    // the shared table (the engine's icWBC, which the CIFF
                    // walk also uses); the row after the first one lifts into
                    // cam_mul when no balance record came earlier.
                    let rows = self.stream.get2() as i32;
                    if rows <= 0x100 {
                        for row_index in 0..rows {
                            let source = self.stream.get2() as i32;
                            if source < 0x100 {
                                let red = self.stream.get2();
                                let green = self.stream.get2();
                                self.ciff_icwbc[source as usize][0] = red;
                                self.ciff_icwbc[source as usize][1] = green;
                                self.ciff_icwbc[source as usize][3] = green;
                                self.ciff_icwbc[source as usize][2] = self.stream.get2();
                                if row_index == 1 && rows > 6 && self.cam_mul[0] <= 0.001 {
                                    for (slot, &value) in
                                        self.ciff_icwbc[source as usize].iter().enumerate()
                                    {
                                        self.cam_mul[slot] = value as f32;
                                    }
                                }
                            } else {
                                // A source outside the index range skips its
                                // three records.
                                self.stream.skip(6);
                            }
                        }
                    }
                }
                0x0118 => {
                    // The RW2 offset record: a long value selects the packed
                    // loader, then the value reads like a strip offset.
                    if kind == TYPE_LONG {
                        self.loader = Some(SensorKind::Panasonic);
                        self.load_flags = 0x2008;
                        self.handle_strip_offsets(ifd, base, len);
                    }
                }
                0x0111 => {
                    self.handle_strip_offsets(ifd, base, len);
                }
                _ => {}
            }
        }

        match tag {
            0x00fe => {
                let value = self.stream.getreal(kind) as i32;
                self.ifds[ifd].newsubfiletype = value;
            }
            0xf000 => {
                // The Fuji HS10 table's pointer is a TIFF directory of its
                // own; the frame's tags keep re-seeking to the value area
                // after an arm recurses, so the outer walk resumes cleanly.
                let offset = self.stream.get4() as i64;
                self.stream.seek((offset + base).max(0) as u64);
                self.parse_tiff_ifd(base);
            }
            // ImageWidth / ImageLength.
            0x0100 | 0xf001 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].width = value;
            }
            0x0101 | 0xf002 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].height = value;
            }
            0x0102 | 0xf003 => {
                // BitsPerSample: the engine updates the per-IFD sample count
                // only from a fresh 0x0102, then takes the widest bit depth.
                if self.ifds[ifd].samples == 0 || tag != 0x0102 {
                    self.ifds[ifd].samples = (len & 7) as i32;
                }
                self.ifds[ifd].bps = self.stream.getint(kind) as i32;
                if (self.tiff_bps as i32) < self.ifds[ifd].bps {
                    self.tiff_bps = self.ifds[ifd].bps as u32;
                }
            }
            0xf006 => {
                // Fuji's compression tag zeroes the raw height, then spells
                // packed rows through the flag in the next value.
                self.raw_height = 0;
                if self.ifds[ifd].bps <= 12 {
                    let flag = self.stream.get4();
                    let next_flags = if flag != 0 { 24 } else { 80 };
                    self.load_flags = next_flags;
                    self.loader = Some(SensorKind::Packed);
                }
            }
            0x0103 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].comp = value;
            }
            0xa010 => {
                // The Samsung raw IFD's size record names the file's own
                // loader through the IFD's compression mark, and hands the
                // strip-table offset to the loader; other compression values
                // leave the choice to the engine's own loader pass.
                self.strip_offset = self.stream.get4() as i64;
                match self.ifds[ifd].comp {
                    0x8002 => self.loader = Some(SensorKind::Samsung),
                    0x8004 => self.loader = Some(SensorKind::Samsung2),
                    0x8005 => self.loader = Some(SensorKind::Samsung3),
                    _ => {}
                }
            }
            0x0106 => {
                let value = self.stream.get2() as i32;
                self.ifds[ifd].phint = value;
            }
            0x010f => {
                self.make = self.stream.read_string(64);
            }
            0x0110 => {
                self.model = self.stream.read_string(64);
            }
            0x0112 => {
                const PLAIN_FLIP: &[u8; 8] = b"50132467";
                let orientation = self.stream.get2() as usize & 7;
                self.ifds[ifd].flip = PLAIN_FLIP[orientation] as i32 - b'0' as i32;
            }
            0x0115 => {
                let value = (self.stream.getint(kind) & 7) as i32;
                self.ifds[ifd].samples = value;
            }
            0x0116 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].rows_per_strip = value;
            }
            // StripByteCounts, StripLineBytes (Fuji) and ThumbnailLength: an
            // array lands on the IFD and restores the position, so the
            // unconditional `bytes` read below takes the first value again; a
            // single value stays as `bytes`. The engine spells all three in
            // one case list.
            0x0117 | 0x0202 | 0xf008 => {
                // StripByteCounts: an array lands on the IFD (and the entry
                // position is restored, so the unconditional `bytes` read
                // below takes the first value again); a single value stays as
                // `bytes`.
                if len > 1 && len < 16384 {
                    let restore = self.stream.tell();
                    let mut counts = Vec::with_capacity(len as usize);
                    for _ in 0..len {
                        counts.push(self.stream.get4() as i64);
                    }
                    self.ifds[ifd].strip_byte_counts = counts;
                    self.stream.seek(restore);
                }
                self.ifds[ifd].bytes = self.stream.get4() as i64;
            }
            0xf00e => {
                // Fuji's "As Shot": stored in G, R, B order, which the
                // engine's GRBG_2_RGBG permutation undoes.
                for c in 0..3usize {
                    let value = self.stream.getint(kind) as f32;
                    self.cam_mul[grbg_to_rgbg_index(c)] = value;
                }
            }
            0xf00a | 0xc61a => {
                // The engine's shared arm marks the DNG level tables too;
                // that bookkeeping belongs to the DNG pass, so a non-DNG
                // container only carries the shared black levels.
                if tag == 0xf00a {
                    // int(MIN(sqrtf(len), 64)), the linear-DNG repeat sizes.
                    let bound = ((len as f32).sqrt().min(64.0)) as i32 as u32;
                    self.cblack[4] = bound;
                    self.cblack[5] = bound;
                }
                if self.ifds[ifd].samples > 1
                    && self.ifds[ifd].samples as u32 == len
                {
                    // Linear DNG, per-channel black.
                    let mut i = 0usize;
                    while i < 4 && len as i32 > i as i32 {
                        let value = self.stream.getrealf(kind);
                        let rounded = value + 0.5;
                        self.cblack[i] = rounded as u32;
                        i += 1;
                    }
                    self.black = 0;
                } else if self.ifds[ifd].samples > 1
                    && (self.ifds[ifd].samples as u32)
                        .wrapping_mul(self.cblack[4])
                        .wrapping_mul(self.cblack[5])
                        == len
                {
                    // Linear DNG with a repeat dimension.
                    for i in 0..(CBLACK_SIZE - 7).min(len as usize) {
                        let value = self.stream.getrealf(kind);
                        let rounded = value + 0.5;
                        self.cblack[i + 6] = rounded as u32;
                    }
                } else if self.cblack[4] * self.cblack[5] < 2 && len == 1 {
                    let value = self.stream.getrealf(kind);
                    self.black = value as u32;
                } else if self.cblack[4] * self.cblack[5] <= len {
                    for c in 0..(self.cblack[4] * self.cblack[5]) as usize {
                        let value = self.stream.getrealf(kind);
                        self.cblack[6 + c] = value as u32;
                    }
                    self.black = 0;
                    for c in 0..4usize {
                        self.cblack[c] = 0;
                    }
                }
            }
            0xf00c => {
                // The camera's two preset rows only; the Tungsten row's
                // RAFData table scan the engine also runs here fills
                // white-balance rows the metadata output never reads, so
                // this pass skips it.
                if self.fuji.raf_data_generation != 4096 {
                    for _ in 0..2 {
                        let mut fwb = [0u32; 4];
                        for slot in fwb.iter_mut() {
                            let value = self.stream.get4();
                            *slot = value;
                        }
                        if fwb[3] < 0x100 {
                            let row = fwb[3] as usize;
                            for c in 0..3usize {
                                let level = fwb[c] as u16;
                                self.ciff_icwbc[row][grbg_to_rgbg_index(c)] = level;
                            }
                            let green = self.ciff_icwbc[row][1];
                            self.ciff_icwbc[row][3] = green;
                        }
                    }
                }
            }
            0xf00d => {
                if self.fuji.raf_data_generation != 4096 {
                    for c in 0..3usize {
                        let value = self.stream.getint(kind) as u16;
                        self.ciff_icwbc[wbi::AUTO][grbg_to_rgbg_index(c)] = value;
                    }
                    let green = self.ciff_icwbc[wbi::AUTO][1];
                    self.ciff_icwbc[wbi::AUTO][3] = green;
                }
            }
            0x0131 => {
                // fgets reads up to 63 bytes, stopping after a newline; a NUL
                // still ends every string comparison.
                let mut software_bytes = [0u8; 63];
                let read = self.stream.read(&mut software_bytes);
                let newline_end = software_bytes[..read]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(read, |position| position + 1);
                let software = software_bytes[..newline_end]
                    .split(|&byte| byte == 0)
                    .next()
                    .unwrap_or(&software_bytes[..newline_end]);
                self.software = String::from_utf8_lossy(software).into_owned();
                const PREFIX_BLOCKED: [&[u8]; 4] = [b"Adobe", b"dcraw", b"UFRaw", b"Bibble"];
                let exact_digital_photo_professional = software == b"Digital Photo Professional";
                if PREFIX_BLOCKED.iter().any(|name| software.starts_with(name))
                    || exact_digital_photo_professional
                {
                    self.is_raw = 0;
                }
            }
            0x013d => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].predictor = value;
            }
            0x0142 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].tile_width = value;
            }
            0x0143 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].tile_length = value;
            }
            0x0144 => {
                // TileOffsets: multi-entry points at the value area itself,
                // and the Sinar four-shot case is not supported by this pass.
                if len > 1 {
                    self.ifds[ifd].offset = self.stream.tell() as i64;
                } else {
                    self.ifds[ifd].offset = self.stream.get4() as i64;
                    self.ifds[ifd].tile_width = 0;
                    self.ifds[ifd].tile_length = 0;
                }
                if len == 4 {
                    self.loader = None;
                    self.is_raw = 5;
                }
            }
            0x014a => {
                if self.model == "DSLR-A100" && self.ifds[ifd].width == 3872 {
                    // The SubIFD pointer is the raw pixel start of this file,
                    // and the frame's directory index slips forward so the
                    // remaining tags of this IFD land in the next slot, on
                    // credit past the IFD count.
                    self.loader = Some(SensorKind::SonyArw);
                    self.data_offset = self.stream.get4() as i64 + base;
                    self.leaked_ifd = Some(ifd + 1);
                } else {
                    self.handle_sub_ifds(ifd, base, len);
                }
            }
            0x0152 => {
                // ExtraSamples, remembered per IFD for the RGB-alpha skip.
                while self.extrasamples.len() <= ifd {
                    self.extrasamples.push(0);
                }
                self.extrasamples[ifd] = (self.stream.getint(kind) & 0xff) + 1024;
            }
            0x0153 => {
                let value = self.stream.getint(kind) as i32;
                self.ifds[ifd].sample_format = value;
            }
            0x7000 => {
                // Sony's raw-file-type byte, `imSony.SonyRawFileType`; the
                // unpacking phase consults it, the metadata pass does not.
                self.stream.get2();
            }
            0x7010 => {
                // Sony's raw tone curve: four knee points into a fresh
                // `{0, 0, 0, 0, 0, 4095}` ladder, then the piecewise-integer
                // rebuild.
                let mut sony_curve = [0u32; 6];
                sony_curve[5] = 4095;
                for step in sony_curve.iter_mut().take(5).skip(1) {
                    *step = u32::from(self.stream.get2() >> 2 & 0xfff);
                }
                for i in 0..5 {
                    let slope = (1u32 << i) as u16;
                    let start = sony_curve[i] as usize + 1;
                    let end = (sony_curve[i + 1] as usize).min(4095);
                    for j in start..=end {
                        let previous = self.curve_head[j - 1];
                        self.curve_head[j] = previous.wrapping_add(slope);
                    }
                }
            }
            0x7200 => {
                self.sony.sr2_offset = self.stream.get4() as i64;
            }
            0x7201 => {
                self.sony.sr2_length = self.stream.get4() as i64;
            }
            0x7221 => {
                self.sony.sr2_key = self.stream.get4();
            }
            0x828d => {
                if self.stream.get2() == 6 && self.stream.get2() == 6 {
                    self.ifds[ifd].filters = 9;
                    self.filters = 9;
                }
            }
            0x828e | 0xfd09 => {
                self.handle_cfa_pattern(ifd, tag, len);
            }
            0x8769 => {
                let exif_offset = self.stream.get4() as i64;
                self.stream.seek((exif_offset + base).max(0) as u64);
                self.parse_exif(base);
            }
            // 0xc612 marks the container as a DNG after all; the meta pass
            // drops it and rawler's own DNG handling covers the file.
            0xc612 => {
                self.dng_version = self.stream.getint(kind);
            }
            // 0xc640: CR2 column slices, carried to the sRAW unpacker.
            0xc640 => {
                self.stream.read_shorts(&mut self.cr2_slice);
                self.stream.seek(parsed.save);
            }
            0xc634 => {
                self.handle_private_data(base, len);
            }
            _ => {}
        }
    }

    /// Tag 0x0111: strip offsets, then the embedded lossless-JPEG header when
    /// the bits-per-sample has not reached the IFD yet.
    fn handle_strip_offsets(&mut self, ifd: usize, base: i64, len: u32) {
        if len > 1 && len < 16384 {
            let restore = self.stream.tell();
            let mut offsets = Vec::with_capacity(len as usize);
            for _ in 0..len {
                offsets.push(self.stream.get4() as i64 + base);
            }
            self.ifds[ifd].strip_offsets = offsets;
            self.stream.seek(restore);
        }
        let offset = self.stream.get4() as i64 + base;
        self.ifds[ifd].offset = offset;

        if self.ifds[ifd].bps == 0 && self.ifds[ifd].offset > 0 {
            self.stream.seek(offset as u64);
            let Some(jhead) = self.ljpeg_start(true) else {
                return;
            };
            let sony_note = self.dng_version == 0 && starts_ignore_case(&self.make, "SONY");
            if sony_note && self.ifds[ifd].phint == 32803 && self.ifds[ifd].comp == 7 {
                // A lossless-compressed Sony IFD: the probe reads the JPEG
                // header for the bit depth alone.
                self.ifds[ifd].comp = 6;
                self.ifds[ifd].bps = jhead.bits;
                self.ifds[ifd].samples = 1;
            } else if sony_note && self.ifds[ifd].phint == 6 && self.ifds[ifd].comp == 7
                && self.ifds[ifd].samples == 3
            {
                // Sony's YCbCr lossless note: bit depth only, the sample count
                // keeps its three.
                self.ifds[ifd].comp = 6;
                self.ifds[ifd].bps = jhead.bits;
            } else {
                self.ifds[ifd].comp = 6;
                self.ifds[ifd].width = jhead.wide;
                self.ifds[ifd].height = jhead.high;
                self.ifds[ifd].bps = jhead.bits;
                self.ifds[ifd].samples = jhead.clrs;
                let is_sraw = jhead.sraw != 0;
                if !(is_sraw || (jhead.clrs & 1) != 0) {
                    self.ifds[ifd].width *= jhead.clrs;
                }
                let stretched = self.ifds[ifd].width > 4 * self.ifds[ifd].height && jhead.clrs % 2 == 0;
                if stretched {
                    self.ifds[ifd].width /= 2;
                    self.ifds[ifd].height *= 2;
                }
                let outer_order = self.stream.order;
                self.parse_tiff(offset + 12);
                self.stream.order = outer_order;
            }
        }
    }

    /// Tag 0x014a: child directories.
    fn handle_sub_ifds(&mut self, ifd: usize, base: i64, len: u32) {
        let len = len.min(1000);
        for _ in 0..len {
            let resume = self.stream.tell();
            let offset = self.stream.get4() as i64;
            self.stream.seek((offset + base).max(0) as u64);
            if self.parse_tiff_ifd(base) {
                return;
            }
            self.stream.seek(resume + 4);
        }
    }

    /// Tag 0xc634: the private-data blob. Adobe's DNG private notes,
    /// Pentax/Ricoh/Samsung camera notes and the Minolta `\0MR` block share
    /// the tag; the parse re-enters the directory walk after them for the
    /// Minolta case.
    fn handle_private_data(&mut self, base: i64, len: u32) {
        let start = self.stream.tell() as i64;
        let marker_order = self.stream.order;
        let mut mbuf = [0u8; 8];
        self.stream.read(&mut mbuf[..6]);
        if &mbuf[..6] == b"Adobe\0" {
            // Adobe's private blob: `MakN` notes hand their byte order in, so
            // the maker note parse runs against a shifted base. The Panama
            // and Fuji raw parts touch DNG-only state and are left out; the
            // SR2 part runs through the shared decoder below.
            self.stream.order = BIG_ENDIAN;
            let mut curr_pos = start + 6;
            while curr_pos + 8 - start <= len as i64 {
                let mut part = [0u8; 4];
                self.stream.read(&mut part);
                curr_pos += 8;
                if &part != b"MakN" {
                    continue;
                }
                let makn_length = self.stream.get4();
                let makn_order = self.stream.get2() as u16;
                let pos_in_original_raw = self.stream.get4() as i64;
                self.stream.order = makn_order;
                let save_pos = self.stream.tell() as i64;
                let note_base = curr_pos + 6 - pos_in_original_raw;
                self.stream.seek(save_pos as u64);
                self.parse_makernote(note_base, 0);
                curr_pos = save_pos + makn_length as i64 - 6;
                self.stream.seek(curr_pos as u64);
                self.stream.read(&mut part);
                curr_pos += 8;
                if &part == b"SR2 " {
                    // The SR2 part carries the same length/order/base header
                    // the maker notes do, then a directory of SR2 pointers.
                    self.stream.order = BIG_ENDIAN;
                    let _sr2_length = self.stream.get4();
                    let sr2_order = self.stream.get2() as u16;
                    let pos_in_original_raw = self.stream.get4() as i64;
                    self.stream.order = sr2_order;
                    let sr2_base = curr_pos + 6 - pos_in_original_raw;
                    let mut sr2_offset = 0u32;
                    let mut sr2_length = 0u32;
                    let mut sr2_key = 0u32;
                    let mut entries = self.stream.get2();
                    while entries != 0 {
                        entries -= 1;
                        let tag = self.stream.get2();
                        let kind = self.stream.get2();
                        let entry_len = self.stream.get4();
                        let resume = self.stream.tell() + 4;
                        let unit = TYPE_UNIT_BYTES
                            [usize::try_from(kind).map_or(0, |kind| if kind < 20 { kind } else { 0 })];
                        if u64::from(entry_len) * u64::from(unit) > 4 {
                            let pointer = self.stream.get4();
                            self.stream.seek((i64::from(pointer) + sr2_base).max(0) as u64);
                        }
                        match tag {
                            0x7200 => sr2_offset = self.stream.get4(),
                            0x7201 => sr2_length = self.stream.get4(),
                            0x7221 => sr2_key = self.stream.get4(),
                            _ => {}
                        }
                        self.stream.seek(resume as u64);
                    }
                    if sr2_length > 0 && sr2_length < 10_240_000 {
                        self.decrypt_sony_sr2_rows(
                            sr2_offset as i64 + base,
                            sr2_length as i64,
                            sr2_key,
                            sr2_offset as i64,
                        );
                    }
                }
                break;
            }
        } else {
            self.stream.read(&mut mbuf[6..]);
            let ricoh = &mbuf[..5] == b"RICOH" && mbuf[5] == 0 && {
                let mark = u16::from(mbuf[6]) | (u16::from(mbuf[7]) << 8);
                mark == LITTLE_ENDIAN || mark == BIG_ENDIAN
            };
            let pentax = &mbuf[..8] == b"PENTAX \0";
            let samsung = &mbuf[..8] == b"SAMSUNG\0";
            if pentax || samsung || ricoh {
                self.stream.seek(start as u64);
                self.parse_makernote(base, 0);
            }
        }
        self.stream.seek(start as u64);
        self.stream.order = marker_order;
        if self.dng_version != 0 {
            return;
        }
        // The Minolta tail: a pointer into a `\0MR` block inside the file,
        // walked both as the Minolta container and as a directory probe.
        let minolta_at = self.stream.get4() as i64 + base;
        self.parse_minolta(minolta_at);
        self.stream.seek(minolta_at.max(0) as u64);
        let _ = self.parse_tiff_ifd(base);
    }

    /// `parse_minolta`: the Sony A100 and its Minolta ancestors embed a
    /// `\0MR` private block; its PRD record carries the raw size, its RIF and
    /// WBG records carry the white balance.
    fn parse_minolta(&mut self, base: i64) {
        self.stream.seek(base.max(0) as u64);
        if self.stream.getc() != Some(0)
            || self.stream.getc() != Some(b'M')
            || self.stream.getc() != Some(b'R')
        {
            return;
        }
        let sorder = self.stream.order;
        self.stream.order = u16::from(self.stream.getc().unwrap_or(0)) * 0x101;
        let mut offset = base + self.stream.get4() as i64 + 8;
        let fsize = self.fsize();
        if offset > fsize - 8 {
            offset = fsize - 8;
        }

        let mut high = 0i32;
        let mut wide = 0i32;
        // The engine's bounds failures leave without the size assignment or
        // the order restore, so both stay untouched there.
        let mut truncated = false;
        while (self.stream.tell() as i64) < offset && !truncated {
            let save = self.stream.tell() as i64;
            let mut name = [0u8; 4];
            self.stream.read(&mut name);
            let tag = u32::from(name[0]) << 24
                | u32::from(name[1]) << 16
                | u32::from(name[2]) << 8
                | u32::from(name[3]);
            let len = self.stream.get4() as i32 as i64;
            if len < 0 || len + save + 8 > fsize {
                truncated = true;
                break;
            }
            match tag {
                0x0050_5244 => {
                    // PRD: the raw plan. The eight skipped bytes hold the
                    // version, then height, width, the image plan, the bit
                    // depths, the storage method and the Bayer pattern.
                    self.stream.skip(8);
                    high = self.stream.get2() as i32;
                    wide = self.stream.get2() as i32;
                    self.stream.get2();
                    self.stream.get2();
                    self.stream.getc();
                    self.stream.getc();
                    self.stream.getc();
                    self.stream.skip(4);
                    self.sony.prd_bayer_pattern = self.stream.getc().unwrap_or(0) as u16;
                }
                0x0052_4946 => {
                    // RIF: per-illuminant red and blue records into the
                    // shared white-balance table, greens at 0x100.
                    self.stream.skip(8);
                    let rows = [
                        (sony_wbi::TUNGSTEN, 0usize),
                        (sony_wbi::TUNGSTEN, 2usize),
                        (sony_wbi::DAYLIGHT, 0),
                        (sony_wbi::DAYLIGHT, 2),
                        (sony_wbi::CLOUDY, 0),
                        (sony_wbi::CLOUDY, 2),
                        (sony_wbi::FL_W, 0),
                        (sony_wbi::FL_W, 2),
                        (sony_wbi::FLASH, 0),
                        (sony_wbi::FLASH, 2),
                        (sony_wbi::CUSTOM, 0),
                        (sony_wbi::CUSTOM, 2),
                    ];
                    for (row, channel) in rows {
                        self.ciff_icwbc[row][channel] = self.stream.get2();
                    }
                    for row in rows.iter().map(|&(row, _)| row) {
                        self.ciff_icwbc[row][1] = 0x100;
                        self.ciff_icwbc[row][3] = 0x100;
                    }
                    if self.model.as_bytes()[..9].eq_ignore_ascii_case(b"DSLR-A100") {
                        let a100_rows = [
                            (sony_wbi::SHADE, 0usize),
                            (sony_wbi::SHADE, 2usize),
                            (sony_wbi::FL_D, 0),
                            (sony_wbi::FL_D, 2),
                            (sony_wbi::FL_N, 0),
                            (sony_wbi::FL_N, 2),
                            (sony_wbi::FL_WW, 0),
                            (sony_wbi::FL_WW, 2),
                        ];
                        for (row, channel) in a100_rows {
                            self.ciff_icwbc[row][channel] = self.stream.get2();
                        }
                        for (row, _) in a100_rows.iter() {
                            self.ciff_icwbc[*row][1] = 0x100;
                            self.ciff_icwbc[*row][3] = 0x100;
                        }
                    }
                }
                0x0057_4247 => {
                    // WBG: the camera white balance, in the Bayer order the
                    // PRD record announced.
                    self.stream.get4();
                    if self.sony.prd_bayer_pattern == 4 {
                        for cell in 0..4usize {
                            let channel = g2brg1_to_rgbg_index(cell);
                            self.cam_mul[channel] = self.stream.get2() as f32;
                        }
                    } else {
                        for cell in 0..4usize {
                            let channel = rggb_to_rgbg_index(cell);
                            self.cam_mul[channel] = self.stream.get2() as f32;
                        }
                    }
                }
                0x0054_5457 => {
                    // TTW: an embedded TIFF, with the raw data following the
                    // private block.
                    let at = self.stream.tell() as i64;
                    self.parse_tiff(at);
                    self.data_offset = offset;
                }
                _ => {}
            }
            self.stream.seek((save + len + 8).max(0) as u64);
        }
        if truncated {
            return;
        }
        self.raw_height = high;
        self.raw_width = wide;
        self.stream.order = sorder;
    }

    /// Tags 0x828e and 0xfd09: the CFA pattern. X-Trans arrives as 36 flat
    /// bytes (or the filter layout is already 9 on tag 0x828e); otherwise
    /// `guess_cfa_pc` builds the filter word from the pattern bytes.
    fn handle_cfa_pattern(&mut self, ifd: usize, tag: u32, len: u32) {
        if tag == 0x828e && self.filters == 9 {
            self.fill_xtrans();
            return;
        }
        if len == 36 {
            self.ifds[ifd].filters = 9;
            self.filters = 9;
            self.colors = 3;
            self.fill_xtrans();
            return;
        }
        if len == 0 {
            return;
        }
        let pattern_len = (len as usize).min(16);
        let mut cfa_pattern = [0u8; 16];
        self.stream.read(&mut cfa_pattern[..pattern_len]);
        let mut seen = 0u32;
        let mut colors = 0usize;
        for &colour in &cfa_pattern[..pattern_len] {
            if colour > 31 {
                continue;
            }
            if colors >= 4 {
                break;
            }
            colors += usize::from((seen & (1 << colour)) == 0);
            seen |= 1 << colour;
        }
        let cfa_pc: [u8; 4] = match seen {
            // CMY.
            0o70 => [3, 4, 5, 0],
            // GMCY.
            0o72 => [5, 3, 4, 1],
            _ => [0, 1, 2, 3],
        };
        self.colors = colors;
        self.guess_cfa_pc(ifd, &cfa_pattern, pattern_len, &cfa_pc);
    }

    /// 36 X-Trans cells, byte for byte, masked to two bits.
    fn fill_xtrans(&mut self) {
        for cell in self.xtrans.iter_mut().flatten() {
            let Some(byte) = self.stream.getc() else {
                break;
            };
            *cell = byte & 3;
        }
    }

    /// `guess_cfa_pc`: build `filters` from the CFA pattern bytes. The table
    /// is 256 entries so pattern bytes past the plane count read as zero.
    fn guess_cfa_pc(&mut self, ifd: usize, cfa_pattern: &[u8; 16], pattern_len: usize, cfa_pc: &[u8]) {
        let mut table = [0u32; 256];
        for (channel, &colour) in cfa_pc.iter().enumerate().take(self.colors.min(4)) {
            table[colour as usize] = channel as u32;
        }
        let mut filters = 0u32;
        for index in (0..16).rev() {
            let colour = cfa_pattern[index % pattern_len] as usize;
            filters = filters << 2 | table[colour];
        }
        filters -= u32::from(filters == 0);
        self.filters = filters;
        self.ifds[ifd].filters = filters;
    }
}

// ===== IFD selection (`apply_tiff`) =========================================

/// `INT_MAX`, the tile size the engine substitutes when no tile tags appeared.
const INT_MAX: u32 = 0x7fff_ffff;

impl<'a> Walker<'a> {
    /// `apply_tiff`: pick the raw IFD out of the parsed records, copy its
    /// facts to the walker and choose the pixel loader. Thumbnail selection
    /// and shutter fill have no counterpart in the metadata output.
    fn apply_tiff(&mut self) {
        let mut max_samp: i32 = 0;
        let mut ties: i32 = 0;
        let mut raw: i32 = -1;

        if self.dng_version != 0 {
            // DNG containers drop out of the meta pass, so only a consistent
            // raw pick is kept here. The `DNG_ADD_ENHANCED`, `_PREVIEWS` and
            // `_MASKS` switches are off by default, which leaves
            // `newsubfiletype == 0` the only qualifying main image.
            let mut frames: Vec<u32> = Vec::new();
            for index in 0..self.ifd_count {
                let record = &self.ifds[index];
                if !(1..=65535).contains(&record.width)
                    || !(1..=65535).contains(&record.height)
                {
                    continue;
                }
                let samples = if record.samples == 2 {
                    1 // Fuji 2-frame
                } else {
                    record.samples
                };
                max_samp = max_samp.max(samples).clamp(1, 3);
                let raw_subfile = record.newsubfiletype == 16
                    || record.newsubfiletype == 0
                    || record.newsubfiletype & 0xffff == 1;
                if raw_subfile && record.phint != 32803 && record.phint != 34892 {
                    continue;
                }
                if record.newsubfiletype == 0 {
                    let frame = ((record.newsubfiletype as u32 & 0xffff) << 16)
                        | ((index as u32) << 8) & 0xff00;
                    frames.push(frame);
                    if record.samples == 2 {
                        frames.push(frame | 1);
                    }
                }
            }
            if frames.is_empty() {
                self.is_raw = 0;
            } else {
                let chosen_index = (self.shot_select.clamp(0, frames.len() as i32 - 1)) as usize;
                let chosen = ((frames[chosen_index] >> 8) & 0xff) as usize;
                let record = &self.ifds[chosen];
                self.raw_width = record.width;
                self.raw_height = record.height;
                self.tiff_bps = record.bps as u32;
                self.tiff_compress = record.comp as u32;
                self.tiff_sampleformat = record.sample_format as u32;
                self.data_offset = record.offset;
                self.data_size = record.bytes;
                self.tiff_flip = record.flip as u32;
                self.tiff_samples = record.samples as u32;
                self.tile_width = record.tile_width as u32;
                self.tile_length = record.tile_length as u32;
                self.fuji_width = record.fuji_width as u32;
                if record.samples != 2 {
                    self.filters = if record.phint == 34892 {
                        0
                    } else if chosen > 0
                        && record.phint == 32803
                        && self.ifds[0].phint == 32803
                        && record.filters == 0
                        && self.ifds[0].filters != 0
                    {
                        self.ifds[0].filters
                    } else {
                        record.filters
                    };
                    self.width = record.vwidth;
                    self.height = record.vheight;
                    self.top_margin = record.top_margin;
                    self.left_margin = record.left_margin;
                }
                if record.dng_levels.dng_whitelevel[0] != 0 {
                    self.maximum = record.dng_levels.dng_whitelevel[0];
                } else if record.sample_format <= 2
                    && record.bps > 0
                    && record.bps < 32
                {
                    self.maximum = (1i64 << record.bps) as u32 - 1;
                } else if record.sample_format == 3 {
                    self.maximum = 1;
                }
                self.raw_ifd = chosen as i32;
                self.is_raw = frames.len() as i32;
            }
            return;
        }

        // Sony's broken bits-per-sample tag: bring it back to the real depth.
        if starts_ignore_case(&self.make, "Sony") {
            for index in 0..self.ifd_count {
                if self.ifds[index].bps > 33 && self.ifds[index].samples == 1 {
                    let mut bps = 14;
                    let white = self.ifds[index].dng_levels.dng_whitelevel[0];
                    if white > 0 {
                        for (level, bit) in (0..16).map(|level| (level, 1i32 << level)) {
                            if bit > white as i32 {
                                bps = level;
                                break;
                            }
                        }
                    }
                    self.ifds[index].bps = bps;
                }
            }
        }

        for index in 0..self.ifd_count {
            let record = &self.ifds[index];
            if !(1..=65535).contains(&record.width)
                || !(1..=65535).contains(&record.height)
            {
                continue;
            }
            if max_samp < record.samples {
                max_samp = record.samples;
            }
            if max_samp > 3 {
                max_samp = 3;
            }
            // The sizes are compared in bits when a depth is known.
            let old_pixels = (self.raw_width as u32 as u64)
                .wrapping_mul(self.raw_height as u32 as u64);
            let new_pixels = (record.width as u32 as u64).wrapping_mul(record.height as u32 as u64);
            let old_bits;
            let new_bits;
            if self.tiff_bps != 0 {
                old_bits = old_pixels * self.tiff_bps as u64;
                new_bits = new_pixels * record.bps as u64;
            } else {
                old_bits = old_pixels;
                new_bits = new_pixels;
            }
            if record.phint == 2
                && self.extrasamples.get(index).copied().unwrap_or(0) > 0
                && record.samples > 3
            {
                continue; // RGB + alpha records stay preview material
            }
            if starts_ignore_case(&self.make, "Sony")
                && record.bps == 8
                && record.phint == 6
                && record.comp == 7
                && record.samples == 3
                && record.newsubfiletype == 1
            {
                continue; // Sony RGB preview
            }
            let sizeable = record.comp != 6
                || record.samples != 3
                || (record.comp == 6
                    && record.samples == 3
                    && starts_ignore_case(&self.make, "Sony")
                    && record.bps >= 12);
            // Both dimensions are already in range, so the engine's combined
            // `t_width | t_height` guard passes for every record here.
            let wins = sizeable
                && (record.bps as u32) < 33
                && (record.samples as u32) < 13
                && new_bits != 0
                && if new_bits > old_bits {
                    ties = 1;
                    true
                } else if new_bits == old_bits {
                    let seen = ties;
                    ties += 1;
                    self.shot_select == seen
                } else {
                    false
                };
            if wins {
                self.raw_width = record.width;
                self.raw_height = record.height;
                self.tiff_bps = record.bps as u32;
                self.tiff_compress = record.comp as u32;
                self.tiff_sampleformat = record.sample_format as u32;
                self.data_offset = record.offset;
                self.data_size = record.bytes;
                self.tiff_flip = record.flip as u32;
                self.tiff_samples = record.samples as u32;
                self.tile_width = record.tile_width as u32;
                self.tile_length = record.tile_length as u32;
                self.raw_ifd = index as i32;
                raw = index as i32;
            }
        }
        if self.is_raw == 1 && ties != 0 {
            self.is_raw = ties;
        }

        if self.nikon_transfer != 0 && raw >= 0 {
            // Debayered Nikon transfers: repair the depth or flag the file.
            let record_bps = self.ifds[raw as usize].bps;
            if record_bps == 16 {
                if self.tiff_compress == 1 {
                    let bytes_doubled = self.raw_width as i64 * self.raw_height as i64 * 3;
                    self.tiff_bps =
                        if bytes_doubled == self.ifds[raw as usize].bytes << 1 { 12 } else { 14 };
                    self.ifds[raw as usize].bps = self.tiff_bps as i32;
                }
            } else if record_bps == 8 && self.tiff_compress == 1 {
                self.nikon_transfer = 2;
                // coolscan_nef_gamma stays at its default in this pass.
            }
        }

        if self.tile_width == 0 {
            self.tile_width = INT_MAX;
        }
        if self.tile_length == 0 {
            self.tile_length = INT_MAX;
        }
        for index in (0..self.ifd_count).rev() {
            if self.ifds[index].flip != 0 {
                self.tiff_flip = self.ifds[index].flip as u32;
            }
        }

        self.choose_loader(raw);
    }

    /// The `load_raw` switch of `apply_tiff`. Loaders the metadata pass does
    /// not model (per-maker Sony, Nikon, Kodak, Olympus, Pentax) leave
    /// `loader` at `None`: the walker rejects those files the same way it
    /// rejects unknown makers.
    fn choose_loader(&mut self, raw: i32) {
        if raw < 0 || self.loader.is_some() {
            return;
        }
        let record = self.ifds[raw as usize].clone();
        let pixel_count = self.raw_width as i64 * self.raw_height as i64;
        match self.tiff_compress {
            32767 => {
                // Sony ARW v2 / ARW / unpacked variants.
                if self.dng_version == 0 && record.bytes == pixel_count {
                    self.tiff_bps = 14;
                    self.loader = Some(SensorKind::SonyArw2);
                } else if self.dng_version == 0
                    && starts_ignore_case(&self.make, "Sony")
                    && record.bytes == pixel_count * 2
                {
                    self.tiff_bps = 14;
                    self.loader = Some(SensorKind::Unpacked);
                } else if record.bytes * 8 != pixel_count * self.tiff_bps as i64 {
                    self.raw_height += 8;
                    self.loader = Some(SensorKind::SonyArw);
                } else {
                    self.load_flags = 79;
                    self.load_flags += 1;
                    self.choose_unpacked(raw);
                }
            }
            32769 => {
                self.load_flags += 1;
                self.choose_unpacked(raw);
            }
            32770 | 32773 => self.choose_unpacked(raw),
            0 | 1 => {
                if self.dng_version != 0
                    && self.tiff_sampleformat == 3
                    && self.tiff_bps > 8
                    && self.tiff_bps % 8 == 0
                    && self.tiff_bps <= 32
                {
                    // uncompressed_fp_dng_load_raw is the DNG pass's loader.
                } else if self.dng_version == 0
                    && starts_ignore_case(&self.make, "Sony")
                    && record.bytes == pixel_count * 2
                {
                    self.tiff_bps = 14;
                    self.loader = Some(SensorKind::Unpacked);
                } else if self.dng_version == 0
                    && starts_ignore_case(&self.make, "Sony")
                    && record.samples == 4
                    && record.bytes == pixel_count * 8
                {
                    // The pixel-shift composites: four interlaced samples per
                    // pixel, green-blue swapped on the way out.
                    self.tiff_bps = 14;
                    self.tiff_samples = 4;
                    self.filters = 0;
                    self.loader = Some(SensorKind::SonyArq);
                } else if starts_ignore_case(&self.make, "Nikon")
                    && (self.software.starts_with("Nikon Scan")
                        || self.nikon_transfer == 2
                        || contains_ignore_case(&self.model, "COOLSCAN"))
                {
                    // nikon_coolscan_load_raw is not modelled yet.
                    self.raw_color = true;
                    self.colors = if self.tiff_samples == 3 { 3 } else { 1 };
                    self.filters = 0;
                } else {
                    if (self.make.starts_with("OLYMPUS")
                        || self.make.starts_with("OM Digi")
                        || (starts_ignore_case(&self.make, "CLAUSS")
                            && starts_ignore_case(&self.model, "piX 5oo")))
                        && record.bytes * 2 == pixel_count * 3
                    {
                        self.load_flags = 24;
                    }
                    if self.dng_version == 0 && record.bytes * 5 == pixel_count * 8 {
                        self.load_flags = 81;
                        self.tiff_bps = 12;
                    }
                    self.choose_unpacked(raw);
                }
            }
            6 | 7 | 99 => {
                let sony_make = self.make.eq_ignore_ascii_case("SONY");
                if self.dng_version == 0 && self.tiff_compress == 6 && sony_make {
                    if record.phint == 6 && record.bps >= 12 && record.samples == 3 {
                        // sony_ycbcr_load_raw: the R1's tiled YCC stream. Its
                        // decompressor is not ported; the loader stays unset,
                        // so the file keeps rawler's decode.
                        self.filters = 0;
                        self.colors = 3;
                    } else {
                        // The single-sample comp-6 files decode through the
                        // shared lossless-JPEG loader with Sony's tile walk.
                        self.loader = Some(SensorKind::SonyLjpeg);
                    }
                } else if self.dng_version == 0
                    && self.tiff_compress == 7
                    && sony_make
                    && (self.model.eq_ignore_ascii_case("ILCE-1")
                        || starts_ignore_case(&self.model, "ILCE-7")
                        || starts_ignore_case(&self.model, "ILCE-9"))
                    && self.tiff_bps == 14
                    && self.tiff_samples == 1
                {
                    self.loader = Some(SensorKind::SonyLjpeg);
                } else {
                    self.loader = Some(SensorKind::CanonLosslessJpeg);
                }
            }
            262 => {} // kodak_262_load_raw is not modelled yet.
            34713 => {
                // The Nikon packed-family arms: all land on non-modelled
                // native loaders, so only their size checks stay for parity.
                let bytes_fits = ((self.raw_width as i64 + 9) / 10 * 16 * self.raw_height as i64)
                    == record.bytes;
                let bytes_triple = self.raw_width as i64 * self.raw_height as i64 * 3
                    == record.bytes << 1;
                let bytes_yuv =
                    self.raw_width as i64 * self.raw_height as i64 * 3 == record.bytes;
                let bytes_pair =
                    self.raw_width as i64 * self.raw_height as i64 * 2 == record.bytes;
                if bytes_fits {
                    self.loader = Some(SensorKind::Packed);
                    self.load_flags = 1;
                } else if bytes_triple {
                    self.loader = Some(SensorKind::Packed);
                    if self.model.starts_with('N') {
                        self.load_flags = 80;
                    }
                } else if bytes_yuv {
                    self.loader = Some(SensorKind::NikonYuv);
                    // The s-curve build the compressed arm makes here runs
                    // before the open stage rebuilds the whole table for the
                    // YUV bodies; keep its marker for the hand-off.
                    self.yuv_gamma = true;
                    self.cblack.fill(0);
                    self.filters = 0;
                } else if bytes_pair {
                    self.loader = Some(SensorKind::Unpacked);
                    self.load_flags = 4;
                    self.stream.order = BIG_ENDIAN;
                } else if record.rows_per_strip != 0
                    && !record.strip_offsets.is_empty()
                    && record.strip_offsets.len() == record.strip_byte_counts.len()
                {
                    // The striped arm fits only when every strip but the last
                    // is three bytes per two sensor columns.
                    let mut fits = true;
                    let strips = record.strip_byte_counts.len();
                    for count in &record.strip_byte_counts[..strips - 1] {
                        if *count * 2 != self.raw_height as i64 * self.raw_width as i64 * 3 {
                            fits = false;
                            break;
                        }
                    }
                    if fits {
                        self.loader = Some(SensorKind::NikonStripedPacked);
                    } else {
                        self.loader = Some(SensorKind::NikonPlain);
                    }
                } else if (((self.raw_width as i64 * 3 / 2 + 15) / 16 * 16
                    * self.raw_height as i64)
                    == record.bytes)
                {
                    self.loader = Some(SensorKind::NikonPaddedPacked);
                    self.load_flags = u32::try_from((self.raw_width as i64 * 3 / 2 + 15) / 16 * 16).unwrap_or(0);
                } else if (self.model.starts_with("NIKON Z 9")
                    || self.model.starts_with("NIKON Z 8")
                    || self.model == "NIKON Z f"
                    || self.model == "NIKON Z6_3")
                    && record.offset != 0
                {
                    // The JPEG-XS probe: four bytes at the strip offset decide
                    // the loader; the stream position is restored either way.
                    let position = self.stream.tell();
                    let mut signature = [0u8; 4];
                    self.stream.seek(record.offset as u64);
                    self.stream.read(&mut signature);
                    self.stream.seek(position);
                    if &signature == b"\xff\x10\xff\x50" {
                        self.loader = Some(SensorKind::NikonHe);
                    } else {
                        self.loader = Some(SensorKind::NikonPlain);
                    }
                } else {
                    self.loader = Some(SensorKind::NikonPlain);
                }
            }
            65535 => self.loader = Some(SensorKind::Pentax),
            65000 => match record.phint {
                2 | 6 => self.filters = 0, // kodak_rgb / kodak_ycbcr
                32803 => {}                // kodak_65000_load_raw
                _ => {}
            },
            32867 | 34892 => {} // the DNG pass's lossless-JPEG containers.
            8 => {}
            52546 | 9 => {
                // JPEG XL / lossless-JPEG DNG compressions: only DNG reaches
                // loaders the engine assigns elsewhere.
                if self.dng_version == 0 {
                    self.is_raw = 0;
                }
            }
            _ => self.is_raw = 0,
        }

        self.reject_non_raw(raw);
    }

    /// The engine's `slr:` label, reached from the unpacked-family arms.
    fn choose_unpacked(&mut self, raw: i32) {
        let record_phint = self.ifds[raw as usize].phint;
        let record_bytes = self.ifds[raw as usize].bytes;
        match self.tiff_bps {
            8 => self.loader = Some(SensorKind::EightBit),
            12 => {
                if record_phint == 2 {
                    self.load_flags = 6;
                }
                let row_bytes = (self.raw_width as u32)
                    .wrapping_mul(self.raw_height as u32)
                    .wrapping_mul(2);
                if starts_ignore_case(&self.make, "NIKON")
                    && starts_ignore_case(&self.model, "COOLPIX A1000")
                    && self.data_size == row_bytes as i64
                {
                    self.loader = Some(SensorKind::Unpacked);
                } else {
                    self.loader = Some(SensorKind::Packed);
                }
            }
            14 => {
                self.load_flags = 0;
                self.choose_unpacked_or_olympus(raw, record_bytes);
            }
            16 => self.choose_unpacked_or_olympus(raw, record_bytes),
            _ => {} // other depths leave the loader unset in the engine, too
        }
    }

    /// The unpacked loader with the Olympus depth-quirk override.
    fn choose_unpacked_or_olympus(&mut self, raw: i32, record_bytes: i64) {
        self.loader = Some(SensorKind::Unpacked);
        let pixel_count = self.raw_width as i64 * self.raw_height as i64;
        let olympus = (self.make.starts_with("OLYMPUS")
            || self.make.starts_with("OM Digi")
            || (starts_ignore_case(&self.make, "CLAUSS")
                && starts_ignore_case(&self.model, "piX 5oo")))
            && record_bytes * 7 > pixel_count;
        if olympus {
            // `olympus_load_raw`: the record says how the rows are split, so
            // the loader reads the body's own bit plan.
            self.loader = Some(SensorKind::Olympus);
        }
    }

    /// The reject pair at the tail of `apply_tiff`: three-sample TIFFs that
    /// are not a supported debay, and 8-bit files from unknown makers.
    fn reject_non_raw(&mut self, raw: i32) {
        if self.dng_version != 0 {
            return;
        }
        // `tiff_ifd[raw]` when no IFD won: the engine reads the record slot
        // in front of the array, which stays empty here (`Ifd::default`).
        let record = if raw >= 0 {
            self.ifds[raw as usize].clone()
        } else {
            Ifd::default()
        };
        // `model2` only fills during Nikon processing and stays empty here.
        let three_sample_reject = self.tiff_samples == 3
            && record.bytes != 0
            && !(self.tiff_bps == 16 && self.make.starts_with("Leaf"))
            && !(record.comp == 6
                && record.phint == 6
                && self.tiff_bps >= 12
                && starts_ignore_case(&self.make, "Sony"))
            && self.tiff_bps != 14
            && (self.tiff_compress & !0xf) != 32768;
        let eight_bit_reject = self.tiff_bps == 8
            && !self.make.starts_with("Phase")
            && !self.make.starts_with("Leaf")
            && !contains_ignore_case(&self.make, "Kodak");
        if (three_sample_reject || eight_bit_reject)
            && !contains_ignore_case(&self.model, "COOLSCAN")
            && !self.software.starts_with("Nikon Scan")
            && self.nikon_transfer != 2
        {
            self.is_raw = 0;
        }
        if self.is_raw != 0
            && raw >= 0
            && record.phint == 2
            && self.extrasamples.get(raw as usize).copied().unwrap_or(0) > 0
            && record.samples > 3
        {
            self.is_raw = 0;
        }
    }
}

/// Parsed SOF3/SOF1/SOF0 header facts (`jhead`).
#[derive(Debug, Clone, Copy, Default)]
struct JHead {
    algo: u32,
    bits: i32,
    high: i32,
    wide: i32,
    clrs: i32,
    sraw: i32,
    restart: u32,
    psv: i32,
}

/// `ljpeg_start` with `info_only`: walk JPEG markers to the frame header and
/// fill `JHead` (no Huffman tables, no decoding state).
///
/// Returns `None` when the data does not start with a JPEG SOI or the header
/// values are out of range.
impl<'a> Walker<'a> {
    fn ljpeg_start(&mut self, info_only: bool) -> Option<JHead> {
        let mut head = JHead { restart: i32::MAX as u32, ..JHead::default() };
        // The SOI bytes arrive through one two-byte read, then the marker
        // loop reads each marker plus its length word as one four-byte read.
        let mut soi = [0u8; 2];
        if self.stream.read(&mut soi) < 2 || soi[1] != 0xd8 {
            return None;
        }
        let mut tag_count = 0i32;
        loop {
            if self.stream.eof() {
                return None;
            }
            tag_count += 1;
            if tag_count > 1024 {
                return None;
            }
            let mut header = [0u8; 4];
            if self.stream.read(&mut header) < 4 {
                return None;
            }
            let tag = ((header[0] as u32) << 8) | header[1] as u32;
            let length = ((header[2] as u32) << 8 | header[3] as u32).wrapping_sub(2);
            if tag <= 0xff00 {
                return None;
            }
            let mut data = vec![0u8; length as usize];
            if self.stream.read(&mut data) < length as usize {
                return None;
            }
            match tag {
                0xffc3 => {
                    // Lossless Huffman, with the sRAW component factors.
                    let factors = data[7];
                    head.sraw = ((i32::from(factors >> 4) * i32::from(factors & 15)) - 1) & 3;
                    head.algo = (tag & 0xff) as u32;
                    head.bits = i32::from(data[0]);
                    head.high = i32::from(data[1]) << 8 | i32::from(data[2]);
                    head.wide = i32::from(data[3]) << 8 | i32::from(data[4]);
                    head.clrs = i32::from(data[5]) + head.sraw;
                    if length == 9 && self.dng_version == 0 {
                        self.stream.getc();
                    }
                }
                0xffc1 | 0xffc0 => {
                    head.algo = (tag & 0xff) as u32;
                    head.bits = i32::from(data[0]);
                    head.high = i32::from(data[1]) << 8 | i32::from(data[2]);
                    head.wide = i32::from(data[3]) << 8 | i32::from(data[4]);
                    head.clrs = i32::from(data[5]) + head.sraw;
                }
                0xffc4 => {
                    // Huffman tables: skipped for an info-only walk.
                    if !info_only {
                        return None;
                    }
                }
                0xffdb => {
                    // Quantisation tables: not needed for metadata.
                }
                0xffda => {
                    head.psv = i32::from(data[1 + usize::try_from(data[0]).ok()? * 2]);
                    head.bits -= i32::from(data[3 + usize::try_from(data[0]).ok()? * 2] & 15);
                }
                0xffdd => {
                    head.restart = (u32::from(data[0]) << 8) | u32::from(data[1]);
                }
                _ => {}
            }
            if tag == 0xffda {
                break;
            }
        }
        let (bits, high, wide, clrs) = (head.bits, head.high, head.wide, head.clrs);
        if bits > 16 || clrs > 6 || bits == 0 || high == 0 || wide == 0 || clrs == 0 {
            return None;
        }
        Some(head)
    }
}

// ===== EXIF directory =======================================================

/// `strncasecmp(value, prefix, strlen(prefix))`: matching needs the full
/// prefix length, and a shorter value fails to match.
fn starts_ignore_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// `strcasestr(value, part)`.
fn contains_ignore_case(value: &str, part: &str) -> bool {
    let value_folded = value.to_lowercase();
    let part_folded = part.to_lowercase();
    value_folded.contains(&part_folded)
}

impl<'a> Walker<'a> {
    /// The engine's `parse_exif`. Most tags fill shooting or lens facts this
    /// pass does not carry; the port keeps the four tags that feed raw
    /// dimensions, the maker-note hand-off or the CFA word.
    fn parse_exif(&mut self, base: i64) {
        // Eastman scans take 0xa002/0xa003 as raw dimensions.
        let kodak = self.make.starts_with("EASTMAN") && self.ifd_count < 3;
        let mut entries = self.stream.get2() as usize;
        if trace() {
            eprintln!("trace:   exif at {} entries {}", self.stream.tell(), entries);
        }
        if self.make.starts_with("Hasselblad") && self.ifd_count > 3 && entries > 512 {
            return;
        }
        let fsize = self.fsize();
        while entries > 0 {
            entries -= 1;
            let parsed = self.tiff_get(base);
            if parsed.len > 8
                && self.stream.tell() as i64 + parsed.len as i64 > 2 * fsize
            {
                self.stream.seek(parsed.save);
                continue;
            }
            match parsed.tag {
                0xa002 if kodak => {
                    self.raw_width = self.stream.get4() as i32;
                }
                0xa003 if kodak => {
                    self.raw_height = self.stream.get4() as i32;
                }
                0x927c => {
                    // The RaspberryPi scan cameras keep the makernote as a
                    // text table; the port skips that arm. The Sony DSC-F828
                    // family likewise leaves at its SRF table, which carries
                    // no raw facts. Every other mark falls into the maker
                    // note, and the loop's own seek covers the tag position
                    // either way.
                    let sony_srf = self.make.starts_with("SONY")
                        && (self.model.starts_with("DSC-V3") || self.model.starts_with("DSC-F828"));
                    if !sony_srf {
                        // A one-byte Nikon record is a pointer to the maker
                        // note's own directory.
                        if parsed.len == 1 && self.make.starts_with("NIKON") {
                            let directory = self.stream.get4() as i64;
                            if directory != 0 {
                                self.stream.seek(directory as u64);
                            }
                            self.nikon_transfer = 1;
                        }
                        self.parse_makernote(base, 0);
                    }
                }
                0xa302 => {
                    // The CFA blob: four bytes, each spread over one 32-bit
                    // word of the filter value.
                    if self.stream.get4() == 0x2_0002 {
                        self.exif_cfa = 0;
                        let mut step = 0usize;
                        while step < 8 {
                            // A read past the end counts as -1, which
                            // multiplied and shifted still lands on the same
                            // filter bytes as the engine's.
                            let byte = match self.stream.getc() {
                                Some(value) => u32::from(value),
                                None => u32::MAX,
                            };
                            self.exif_cfa |= byte.wrapping_mul(0x0101_0101) << step;
                            step += 2;
                        }
                    }
                }
                _ => {}
            }
            self.stream.seek(parsed.save);
        }
    }
}

// ===== maker notes ==========================================================

/// The engine's `xlat` tables for the Nikon serial-number cipher.
const XLAT_FIRST: [u8; 256] = [
    0xc1, 0xbf, 0x6d, 0x0d, 0x59, 0xc5, 0x13, 0x9d, 0x83, 0x61, 0x6b, 0x4f, 0xc7, 0x7f, 0x3d, 0x3d,
    0x53, 0x59, 0xe3, 0xc7, 0xe9, 0x2f, 0x95, 0xa7, 0x95, 0x1f, 0xdf, 0x7f, 0x2b, 0x29, 0xc7, 0x0d,
    0xdf, 0x07, 0xef, 0x71, 0x89, 0x3d, 0x13, 0x3d, 0x3b, 0x13, 0xfb, 0x0d, 0x89, 0xc1, 0x65, 0x1f,
    0xb3, 0x0d, 0x6b, 0x29, 0xe3, 0xfb, 0xef, 0xa3, 0x6b, 0x47, 0x7f, 0x95, 0x35, 0xa7, 0x47, 0x4f,
    0xc7, 0xf1, 0x59, 0x95, 0x35, 0x11, 0x29, 0x61, 0xf1, 0x3d, 0xb3, 0x2b, 0x0d, 0x43, 0x89, 0xc1,
    0x9d, 0x9d, 0x89, 0x65, 0xf1, 0xe9, 0xdf, 0xbf, 0x3d, 0x7f, 0x53, 0x97, 0xe5, 0xe9, 0x95, 0x17,
    0x1d, 0x3d, 0x8b, 0xfb, 0xc7, 0xe3, 0x67, 0xa7, 0x07, 0xf1, 0x71, 0xa7, 0x53, 0xb5, 0x29, 0x89,
    0xe5, 0x2b, 0xa7, 0x17, 0x29, 0xe9, 0x4f, 0xc5, 0x65, 0x6d, 0x6b, 0xef, 0x0d, 0x89, 0x49, 0x2f,
    0xb3, 0x43, 0x53, 0x65, 0x1d, 0x49, 0xa3, 0x13, 0x89, 0x59, 0xef, 0x6b, 0xef, 0x65, 0x1d, 0x0b,
    0x59, 0x13, 0xe3, 0x4f, 0x9d, 0xb3, 0x29, 0x43, 0x2b, 0x07, 0x1d, 0x95, 0x59, 0x59, 0x47, 0xfb,
    0xe5, 0xe9, 0x61, 0x47, 0x2f, 0x35, 0x7f, 0x17, 0x7f, 0xef, 0x7f, 0x95, 0x95, 0x71, 0xd3, 0xa3,
    0x0b, 0x71, 0xa3, 0xad, 0x0b, 0x3b, 0xb5, 0xfb, 0xa3, 0xbf, 0x4f, 0x83, 0x1d, 0xad, 0xe9, 0x2f,
    0x71, 0x65, 0xa3, 0xe5, 0x07, 0x35, 0x3d, 0x0d, 0xb5, 0xe9, 0xe5, 0x47, 0x3b, 0x9d, 0xef, 0x35,
    0xa3, 0xbf, 0xb3, 0xdf, 0x53, 0xd3, 0x97, 0x53, 0x49, 0x71, 0x07, 0x35, 0x61, 0x71, 0x2f, 0x43,
    0x2f, 0x11, 0xdf, 0x17, 0x97, 0xfb, 0x95, 0x3b, 0x7f, 0x6b, 0xd3, 0x25, 0xbf, 0xad, 0xc7, 0xc5,
    0xc5, 0xb5, 0x8b, 0xef, 0x2f, 0xd3, 0x07, 0x6b, 0x25, 0x49, 0x95, 0x25, 0x49, 0x6d, 0x71, 0xc7,
];
const XLAT_SECOND: [u8; 256] = [
    0xa7, 0xbc, 0xc9, 0xad, 0x91, 0xdf, 0x85, 0xe5, 0xd4, 0x78, 0xd5, 0x17, 0x46, 0x7c, 0x29, 0x4c,
    0x4d, 0x03, 0xe9, 0x25, 0x68, 0x11, 0x86, 0xb3, 0xbd, 0xf7, 0x6f, 0x61, 0x22, 0xa2, 0x26, 0x34,
    0x2a, 0xbe, 0x1e, 0x46, 0x14, 0x68, 0x9d, 0x44, 0x18, 0xc2, 0x40, 0xf4, 0x7e, 0x5f, 0x1b, 0xad,
    0x0b, 0x94, 0xb6, 0x67, 0xb4, 0x0b, 0xe1, 0xea, 0x95, 0x9c, 0x66, 0xdc, 0xe7, 0x5d, 0x6c, 0x05,
    0xda, 0xd5, 0xdf, 0x7a, 0xef, 0xf6, 0xdb, 0x1f, 0x82, 0x4c, 0xc0, 0x68, 0x47, 0xa1, 0xbd, 0xee,
    0x39, 0x50, 0x56, 0x4a, 0xdd, 0xdf, 0xa5, 0xf8, 0xc6, 0xda, 0xca, 0x90, 0xca, 0x01, 0x42, 0x9d,
    0x8b, 0x0c, 0x73, 0x43, 0x75, 0x05, 0x94, 0xde, 0x24, 0xb3, 0x80, 0x34, 0xe5, 0x2c, 0xdc, 0x9b,
    0x3f, 0xca, 0x33, 0x45, 0xd0, 0xdb, 0x5f, 0xf5, 0x52, 0xc3, 0x21, 0xda, 0xe2, 0x22, 0x72, 0x6b,
    0x3e, 0xd0, 0x5b, 0xa8, 0x87, 0x8c, 0x06, 0x5d, 0x0f, 0xdd, 0x09, 0x19, 0x93, 0xd0, 0xb9, 0xfc,
    0x8b, 0x0f, 0x84, 0x60, 0x33, 0x1c, 0x9b, 0x45, 0xf1, 0xf0, 0xa3, 0x94, 0x3a, 0x12, 0x77, 0x33,
    0x4d, 0x44, 0x78, 0x28, 0x3c, 0x9e, 0xfd, 0x65, 0x57, 0x16, 0x94, 0x6b, 0xfb, 0x59, 0xd0, 0xc8,
    0x22, 0x36, 0xdb, 0xd2, 0x63, 0x98, 0x43, 0xa1, 0x04, 0x87, 0x86, 0xf7, 0xa6, 0x26, 0xbb, 0xd6,
    0x59, 0x4d, 0xbf, 0x6a, 0x2e, 0xaa, 0x2b, 0xef, 0xe6, 0x78, 0xb6, 0x4e, 0xe0, 0x2f, 0xdc, 0x7c,
    0xbe, 0x57, 0x19, 0x32, 0x7e, 0x2a, 0xd0, 0xb8, 0xba, 0x29, 0x00, 0x3c, 0x52, 0x7d, 0xa8, 0x49,
    0x3b, 0x2d, 0xeb, 0x25, 0x49, 0xfa, 0xa3, 0xaa, 0x39, 0xa7, 0xc5, 0xa7, 0x50, 0x11, 0x36, 0xfb,
    0xc6, 0x67, 0x4a, 0xf5, 0xa5, 0x12, 0x65, 0x7e, 0xb0, 0xdf, 0xaf, 0x4e, 0xb3, 0x61, 0x7f, 0x2f,
];

/// The engine's `RGGB_2_RGBG` macro: a Bayer plane re-expressed with the two
/// greens first.
fn rggb_to_rgbg(plane: u32) -> u32 {
    plane ^ (plane >> 1)
}

/// The engine's `RBGG_2_RGBG` macro.
fn rbgg_to_rgbg(plane: u32) -> u32 {
    (plane >> 1) | ((plane & 1) << 1)
}

impl<'a> Walker<'a> {
    /// `sget2` over already-read bytes, honouring the byte order in force.
    fn sget2_bytes(&self, bytes: &[u8]) -> u16 {
        if self.stream.order == BIG_ENDIAN {
            (u16::from(bytes[0]) << 8) | u16::from(bytes[1])
        } else {
            (u16::from(bytes[1]) << 8) | u16::from(bytes[0])
        }
    }

    /// The engine's `parseNikonMakernote`. The note's entries keep white
    /// balance, black levels and the nested thumbnail IFD this pass tracks;
    /// the shutter-count tag also decrypts the colour-balance block with the
    /// serial-number cipher. Tags that only feed lens facts or dead state are
    /// listed in the match's fallback.
    fn parse_nikon_makernote(&mut self, base: i64, uptag: u32) {
        let sorder = self.stream.order;
        let fsize = self.fsize();
        let mut base = base;
        let mut serial: u32 = 0;
        let mut custom_serial: u32 = 0;
        let mut color_balance_version: u32 = 0;
        let mut color_balance_ready = false;
        let mut color_balance_data = [0u8; 324];

        let mut mark = [0u8; 10];
        self.stream.read(&mut mark);
        if &mark[..5] == b"Nikon" {
            if mark[6] != 2 {
                return;
            }
            // The inline form's own directory starts right here with a fresh
            // byte-order mark.
            base = self.stream.tell() as i64;
            self.stream.order = self.stream.get2();
            if self.stream.get2() != 42 {
                self.stream.order = sorder;
                return;
            }
            let ifd_offset = self.stream.get4() as i64;
            self.stream.skip(ifd_offset - 8);
        } else {
            self.stream.skip(-10);
        }

        let entries = self.stream.get2();
        if entries > 1000 {
            // The engine leaves its clobbered byte order behind on this path.
            return;
        }
        let morder = self.stream.order;

        for _entry in 0..entries {
            self.stream.order = morder;
            let parsed = self.tiff_get(base);
            let position = self.stream.tell() as i64;
            if parsed.len > 8 && position + parsed.len as i64 > 2 * fsize {
                self.stream.seek(parsed.save);
                continue;
            }
            let tag = parsed.tag | (uptag << 16);
            if parsed.len > 100 * 1024 * 1024 {
                self.stream.seek(parsed.save);
                continue;
            }
            match tag {
                // White balance as four rationals, laid out R B G G2.
                0x000c if parsed.len == 4 && parsed.kind == TYPE_RATIONAL => {
                    self.cam_mul[0] = self.stream.getrealf(parsed.kind);
                    self.cam_mul[2] = self.stream.getrealf(parsed.kind);
                    self.cam_mul[1] = self.stream.getrealf(parsed.kind);
                    self.cam_mul[3] = self.stream.getrealf(parsed.kind);
                }
                // Flash exposure compensation: recorded state the colour
                // pipeline never reads.
                0x0012 => {}
                // The Coolpix balance block: the E-name bodies set the
                // camera multipliers, and the NRW bodies set the P-series
                // levels. The balance rows behind them fill `icWBC`, which
                // nothing in this pipeline reads.
                0x0014 if parsed.kind == TYPE_UNDEFINED => {
                    if parsed.len == 2560 {
                        // E5400, E8400, E8700, E8800: the camera pair sits
                        // past a short block, read big-endian.
                        self.stream.skip(0x4e0);
                        self.stream.order = BIG_ENDIAN;
                        self.cam_mul[0] = f32::from(self.stream.get2()) / 256.0;
                        self.cam_mul[2] = f32::from(self.stream.get2()) / 256.0;
                        self.cam_mul[1] = 1.0;
                        self.cam_mul[3] = 1.0;
                    } else if parsed.len == 1280 {
                        // E5000, E5700: no balance block, neutral levels.
                        self.cam_mul = [1.0, 1.0, 1.0, 1.0];
                    } else {
                        // The NRW-signed bodies: P6000, P7000, P7100, B700,
                        // P1000, COOLPIX A1000.
                        let mut sign = [0u8; 10];
                        self.stream.read(&mut sign);
                        sign[9] = 0;
                        if &sign[..4] == b"NRW " {
                            if &sign[4..9] == b"0100\x00" {
                                // P6000.
                                self.stream.skip(0x13de);
                                self.cam_mul[0] = self.stream.get4().wrapping_shl(1) as f32;
                                self.cam_mul[1] = self.stream.get4() as f32;
                                self.cam_mul[3] = self.stream.get4() as f32;
                                self.cam_mul[2] = self.stream.get4().wrapping_shl(1) as f32;
                            } else {
                                // P7000, P7100, B700, P1000.
                                self.stream.skip(0x16);
                                self.black = u32::from(self.stream.get2());
                                if self.cam_mul[0] < 0.1 {
                                    self.stream.skip(0x16);
                                    self.cam_mul[0] =
                                        self.stream.get4().wrapping_shl(1) as f32;
                                    self.cam_mul[1] = self.stream.get4() as f32;
                                    self.cam_mul[3] = self.stream.get4() as f32;
                                    self.cam_mul[2] =
                                        self.stream.get4().wrapping_shl(1) as f32;
                                } else {
                                    self.stream.skip(0x26);
                                }
                            }
                        }
                    }
                }
                // The thumbnail's own directory; its compression tag can set
                // raw_color, so the nested parse is kept.
                0x0011 => {
                    if self.is_raw != 0 {
                        let directory = self.stream.get4() as i64;
                        self.stream.seek((directory + base).max(0) as u64);
                        self.parse_tiff_ifd(base);
                    }
                }
                // Black levels, uncorrected for the file's bit depth;
                // finish_open divides them down.
                0x003d => {
                    for plane in 0..4u32 {
                        self.cblack[rggb_to_rgbg(plane) as usize] = u32::from(self.stream.get2());
                    }
                    let mut shared = self.cblack[3];
                    for &level in &self.cblack[..3] {
                        if shared > level {
                            shared = level;
                        }
                    }
                    for level in &mut self.cblack[..4] {
                        *level -= shared;
                    }
                    self.black += shared;
                }
                0x008c | 0x0096 => {
                    self.meta_offset = self.stream.tell() as i64;
                }
                // Serial number: digits build the value, and the first
                // non-digit ends the digits and marks a separate cipher
                // alphabet.
                0x001d => {
                    let mut remaining = parsed.len;
                    loop {
                        let byte = match self.stream.getc() {
                            Some(0) | None => break,
                            Some(value) => value,
                        };
                        if remaining == 0 {
                            break;
                        }
                        remaining -= 1;
                        if custom_serial == 0 && !byte.is_ascii_digit() {
                            let model_bytes = self.model.as_bytes();
                            let model_len = model_bytes.len();
                            let d50_named = (model_len == 3 && self.model == "D50")
                                || (model_len >= 4
                                    && !model_bytes[model_len - 4].is_ascii_alphanumeric()
                                    && &model_bytes[model_len - 3..] == b"D50");
                            custom_serial = if d50_named { 34 } else { 96 };
                            break;
                        }
                        let digit = if byte.is_ascii_digit() {
                            byte - b'0'
                        } else {
                            byte % 10
                        };
                        serial = serial.wrapping_mul(10).wrapping_add(u32::from(digit));
                    }
                }
                // Shutter count. The count itself is dead state here, but the
                // tag also decrypts the colour-balance block read above.
                0x00a7 => {
                    let mut key = 0u8;
                    for _ in 0..4 {
                        // A byte past the end reads as EOF, whose low byte is
                        // all ones.
                        key ^= self.stream.getc().unwrap_or(0xff);
                    }
                    let key_index = if custom_serial != 0 {
                        custom_serial as usize
                    } else {
                        (serial & 0xff) as usize
                    };
                    let first = XLAT_FIRST[key_index];
                    let mut second = XLAT_SECOND[usize::from(key)];
                    let mut third = 0x60u8;
                    if color_balance_version.wrapping_sub(200) < 18 && color_balance_ready {
                        for byte in color_balance_data.iter_mut() {
                            second = second.wrapping_add(first.wrapping_mul(third));
                            third = third.wrapping_add(1);
                            *byte ^= second;
                        }
                        let row = b"66666>666;6A;:;555"[(color_balance_version - 200) as usize]
                            - b'0';
                        let start = (row & !1) as usize;
                        for plane in 0..4u32 {
                            let target =
                                plane ^ (plane >> 1) ^ (u32::from(row) & 1);
                            let chunk_offset = start + plane as usize * 2;
                            self.cam_mul[target as usize] =
                                f32::from(self.sget2_bytes(&color_balance_data[chunk_offset..]));
                        }
                    }
                }
                // Colour-balance versions: the 1xx blocks write cam_mul
                // directly, the 2xx block stashes 324 bytes for the cipher.
                0x0097 => {
                    for _ in 0..4 {
                        let byte = match self.stream.getc() {
                            Some(value) => u32::from(value),
                            None => u32::MAX,
                        };
                        color_balance_version = color_balance_version
                            .wrapping_mul(10)
                            .wrapping_add(byte)
                            .wrapping_sub(48);
                    }
                    match color_balance_version {
                        100 => {
                            self.stream.skip(0x44);
                            for plane in 0..4u32 {
                                self.cam_mul[rbgg_to_rgbg(plane) as usize] = f32::from(self.stream.get2());
                            }
                        }
                        102 => {
                            self.stream.skip(6);
                            for plane in 0..4u32 {
                                self.cam_mul[rggb_to_rgbg(plane) as usize] = f32::from(self.stream.get2());
                            }
                        }
                        103 => {
                            self.stream.skip(0x10);
                            for plane in 0..4u32 {
                                self.cam_mul[plane as usize] = f32::from(self.stream.get2());
                            }
                        }
                        _ => {}
                    }
                    if color_balance_version >= 200 {
                        if color_balance_version != 205 {
                            self.stream.skip(0x118);
                        }
                        let read = self.stream.read(&mut color_balance_data);
                        color_balance_ready = read == color_balance_data.len();
                    }
                }
                // The rest of the note feeds lens, focus, flash, thumbnail and
                // in-camera-edit facts, or dead state this pass drops (the
                // ShotInfo 0x0091 and LensData 0x0098 blocks decrypt to
                // fields nothing later consumes), none of which affect the
                // tracked raw facts. Their cursor effect is the value read
                // `tiff_get` already positioned.
                _ => {}
            }
            self.stream.seek(parsed.save);
        }
        self.stream.order = sorder;
    }

    /// The engine's `parse_makernote`: the maker note header decides which
    /// directory shape follows, then the entries dispatch per maker with the
    /// generic Canon, white-balance and model-id tags behind them.
    fn parse_makernote(&mut self, base: i64, uptag: u32) {
        self.metadata_blocks += 1;
        if self.metadata_blocks > METADATA_MAXBLOCKS {
            // The engine aborts the whole identify here; the port gives up on
            // this block only, which keeps the rest of the file readable.
            return;
        }
        if self.make.starts_with("NIKON") {
            self.parse_nikon_makernote(base, uptag);
            return;
        }
        if starts_ignore_case(&self.make, "LEICA") {
            // Leica maker notes land with their maker's metadata work; early
            // out so the prologue never runs for them.
            return;
        }
        if self.make.starts_with("Nokia") {
            return;
        }

        let sorder = self.stream.order;
        let mut base = base;
        // The prologue peeks twelve bytes back from the note itself; the read
        // is net-zero, so the marker below lands on the note head. A Sony mark
        // in the rewind means the note belongs to a Sony sensor built into
        // another body, and the entry loop then runs the Sony arms.
        self.stream.skip(-12);
        let mut another_buf = [0u8; 12];
        self.stream.read(&mut another_buf);
        if &another_buf[..4] == b"SONY" || &another_buf[..4] == b"VHAB" {
            self.sony.is_sony = true;
        }
        let mut marker = [0u8; 10];
        self.stream.read(&mut marker);
        if trace() {
            eprintln!("trace:   makernote at {} marker {:?}", self.stream.tell(), &marker[..4]);
        }

        if &marker[..3] == b"KDK" || &marker[..3] == b"VER" {
            return;
        }
        if &marker[..4] == b"IIII" || &marker[..4] == b"MMMM" {
            return;
        }
        // `strcmp` against fixed names: the match needs the byte after the
        // name to read as zero as well.
        if &marker[..5] == b"EPSON" && marker[5] == 0 {
            return;
        }
        if &marker[..5] == b"SIGMA" && marker[5] == 0 {
            return;
        }

        // The parse itself may rewrite the note's byte order; every reader
        // past this point starts from the value area the caller pointed at.
        let note_base = self.stream.tell() as i64 - 10;

        if &marker[..2] == b"KC" || &marker[..3] == b"MLY" {
            // Konica and Minolta G bodies write white-balance quadruples in a
            // rolling stream below the raw data offset.
            self.stream.order = BIG_ENDIAN;
            let mut wb = [0u32; 4];
            loop {
                let position = self.stream.tell() as i64;
                if position >= self.data_offset || position >= 16384 {
                    break;
                }
                wb[0] = wb[2];
                wb[2] = wb[1];
                wb[1] = wb[3];
                if self.stream.eof() {
                    break;
                }
                wb[3] = u32::from(self.stream.get2());
                let even_spread = wb[1] == 256
                    && wb[3] == 256
                    && wb[0] > 256
                    && wb[0] < 640
                    && wb[2] > 256
                    && wb[2] < 640;
                if even_spread {
                    self.cam_mul =
                        [wb[0] as f32, wb[1] as f32, wb[2] as f32, wb[3] as f32];
                }
            }
            self.stream.order = sorder;
            return;
        }

        if &marker[..7] == b"OLYMPUS" && marker[7] == 0
            || &marker[..6] == b"OM SYS"
            || &marker[..7] == b"PENTAX " && marker[7] == 0
        {
            base = note_base;
            self.stream.skip(-2);
            if marker[1] == b'M' {
                _ = self.stream.get4();
            }
            self.stream.order = self.stream.get2() as u16;
            if marker[0] == b'O' {
                _ = self.stream.get2();
            }
        } else if &marker[..4] == b"SONY" || &marker[..9] == b"Panasonic" && marker[9] == 0 {
            // `goto nf`; a Sony note marks the parse for its Sony arms.
            if marker[0] == b'S' {
                self.sony.is_sony = true;
            }
            self.stream.order = LITTLE_ENDIAN;
            self.stream.skip(2);
        } else if &marker[..8] == b"FUJIFILM" {
            base = note_base;
            self.stream.order = LITTLE_ENDIAN;
            self.stream.skip(2);
        } else if &marker[..6] == b"OLYMP\0"
            || &marker[..5] == b"LEICA"
            || &marker[..6] == b"Ricoh\0"
        {
            self.stream.skip(-2);
        } else if &marker[..3] == b"AOC" && marker[3] == 0
            || &marker[..3] == b"QVC" && marker[3] == 0
        {
            self.stream.skip(-4);
        } else if &marker[..4] == b"CMT3" {
            self.stream.order = self.sget2_bytes(&marker[4..6]) as u16;
            self.stream.skip(2);
        } else if self.canon.cr3_ctmd {
            self.stream.order = self.sget2_bytes(&marker[..2]) as u16;
            self.stream.skip(-2);
        } else {
            self.stream.skip(-10);
            if self.make.starts_with("SAMSUNG") {
                base = note_base;
            }
        }

        // The maker's Olympus mark routes this note's entries through the
        // body-arm block below, like the engine's pre-entries check.
        if !self.oly.is_olympus
            && (starts_ignore_case(&self.make, "Olympus")
                || self.make.starts_with("OM Digi")
                || (starts_ignore_case(&self.make, "CLAUSS")
                    && starts_ignore_case(&self.model, "piX 5oo")))
        {
            self.oly.is_olympus = true;
        }

        // The entry count caps one thousand; higher counts give up without
        // the byte-order restore, which the port reproduces.
        let mut entries = self.stream.get2() as usize;
        if entries > 1000 {
            return;
        }
        // The maker's own name arms the Sony entry loop, like the engine's
        // post-prologue `is_Sony` check.
        if !self.sony.is_sony {
            let hasselblad_sony = starts_ignore_case(&self.make, "Hasselblad")
                && (starts_ignore_case(&self.model, "Stellar")
                    || starts_ignore_case(&self.model, "Lunar")
                    || starts_ignore_case(&self.model, "Lusso")
                    || starts_ignore_case(&self.model, "HV"));
            if starts_ignore_case(&self.make, "SONY")
                || starts_ignore_case(&self.make, "Konica")
                || starts_ignore_case(&self.make, "Minolta")
                || hasselblad_sony
            {
                self.sony.is_sony = true;
            }
        }
        let morder = self.stream.order;
        while entries > 0 {
            entries -= 1;
            self.stream.order = morder;
            let parsed = self.tiff_get(base);
            let tag = parsed.tag | (uptag << 16);
            let value_position = self.stream.tell() as i64;
            if parsed.len > 100 * 1024 * 1024 {
                self.stream.seek(parsed.save);
                continue;
            }
            if parsed.len > 8 && value_position + parsed.len as i64 > 2 * self.fsize() {
                self.stream.seek(parsed.save);
                continue;
            }

            if self.make.starts_with("Canon") {
                if tag == 0x000d && parsed.len < 256_000 {
                    // The camera-info blob feeds the per-body feature pass,
                    // which the port does not carry; its bytes would be
                    // consumed here, with the value position restored just
                    // below either way.
                } else if tag == 0x0010 {
                    self.canon.unique_id = self.stream.get4() as u64;
                    // `setCanonBodyFeatures` and `processCanonCameraInfo`
                    // translate the blob into lens notes, the rotation index
                    // and the colour-data layout of the body. None of them
                    // feed the metadata the oracle compares, so the port
                    // keeps the model id only.
                } else {
                    self.parse_canon_makernotes(tag, parsed.kind, parsed.len);
                }
            }

            // The maker arms for Fuji, Hasselblad, Pentax, Ricoh, Samsung
            // and Sony follow the Canon one; their parsers are still to come
            // and none of them leaves a byte behind.

            if self.sony.is_sony {
                if tag == 0xb028 && parsed.len == 1 && parsed.kind == TYPE_LONG {
                    // The DSLR-A100 private-data note sits behind a pointer.
                    let note_at = self.stream.get4();
                    if note_at != 0 {
                        self.stream.seek(note_at as u64);
                        self.parse_makernote(base, tag);
                    }
                }
                // `parseSonyMakernotes` reads the Sony body id and the feature
                // tables into per-body state this pass does not carry; the
                // model-id tag still lands in the generic arm below, as the
                // engine's second read of it shows.
            }

            if self.make.starts_with("PENTAX")
                || self.make.starts_with("RICOH")
                || self.model.starts_with("PENTAX")
            {
                // The GR and GXR bodies carry a Ricoh parser of their own
                // whose records feed lens and shooting facts only.
                if self.model.starts_with("GR") || self.model.starts_with("GXR") {
                } else {
                    self.parse_pentax_makernotes(tag, parsed.kind, parsed.len);
                }
            }

            if self.make.starts_with("SAMSUNG") {
                if self.dng_version == 0 {
                    self.parse_samsung_makernotes(tag, parsed.kind, parsed.len);
                } else {
                    // A DNG-wrapped Samsung body parses through the Pentax
                    // rules with the container records on.
                    self.parse_pentax_makernotes(tag, parsed.kind, parsed.len);
                }
            }

            self.stream.seek(value_position as u64);

            // The Hasselblad and Konica body tags sit behind the maker
            // dispatch; their facts are not carried yet, so the port reaches
            // the generic tags directly. Every branch below matches one tag,
            // which means the value position is still in force at each read.

            // The Olympus body arm: the grouped note tags (the group's own
            // id in `uptag` marks this recursion) read the tagged record's
            // own sub-entries again through the same loop, one level down;
            // everything else goes to the Olympus parser.
            if self.oly.is_olympus {
                let entry_position = self.stream.tell();
                if matches!(
                    tag,
                    0x2010 | 0x2020 | 0x2030 | 0x2031 | 0x2040 | 0x2050 | 0x3000
                ) {
                    if parsed.kind == TYPE_UNDEFINED {
                        self.parse_makernote(base, tag);
                    } else if parsed.kind == TYPE_IFD || parsed.kind == TYPE_LONG {
                        let target = base + self.stream.get4() as i64;
                        if target >= 0 {
                            // A pointer past the file start is a no-op seek
                            // in the engine, leaving the read position put.
                            self.stream.seek(target as u64);
                        }
                        self.parse_makernote(base, tag);
                    }
                } else {
                    self.parse_olympus_makernotes(base, tag, parsed.kind, parsed.len);
                }
                self.stream.seek(entry_position);
            }

            if parsed.kind == 7 && (tag == 0x008c || tag == 0x0096) {
                self.meta_offset = value_position;
            }
            if tag == 0x00a1
                && parsed.kind == 7
                && !starts_ignore_case(&self.make, "Samsung")
            {
                self.stream.order = LITTLE_ENDIAN;
                self.stream.skip(140);
                for channel in 0..3 {
                    self.cam_mul[channel] = self.stream.get4() as f32;
                }
            }
            if tag == 0xb001 && parsed.kind == 3 {
                self.canon.unique_id = u64::from(self.stream.get2());
                self.sony.body_id = self.canon.unique_id;
            }
            if (tag == 0x0f00 && parsed.kind == 7) || (tag == 0x2011 && parsed.len == 2) {
                if tag == 0x0f00 {
                    if parsed.len == 614 {
                        self.stream.skip(176);
                    } else if parsed.len == 734 || parsed.len == 1502 {
                        self.stream.skip(148);
                    } else {
                        self.stream.seek(parsed.save);
                        continue;
                    }
                }
                // The `get2_256` label: the Casio quadruple shares the tag's
                // trailing arms.
                self.stream.order = BIG_ENDIAN;
                self.cam_mul[0] = self.stream.get2() as f32 / 256.0;
                self.cam_mul[2] = self.stream.get2() as f32 / 256.0;
            }

            self.stream.seek(parsed.save);
        }
        self.stream.order = sorder;
    }
}

// ===== Olympus maker notes ==================================================

impl<'a> Walker<'a> {
    /// The engine's `getOlympus_CameraType2`. The body code's bytes pack into
    /// the id most-significant first, stopping at a trailing space.
    fn get_olympus_camera_type2(&mut self) {
        if self.oly.oly_id != 0 {
            return;
        }
        let mut chars = [0u8; 6];
        self.stream.read(&mut chars);
        let mut packed = 0u64;
        for (index, &byte) in chars.iter().enumerate() {
            if byte == 0 {
                break;
            }
            packed = packed << 8 | u64::from(byte);
            if index < 5 && isspace(chars[index + 1]) {
                chars[index + 1] = 0;
                break;
            }
        }
        if packed == oly_id::NORMA {
            packed = if self.model != "SP510UZ" { oly_id::SP_510UZ } else { 0 };
        }
        self.oly.oly_id = packed;
        // `unique_id = OlyID`, alongside the `setOlympusBodyFeatures` mount
        // notes this pass does not carry.
        self.canon.unique_id = packed;
    }

    /// The engine's `parseOlympusMakernotes`: the four group prefixes route
    /// to their own parsers, and the direct switch keeps this pass's tags.
    fn parse_olympus_makernotes(&mut self, base: i64, tag: u32, kind: u32, len: u32) {
        if (0x2010_0000..=0x2010_ffff).contains(&tag) {
            self.parse_olympus_equipment(tag & 0xffff, len);
        } else if (0x2020_0000..=0x2020_ffff).contains(&tag) {
            self.parse_olympus_camera_settings(base, tag & 0xffff, kind, len);
        } else if (0x2040_0000..=0x2040_ffff).contains(&tag) {
            self.parse_olympus_image_processing(tag & 0xffff, kind, len);
        } else if (0x3000_0000..=0x3000_ffff).contains(&tag) {
            self.parse_olympus_raw_info(tag & 0xffff, kind, len);
        } else {
            match tag {
                // SpecialMode, the lens numbers, temperatures, focus modes
                // and step counts feed shooting records this pass does not
                // carry; the value position is restored either way.
                0x0207 => self.get_olympus_camera_type2(),
                0x1011 => {
                    // The engine's `software` field keeps its trailing
                    // newline, so its `v757-71` comparison never equalizes
                    // and the matrix always reads.
                    if self.software != "v757-71" {
                        self.parse_olympus_colour_rows();
                    }
                }
                0x1012 => self.parse_olympus_channel_black(),
                0x1017 => self.cam_mul[0] = f32::from(self.stream.get2()) / 256.0,
                0x1018 => self.cam_mul[2] = f32::from(self.stream.get2()) / 256.0,
                0x102c => self.oly.valid_bits = u32::from(self.stream.get2()),
                0x20300108 | 0x20310109 => {
                    self.oly.color_space = u32::from(self.stream.get2());
                    // The C switch falls through into the focus-point read,
                    // which nothing here compares; the read still runs.
                    let _auto_focus = self.stream.get2();
                }
                0x20500209 => {
                    let _auto_focus = self.stream.get2();
                }
                _ => {}
            }
        }
    }

    /// The engine's `parseOlympus_Equipment`: this pass keeps the body code
    /// only; the lens records and serials feed lens state it does not carry.
    fn parse_olympus_equipment(&mut self, tag: u32, _len: u32) {
        if tag == 0x0100 {
            self.get_olympus_camera_type2();
        }
    }

    /// The engine's `parseOlympus_CameraSettings`: the colour-space flag is
    /// what routes the matrix records below, and the rest feeds shooting
    /// records, thumbnails and AF state the pass does not carry.
    fn parse_olympus_camera_settings(&mut self, _base: i64, tag: u32, _kind: u32, _len: u32) {
        if tag == 0x0507 {
            self.oly.color_space = u32::from(self.stream.get2());
        }
    }

    /// The engine's `parseOlympus_ImageProcessing`.
    fn parse_olympus_image_processing(&mut self, tag: u32, kind: u32, len: u32) {
        if tag == 0x0100 {
            self.cam_mul[0] = f32::from(self.stream.get2()) / 256.0;
            self.cam_mul[2] = f32::from(self.stream.get2()) / 256.0;
        } else if tag == 0x0200 {
            // The colour-space flag inside the matrix reads follows the
            // same `software` rule as the direct 0x1011 arm.
            if self.software != "v757-71" {
                self.parse_olympus_colour_rows();
            }
        } else if tag == 0x0600 {
            self.parse_olympus_channel_black();
        } else if tag == 0x0611 {
            self.oly.valid_bits = u32::from(self.stream.get2());
        } else if tag == 0x1112 {
            // `imgdata.sizes.raw_aspect`: one big-endian short maps through
            // the engine's aspect table.
            let sorder = self.stream.order;
            self.stream.order = BIG_ENDIAN;
            let record = u32::from(self.stream.get2());
            self.stream.order = sorder;
            self.oly.raw_aspect = match record {
                0x0101 | 0x0901 | 0x0909 => ASPECT_4TO3,
                0x0104 | 0x0401 => ASPECT_1TO1,
                0x0201 | 0x0202 => ASPECT_3TO2,
                0x0301 | 0x0303 => ASPECT_16TO9,
                // The engine's 0x0404 case reads the same 1-to-1 constant.
                0x0404 => ASPECT_1TO1,
                0x0505 => ASPECT_5TO4,
                // The engine spells the last three on a wider aspect scale,
                // whose 7-to-6, 6-to-5 and 7-to-5 marks share the "other"
                // bucket here.
                _ => ASPECT_OTHER,
            };
        } else if tag == 0x0805 && len == 2 {
            // The sensor calibration pair lands in the colour state's
            // linear maximum when the body is not the XZ-1 one; the port
            // carries no linear maximum, so the reads run and drop.
            let _first = self.stream.getreal(kind);
            let _second = self.stream.getreal(kind);
        } else if tag == 0x0640 {
            self.oly.tag_x640 = u32::from(self.stream.get2());
        } else if tag == 0x0641 {
            self.oly.tag_x641 = u32::from(self.stream.get2());
        } else if tag == 0x0642 {
            self.oly.tag_x642 = u32::from(self.stream.get2());
        } else if tag == 0x0643 {
            self.oly.tag_x643 = u32::from(self.stream.get2());
        } else if tag == 0x0644 {
            self.oly.tag_x644 = u32::from(self.stream.get2());
        } else if tag == 0x0645 {
            self.oly.tag_x645 = u32::from(self.stream.get2());
        } else if tag == 0x0646 {
            self.oly.tag_x646 = u32::from(self.stream.get2());
        } else if tag == 0x0647 {
            self.oly.tag_x647 = u32::from(self.stream.get2());
        } else if tag == 0x0648 {
            self.oly.tag_x648 = u32::from(self.stream.get2());
        } else if tag == 0x0649 {
            self.oly.tag_x649 = u32::from(self.stream.get2());
        } else if tag == 0x0650 {
            self.oly.tag_x650 = u32::from(self.stream.get2());
        } else if tag == 0x0651 {
            self.oly.tag_x651 = u32::from(self.stream.get2());
        } else if tag == 0x0652 {
            self.oly.tag_x652 = u32::from(self.stream.get2());
        } else if tag == 0x0653 {
            self.oly.tag_x653 = u32::from(self.stream.get2());
        }
        // The white-balance fills (0x0102-0x011f, 0x0121), the inset crop
        // corners (0x0612-0x0615), the 0x1113 aspect frame and the 0x1306
        // thermometer all feed records the oracle does not compare and this
        // pass does not keep.
    }

    /// The engine's `parseOlympus_RawInfo`.
    fn parse_olympus_raw_info(&mut self, tag: u32, _kind: u32, _len: u32) {
        if tag == 0x0200 {
            // The raw-info matrix read is not software-gated.
            self.parse_olympus_colour_rows();
        } else if tag == 0x0600 {
            self.parse_olympus_channel_black();
        }
        // The auto and preset white-balance fills (0x0110, 0x0120-0x0133)
        // and the inset crop corners feed records this pass does not keep.
    }

    /// The engine's matrix record: three rows of `(short)/256`, landing in
    /// the container matrix while the colour-space flag is clear, or into
    /// the colour-space matrix it does not compare.
    fn parse_olympus_colour_rows(&mut self) {
        for row in 0..3 {
            let values = [
                self.stream.get2() as i16 as f32 / 256.0,
                self.stream.get2() as i16 as f32 / 256.0,
                self.stream.get2() as i16 as f32 / 256.0,
            ];
            if self.oly.color_space == 0 {
                self.cmatrix[row] = [values[0], values[1], values[2], 0.0];
            }
        }
    }

    /// The engine's black-level record: four shorts land in the Bayer
    /// positions through the same colour swap.
    fn parse_olympus_channel_black(&mut self) {
        for channel in 0..4 {
            self.cblack[RGGB_2_RGBG[channel]] = u32::from(self.stream.get2());
        }
    }
}

/// The engine's `isspace` on a single byte.
fn isspace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

// ===== Pentax maker notes ===================================================

impl<'a> Walker<'a> {
    /// The engine's `parsePentaxMakernotes`: the body id, the CFA black
    /// levels, the as-shot balance and the loader's own table offset. The
    /// shooting, lens, AF and colour-matrix records stay unread; the engine
    /// calls this only from the plain walk, so every record's compare gate is
    /// the non-DNG one and holds.
    fn parse_pentax_makernotes(&mut self, tag: u32, _kind: u32, _len: u32) {
        match tag {
            0x001b => {
                self.cam_mul[2] = self.stream.get2() as f32 / 256.0;
            }
            0x001c => {
                self.cam_mul[0] = self.stream.get2() as f32 / 256.0;
            }
            0x0005 => {
                self.pentax_id = u64::from(self.stream.get4());
                // `setPentaxBodyFeatures` fills the lens-mount facts the
                // compare does not carry.
            }
            0x0200 => {
                for channel in 0..4 {
                    self.cblack[RGGB_2_RGBG[channel]] = u32::from(self.stream.get2());
                }
            }
            0x0201 => {
                // The as-shot balance arrives as raw 16-bit records like the
                // engine's own; the divide against the green pair happens in
                // the later hand-off.
                for channel in 0..4 {
                    self.cam_mul[RGGB_2_RGBG[channel]] = self.stream.get2() as f32;
                }
            }
            0x0220 => {
                // The Huffman table the pixel loader reads starts right here,
                // so the engine notes the position and reads nothing.
                self.meta_offset = self.stream.tell() as i64;
            }
            _ => {}
        }
    }

    /// The engine's `parseSamsungMakernotes`: the body's key words hide the
    /// offsets the balance, black and colour records carry. The shooting,
    /// lens and colour-space records stay unread; the Auto, tungsten and
    /// daylight white-balance records fill the engine's own per-illuminant
    /// table, which no compared field reads.
    fn parse_samsung_makernotes(&mut self, tag: u32, _kind: u32, len: u32) {
        match tag {
            0x0003 => {
                // The body id, shared with the Pentax-branded twins.
                self.pentax_id = u64::from(self.stream.get4());
            }
            0xa019 | 0xa01a => {
                // The note's aperture and 35mm-focal-length records feed the
                // engine's lens facts alone.
            }
            0xa020 => {
                for word in &mut self.samsung_key {
                    *word = self.stream.get4();
                }
            }
            0xa021 => {
                for (slot, &channel) in RGGB_2_RGBG.iter().enumerate() {
                    self.cam_mul[channel] =
                        self.stream.get4().wrapping_sub(self.samsung_key[slot]) as f32;
                }
            }
            0xa025 => {
                // The body's exposure multiplier over its own processing; no
                // compared field carries it.
            }
            0xa028 => {
                for (slot, &channel) in RGGB_2_RGBG.iter().enumerate() {
                    self.cblack[channel] =
                        self.stream.get4().wrapping_sub(self.samsung_key[slot]);
                }
            }
            0xa032 if len == 9 => {
                // The body's colour record: each word hides its key, the sum
                // truncates to a signed short over the unsigned read, and the
                // fixed Adobe RGB transform lands the values in the container
                // matrix.
                let mut camera = [[0.0f64; 3]; 3];
                for index in 0..9 {
                    let word = self.stream.get4().wrapping_add(self.samsung_key[index]);
                    camera[index / 3][index % 3] = ((word as u16) as i16) as f64 / 256.0;
                }
                self.cmatrix = adobe_rgb_coeff(&camera);
            }
            _ => {}
        }
    }
}

/// The engine's `aRGB_coeff`, from a Samsung body's record: the colour of
/// Adobe RGB space folded through the fixed transform into the container
/// matrix.
fn adobe_rgb_coeff(camera: &[[f64; 3]; 3]) -> [[f32; 4]; 3] {
    // rgb_aRGB, the engine's Adobe RGB camera transform.
    const ADOBE_RGB_WEIGHTS: [[f64; 3]; 3] = [
        [1.39828313770000, -0.3982830047, 9.64980900741708e-8],
        [6.09219200572997e-8, 0.9999999809, 1.33230799934103e-8],
        [2.17237099975343e-8, -0.0429383201, 1.04293828050000],
    ];
    let mut matrix = [[0.0f32; 4]; 3];
    for record_row in 0..3 {
        for column in 0..3 {
            let mut folded = 0.0f64;
            for channel in 0..3 {
                folded += ADOBE_RGB_WEIGHTS[record_row][channel] * camera[channel][column];
            }
            matrix[record_row][column] = folded as f32;
        }
    }
    matrix
}

// ===== Canon maker notes ====================================================

impl<'a> Walker<'a> {
    /// The engine's `parseCanonMakernotes`. Lens, shooting and AF facts stay
    /// unread; the port keeps the tags that feed raw dimensions, the CFA, the
    /// white balance, the black levels and the sensor crop.
    fn parse_canon_makernotes(&mut self, tag: u32, kind: u32, len: u32) {
        match tag {
            0x0004 => {
                // ShotInfo: the engine's comma pair `imCanon.wbi =
                // (get2(), get2())` sits behind the shot and gain fields;
                // the index is the second comma read, so the port skips
                // ahead to it. The later aperture reads are dropped, since
                // the maker dispatch restores the value position anyway.
                _ = self.stream.get2();
                _ = self.stream.get2();
                _ = self.stream.get2();
                _ = self.stream.get4();
                _ = self.stream.get2();
                _ = self.stream.get2();
                self.canon.wbi = self.stream.get2() as i32;
            }
            0x0081 => {
                // Early one-D bodies: the raw stream starts inside the tag.
                self.data_offset = self.stream.get4() as i64;
                self.stream.seek(self.data_offset as u64 + 41);
                self.raw_height = self.stream.get2() as i32 * 2;
                self.raw_width = self.stream.get2() as i32;
                self.filters = 0x6161_6161;
            }
            0x009a => {
                // AspectInfo: the sensor aspect marker and the in-camera crop.
                let marker = self.stream.get4();
                self.canon.raw_aspect = match marker {
                    0 | 12 | 13 => ASPECT_3TO2,
                    1 => ASPECT_1TO1,
                    2 | 0x102 => ASPECT_4TO3,
                    7 => ASPECT_16TO9,
                    8 => ASPECT_5TO4,
                    _ => ASPECT_OTHER,
                };
                let width = self.stream.get4() as i32;
                let height = self.stream.get4() as i32;
                let left = self.stream.get4() as i32;
                let top = self.stream.get4() as i32;
                self.canon.inset_crops = Some(CanonInset { width, height, left, top });
            }
            0x00a4 => {
                // The one-D white-balance table sits behind the ShotInfo
                // offset for the current index.
                self.stream.skip(i64::from(self.canon.wbi) * 48);
                for channel in 0..3 {
                    self.cam_mul[channel] = self.stream.get2() as f32;
                }
            }
            0x00e0 => {
                // SensorInfo: the sensor size and the two crop areas, each
                // area reading left, top, right, bottom.
                _ = self.stream.get2();
                self.canon.sensor_width = self.stream.get2() as i32;
                self.canon.sensor_height = self.stream.get2() as i32;
                self.stream.skip(4);
                self.canon.default_crop_absolute = Some(self.read_canon_area());
                self.canon.left_optical_black = Some(self.read_canon_area());
            }
            0x4001 if len > 500 => {
                self.parse_canon_color_data(len);
            }
            0x4021 => {
                // Multi-shot: when the record reads as a live multi-shot
                // pair, the white balance is a flat 1024.
                let first = self.stream.get4();
                let mut second = 0;
                if first != 0 {
                    second = self.stream.get4();
                }
                if first != 0 && second != 0 {
                    if len >= 4 {
                        _ = self.stream.get4();
                        _ = self.stream.get4();
                    }
                    self.cam_mul = [1024.0; 4];
                }
            }
            0x0007 if self.dng_version == 0 => {
                // The firmware string drives the sRAW hue flip: the loader
                // sniffs a three-part version out of `model2`. The engine's
                // fgets stops after 63 characters, at the newline, or at the
                // file's end, and C-string reads see only the first NUL (the
                // software row's 0x000b arm carries the same shape).
                let mut firmware_bytes = [0u8; 64];
                let read = self.stream.read(&mut firmware_bytes[..63]);
                let newline_end = firmware_bytes[..read]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(read, |position| position + 1);
                let firmware = &firmware_bytes[..newline_end];
                let firmware = firmware.split(|&byte| byte == 0).next().unwrap_or(firmware);
                self.model2 = String::from_utf8_lossy(firmware).into_owned();
            }
            _ => {
                // Tags 0x0001, 0x0002, 0x0008 through 0x000c, 0x0012 and
                // 0x0026 or 0x003c (AF data), 0x0029 (PowerShot G9 presets),
                // 0x0093, 0x0095, 0x00a9, 0x00b4, 0x4013, 0x4018 and 0x4026
                // feed lens, shooting and colour-space facts this pass does
                // not carry; the value position below is theirs to restore.
                let _ = kind;
            }
        }
    }

    /// `get_CanonArea`: four `get2` values, left, top, right, bottom.
    fn read_canon_area(&mut self) -> CanonArea {
        CanonArea {
            l: self.stream.get2() as i32,
            t: self.stream.get2() as i32,
            r: self.stream.get2() as i32,
            b: self.stream.get2() as i32,
        }
    }

    /// `AsShot_Auto_MeasuredWB`: the first as-shot quadruple of a colour-data
    /// record lands in `cam_mul`; the auto and measured tables that follow
    /// are not carried.
    fn canon_as_shot_wb(&mut self, save1: i64, offset: i64) {
        self.canon.color_data_sub_ver = self.stream.get2() as u32;
        self.stream.seek((save1 + (offset << 1)) as u64);
        for channel in 0..4 {
            self.cam_mul[RGGB_2_RGBG[channel]] = self.stream.get2() as f32;
        }
    }

    /// `sRAW_WB`: the sRAW colour multipliers, read in record order and
    /// rescaled by the largest entry over 1024.
    fn canon_sraw_wb(&mut self, save1: i64, offset: i64) {
        self.stream.seek((save1 + (offset << 1)) as u64);
        let mut largest = 0u16;
        for channel in 0..4 {
            let value = self.stream.get2();
            self.canon.sraw_mul[RGGB_2_RGBG[channel]] = value;
            largest = largest.max(value);
        }
        let scale = largest as f32 / 1024.0;
        for multiplier in &mut self.canon.sraw_mul {
            *multiplier = ((*multiplier as f32) * scale) as u16;
        }
    }

    /// Tag 0x4001: the per-body colour-data record. The layouts differ by
    /// record length; the port reads the cam_mul, sraw_mul, black and white
    /// entries, and the white-balance preset and colour-temperature tables
    /// the engine walks in between are not carried.
    fn parse_canon_color_data(&mut self, len: u32) {
        let save1 = self.stream.tell() as i64;
        let mut black_level_offset: i64 = 0;
        let mut black_level_offset2: i64 = 0;
        let mut white_level_offset: i64 = 0;
        let mut bls: i32 = 0;

        match len {
            582 => {
                // 20D, 350D.
                self.canon.color_data_ver = 1;
                self.stream.seek((save1 + (0x0019 << 1)) as u64);
                for channel in 0..4 {
                    self.cam_mul[RGGB_2_RGBG[channel]] = self.stream.get2() as f32;
                }
                black_level_offset = save1 + (0x00a6 << 1);
            }
            653 => {
                // 1D Mark II, 1Ds Mark II.
                self.canon.color_data_ver = 2;
                self.stream.seek((save1 + (0x0022 << 1)) as u64);
                for channel in 0..4 {
                    self.cam_mul[RGGB_2_RGBG[channel]] = self.stream.get2() as f32;
                }
                black_level_offset = save1 + (0x011e << 1);
            }
            796 => {
                // 1D Mark II N, 5D, 30D, 400D.
                self.canon.color_data_ver = 3;
                self.canon_as_shot_wb(save1, 0x003f);
                black_level_offset = save1 + (0x00c4 << 1);
            }
            674 | 692 | 702 | 1227 | 1250 | 1251 | 1337 | 1338 | 1346 => {
                // 1D Mark III through 1100D, 60D.
                self.canon.color_data_ver = 4;
                self.canon_as_shot_wb(save1, 0x003f);
                self.canon_sraw_wb(save1, 0x004e);
                if self.canon.color_data_sub_ver == 4 || self.canon.color_data_sub_ver == 5 {
                    black_level_offset = save1 + (0x02b4 << 1);
                    white_level_offset = save1 + (0x02b8 << 1);
                } else if self.canon.color_data_sub_ver == 6 || self.canon.color_data_sub_ver == 7
                {
                    black_level_offset = save1 + (0x02cb << 1);
                    white_level_offset = save1 + (0x02cf << 1);
                } else if self.canon.color_data_sub_ver == 9 {
                    black_level_offset = save1 + (0x02cf << 1);
                    white_level_offset = save1 + (0x02d3 << 1);
                } else {
                    black_level_offset = save1 + (0x00e7 << 1);
                }
            }
            5120 => {
                // PowerShot G and S bodies.
                self.canon.color_data_ver = 5;
                self.canon.color_data_sub_ver = self.stream.get2() as u32;
                self.stream.seek((save1 + (0x0047 << 1)) as u64);
                for channel in 0..4 {
                    self.cam_mul[RGGB_2_RGBG[channel]] = self.stream.get2() as f32;
                }
                if self.canon.color_data_sub_ver == 0xfffc {
                    black_level_offset = save1 + (0x014d << 1);
                    white_level_offset = save1 + (0x0569 << 1);
                } else if self.canon.color_data_sub_ver == 0xfffd {
                    black_level_offset = save1 + (0x0108 << 1);
                }
            }
            1273 | 1275 => {
                // 600D, 1200D.
                self.canon.color_data_ver = 6;
                self.canon_as_shot_wb(save1, 0x003f);
                self.canon_sraw_wb(save1, 0x0062);
                black_level_offset = save1 + (0x01df << 1);
                white_level_offset = save1 + (0x01e3 << 1);
            }
            1312 | 1313 | 1316 | 1506 => {
                // 5D Mark III, 650D through 1D X, 7D Mark II.
                self.canon.color_data_ver = 7;
                self.canon_as_shot_wb(save1, 0x003f);
                self.canon_sraw_wb(save1, 0x007b);
                if self.canon.color_data_sub_ver == 10 {
                    black_level_offset = save1 + (0x01f8 << 1);
                    white_level_offset = save1 + (0x01fc << 1);
                } else if self.canon.color_data_sub_ver == 11 {
                    black_level_offset = save1 + (0x02d8 << 1);
                    white_level_offset = save1 + (0x02dc << 1);
                }
            }
            1560 | 1592 | 1353 | 1602 => {
                // 5DS, 5D Mark IV, 80D, 1D X Mark II, 1300D through 200D.
                self.canon.color_data_ver = 8;
                self.canon_as_shot_wb(save1, 0x003f);
                self.canon_sraw_wb(save1, 0x0080);
                if self.canon.color_data_sub_ver == 14 {
                    black_level_offset = save1 + (0x022c << 1);
                    white_level_offset = save1 + (0x0230 << 1);
                } else {
                    black_level_offset = save1 + (0x030a << 1);
                    white_level_offset = save1 + (0x030e << 1);
                }
            }
            1820 | 1824 | 1816 => {
                // M50, R, RP and the 2020 bodies.
                self.canon.color_data_ver = 9;
                self.canon_as_shot_wb(save1, 0x0047);
                black_level_offset2 = save1 + ((0x0047 + 0x0102) << 1);
                black_level_offset = save1 + ((0x0047 + 0x02d1) << 1);
                white_level_offset = save1 + ((0x0047 + 0x02d5) << 1);
            }
            1770 | 2024 | 3656 => {
                // R5, 1D X Mark III, R6.
                self.canon.color_data_ver = 10;
                self.canon_as_shot_wb(save1, 0x0055);
                black_level_offset2 = save1 + ((0x0055 + 0x0102) << 1);
                black_level_offset = save1 + ((0x0055 + 0x02d1) << 1);
                white_level_offset = save1 + ((0x0055 + 0x02d5) << 1);
            }
            3973 | 3778 => {
                // R3, R6 Mark II through R50.
                self.canon.color_data_ver = 11;
                self.canon_as_shot_wb(save1, 0x0069);
                black_level_offset2 = save1 + ((0x0069 + 0x0102) << 1);
                black_level_offset = save1 + ((0x0069 + 0x0213) << 1);
                white_level_offset = save1 + ((0x0069 + 0x0217) << 1);
            }
            4528 => {
                // R1, R5 Mark II.
                self.canon.color_data_ver = 12;
                self.canon_as_shot_wb(save1, 0x0069);
                black_level_offset2 = save1 + ((0x0069 + 0x0116) << 1);
                black_level_offset = save1 + ((0x0069 + 0x0227) << 1);
                white_level_offset = save1 + ((0x0069 + 0x022b) << 1);
            }
            _ => {
                self.canon.color_data_sub_ver = self.stream.get2() as u32;
            }
        }

        // The black and white levels sit at the offsets their layout picked;
        // a second black block steps in when the first read zero across.
        if black_level_offset != 0 {
            self.stream.seek(black_level_offset.max(0) as u64);
            for channel in 0..4 {
                let value = self.stream.get2() as u32;
                self.canon.channel_black[RGGB_2_RGBG[channel]] = value;
                bls += value as i32;
            }
            self.canon.average_black = (bls / 4) as u32;
        }
        if white_level_offset != 0 {
            if white_level_offset - black_level_offset != 8 {
                self.stream.seek(white_level_offset.max(0) as u64);
            }
            self.canon.normal_white = self.stream.get2() as u32;
            self.canon.specular_white = self.stream.get2() as u32;
        }
        if self.canon.average_black == 0 && black_level_offset2 != 0 {
            self.stream.seek(black_level_offset2.max(0) as u64);
            for channel in 0..4 {
                let value = self.stream.get2() as u32;
                self.canon.channel_black[RGGB_2_RGBG[channel]] = value;
                bls += value as i32;
            }
            self.canon.average_black = (bls / 4) as u32;
        }
        self.stream.seek(save1.max(0) as u64);
    }
}

// ===== CIFF container (CRW) =================================================

/// `Canon_D30_linenums_2_StdWBi`: the EOS D30 white-balance table rows.
const CANON_D30_LINENUMS: [usize; 6] = [
    wbi::DAYLIGHT,
    wbi::CLOUDY,
    wbi::TUNGSTEN,
    wbi::FLUORESCENT,
    wbi::FLASH,
    wbi::CUSTOM,
];

/// `Canon_KeyIsZero_Len2048_linenums_2_StdWBi`: G2, S30, S40, G3, G5, S45/S50.
const CANON_KEYIS_ZERO_LEN2048: [usize; 8] = [
    wbi::CUSTOM1,
    wbi::CUSTOM2,
    wbi::DAYLIGHT,
    wbi::CLOUDY,
    wbi::TUNGSTEN,
    wbi::FLUORESCENT,
    wbi::UNKNOWN,
    wbi::FLASH,
];

/// `Canon_KeyIs0x0410_Len3072_linenums_2_StdWBi`: G6, S60, S70.
const CANON_KEYIS_0410_LEN3072: [usize; 11] = [
    wbi::CUSTOM1,
    wbi::CUSTOM2,
    wbi::DAYLIGHT,
    wbi::CLOUDY,
    wbi::TUNGSTEN,
    wbi::FLUORESCENT,
    wbi::FLUORESCENT_HIGH,
    wbi::UNKNOWN,
    wbi::UNDERWATER,
    wbi::UNKNOWN,
    wbi::FLASH,
];

/// `Canon_KeyIs0x0410_Len2048_linenums_2_StdWBi`: Pro1.
const CANON_KEYIS_0410_LEN2048: [usize; 11] = [
    wbi::CUSTOM1,
    wbi::CUSTOM2,
    wbi::DAYLIGHT,
    wbi::CLOUDY,
    wbi::TUNGSTEN,
    wbi::FLUORESCENT,
    wbi::UNKNOWN,
    wbi::FLASH,
    wbi::UNKNOWN,
    wbi::UNKNOWN,
    wbi::UNKNOWN,
];

/// C `atof`'s prefix parse: leading whitespace, an optional sign and a decimal
/// number, stopping at the first character it cannot use (`1.2.3` reads 1.2).
fn atof_prefix(text: &str) -> f64 {
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        end = 1;
    }
    let digit_start = end;
    while matches!(bytes.get(end), Some(b'0'..=b'9')) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while matches!(bytes.get(end), Some(b'0'..=b'9')) {
            end += 1;
        }
    }
    if end == digit_start {
        // C's atof still reads a lone sign, but no usable digits mean zero.
        return 0.0;
    }
    trimmed[..end].parse::<f64>().unwrap_or(0.0)
}

impl<'a> Walker<'a> {
    /// `Canon_WBpresets` without its writes: `WB_Coeffs` is not tracked here,
    /// so the port consumes the same reads only.
    fn consume_canon_wb_presets(&mut self, skip1: i64, skip2: i64) {
        for row in 0..6 {
            for _ in 0..4 {
                self.stream.get2();
            }
            let skip = match row {
                0..=3 => skip1,
                4 => skip2,
                _ => 0,
            };
            if skip != 0 {
                self.stream.skip(skip);
            }
        }
    }

    /// `parse_ciff`: the CRW record walk, one invocation per container level
    /// with the engine's own `key`, `Appendix_A` and as-shot bookkeeping.
    fn parse_ciff(&mut self, offset: i64, length: i32, depth: u32) {
        self.metadata_blocks += 1;
        if self.metadata_blocks > METADATA_MAXBLOCKS {
            // The engine throws and aborts the whole parse; the port stops
            // this walk and keeps what earlier records set.
            return;
        }
        let file_size = self.fsize();
        self.stream.seek((offset + length as i64 - 4).max(0) as u64);
        let table_offset = self.stream.get4() as i64 + offset;
        self.stream.seek(table_offset.max(0) as u64);
        let mut records_left = self.stream.get2() as i32;
        if records_left < 1 {
            return;
        }
        if records_left | depth as i32 > 127 {
            return;
        }
        if records_left as i64 * 10 + offset > file_size {
            return;
        }

        // Per-invocation state the engine keeps on the stack.
        let mut key: [u16; 2] = [0x0410, 0x45f3];
        let mut wb_table_offset: i64 = 0;
        let mut appendix_a = false;
        let mut got_as_shot_wb = false;
        let mut wbi: i32 = -1;

        while records_left > 0 {
            records_left -= 1;
            let record = self.stream.get2() as u32;
            let length = self.stream.get4() as i32;
            let see = offset + self.stream.get4() as i64;
            let save = self.stream.tell();

            // Records that carry their value in the length field are never
            // sub-tables and skip the file-size check.
            let value_in_length = matches!(
                record,
                0x2007 | 0x580b | 0x501c | 0x5029 | 0x5813 | 0x5814 | 0x5817 | 0x5834 | 0x580e
            );
            if !value_in_length {
                if see >= file_size {
                    self.stream.seek(save);
                    continue;
                }
                self.stream.seek(see.max(0) as u64);
                // High bytes 0x28 and 0x30 are sub-tables.
                if ((record >> 8).wrapping_add(8) | 8) == 0x38 {
                    let sub_table = self.stream.tell() as i64;
                    self.parse_ciff(sub_table, length, depth + 1);
                }
            }

            if record == 0x3004 {
                // The engine parses the same sub-table twice (once in the
                // common branch above, once here); both are idempotent walks.
                let sub_table = self.stream.tell() as i64;
                self.parse_ciff(sub_table, length, depth + 1);
            } else if record == 0x080a {
                // `fread(make, 64)`, back up over the padding, `fread(model,
                // 64)`: the engine's `strbuflen(make) - 63` seek moves
                // backwards whenever the name is shorter.
                let mut name_buffer = [0u8; 64];
                self.stream.read(&mut name_buffer);
                let make_len = name_buffer
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(63)
                    .min(63);
                self.make = String::from_utf8_lossy(&name_buffer[..make_len]).into_owned();
                self.stream.skip(make_len as i64 - 63);
                let mut model_buffer = [0u8; 64];
                self.stream.read(&mut model_buffer);
                let model_len = model_buffer
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(63)
                    .min(63);
                self.model = String::from_utf8_lossy(&model_buffer[..model_len]).into_owned();
            } else if record == 0x080b {
                // `stmread(len)`, then drop a leading "Firmware Version" and
                // trim spaces; only the atof prefix matters to this pass.
                let mut firmware_buffer = [0u8; 64];
                let units = (length.max(0) as usize).min(64);
                self.stream.read(&mut firmware_buffer[..units]);
                firmware_buffer[63] = 0;
                let end = firmware_buffer
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(63);
                self.firmware = String::from_utf8_lossy(&firmware_buffer[..end]).into_owned();
                if self.firmware.get(..16).is_some_and(|prefix| {
                    prefix.eq_ignore_ascii_case("Firmware Version")
                }) {
                    self.firmware = self.firmware[16..].to_string();
                }
                self.firmware = self
                    .firmware
                    .trim_matches(|character: char| character.is_ascii_whitespace())
                    .to_string();
            } else if record == 0x1810 {
                self.width = self.stream.get4() as i32;
                self.height = self.stream.get4() as i32;
                self.pixel_aspect = f32::from_bits(self.stream.get4()) as f64;
                self.flip = self.stream.get4();
            } else if record == 0x1835 {
                self.tiff_compress = self.stream.get4();
            } else if record == 0x102a {
                // ShotInfo: iso, aperture and shutter precede the white
                // balance index; the engine's comma pair hands the second read
                // to `imCanon.wbi`.
                self.stream.get2();
                self.stream.get2();
                self.stream.get2();
                self.stream.get2();
                self.stream.get2();
                self.stream.get2();
                self.stream.get2();
                let raw_wbi = self.stream.get2() as i32;
                self.canon.wbi = raw_wbi;
                wbi = raw_wbi;
                if wbi >= CANON_WBI2STD.len() as i32 {
                    wbi = 0;
                }
                self.stream.skip(32);
            } else if record == 0x102c {
                // ColourInfo2 / Appendix A.
                let info_type = self.stream.get2() as i32;
                if info_type > 512 {
                    // Pro90 IS, G1: `BG2RG1_2_RGBG`.
                    self.stream.skip(118);
                    for channel in 0..4 {
                        self.cam_mul[bg2rg1_2rgbg(channel)] = self.stream.get2() as f32;
                    }
                } else if info_type != 276 {
                    // G2, S30, S40.
                    appendix_a = true;
                    wb_table_offset = -14;
                    self.stream.skip(98);
                    for channel in 0..4 {
                        self.cam_mul[GRBG_2_RGBG[channel]] = self.stream.get2() as f32;
                    }
                    if self.cam_mul[0] > 0.001 {
                        got_as_shot_wb = true;
                    }
                }
            } else if record == 0x10a9 {
                // ColorBalance: D60, 10D, 300D.
                let count = (self.stream.get2() as i32 - 2) / 8 - 1;
                if count != 0 {
                    for channel in 0..4 {
                        self.ciff_icwbc[wbi::AUTO][RGGB_2_RGBG[channel]] = self.stream.get2();
                    }
                }
                if count >= 7 {
                    self.consume_canon_wb_presets(0, 0);
                } else {
                    for channel in 0..4 {
                        self.cam_mul[channel] = self.ciff_icwbc[wbi::AUTO][channel] as f32;
                    }
                }
                if count == 7 {
                    // Mostly EOS D60 and some 300D firmware.
                    if (0..9).contains(&wbi) && wbi != 6 {
                        for channel in 0..4 {
                            self.cam_mul[channel] =
                                self.ciff_icwbc[CANON_WBI2STD[wbi as usize]][channel] as f32;
                        }
                    } else {
                        for channel in 0..4 {
                            self.cam_mul[channel] = self.ciff_icwbc[wbi::AUTO][channel] as f32;
                        }
                    }
                } else if count == 9 {
                    // 10D, 300D.
                    for channel in 0..4 {
                        self.ciff_icwbc[wbi::CUSTOM][RGGB_2_RGBG[channel]] = self.stream.get2();
                    }
                    for channel in 0..4 {
                        self.ciff_icwbc[wbi::KELVIN][RGGB_2_RGBG[channel]] = self.stream.get2();
                    }
                    if (0..10).contains(&wbi) {
                        for channel in 0..4 {
                            self.cam_mul[channel] =
                                self.ciff_icwbc[CANON_WBI2STD[wbi as usize]][channel] as f32;
                        }
                    } else {
                        for channel in 0..4 {
                            self.cam_mul[channel] = self.ciff_icwbc[wbi::AUTO][channel] as f32;
                        }
                    }
                }
                let mut black_sum: i32 = 0;
                for channel in 0..4 {
                    let value = self.stream.get2() as u32;
                    self.canon.channel_black[RGGB_2_RGBG[channel]] = value;
                    black_sum += value as i32;
                }
                self.canon.average_black = (black_sum / 4) as u32;
            } else if record == 0x0032 {
                // ColourInfo1: the D30 table, then the keyed tables.
                if length == 768 {
                    self.stream.skip(4);
                    for table_row in CANON_D30_LINENUMS {
                        for channel in 0..4 {
                            let value = self.stream.get2() as u32;
                            self.ciff_icwbc[table_row][RGGB_2_RGBG[channel]] =
                                (1024000.0f32 / value.max(1) as f32).round() as i32 as u16;
                        }
                    }
                    self.stream.skip(68 - CANON_D30_LINENUMS.len() as i64 * 8);
                    for channel in 0..4 {
                        let value = self.stream.get2() as u32;
                        self.cam_mul[RGGB_2_RGBG[channel]] = 1024.0f32 / value.max(1) as f32;
                    }
                    if wbi == 0 {
                        self.cam_mul[0] = -1.0;
                    }
                } else if self.cam_mul[0] <= 0.001 || appendix_a {
                    // Pro1, G3, G5, G6, S45-S70 and the Appendix A bodies.
                    let info_key = self.stream.get2();
                    let mut table: &[usize] = &[];
                    let table_known;
                    let mut use_wb_from_table = true;
                    if info_key == key[0] && length == 2048 {
                        table = &CANON_KEYIS_0410_LEN2048;
                        wb_table_offset = 8;
                        table_known = true;
                    } else if info_key == key[0] && length == 3072 {
                        table = &CANON_KEYIS_0410_LEN3072;
                        wb_table_offset = 16;
                        table_known = true;
                    } else if info_key == 0 && length == 2048 {
                        key[0] = 0;
                        key[1] = 0;
                        table = &CANON_KEYIS_ZERO_LEN2048;
                        if atof_prefix(&self.firmware) < 1.02 {
                            use_wb_from_table = false;
                        }
                        table_known = true;
                    } else {
                        table_known = false;
                    }
                    if table_known {
                        // The engine reads `Canon_wbi2std[wbi]` here even when
                        // the index is still -1 (undefined behaviour); the port
                        // reads row 0 in that case.
                        let wbi_index = wbi.clamp(0, 23) as usize;
                        if CANON_WBI2STD[wbi_index] == wbi::AUTO
                            || CANON_WBI2STD[wbi_index] == wbi::UNKNOWN
                            || got_as_shot_wb
                        {
                            use_wb_from_table = false;
                        }
                        let mut as_shot_linenum = table.len();
                        if use_wb_from_table {
                            let target_wbi = if CANON_WBI2STD[wbi_index] == wbi::CUSTOM {
                                wbi::DAYLIGHT
                            } else {
                                wbi_index
                            };
                            for (linenum, table_row) in table.iter().enumerate() {
                                if CANON_WBI2STD[target_wbi] == *table_row {
                                    as_shot_linenum = linenum;
                                    break;
                                }
                            }
                        }
                        self.stream.skip(78 + wb_table_offset);
                        for (linenum, table_row) in table.iter().enumerate() {
                            if *table_row != wbi::UNKNOWN {
                                for channel in 0..4 {
                                    let value = self.stream.get2();
                                    self.ciff_icwbc[*table_row][GRBG_2_RGBG[channel]] =
                                        value ^ key[channel & 1];
                                }
                                if use_wb_from_table && as_shot_linenum == linenum {
                                    for channel in 0..4 {
                                        self.cam_mul[channel] =
                                            self.ciff_icwbc[*table_row][channel] as f32;
                                    }
                                    got_as_shot_wb = true;
                                }
                            } else {
                                self.stream.skip(8);
                            }
                        }
                        if !got_as_shot_wb {
                            self.cam_mul[0] = -1.0;
                        }
                    }
                }
            } else if record == 0x1030 && wbi >= 0 && ((0x18040 >> wbi) & 1) != 0 {
                // `ciff_block_1030` fills the sensor's white frame; the pixels
                // are re-derived later from the unpacker, so the port skips it.
            } else if record == 0x1031 {
                // `raw_width = imCanon.SensorWidth = (get2(), get2())`: the
                // comma pair keeps the second read.
                self.stream.get2();
                let sensor_width = self.stream.get2() as i32;
                self.raw_width = sensor_width;
                self.canon.sensor_width = sensor_width;
                let sensor_height = self.stream.get2() as i32;
                self.raw_height = sensor_height;
                self.canon.sensor_height = sensor_height;
                self.stream.skip(4);
                self.canon.default_crop_absolute = Some(self.read_canon_area());
                self.canon.left_optical_black = Some(self.read_canon_area());
            } else if record == 0x5834 {
                // BodySerial carries the body id in its length; the port keeps
                // `unique_id` only for the dimension-table fix-ups.
                self.canon.unique_id = (length as i32 as u32) as u64;
            }
            self.stream.seek(save);
        }
    }
}

// ===== identify tail ========================================================

/// Sony body ids from tag 0xb001, the width-trim arms' keys. The camera names
/// come from the engine's body-id list.
const SONY_ID_ILCE_7RM5: u64 = 0x186;
const SONY_ID_ILCE_7CR: u64 = 0x18c;
const SONY_ID_ILX_LR1: u64 = 0x18e;
const SONY_ID_ILCE_9M3: u64 = 0x188;
const SONY_ID_ZV_E1: u64 = 0x189;
const SONY_ID_ILCE_1: u64 = 0x180;
const SONY_ID_ILCE_7M4: u64 = 0x184;
const SONY_ID_ILCE_7CM2: u64 = 0x18d;
const SONY_ID_ILME_FX2: u64 = 0x196;
const SONY_ID_ILCE_6700: u64 = 0x18a;
const SONY_ID_ZV_E10M2: u64 = 0x18f;
const SONY_ID_ILME_FX30: u64 = 0x187;
const SONY_ID_SLT_A99: u64 = 0x126;

/// The Nikon sRAW s-curve the open stage builds over the gamma table's
/// head: 3073 beta-polynomial entries replace the tone curve's head and
/// the rest keep the gamma they were built with.
fn rebuild_nikon_sraw_curve(curve: &mut [u16]) {
    const BETA1: f64 = -5.79342238397656E-02;
    const BETA2: f64 = 3.28163551282665;
    const BETA3: f64 = -8.43136004842678;
    const BETA4: f64 = 1.03533181861023E+01;
    for i in 0..=3072i32 {
        let x = i as f64 / 3072.0;
        let shaping =
            -BETA1 * x - BETA2 * x * x - BETA3 * x * x * x - BETA4 * x * x * x * x;
        let y = 1.0 - shaping.exp();
        let y = if y < 0.0 { 0.0 } else { y };
        curve[i as usize] = (y * 16383.0f64) as u16;
    }
}

impl<'a> Walker<'a> {
    /// `adobe_coeff`: fill the levels and, through `cam_xyz_coeff`, the colour
    /// matrix from the camera table. `internal_only` is the engine's third
    /// argument: it keeps `rgb_cam` untouched.
    fn apply_adobe_coeff(&mut self, internal_only: bool) {
        // The engine looks its table up with the alias-resolved model, not
        // the one the file records (the E-name Coolpix bodies, for one).
        let adobe_model =
            super::camera_colors::get_normalized_model(&self.make, self.model.trim_end());
        let row = super::camera_colors::find_meta_color_row(
            &self.make,
            &adobe_model,
            self.black,
            &self.cblack,
            self.maximum,
        );
        if let Some(black) = row.black {
            self.black = black;
        }
        if row.black.is_some() {
            // The engine clears `cblack` inside its black-level arms.
            self.cblack.fill(0);
        }
        if let Some(maximum) = row.maximum {
            self.maximum = maximum;
        }
        if let Some(camera_xyz) = row.camera_xyz {
            self.cam_xyz = camera_xyz.map(|row| row.map(|value| value as f32));
            self.raw_color = false;
            if !internal_only {
                self.rgb_cam = cam_xyz_coeff(&camera_xyz, self.colors, &mut self.pre_mul);
            }
        }
        self.cm_found = row.matched;
    }

    /// The tail of `identify()`: maker and model clean-up, the Canon
    /// dimensions table, the sRAW and PowerShot fix-ups, the reject gates and
    /// the default fills. The engine's `fsize` is zero for every maker it
    /// knows, which is all this pass reaches, and the file-size keyed
    /// fix-ups are dead with it.
    fn finish_identify(&mut self) {
        // `simplify_make_model`: canonical maker, then the maker name off the
        // model. The FinePix prefix the Fuji bodies carry with the make name
        // spelled the same way goes next.
        let normalized_make = super::camera_colors::normalized_make(&self.make);
        if !normalized_make.is_empty() {
            self.make = normalized_make.to_string();
        }
        let make_len = self.make.len();
        if self.model.get(..make_len).is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case(&self.make)
        }) && self.model.as_bytes().get(make_len) == Some(&b' ')
        {
            self.model = self.model[make_len + 1..].to_string();
        }
        if self.make == "Fujifilm" && self.model.starts_with("FinePix") {
            self.model = self.model[7..].to_string();
            if self.model.starts_with(' ') {
                self.model = self.model[1..].to_string();
            }
        }
        self.model = self.model.trim_end().to_string();

        if self.is_raw == 0 {
            self.resolve_flip();
            return;
        }
        if self.height == 0 {
            self.height = self.raw_height;
        }
        if self.width == 0 {
            self.width = self.raw_width;
        }
        // `identify_finetune_pentax`: the body id keyed to the frame's size.
        self.finetune_pentax();

        // `makeIs(Canon) && !fsize && tiff_bps != 15`: the dimensions table.
        if self.make == "Canon" && self.tiff_bps != 15 {
            let mut from_table = false;
            if self.loader.is_none() {
                self.loader = Some(SensorKind::CanonLosslessJpeg);
            }
            for row in CANON_DIMENSIONS {
                if self.raw_width == row[0] as i32 && self.raw_height == row[1] as i32 {
                    self.left_margin = row[2] as i32;
                    self.width = self.raw_width - self.left_margin;
                    self.top_margin = row[3] as i32;
                    self.height = self.raw_height - self.top_margin;
                    self.width -= row[4] as i32;
                    self.height -= row[5] as i32;
                    self.mask[0][1] = row[6] as i32;
                    self.mask[0][3] = -(row[7] as i32);
                    self.mask[1][1] = row[8] as i32;
                    self.mask[1][3] = -(row[9] as i32);
                    if row[10] != 0 {
                        self.filters = row[10] as u32 * 0x0101_0101;
                    }
                    from_table = true;
                }
            }
            if (self.canon.unique_id | 0x20000) == 0x2720000 {
                // "PowerShot G11", "PowerShot S90".
                self.left_margin = 8;
                self.top_margin = 16;
            }
            if !from_table && self.canon.average_black != 0 {
                // Not in the table but known from the metadata.
                self.cblack[..4].copy_from_slice(&self.canon.channel_black);
                self.black = 0;
                self.cblack[4] = 0;
                self.cblack[5] = 0;
                self.mask[0][3] = 1;
                self.mask[0][1] = 2;
                if self.canon.sensor_width == self.raw_width
                    && self.canon.sensor_height == self.raw_height
                {
                    // Tag 0x1031 fills both the sensor areas and the crop, so
                    // the crop record is present whenever these match.
                    if let Some(crop) = self.canon.default_crop_absolute {
                        self.left_margin = (crop.l + 1) & 0xfffe;
                        self.width = crop.r - self.left_margin;
                        self.top_margin = (crop.t + 1) & 0xfffe;
                        self.height = crop.b - self.top_margin;
                    }
                }
            }
        }

        // `identify_finetune_by_filesize(0)`: its file-size keyed rows never
        // hit, and the "KAI-0340" check below only matches that sensor name.
        self.resolve_model();

        self.finetune_dcr_canon();
        self.finetune_dcr_nikon();
        // The engine tucks the Sony black-level arm into the same maker
        // chain, ahead of every other maker's body fix-ups.
        self.finetune_dcr_sony_black();
        self.finetune_dcr_sony();
        self.finetune_dcr_olympus();
        self.finetune_dcr_panasonic();
        self.finetune_dcr_fuji();
        self.finetune_dcr_pentax();
        self.finetune_dcr_samsung();

        // The early reject for damaged images, before the model default. The
        // engine's float-loader terms (deflate DNG, uncompressed floats) have
        // no loader the CRW/CR2/CR3 walks can set, so `tiff_bps > 16` alone
        // covers their `bps > 16 && not a float loader` rule.
        if self.loader.is_none()
            || self.height < 22
            || self.width < 22
            || self.tiff_bps > 16
            || self.tiff_samples > 4
            || self.colors > 4
            || self.colors < 1
            || self.width as i64 + self.left_margin as i64 > 65535
            || self.height as i64 + self.top_margin as i64 > 65535
        {
            self.is_raw = 0;
            self.resolve_flip();
            return;
        }

        if self.model.is_empty() {
            self.model = format!("{}x{}", self.width, self.height);
        }

        if self.filters == u32::MAX {
            self.filters = 0x94949494;
        }

        // The reject at `dng_skip`: with no float loader the `bps > 16`
        // shorthand above covers the engine's two-part rule again.
        if self.loader.is_none()
            || self.height < 22
            || self.width < 22
            || self.tiff_bps > 16
            || self.tiff_samples > 4
            || self.colors > 4
            || self.colors < 1
            || self.width as i64 + self.left_margin as i64 > 65535
            || self.height as i64 + self.top_margin as i64 > 65535
        {
            self.is_raw = 0;
            self.resolve_flip();
            return;
        }

        self.clamp_cam_mul();
        if trace() {
            eprintln!(
                "trace: pre-cm raw_color {} cm_found {} cm00 {} cam_xyz00 {} model {:?}",
                self.raw_color, self.cm_found, self.cmatrix[0][0], self.cam_xyz[0][0], self.model
            );
        }
        // The container-matrix rule with the engine's open defaults
        // (`use_camera_wb` starts 0, `use_camera_matrix` 1): for the non-DNG
        // files alone this pass reaches the rule, so the gate evaluates to
        // `1 & (0 | 2)` = 0 and the container matrix never replaces
        // rgb_cam — the camera table's own row does.
        let matrix_gate = USE_CAMERA_MATRIX_OPEN
            & (u32::from((self.use_camera_wb || self.dng_version != 0) as u8) | 0x2);
        if matrix_gate != 0 && self.cmatrix[0][0] > 0.125 {
            self.rgb_cam = self.cmatrix;
            self.raw_color = false;
        }
        if self.raw_color && !self.cm_found {
            self.apply_adobe_coeff(false);
        } else if (self.cam_xyz[0][0] as f64) < 0.01 && !self.cm_found {
            self.apply_adobe_coeff(true);
        }

        // Clear erroneous fuji_width if it did not come through the body's own
        // records or a DNG container.
        if self.fuji_width != 0 && self.dng_version == 0 && !self.parsefuji_processed {
            self.fuji_width = 0;
        }
        if self.fuji_width != 0 {
            // The SuperCCD interleave: half the scan lines per plane, counted
            // past the stored frame height.
            let fuji_width = self.width >> (self.fuji_layout == 0) as u32;
            // The engine reassigns the field here; the RAF head's 1/0 mark
            // only decides the branch.
            self.fuji_width = fuji_width as u32;
            self.filters = if fuji_width & 1 == 1 {
                0x9494_9494
            } else {
                0x4949_4949
            };
            self.width = (self.height >> self.fuji_layout as u32) + fuji_width as i32;
            self.height = self.width - 1;
            self.pixel_aspect = 1.0;
            // Prevent incorrect-sized fuji-rotated files.
            let frame_pixels = self.width as i64 * self.height as i64;
            let sensor_pixels = self.raw_width as i64 * self.raw_height as i64 * 8;
            if frame_pixels > sensor_pixels {
                self.is_raw = 0;
            }
        } else {
            if self.raw_height < self.height {
                self.raw_height = self.height;
            }
            if self.raw_width < self.width {
                self.raw_width = self.width;
            }
        }

        if self.tiff_bps == 0 {
            self.tiff_bps = 12;
        }
        if self.maximum == 0 {
            let level = 1u32 << (self.tiff_bps % 32);
            self.maximum = level.wrapping_sub(1);
            // Sony's arw2 stream consults the tone curve at the default
            // ceiling; the curve is the identity above the Sony ladder, whose
            // head only `curve_head` carries.
            if self.maximum < 0x10000 && self.loader == Some(SensorKind::SonyArw2) {
                let curve_value = if self.maximum < self.curve_head.len() as u32 {
                    u32::from(self.curve_head[self.maximum as usize])
                } else {
                    self.maximum
                };
                if curve_value > 0 {
                    self.maximum = curve_value;
                }
            }
        }
        if self.maximum > 0xffff {
            self.maximum = 0xffff;
        }

        // The early reject again, after the defaults: this one flags the file
        // instead of returning, so the flip resolution still runs.
        if self.loader.is_none()
            || self.height < 22
            || self.width < 22
            || self.tiff_bps > 16
            || self.tiff_samples > 6
            || self.colors > 4
            || self.colors == 2
        {
            self.is_raw = 0;
        }
        if self.raw_width < 22 || self.raw_width > 64000 || self.raw_height < 22
            || !(0.1..=10.0).contains(&self.pixel_aspect) || self.raw_height > 64000
        {
            self.is_raw = 0;
        }
        if self.raw_width <= self.left_margin || self.raw_height <= self.top_margin {
            self.is_raw = 0;
        }

        // `NO_JPEG` is defined in the engine build: the lossless-jpeg loaders
        // the reject names cannot be reached from here.

        if self.raw_height == 0 {
            self.raw_height = self.height;
        }
        if self.raw_width == 0 {
            self.raw_width = self.width;
        }
        if self.filters > 999 && self.colors == 3 {
            let shifted = (self.filters >> 2 & 0x2222_2222) | (self.filters << 2 & 0x8888_8888);
            self.filters |= shifted & (self.filters << 1);
        }
        self.resolve_flip();
    }

    /// `GetNormalizedModel`, in the reduced form the engine's model clean-up
    /// already covers for this pass.
    fn resolve_model(&mut self) {
        // Samsung bodies come in under their retail names; the engine
        // rewrites the model by substring, then folds the alias spellings
        // into the body's own.
        if self.make == "Samsung" {
            if self.model.contains("WB5500") || self.model.contains("HZ50W") {
                self.model = "WB5500".to_string();
            } else if self.model.contains("WB5000") || self.model.contains("HZ25W") {
                self.model = "WB5000".to_string();
            } else if self.model.contains("WB550") || self.model.contains("HZ15W") {
                self.model = "WB550".to_string();
            } else if self.model.contains("WB500") || self.model.contains("HZ10W") {
                self.model = "WB500".to_string();
            } else {
                let aliases: [(&str, &[&str]); 4] = [
                    ("EX1", &["TL500"]),
                    ("NX U", &["EK-GN100", "EK-GN110", "EK-GN120", "EK-KN120", "Galaxy NX"]),
                    ("NX mini", &["NXF1"]),
                    ("WB2000", &["TL350"]),
                ];
                for (primary, sales) in aliases {
                    if sales.contains(&self.model.as_str()) {
                        self.model = primary.to_string();
                        break;
                    }
                }
            }
        }
        // `normalized_model` trims the maker prefix again; a no-op after
        // `simplify_make_model` unless the maker row also matched.
        self.model = super::identify::normalized_model(&self.make, &self.model);
    }

    /// The cam_mul sanity clamp before the colour matrix rules.
    fn clamp_cam_mul(&mut self) {
        let colors = self.colors.min(4);
        if std::env::var("NERTHUS_RAW_TRACE").is_ok() {
            eprintln!(
                "trace: clamp cam_mul in {:?} colors {} filters {:08x}",
                self.cam_mul, self.colors, self.filters
            );
        }
        let mut cam_mul_ok = self.cam_mul[..colors].iter().all(|&value| value > 0.001);
        if cam_mul_ok {
            let mut lowest = self.cam_mul[0] as f64;
            for &value in self.cam_mul[..colors].iter() {
                lowest = lowest.min(value as f64);
            }
            let mut normalised = self.cam_mul[0] as f64 / lowest;
            let mut highest = normalised;
            for &value in self.cam_mul[..colors].iter() {
                let ratio = value as f64 / lowest;
                normalised = normalised.min(ratio);
                highest = highest.max(ratio);
            }
            if normalised <= 0.01f32 as f64 || highest > 100.0 {
                cam_mul_ok = false;
            }
        }
        if !cam_mul_ok {
            if self.cam_mul[0] > 0.0 {
                self.cam_mul[0] = 0.0;
            }
            self.cam_mul[3] = 0.0;
        }
    }

    /// `identify_finetune_dcr`, Nikon branch: the loader default, the compact
    /// E bodies' padding flags, and the per-model size fix-ups (first match
    /// only, in the engine's order).
    fn finetune_dcr_nikon(&mut self) {
        if self.make != "Nikon" {
            return;
        }
        if self.loader.is_none() {
            self.loader = Some(SensorKind::Packed);
        }
        if self.model.as_bytes().first() == Some(&b'E') {
            // E5400 ... E8800 compact bodies.
            let offset_flag = u32::from(self.data_offset == 0);
            self.load_flags |= offset_flag << 2 | 2;
        }
        let model = self.model.as_str();
        if model == "D1" {
            // The analog balance keeps the old values; white balance resets.
            self.cam_mul = [1.0, 1.0, 1.0, self.cam_mul[3]];
        } else if model == "D1X" {
            self.width -= 4;
            self.pixel_aspect = 0.5;
        } else if model == "D40X" || model == "D60" || model == "D80" || model == "D3000" {
            self.height -= 3;
            self.width -= 4;
        } else if model == "D3" || model == "D3S" || model == "D700" {
            self.width -= 4;
            self.left_margin = 2;
        } else if model == "D3100" {
            self.width -= 28;
            self.left_margin = 6;
        } else if model == "D5000" || model == "D90" {
            self.width -= 42;
        } else if model == "D5100" || model == "D7000" || model == "COOLPIX A" {
            self.width -= 44;
        } else if model == "D3200"
            || model == "D600"
            || model == "D610"
            || model.starts_with("D800")
        {
            self.width -= 46;
        } else if model == "D4" || model == "Df" {
            self.width -= 52;
            self.left_margin = 2;
        } else if model == "D500" {
            // The engine's empty arm, so the prefix rules below never trim a
            // D500's width.
        } else if model.starts_with("D40") || model.starts_with("D50") || model.starts_with("D70")
        {
            self.width -= 1;
        } else if model == "D100" {
            if self.load_flags != 0 {
                // Compressed NEFs of the D100 pad both edges.
                self.width += 3;
                self.raw_width = self.width + 3;
            }
        } else if model == "D200" {
            self.left_margin = 1;
            self.width -= 4;
            self.filters = 0x94949494;
        } else if model.starts_with("D2H") {
            self.left_margin = 6;
            self.width -= 14;
        } else if model.starts_with("D2X") {
            if self.width == 3264 {
                // In-camera high-speed crop.
                self.width -= 32;
            } else {
                self.width -= 8;
            }
        } else if model.starts_with("D300") {
            self.width -= 32;
        } else if self.raw_width == 4032 {
            // "COOLPIX P7700", "COOLPIX P7800", "COOLPIX P330", "COOLPIX P340".
            if model == "COOLPIX P7700" || model == "COOLPIX P7800" {
                self.maximum = 65504;
                self.load_flags = 0;
            } else if model == "COOLPIX P340" {
                self.load_flags = 0;
            }
        } else if model.starts_with("COOLPIX P") && self.raw_width != 4032 {
            self.load_flags = 24;
            self.filters = 0x94949494;
            // The engine's P7000/P7100 black override needs iso_speed, which
            // this pass does not track, and is unused when the metadata
            // carries the black level.
        } else if model.starts_with("COOLPIX B700") {
            self.load_flags = 24;
        } else if model.starts_with("1 ") {
            self.height -= 2;
        }
        // The engine's file-size keyed E900/E8xx/E9xx/E3xxx/E4xxx/E2500/E4500
        // rows and their pre_mul tables are not modelled; they need the
        // simple_coeff table and only hit compact E bodies.
    }

    /// `identify_finetune_dcr`, Sony levels arm: bodies wider than 3888
    /// pixels without a metadata black level default to 512 when the arw2
    /// stream unpacks them, or the bit-depth-scaled value otherwise.
    fn finetune_dcr_sony_black(&mut self) {
        if self.make != "Sony" || self.raw_width <= 3888 || self.black != 0 || self.cblack[0] != 0
        {
            return;
        }
        self.black = if self.loader == Some(SensorKind::SonyArw2) {
            512
        } else {
            128u32 << self.tiff_bps.saturating_sub(12)
        };
    }

    /// `identify_finetune_dcr`, Sony body chain: the compacts' fixed offsets,
    /// then the width trims keyed first on the raw size and finally on the
    /// body id, with the DSLR-A100 rewrite last.
    fn finetune_dcr_sony(&mut self) {
        if self.make != "Sony" {
            return;
        }
        let model = self.model.as_str();
        if model == "DSC-F828" {
            // The four-colour compact; `colors = 4` and the RGBE names ride
            // along with the fixed framing. The engine also picks its
            // `sony_load_raw` stream here, a loader this pass does not model,
            // so the reject gate will keep these bodies aside for now.
            self.width = 3288;
            self.left_margin = 5;
            self.mask[1][3] = -17;
            self.data_offset = 862144;
            self.filters = 0x9c9c_9c9c;
            self.colors = 4;
        } else if model == "DSC-V3" {
            self.width = 3109;
            self.left_margin = 59;
            self.mask[0][1] = 9;
            self.data_offset = 787392;
        } else if self.sony.body_id == SONY_ID_ILCE_7RM5
            || self.sony.body_id == SONY_ID_ILCE_7CR
            || self.sony.body_id == SONY_ID_ILX_LR1
        {
            if self.raw_width != 6304 {
                // 6304 is the APS-C uncompressed crop, framed as written.
                if self.raw_width == 6656 {
                    self.width = 6272;
                    self.height = 4180;
                } else if self.raw_width == 9728 {
                    self.width = 9566;
                    self.height = 6374;
                } else if self.raw_width == 5120 {
                    self.width = 4776;
                    self.height = 3180;
                } else if self.raw_width == 9600 {
                    self.width -= 36;
                } else {
                    // The engine flags the vendor crop it suggests; this pass
                    // carries no warnings.
                    self.width -= 32;
                }
            }
        } else if self.raw_width == 3984 {
            // DSC-R1.
            self.width = 3925;
        } else if self.raw_width == 4288 {
            self.width -= 32;
        } else if self.raw_width == 4600 {
            if model == "DSLR-A350" {
                self.height -= 4;
            }
            self.black = 0;
        } else if self.raw_width == 4928 {
            if self.height < 3280 {
                self.width -= 8;
            }
        } else if self.raw_width == 5504 {
            self.width -= if self.height > 3664 { 8 } else { 32 };
        } else if self.sony.body_id == SONY_ID_ILCE_9M3 {
            if self.raw_width == 6048 {
                self.width -= 36;
            } else if self.raw_width == 6144 {
                self.width = 6012;
                self.height = 4020;
            } else if self.raw_width == 4096 {
                self.width = 3944;
                self.height = 2644;
            } else if self.raw_width == 3072 {
                self.width = 3016;
                self.height = 2008;
            } else if self.raw_width == 3968 {
                self.width = 3948;
            } else {
                self.width -= 32;
            }
        } else if self.raw_width == 6048 {
            self.width -= 24;
            if self.model.contains("RX1") || self.model.contains("A99") {
                self.width -= 6;
            }
        } else if self.raw_width == 7392 {
            self.width -= 30;
        } else if self.raw_width == 8000 {
            self.width -= 32;
        } else if self.raw_width == 9600 {
            self.width -= 32;
        } else if self.sony.body_id == SONY_ID_ZV_E1
            && self.raw_width == 4608
            && self.raw_height == 3072
        {
            self.width = 4256;
            self.height = 2846;
        } else if self.sony.body_id == SONY_ID_ILCE_1 {
            if self.raw_width == 8704 && self.raw_height == 6144 {
                self.width = 8660;
                self.height = 5784;
            } else if self.raw_width == 8672 {
                self.width -= 12;
            } else if self.raw_width == 6144 && self.raw_height == 4096 {
                self.width = 5636;
                self.height = 3768;
            } else if self.raw_width == 5664 {
                self.width -= 28;
            } else if self.raw_width == 5632 {
                self.width -= 4;
                self.height = 3756;
            } else if self.raw_width == 4608 {
                self.width = 4332;
                self.height = 2892;
            }
        } else if self.sony.body_id == SONY_ID_ILCE_7M4
            || self.sony.body_id == SONY_ID_ILCE_7CM2
            || self.sony.body_id == SONY_ID_ILME_FX2
        {
            if self.raw_width == 7168 && self.raw_height == 5120 {
                self.width = 7028;
                self.height = 4688;
            } else if self.raw_width == 4736 {
                self.width = 4692;
            } else if self.raw_width == 5120 {
                self.width = 4624;
                self.height = 3080;
            } else if self.raw_width == 3584 {
                self.width = 3516;
                self.height = 2344;
            } else if self.raw_width == 7040 {
                self.width -= 12;
            }
        } else if self.sony.body_id == SONY_ID_ILCE_6700
            || self.sony.body_id == SONY_ID_ZV_E10M2
            || self.sony.body_id == SONY_ID_ILME_FX30
        {
            if self.raw_width == 6656 {
                self.width = 6240;
                self.height = 4168;
            } else {
                self.width = self.raw_width - 32;
            }
        } else if model == "DSLR-A100" {
            // The A100's 12-bit stream is one row narrower than the directory
            // claims, one row shorter and four pixels short on both edges.
            if self.width == 3880 {
                self.height -= 1;
                self.raw_width += 1;
                self.width = self.raw_width;
            } else {
                self.height -= 4;
                self.width -= 4;
                // `order = 0x4d4d`: the engine flips the parser state, which
                // no later step this pass reaches consumes.
                self.load_flags = 2;
            }
            self.filters = 0x6161_6161;
        }
    }

    /// `identify_finetune_dcr`, Olympus branch: the C 740UZ probe, the
    /// C 770UZ frame, the plain bodies' width and depth fix-ups and the
    /// per-body width cuts.
    fn finetune_dcr_olympus(&mut self) {
        if self.make != "Olympus" {
            return;
        }
        if self.oly.oly_id == oly_id::C_740UZ {
            // The `find_green` histogram probe picks that camera's edge
            // order; it reads the unpacked pixels, which the metadata pass
            // does not have, and no fixture exercises the body.
        } else if self.oly.oly_id == oly_id::C_770UZ {
            self.height = 1718;
            self.width = 2304;
            self.filters = 0x1616_1616;
            self.loader = Some(SensorKind::Packed);
            self.load_flags = 30;
        } else {
            self.height += self.height & 1;
            if self.exif_cfa != 0 {
                self.filters = self.exif_cfa;
            }
            // The width cuts are independent of each other, in order.
            if self.width == 4100 {
                // Olympus E-PL2, E-PL1, E-P2, E-P1, E-620, E-600, E-5, E-30.
                self.width -= 4;
            }
            if self.width == 4080 {
                // Olympus E-PM1, E-PL3, E-P3.
                self.width -= 24;
            }
            if self.width == 10400 {
                // Olympus PEN-F, E-M1-II, E-M1-III, E-M1X, OM-1.
                self.width -= 12;
            }
            if self.width == 8200 {
                // E-M1-III in 50Mp mode, E-M1X.
                self.width -= 30;
            }
            if self.width == 8180 {
                // OM-1 in 50Mp.
                self.width -= 10;
            }
            if self.width == 9280 {
                // Olympus E-M5 Mark II.
                self.width -= 6;
                self.height -= 6;
            }
            if self.loader == Some(SensorKind::Unpacked) {
                self.load_flags = 4;
                if self.oly.valid_bits == 10 {
                    self.load_flags += 2;
                }
            }
            self.tiff_bps = self.oly.valid_bits;
            if self.tiff_bps == 14 {
                self.black = self.black.wrapping_mul(4);
                for cell in self.cblack.iter_mut().take(4) {
                    *cell *= 4;
                }
                // The colour state's linear maximum cuts the same way; the
                // port carries no linear maximum.
                let count = self
                    .cblack[4]
                    .wrapping_mul(self.cblack[5])
                    .min(CBLACK_SIZE as u32 - 6);
                for cell in self.cblack.iter_mut().take(count as usize) {
                    *cell *= 4;
                }
            }
            if self.oly.oly_id == oly_id::E_300 || self.oly.oly_id == oly_id::E_500 {
                self.width -= 20;
                if self.loader == Some(SensorKind::Unpacked) {
                    self.maximum = 0xfc3;
                    self.cblack.fill(0);
                }
            } else if self.oly.oly_id == oly_id::STYLUS_1 {
                self.width -= 16;
                self.maximum = 0xfff;
            } else if self.oly.oly_id == oly_id::E_330 {
                self.width -= 30;
                if self.loader == Some(SensorKind::Unpacked) {
                    self.maximum = 0xf79;
                }
            } else if self.oly.oly_id == oly_id::SP_550UZ {
                // The flash thumbnail the SP 550UZ writes is not carried.
            } else if self.oly.oly_id == oly_id::TG_4 {
                self.width -= 16;
            } else if self.oly.oly_id == oly_id::TG_5
                || self.oly.oly_id == oly_id::TG_6
                || self.oly.oly_id == oly_id::TG_7
            {
                self.width -= 26;
            }
        }
    }

    /// `identify_finetune_pentax`: the body id keyed to the frame's size. The
    /// Samsung bodies share the two oldest rules; the K-1 arm and the *ist
    /// bodies keep their own.
    fn finetune_pentax(&mut self) {
        // A linear DNG written from a Pentax source keeps its own geometry.
        if self.dng_version != 0 && self.data_offset != 0 {
            for record in &self.ifds[..self.ifd_count] {
                if record.offset == self.data_offset {
                    if record.phint == 34892 {
                        return;
                    }
                    break;
                }
            }
        }
        if self.make != "Pentax" && self.make != "Samsung" {
            return;
        }
        // Pentax K10D, Samsung GX10.
        if self.height == 2624 && self.width == 3936 {
            self.height = 2616;
            self.width = 3896;
        }
        // Pentax K20D, Samsung GX20.
        if self.height == 3136 && self.width == 4864 {
            self.height = 3124;
            self.width = 4688;
            self.filters = 0x1616_1616;
        }
        if self.make != "Pentax" {
            return;
        }
        if self.width == 4352
            && (self.pentax_id == PENTAX_ID_K_R || self.pentax_id == PENTAX_ID_K_X)
        {
            self.width = 4309;
            self.filters = 0x1616_1616;
        }
        if self.width >= 4960
            && (self.pentax_id == PENTAX_ID_K_5
                || self.pentax_id == PENTAX_ID_K_5_II
                || self.pentax_id == PENTAX_ID_K_5_II_S)
        {
            self.left_margin = 10;
            self.width = 4950;
            self.filters = 0x1616_1616;
        }
        if self.width == 6080 && self.pentax_id == PENTAX_ID_K_70 {
            self.height = 4016;
            self.top_margin = 32;
            self.width = 6020;
            self.left_margin = 60;
        }
        if self.width == 4736 && self.pentax_id == PENTAX_ID_K_7 {
            self.height = 3122;
            self.width = 4684;
            self.filters = 0x1616_1616;
            self.top_margin = 2;
        }
        if self.width == 6080 && self.pentax_id == PENTAX_ID_KF {
            self.top_margin = 28;
            self.height = 4024;
            self.left_margin = 56;
            self.width = 6024;
        }
        if self.width == 6080 && self.pentax_id == PENTAX_ID_K_3_II {
            self.left_margin = 4;
            self.width = 6040;
        }
        if self.width == 6304 && self.pentax_id == PENTAX_ID_K_3_III {
            // From DNG ActiveArea.
            self.left_margin = 26;
            self.width = 6224;
            self.top_margin = 34;
            self.height = 4160;
        }
        if self.pentax_id == PENTAX_ID_K_3_III_MONO {
            if self.width == 6304 {
                self.left_margin = 26;
                self.width = 6224;
                self.top_margin = 34;
                self.height = 4160;
            }
            self.filters = 0;
            self.colors = 1;
        }
        if self.width == 6112 && self.pentax_id == PENTAX_ID_KP {
            // From DNG, maybe too strict.
            self.left_margin = 54;
            self.top_margin = 28;
            self.width = 6028;
            self.height = self.raw_height - self.top_margin;
        }
        if self.width == 6080 && self.pentax_id == PENTAX_ID_K_3 {
            self.left_margin = 4;
            self.width = 6040;
        }
        if self.width == 7424 && self.pentax_id == PENTAX_ID_645D {
            self.height = 5502;
            self.width = 7328;
            self.filters = 0x6161_6161;
            self.top_margin = 29;
            self.left_margin = 48;
        }
    }

    /// The Pentax share of `identify_finetune_dcr`: the full-frame bodies'
    /// active rows, the early small bodies' balance scales and layouts.
    fn finetune_dcr_pentax(&mut self) {
        if self.make != "Pentax" {
            return;
        }
        if self.pentax_id == PENTAX_ID_K_1 || self.pentax_id == PENTAX_ID_K_1_MARK_II {
            self.top_margin = 18;
            self.height = self.raw_height - self.top_margin;
            if self.raw_width == 7392 {
                self.left_margin = 6;
                self.width = 7376;
            }
        } else if self.pentax_id == PENTAX_ID_OPTIO_S_V101 {
            // fsize == 3178560.
            self.cam_mul[0] *= 4.0;
            self.cam_mul[2] *= 4.0;
        } else if self.pentax_id == PENTAX_ID_OPTIO_33WR {
            // fsize == 4775936.
            self.flip = 1;
            self.filters = 0x1616_1616;
        } else if self.pentax_id == PENTAX_ID_STARIST_D {
            self.loader = Some(SensorKind::Unpacked);
            // The engine leaves a `data_error` raise commented out here.
        } else if self.pentax_id == PENTAX_ID_STARIST_DS {
            self.height -= 2;
        }
    }

    /// `identify_finetune_dcr`, Panasonic branch: the RW2 loader choice, the
    /// twelve-row over-clip rule, the sensor table keyed to the raw size, the
    /// bit-depth fallback and the CFA-byte fold. The engine's DMC-LX100 arm,
    /// which sets a 4288-pixel width, is DNG-gated and cannot fire here.
    fn finetune_dcr_panasonic(&mut self) {
        if self.make != "Panasonic" {
            return;
        }
        // When the frame's bytes divide exactly into the packed rows the
        // body's seven-bit layout makes, they are `panasonic_load_raw` rows.
        // The 0x0118 offset tag sets the same loader earlier; this rule and
        // the unpacked fallback cover every other layout.
        let packed_row_bytes = self.raw_width as i64 * 8 / 7;
        let packed_rows = (self.fsize() - self.data_offset) / packed_row_bytes;
        if self.raw_width > 0 && packed_rows == self.raw_height as i64 {
            self.loader = Some(SensorKind::Panasonic);
        }
        if self.loader.is_none() {
            self.loader = Some(SensorKind::Unpacked);
            self.load_flags = 4;
        }
        self.zero_is_bad = true;
        // The bodies clip the frame's last block of pixels off; the record
        // claims twelve rows more than the sensor holds when none were.
        self.height += 12;
        if self.height > self.raw_height {
            self.height = self.raw_height;
        }
        for row in PANA_ROWS {
            if self.raw_width == row[0] && self.raw_height == row[1] {
                self.left_margin = row[2];
                self.top_margin = row[3];
                self.width += row[4];
                self.height += row[5];
            }
        }
        if self.tiff_bps == 0 && self.pana.bits >= 12 && self.pana.bits <= 14 {
            self.tiff_bps = self.pana.bits;
        }
        if self.model == "DC-LX100M2"
            && self.raw_height == 3568
            && self.raw_width == 4816
            && self.filters == 3
        {
            self.filters = 4;
        }
        // The CFA flag folds into the pattern byte that matches the margins'
        // parity, then the identification tail's flag pass widens it.
        let fold_index = (self.filters.wrapping_sub(1)
            ^ ((self.left_margin & 1) as u32)
            ^ ((self.top_margin << 1) as u32))
            & 3;
        self.filters = PANA_FOLD[fold_index as usize] * 0x0101_0101;
    }

    /// The Samsung share of `identify_finetune_dcr`: the NX bodies' margins
    /// and layouts, keyed off the body's own width, height and depth.
    fn finetune_dcr_samsung(&mut self) {
        if self.make != "Samsung" {
            return;
        }
        // Samsung NX100, NX10, NX11.
        if self.raw_width == 4704 {
            self.top_margin = 8;
            self.height -= self.top_margin;
            self.left_margin = 8;
            self.width -= 2 * self.left_margin;
            self.load_flags = 32;
        } else if self.model == "NX3000" {
            // Samsung NX3000; raw_width 5600.
            self.top_margin = 38;
            self.left_margin = 92;
            self.width = 5456;
            self.height = 3634;
            self.filters = 0x6161_6161;
            self.colors = 3;
        } else if self.raw_height == 3714 {
            // Samsung NX2000, NX300M, NX300, NX30, "NX U", Galaxy NX. The
            // height cut stays even where the later record check resets the
            // offsets, as the engine keeps it.
            self.top_margin = 18;
            self.height -= self.top_margin;
            self.width = 5536;
            self.left_margin = self.raw_width - self.width;
            if self.raw_width != 5600 {
                self.left_margin = 0;
                self.top_margin = 0;
            }
            self.filters = 0x6161_6161;
            self.colors = 3;
        } else if self.raw_width == 5632 {
            // Samsung NX1000, NX200, NX20, NX210.
            self.stream.order = BIG_ENDIAN;
            self.height = 3694;
            self.top_margin = 2;
            self.left_margin = 32 + self.tiff_bps as i32;
            self.width = 5574 - self.left_margin;
            if self.tiff_bps == 12 {
                self.load_flags = 80;
            }
        } else if self.raw_width == 5664 {
            // Samsung NX mini.
            self.top_margin = 17;
            self.height -= self.top_margin;
            self.left_margin = 96;
            self.width = 5544;
            self.filters = 0x4949_4949;
        } else if self.raw_width == 6496 {
            // Samsung NX1, NX500.
            self.filters = 0x6161_6161;
            if self.black == 0 && self.cblack[..4].iter().all(|&level| level == 0) {
                self.black = 1u32 << (self.tiff_bps - 7);
            }
        } else if self.model == "EX1" {
            // Samsung EX1; raw_width 3688.
            self.stream.order = BIG_ENDIAN;
            self.height -= 20;
            self.top_margin = 2;
            self.width -= 6;
            if self.width > 3682 {
                self.height -= 10;
                self.width -= 46;
                self.top_margin = 8;
            }
        } else if self.model == "WB2000" {
            // Samsung WB2000; raw_width 3728.
            self.stream.order = BIG_ENDIAN;
            self.height -= 3;
            self.top_margin = 2;
            self.width -= 10;
            if self.width > 3718 {
                self.height -= 28;
                self.width -= 56;
                self.top_margin = 8;
            }
        } else if self.model == "WB550" {
            // Samsung WB550; raw_width 4000.
            self.stream.order = BIG_ENDIAN;
        } else if self.model == "EX2F" {
            // Samsung EX2F; raw_width 4176.
            self.height = 3030;
            self.width = 4040;
            self.top_margin = 15;
            self.left_margin = 24;
            self.stream.order = BIG_ENDIAN;
            self.filters = 0x4949_4949;
            self.loader = Some(SensorKind::Unpacked);
        }
    }

    /// `identify_finetune_dcr`, Fujifilm branch: the S2Pro sizes, the Bayer
    /// ceiling, the crop table, and the raw-width switches the RAFData
    /// generation keys. The engine's FujiCropMode arms (1, 2 and 4) sit in the
    /// Adobe DNG makernote pass and cannot fire here.
    fn finetune_dcr_fuji(&mut self) {
        if self.make != "Fujifilm" {
            return;
        }
        if self.fuji.raf_data_generation == 0 && self.raw_width == 2944 {
            // S2Pro.
            self.height = 2144;
            self.width = 2880;
            self.flip = 6;
        } else if self.loader != Some(SensorKind::Packed)
            && !self.model.starts_with("X-")
            && self.filters >= 1000
        {
            // Bayer and not an X-model.
            let ceiling = if self.is_raw == 2 && self.shot_select != 0 {
                0x2f00
            } else {
                0x3e00
            };
            self.maximum = ceiling;
        }

        if self.fuji.raf_data_generation != 0 && self.fuji.raf_data_generation != 4096 {
            let table_width = self.fuji.raf_data_image_size_table[0] as i32;
            let table_height = self.fuji.raf_data_image_size_table[1] as i32;
            self.width = table_width;
            self.height = table_height;
        }
        if self.raw_height >= self.height {
            self.top_margin = (self.raw_height - self.height) >> 2 << 1;
        } else {
            self.top_margin = 0;
        }
        if self.raw_width >= self.width {
            self.left_margin = (self.raw_width - self.width) >> 2 << 1;
        } else {
            self.left_margin = 0;
        }

        if self.fuji.raf_data_generation != 0 && self.fuji.raf_data_generation != 4096 {
            match self.raw_width {
                2944 => {
                    // X-S1, X10, XF1.
                    self.filters = 0x1616_1616;
                }
                4096 | 5120 | 6048 | 6160 => {
                    // X20 and its family, or the X-Trans I bodies.
                    self.left_margin = 0;
                }
                4992 => {
                    // X-E2S, X-E2, X-T10, X-T1, X100S, X100T, X70.
                    self.left_margin = 4;
                }
                7872 => {
                    // X-H2, full image.
                    self.top_margin = 6;
                    self.left_margin = 0;
                    self.width = 7752;
                    self.height = 5178;
                }
                6336 => {
                    // X-H2S.
                    self.top_margin = 6;
                    self.left_margin = 0;
                    // The X-S20 spells its frame one column short.
                    let wide = if self.model.eq_ignore_ascii_case("X-S20") {
                        6252
                    } else {
                        6264
                    };
                    self.width = wide;
                    self.height = 4176;
                }
                6384 => {
                    // X-T3, X-T4, X100V, X-S10, X-T30, X-Pro3.
                    self.top_margin = 6;
                    self.left_margin = 0;
                    self.width = 6246;
                    self.height = 4170;
                }
                6912 | 9216 => {
                    // GFX 50S, GFX 50R.
                    self.left_margin = 0;
                    self.top_margin = 0;
                }
                8472 => {
                    // GFX 50S II.
                    self.left_margin = 0;
                    self.top_margin = 0;
                    self.width = self.raw_width - 192;
                }
                9696 | 11808 => {
                    // GFX 100; the engine's arm runs into the empty default
                    // label without a break.
                    self.left_margin = 0;
                    self.width = self.raw_width - 146;
                    self.height = self.raw_height - 8;
                    self.top_margin = 2;
                    if self.tiff_bps == 16 {
                        self.maximum = 0xffff;
                    }
                }
                _ => {}
            }
        } else if self.fuji.raf_data_generation == 0 {
            match self.raw_width {
                2304 => {
                    // S5100.
                    self.top_margin = 6;
                    self.height -= self.top_margin;
                }
                3328 => {
                    // F550EXR and its family, HS20EXR through HS50EXR.
                    self.width = self.raw_width - 66;
                    if self.width != 0 {
                        self.left_margin = 34;
                    }
                    if self.fuji.inset_crops[0].cleft == 8 {
                        // HS50EXR, F900EXR.
                        self.left_margin = 0;
                        self.width += 2;
                        self.filters = 0x1616_1616;
                    }
                }
                3664 => {
                    // "HS10 HS11".
                    self.filters = 0x1616_1616;
                }
                5504 => {
                    // DBP for GX680, aka DX-2000.
                    self.left_margin = 32;
                    self.top_margin = 8;
                    self.width = self.raw_width - 2 * self.left_margin;
                    self.height = self.raw_height - 2 * self.top_margin;
                    self.loader = Some(SensorKind::FujiDbp);
                    self.filters = 0x1616_1616;
                    self.load_flags = 0;
                    self.flip = 6;
                }
                _ => {
                    if !self.model.starts_with("S6000") && !self.model.starts_with("S6500") {
                        self.fuji_guess_inset();
                    }
                }
            }
        }
        if self.fuji_layout != 0 {
            self.raw_width = self.raw_width.wrapping_mul(self.is_raw);
        }
        if self.filters == 9 {
            for cell in 0..36usize {
                let row = (cell / 6 + self.top_margin as usize) % 6;
                let column = (cell + self.left_margin as usize) % 6;
                let byte = self.xtrans_abs[row][column];
                self.xtrans[cell / 6][cell % 6] = byte;
            }
        }
    }

    /// The crop-inset guess the shared Fuji arms fall back to.
    fn fuji_guess_inset(&mut self) {
        let inset = self.fuji.inset_crops[0];
        if inset.cwidth > 0
            && inset.cwidth as i32 <= self.raw_width
            && inset.cheight > 0
            && inset.cheight as i32 <= self.raw_height
        {
            let top = inset.ctop as i32;
            let left = inset.cleft as i32;
            let width = inset.cwidth as i32;
            let height = inset.cheight as i32;
            self.top_margin = top;
            self.left_margin = left;
            self.width = width;
            self.height = height;
        }
    }

    /// `parse_fuji_compressed_header`: the block-compressed stream's 0x4953
    /// header at the record's start, believed only after the engine's whole
    /// validation chain holds.
    fn parse_fuji_compressed_header(&mut self) {
        self.stream.seek(self.data_offset.max(0) as u64);
        let mut header = [0u8; 16];
        if self.stream.read(&mut header) != 16 {
            return;
        }
        // `sgetn` spells its words big-endian whatever the file's order.
        let signature = u32::from(u16::from_be_bytes([header[0], header[1]]));
        let lossless = u32::from(header[2]);
        let raw_type = u32::from(header[3]);
        let raw_bits = u32::from(header[4]);
        let raw_height = u32::from(u16::from_be_bytes([header[5], header[6]]));
        let rounded_width = u32::from(u16::from_be_bytes([header[7], header[8]]));
        let raw_width = u32::from(u16::from_be_bytes([header[9], header[10]]));
        let block_size = u32::from(u16::from_be_bytes([header[11], header[12]]));
        let blocks_in_row = u32::from(header[13]);
        let total_lines = u32::from(u16::from_be_bytes([header[14], header[15]]));

        let shape_checks = signature != 0x4953
            || lossless > 1
            || raw_height > 0x4002
            || raw_height < 6
            || raw_height % 6 != 0
            || block_size < 1
            || raw_width > 0x4200
            || raw_width < 0x300
            || raw_width % 24 != 0
            || rounded_width > 0x4200
            || rounded_width < block_size
            || rounded_width % block_size != 0
            || rounded_width.wrapping_sub(raw_width) >= block_size
            || block_size != 0x300
            || blocks_in_row > 0x10
            || blocks_in_row == 0
            || blocks_in_row != rounded_width / block_size;
        let record_checks = total_lines > 0xAAB
            || total_lines == 0
            || total_lines != raw_height / 6
            || (raw_bits != 12 && raw_bits != 14 && raw_bits != 16)
            || (raw_type != 16 && raw_type != 0);
        if shape_checks || record_checks {
            return;
        }
        self.fuji.total_lines = total_lines;
        self.fuji.total_blocks = blocks_in_row;
        self.fuji.block_width = block_size;
        self.fuji.compr_bits = raw_bits;
        self.fuji.compr_raw_type = raw_type;
        self.fuji.compr_lossless = lossless;
        self.raw_width = raw_width as i32;
        self.raw_height = raw_height as i32;
        self.data_offset += 16;
        self.loader = Some(SensorKind::FujiCompressed);
    }

    /// `identify_finetune_dcr`, Canon branch: sRAW geometry and the old
    /// PowerShot bodies. The engine's makernotes-rotation rule above it needs
    /// `MakernotesFlip`, which this pass does not track, and stays a
    /// documented deviation for files that carry an auto-rotate marker.
    fn finetune_dcr_canon(&mut self) {
        if self.make != "Canon" {
            return;
        }
        if self.tiff_bps == 15 {
            // Canon sRAW.
            if self.width == 3344 {
                self.width = 3272;
            } else if self.width == 3872 {
                self.width = 3866;
            }
            if self.height > self.width {
                std::mem::swap(&mut self.height, &mut self.width);
                std::mem::swap(&mut self.raw_height, &mut self.raw_width);
            }
            if self.width == 7200 && self.height == 3888 {
                // EOS 5DS (R).
                self.raw_width = 6480;
                self.width = 6480;
                self.raw_height = 4320;
                self.height = 4320;
            }
            self.filters = 0;
            self.tiff_samples = 3;
            self.colors = 3;
            self.loader = Some(SensorKind::CanonSraw);
        }

        if self.model == "PowerShot 600" {
            self.height = 613;
            self.width = 854;
            self.raw_width = 896;
            self.colors = 4;
            self.filters = 0xe1e4e1e4;
            // canon_600_load_raw: a dedicated pixel loader, added with the
            // unpacking phase; the metadata pass keeps the geometry.
        } else if self.model == "PowerShot A5" || self.model == "PowerShot A5 Zoom" {
            self.height = 773;
            self.width = 960;
            self.raw_width = 992;
            self.pixel_aspect = 256.0 / 235.0;
            self.filters = 0x1e4e1e4e;
            self.canon_a5();
        } else if self.model == "PowerShot A50" {
            self.height = 968;
            self.width = 1290;
            self.raw_width = 1320;
            self.filters = 0x1b4e4b1e;
            self.canon_a5();
        } else if self.model == "PowerShot Pro70" {
            self.height = 1024;
            self.width = 1552;
            self.filters = 0x1e4b4e1b;
            self.canon_a5();
        } else if self.model == "PowerShot Pro90 IS" || self.model == "PowerShot G1" {
            self.colors = 4;
            self.filters = 0xb4b4b4b4;
        }
    }

    /// The `canon_a5` label's shared tail.
    fn canon_a5(&mut self) {
        self.colors = 4;
        self.tiff_bps = 10;
        // packed_load_raw with 40 bytes of stride is a shared unpacker; the
        // flags stay, the loader choice stays with the pixel phase.
        self.load_flags = 40;
    }

    /// `notraw`: flip resolution from the container value.
    fn resolve_flip(&mut self) {
        if self.flip == u32::MAX {
            self.flip = self.tiff_flip;
        }
        if self.flip == u32::MAX {
            self.flip = 0;
        }
        // Convert from degrees to bit-field if needed.
        let degrees = self.flip as i32;
        if degrees > 89 || degrees < -89 {
            self.flip = match (degrees + 3600) % 360 {
                270 => 5,
                180 => 3,
                90 => 6,
                _ => degrees,
            } as u32;
        }
    }
}

// ===== CR3 container ========================================================

/// `LIBRAW_CRXTRACKS_MAXCOUNT`.
const CRX_TRACKS_MAXCOUNT: usize = 16;

/// The engine's atom-name stack holds up to 31 levels of four bytes plus a
/// terminator.
const CR3_STACK_BYTES: usize = 200;

/// The Canon maker uuid: a `moovuuid` atom with this payload is a container
/// holding the CMT metadata records.
const UUID_CANON: [u8; 16] = [
    0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48,
];

/// Atom classes: `0` unknown, `1` container, `2` leaf, `3` both. In the
/// engine's list order, so a repeated name keeps the later row.
const ATOM_NAMES: &[(&[u8; 4], u8)] = &[
    (b"dinf", 1),
    (b"edts", 1),
    (b"fiin", 1),
    (b"ipro", 1),
    (b"iprp", 1),
    (b"mdia", 1),
    (b"meco", 1),
    (b"mere", 1),
    (b"mfra", 1),
    (b"minf", 1),
    (b"moof", 1),
    (b"moov", 1),
    (b"mvex", 1),
    (b"paen", 1),
    (b"schi", 1),
    (b"sinf", 1),
    (b"skip", 1),
    (b"stbl", 1),
    (b"stsd", 1),
    (b"strk", 1),
    (b"tapt", 1),
    (b"traf", 1),
    (b"trak", 1),
    (b"cdsc", 2),
    (b"colr", 2),
    (b"dimg", 2),
    (b"free", 2),
    (b"frma", 2),
    (b"ftyp", 2),
    (b"hdlr", 2),
    (b"hvcC", 2),
    (b"iinf", 2),
    (b"iloc", 2),
    (b"infe", 2),
    (b"ipco", 2),
    (b"ipma", 2),
    (b"iref", 2),
    (b"irot", 2),
    (b"ispe", 2),
    (b"meta", 2),
    (b"mvhd", 2),
    (b"pitm", 2),
    (b"pixi", 2),
    (b"schm", 2),
    (b"thmb", 2),
    (b"tkhd", 2),
    (b"url ", 2),
    (b"urn ", 2),
    (b"CCTP", 1),
    (b"CRAW", 1),
    (b"JPEG", 2),
    (b"CDI1", 2),
    (b"CMP1", 2),
    (b"CNCV", 2),
    (b"CCDT", 2),
    (b"CTBO", 2),
    (b"CMT1", 2),
    (b"CMT2", 2),
    (b"CMT3", 2),
    (b"CMT4", 2),
    (b"CNOP", 2),
    (b"THMB", 2),
    (b"co64", 2),
    (b"mdat", 2),
    (b"mdhd", 2),
    (b"nmhd", 2),
    (b"stsc", 2),
    (b"stsz", 2),
    (b"stts", 2),
    (b"vmhd", 2),
    (b"dref", 3),
    (b"uuid", 3),
];

impl<'a> Walker<'a> {
    /// The atom-name stack as a string, up to its terminator.
    fn atom_stack(&self) -> &[u8] {
        let end = self
            .cr3
            .name_stack
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.cr3.name_stack.len());
        &self.cr3.name_stack[..end]
    }

    /// `bad_hdr` for the CMT records: a byte-order mark, the TIFF magic and
    /// the fixed 8-byte header offset.
    fn cmt_bad_header(&mut self) -> bool {
        if self.stream.order != BIG_ENDIAN && self.stream.order != LITTLE_ENDIAN {
            return true;
        }
        if self.stream.get2() != 0x2a {
            return true;
        }
        self.stream.get4() != 8
    }

    /// `parseCR3`: walk the ISO-BMFF atom tree, returning `0` on success, a
    /// positive code for an unknown atom, or a negative error.
    fn parse_cr3(&mut self, o_atom_list: i64, sz_atom_list: i64) -> i32 {
        self.cr3.nesting += 1;
        if self.cr3.nesting > 31 {
            // The engine's early return skips its own cleanup, so the nesting
            // count stays bumped — reproduce that.
            return -14;
        }
        self.cr3.media_format_ids[self.cr3.nesting as usize] = [0; 4];
        let saved_order = self.stream.order;
        let err = self.parse_cr3_walk(o_atom_list, sz_atom_list);
        self.cr3.nesting -= 1;
        if self.cr3.nesting >= 0 {
            let level = self.cr3.nesting as usize;
            if level * 4 < self.cr3.name_stack.len() {
                self.cr3.name_stack[level * 4] = 0;
            }
        }
        self.stream.order = saved_order;
        err
    }

    /// The walk of one atom list. Errors break out; the caller applies the
    /// shared cleanup (`fin`).
    fn parse_cr3_walk(&mut self, o_atom_list: i64, sz_atom_list: i64) -> i32 {
        let atom_list_end = o_atom_list + sz_atom_list;
        let mut o_atom = o_atom_list;
        while o_atom + 8 <= atom_list_end {
            let mut err: i32 = 0;
            let mut header_len: i64 = 0;
            let mut atom_type: u8 = 0;
            self.stream.order = BIG_ENDIAN;
            self.stream.seek(o_atom.max(0) as u64);
            let head_probe = if self.cr3.nesting == 0 {
                // The engine taps the first four bytes again for the
                // TIFF lookalike check at the top level.
                let mut probe = [0u8; 4];
                self.stream.read(&mut probe);
                self.stream.seek(o_atom.max(0) as u64);
                probe
            } else {
                [0; 4]
            };

            let mut size_atom = self.stream.get4() as i64;
            let mut t_l: i64 = 4;
            let mut name_atom = [0u8; 4];
            let stack_base = (self.cr3.nesting as usize) * 4;
            for (slot, byte) in name_atom.iter_mut().enumerate() {
                let value = self.stream.getc().unwrap_or(0xff);
                *byte = value;
                let index = stack_base + slot;
                if index < self.cr3.name_stack.len() {
                    self.cr3.name_stack[index] = value;
                }
            }
            let stack_end = stack_base + 4;
            if stack_end < self.cr3.name_stack.len() {
                self.cr3.name_stack[stack_end] = 0;
            }
            let stack = self.atom_stack().to_vec();
            if trace() {
                eprintln!(
                    "trace:   cr3w n{} o {} sz {} name {:?} (list end {})",
                    self.cr3.nesting, o_atom, size_atom,
                    String::from_utf8_lossy(&name_atom), atom_list_end
                );
            }

            for (name, kind) in ATOM_NAMES {
                if &name_atom == *name {
                    atom_type = *kind;
                    break;
                }
            }
            if atom_type == 0 {
                if self.cr3.nesting == 0
                    && (head_probe == *b"II*\0"
                        || head_probe == *b"MM*\0"
                        || self.cr3.track >= 0)
                {
                    // A TIFF lookalike body, or anything once a track was
                    // seen, ends the walk cleanly.
                    break;
                }
                // Unknown atoms mark the walk and keep going.
                err = 1;
            }

            // Atom content, with the extended-size form when needed. A zero
            // size is only valid at the top level, where it spans to the
            // end of the list.
            let (content_offset, content_size);
            if size_atom == 0 {
                if self.cr3.nesting != 0 {
                    return -2;
                }
                size_atom = atom_list_end - o_atom;
                content_offset = o_atom + 8;
                content_size = size_atom - 8;
            } else if size_atom == 1 {
                if o_atom + 16 > atom_list_end {
                    return -3;
                }
                t_l = 8;
                let high = self.stream.get4() as i64;
                let low = self.stream.get4() as i64;
                let extended = (high << 32) | low;
                content_offset = o_atom + 16;
                content_size = extended - 16;
            } else {
                content_offset = o_atom + 8;
                content_size = size_atom - 8;
            }

            // A top-level `uuid` record only carries XMP payloads or
            // previews here, which the metadata pass drops; the engine's
            // reads are purely cursor moves and it falls through.

            if name_atom == *b"trak" {
                self.cr3.track += 1;
                self.cr3.track_type = 0;
                if self.cr3.track as usize >= CRX_TRACKS_MAXCOUNT {
                    break;
                }
            }

            if &stack[..] == b"moovuuid" {
                let mut uuid_value = [0u8; 16];
                self.stream.read(&mut uuid_value);
                if uuid_value == UUID_CANON {
                    atom_type = 1;
                    // The engine's `lHdr` stays 16 through the recursion, so
                    // the child list starts after the UUID.
                    header_len = 16;
                } else {
                    self.stream.skip(-16);
                }
            } else if &stack[..] == b"moovuuidCCTP" {
                header_len = 12;
            } else if &stack[..] == b"moovuuidCMT1" {
                let saved_order = self.stream.order;
                self.stream.order = self.stream.get2() as u16;
                if t_l != 4 || self.cmt_bad_header() {
                    return -4;
                }
                if self.cr3.ifd0_length == 0 {
                    self.cr3.ifd0_length = content_size as u32;
                }
                self.parse_tiff_ifd(content_offset);
                self.stream.order = saved_order;
            } else if &stack[..] == b"moovuuidCMT2" {
                let saved_order = self.stream.order;
                self.stream.order = self.stream.get2() as u16;
                if t_l != 4 || self.cmt_bad_header() {
                    return -5;
                }
                if self.cr3.exif_length == 0 {
                    self.cr3.exif_length = content_size as u32;
                }
                self.parse_exif(content_offset);
                self.stream.order = saved_order;
            } else if &stack[..] == b"moovuuidCMT3" {
                let saved_order = self.stream.order;
                self.stream.order = self.stream.get2() as u16;
                if t_l != 4 || self.cmt_bad_header() {
                    return -6;
                }
                self.stream.skip(-12);
                self.parse_makernote(content_offset, 0);
                self.stream.order = saved_order;
            } else if &stack[..] == b"moovuuidCMT4" {
                // GPS records have no metadata counterpart, but their TIFF
                // headers are checked before the engine parses them.
                let saved_order = self.stream.order;
                self.stream.order = self.stream.get2() as u16;
                if t_l != 4 || self.cmt_bad_header() {
                    return -6;
                }
                self.stream.order = saved_order;
            } else if &stack[..] == b"moovtrakmdiahdlr" {
                self.stream.skip(8);
                let mut handler = [0u8; 4];
                for byte in handler.iter_mut() {
                    *byte = self.stream.getc().unwrap_or(0xff);
                }
                self.cr3.track_type = match &handler {
                    b"soun" => 1,
                    b"vide" => 2,
                    b"hint" => 3,
                    b"meta" => 4,
                    _ => 0,
                };
            } else if &stack[..] == b"moovtrakmdiaminfstblstsd" {
                if content_size >= 16 {
                    self.stream.skip(12);
                    header_len = 8;
                } else {
                    return -7;
                }
                let mut format_id = [0u8; 4];
                for byte in format_id.iter_mut() {
                    *byte = self.stream.getc().unwrap_or(0xff);
                }
                self.cr3.media_format_ids[self.cr3.nesting as usize] = format_id;
                if self.cr3.track_type == 2 && format_id == *b"CRAW" {
                    if content_size < 44 {
                        return -8;
                    }
                    self.stream.skip(24);
                } else {
                    // Only CRAW records continue as containers.
                    atom_type = 2;
                    header_len = 0;
                }
                // The sample width and height reads advance the cursor only.
                let _ = self.stream.get2();
                let _ = self.stream.get2();
            } else if &stack[..] == b"moovtrakmdiaminfstblstsdCRAW" {
                header_len = 82;
            } else if &stack[..] == b"moovtrakmdiaminfstblstsdCRAWCMP1" {
                let read_size = content_size.min(85);
                if content_size < 40 {
                    return -7;
                }
                let mut cmp1 = [0u8; 85];
                self.stream.read(&mut cmp1[..read_size as usize]);
                let track = self.cr3.track;
                if self.crx_parse_image_header(&cmp1, track, read_size) == 0 {
                    self.cr3.tracks[track as usize].media_type = 1;
                }
            } else if &stack[..] == b"moovtrakmdiaminfstblstsdCRAWCDI1" {
                // The engine stores the sensor, optical-black and active
                // areas from this record, but no code anywhere reads them —
                // dead stores, so the port skips the record entirely.
            } else if &stack[..] == b"moovtrakmdiaminfstblstsdCRAWJPEG" {
                self.cr3.tracks[self.cr3.track as usize].media_type = 2;
            } else if &stack[..] == b"moovtrakmdiaminfstblstsc" {
                if content_size >= 12 {
                    self.stream.skip(4);
                    let entries = self.stream.get4() as i64;
                    if !(1..=1_000_000).contains(&entries) {
                        return -9;
                    }
                    let mut stsc = Vec::with_capacity(entries as usize);
                    for _ in 0..entries {
                        let first = self.stream.get4() as i64;
                        let count = self.stream.get4() as i64;
                        let id = self.stream.get4() as i64;
                        stsc.push(StscEntry { first, count, id });
                    }
                    self.cr3.tracks[self.cr3.track as usize].stsc = stsc;
                }
            } else if &stack[..] == b"moovtrakmdiaminfstblstsz" {
                if content_size >= 12 {
                    self.stream.skip(4);
                    let sample_size = self.stream.get4() as i64;
                    let entries = self.stream.get4() as i64;
                    let track = self.cr3.track as usize;
                    self.cr3.tracks[track].sample_count = entries;
                    if sample_size != 0 {
                        self.cr3.tracks[track].media_size = sample_size;
                        self.cr3.tracks[track].sample_size = sample_size;
                    } else {
                        self.cr3.tracks[track].sample_size = 0;
                        if !(1..=1_000_000).contains(&entries) {
                            return -10;
                        }
                        let mut sizes = Vec::with_capacity(entries as usize);
                        for _ in 0..entries {
                            sizes.push(self.stream.get4() as i64);
                        }
                        self.cr3.tracks[track].media_size = sizes[0];
                        self.cr3.tracks[track].sample_sizes = sizes;
                    }
                }
            } else if &stack[..] == b"moovtrakmdiaminfstblco64" {
                if content_size >= 16 {
                    self.stream.skip(4);
                    let entries = self.stream.get4();
                    if entries < 1 || entries > 1_000_000 {
                        return -11;
                    }
                    let mut offsets = Vec::with_capacity(entries as usize);
                    for _ in 0..entries {
                        let high = self.stream.get4() as i64;
                        let low = self.stream.get4() as i64;
                        offsets.push((high << 32) | low);
                    }
                    let track = self.cr3.track as usize;
                    self.cr3.tracks[track].chunk_offsets = offsets;
                    self.cr3.tracks[track].chunk_count = entries as i64;
                    self.cr3.tracks[track].media_offset =
                        self.cr3.tracks[track].chunk_offsets[0];
                }
            }

            if self.cr3.track >= 0
                && (self.cr3.track as usize) < CRX_TRACKS_MAXCOUNT
                && self.cr3.tracks[self.cr3.track as usize].media_size != 0
                && self.cr3.tracks[self.cr3.track as usize].media_offset != 0
                && o_atom + size_atom >= atom_list_end
                && stack.starts_with(b"moovtrakmdiaminfstbl")
            {
                // The last table record of a metadata track with the CTMD
                // sample entry marks the track itself.
                if self.cr3.track_type == 4
                    && self.cr3.media_format_ids[self.cr3.nesting as usize] == *b"CTMD"
                {
                    self.cr3.tracks[self.cr3.track as usize].media_type = 3;
                }
            }

            if atom_type == 1 {
                if trace() {
                    eprintln!(
                        "trace:   cr3r stack {:?} hl {} from {} track_type {}",
                        String::from_utf8_lossy(&stack), header_len, content_offset, self.cr3.track_type
                    );
                }
                err = self.parse_cr3(content_offset + header_len, content_size - header_len);
                if err != 0 {
                    return err;
                }
                err = 0;
            }
            o_atom += size_atom;
        }
        0
    }

    /// `crxParseImageHeader`: the CRX image header at the start of the
    /// compressed frame record. The 85-byte buffer is always big enough for
    /// every field read; only `size`, the atom's usable length, gates them.
    fn crx_parse_image_header(&mut self, cmp1: &[u8], track: i32, size: i64) -> i32 {
        if track < 0 || track as usize >= CRX_TRACKS_MAXCOUNT || size < 32 {
            return -1;
        }
        let record = &mut self.cr3.tracks[track as usize];
        record.version = sgetn(&cmp1[4..6]) as i32;
        record.f_width = sgetn(&cmp1[8..12]) as i32;
        record.f_height = sgetn(&cmp1[12..16]) as i32;
        record.tile_width = sgetn(&cmp1[16..20]) as i32;
        record.tile_height = sgetn(&cmp1[20..24]) as i32;
        record.n_bits = cmp1[24] as u32;
        record.n_planes = (cmp1[25] >> 4) as u32;
        record.cfa_layout = (cmp1[25] & 0xf) as u32;
        record.enc_type = (cmp1[26] >> 4) as u32;
        record.image_levels = (cmp1[26] & 0xf) as u32;
        record.has_tile_cols = (cmp1[27] >> 7) as u32;
        record.has_tile_rows = ((cmp1[27] >> 6) & 1) as u32;
        record.mdat_header_size = sgetn(&cmp1[28..32]) as u32;
        let ext_header = cmp1[32] >> 7;
        let mut use_median_bits = 0u32;
        record.median_bits = record.n_bits;
        if ext_header != 0 && size >= 56 && record.n_planes == 4 {
            use_median_bits = ((cmp1[56] >> 6) & 1) as u32;
        }
        if use_median_bits != 0 && size >= 84 {
            record.median_bits = cmp1[84] as u32;
        }

        if record.version != 0x100 && record.version != 0x200 {
            return -1;
        }
        if record.mdat_header_size == 0 {
            return -1;
        }
        if record.enc_type == 1 {
            if record.n_bits > 15 {
                return -1;
            }
        } else {
            if record.enc_type != 0 && record.enc_type != 3 {
                return -1;
            }
            if record.n_bits > 14 {
                return -1;
            }
        }
        if record.n_planes == 1 {
            if record.cfa_layout != 0 || record.enc_type != 0 || record.n_bits != 8 {
                return -1;
            }
        } else if record.n_planes != 4
            || record.f_width & 1 != 0
            || record.f_height & 1 != 0
            || record.tile_width & 1 != 0
            || record.tile_height & 1 != 0
            || record.cfa_layout > 3
            || record.n_bits == 8
        {
            return -1;
        }
        if record.tile_width > record.f_width || record.tile_height > record.f_height {
            return -1;
        }
        if record.image_levels > 3 || record.has_tile_cols > 1 || record.has_tile_rows > 1 {
            return -1;
        }
        0
    }

    /// `selectCRXFrame`: point the chosen track's media at one sample.
    fn select_crx_frame(&mut self, track_num: usize, frame_index: i64) -> i32 {
        let mut stsc_index: usize = 0;
        let mut current_sample: i64 = 0;
        let chunk_offset_count = self.cr3.tracks[track_num].chunk_count as usize;
        if frame_index >= self.cr3.tracks[track_num].sample_count {
            return -1;
        }
        for chunk in 0..chunk_offset_count {
            let mut current_offset = self.cr3.tracks[track_num].chunk_offsets[chunk];
            // The engine reads one entry past the end, where its buffer
            // holds a zero, which stops the advance.
            while stsc_index < self.cr3.tracks[track_num].stsc.len()
                && (chunk as i64 + 1)
                    == self
                        .cr3
                        .tracks[track_num]
                        .stsc
                        .get(stsc_index + 1)
                        .map_or(0, |entry| entry.first)
            {
                stsc_index += 1;
            }
            // The engine reads one entry past its calloc'd table, where a
            // zero stands in, so the count falls back to zero there too.
            let samples_per_chunk = self.cr3.tracks[track_num]
                .stsc
                .get(stsc_index)
                .map_or(0, |entry| entry.count);
            for _ in 0..samples_per_chunk {
                if current_sample > self.cr3.tracks[track_num].sample_count {
                    return -1;
                }
                let sample_size = if self.cr3.tracks[track_num].sample_size > 0 {
                    self.cr3.tracks[track_num].sample_size
                } else {
                    self.cr3
                        .tracks[track_num]
                        .sample_sizes
                        .get(current_sample as usize)
                        .copied()
                        .unwrap_or(0)
                };
                if current_sample == frame_index {
                    let track = &mut self.cr3.tracks[track_num];
                    track.media_offset = current_offset;
                    track.media_size = sample_size;
                    return 0;
                }
                current_offset += sample_size;
                current_sample += 1;
            }
        }
        -1
    }

    /// `selectCRXTrack`: pick the widest raw track, its frame, then parse
    /// the side metadata tracks. The `tiff_idx` flip pick that compares
    /// heights against top pixels is an engine quirk kept as written.
    fn select_crx_track(&mut self) {
        let max_track = self.cr3.track;
        if max_track < 0 {
            return;
        }
        let last_track = (max_track as usize).min(CRX_TRACKS_MAXCOUNT - 1);
        let mut bitcounts = vec![0i64; CRX_TRACKS_MAXCOUNT];
        let mut frame_counts = vec![0i32; CRX_TRACKS_MAXCOUNT];
        let mut max_bitcount: i64 = 0;

        for track in 0..=last_track {
            let record = &self.cr3.tracks[track];
            if record.media_type == 1 {
                let bits = record.n_bits as i64 * record.f_width as i64 * record.f_height as i64;
                bitcounts[track] = bits;
                max_bitcount = max_bitcount.max(bits);
                if record.sample_count > 1 {
                    frame_counts[track] = record.sample_count as i32;
                }
            }
        }
        if max_bitcount < 8 {
            return;
        }
        let mut media_tracks = 0i32;
        let mut frame_count = 0i32;
        for track in 0..=last_track {
            if bitcounts[track] == max_bitcount {
                media_tracks += 1;
                if frame_counts[track] > 1 {
                    frame_count = frame_count.max(frame_counts[track]);
                }
            }
        }
        if frame_count != 0 && media_tracks > 1 {
            // A file with several media tracks and several frames is
            // unsupported there.
            return;
        }
        // One media track: the shot picks a frame; several: it picks a track.
        let (mut frame_select, track_select) = if frame_count != 0 {
            (self.shot_select, 0i32)
        } else {
            (0i32, self.shot_select)
        };
        let mut chosen: i32 = -1;
        let mut track_count = 0;
        for track in 0..=last_track {
            if bitcounts[track] == max_bitcount {
                if track_count <= track_select {
                    chosen = track as i32;
                }
                track_count += 1;
            }
        }
        if chosen < 0 {
            return;
        }
        let chosen = chosen as usize;
        frame_count = frame_counts[chosen];
        frame_select = frame_select.clamp(0, frame_count);
        if frame_select > 0 && self.select_crx_frame(chosen, frame_select as i64) != 0 {
            return;
        }

        let mut ctmd_count = 0i32;
        for track in 0..=last_track {
            let record = &mut self.cr3.tracks[track];
            let sample_count = record.sample_count as i32;
            let fsel = frame_select.clamp(0, sample_count);
            if record.media_type == 3 {
                if fsel != 0 {
                    self.select_crx_frame(track, fsel as i64);
                }
                self.parse_cr3_ctmd(track);
                ctmd_count += 1;
            } else if record.media_type == 2 && fsel != 0 {
                // JPEG tracks only pick thumbnails, which the metadata pass
                // does not carry.
                self.select_crx_frame(track, fsel as i64);
            }
        }

        self.is_raw = if frame_count != 0 {
            frame_count
        } else {
            media_tracks
        };

        let record = self.cr3.tracks[chosen].clone();
        self.data_offset = record.media_offset;
        self.data_size = record.media_size;
        self.raw_width = record.f_width;
        self.raw_height = record.f_height;
        self.loader = Some(SensorKind::CanonCrx);
        self.tiff_bps = if record.enc_type == 3 {
            record.median_bits
        } else {
            record.n_bits
        };
        match record.cfa_layout {
            0 => self.filters = 0x94949494,
            1 => self.filters = 0x61616161,
            2 => self.filters = 0x49494949,
            3 => self.filters = 0x16161616,
            _ => {}
        }
        // The CRX loader later reads the chosen track's record from here.
        self.cr3.chosen_track = chosen;

        // The IFD flip pick scans for the largest height, but compares the
        // height against pixels squared, as the engine does.
        let mut tiff_index: i32 = -1;
        let mut top_pixels: i64 = 0;
        for index in 0..self.ifd_count {
            let height = self.ifds[index].height;
            let pixels = height as i64 * height as i64;
            if pixels > top_pixels {
                top_pixels = pixels;
                tiff_index = index as i32;
            }
        }
        if tiff_index >= 0 {
            self.flip = self.ifds[tiff_index as usize].flip as u32;
        }
        // The multishot cam_mul reset needs the multishot markers, which this
        // pass does not track.
        let _ = ctmd_count;
    }

    /// `parseCR3_CTMD`: the per-item makernote boxes of a CTMD track.
    fn parse_cr3_ctmd(&mut self, track_num: usize) {
        // Error codes: -10 wrong media type, -11 bounds, -12 tag header,
        // -13 TIFF header.
        let saved_order = self.stream.order;
        self.stream.order = LITTLE_ENDIAN;
        let media_offset = self.cr3.tracks[track_num].media_offset;
        let media_size = self.cr3.tracks[track_num].media_size;
        let file_size = self.fsize();
        let mut err = 0;
        if self.cr3.tracks[track_num].media_type != 3 {
            err = -10;
        }
        let mut relpos_in_dir: u32 = 0;
        while err == 0 && (relpos_in_dir.wrapping_add(6) as i64) < media_size {
            if media_offset + relpos_in_dir as i64 > file_size - 6 {
                err = -11;
                break;
            }
            self.stream.seek(media_offset.max(0) as u64 + relpos_in_dir as u64);
            let item_size = self.stream.get4();
            let item_type = self.stream.get2();
            if item_size < 1
                || (relpos_in_dir.wrapping_add(item_size) as i64) > media_size
            {
                err = -11;
                break;
            }
            if item_type == 7 || item_type == 8 || item_type == 9 {
                let boundary = relpos_in_dir.wrapping_add(item_size);
                let mut relpos_in_box = relpos_in_dir.wrapping_add(12);
                while (relpos_in_box.wrapping_add(8) as i64) < boundary as i64 {
                    if media_offset + relpos_in_box as i64 > file_size - 8 {
                        err = -11;
                        break;
                    }
                    self.stream.seek(media_offset.max(0) as u64 + relpos_in_box as u64);
                    let tag_length = self.stream.get4();
                    let tag = self.stream.get4();
                    if tag_length < 8 {
                        err = -12;
                        break;
                    }
                    if (relpos_in_box.wrapping_add(tag_length) as i64) > boundary as i64 {
                        err = -11;
                        break;
                    }
                    if tag == 0x927c {
                        let makernote_offset = media_offset + relpos_in_box as i64 + 8;
                        self.stream.seek(makernote_offset.max(0) as u64);
                        let makernote_order = self.stream.order;
                        self.stream.order = self.stream.get2() as u16;
                        if self.cmt_bad_header() {
                            err = -13;
                            break;
                        }
                        if item_type == 7 || item_type == 8 {
                            self.stream.skip(-8);
                            self.canon.cr3_ctmd = true;
                            self.parse_makernote(makernote_offset, 0);
                            self.canon.cr3_ctmd = false;
                        }
                        self.stream.order = makernote_order;
                    }
                    relpos_in_box = relpos_in_box.wrapping_add(tag_length);
                }
                if err != 0 {
                    break;
                }
            }
            relpos_in_dir = relpos_in_dir.wrapping_add(item_size);
        }
        self.stream.order = saved_order;
        let _ = err;
    }

    // ===== open-stage rules =================================================

    /// The Canon geometry rules of `open_datastream`. Inset-crop records, the
    /// raw-depth level divisions, the raw-aspect crop and the multishot white
    /// balance touch fields the metadata output does not carry, so they stay
    /// engine-only.
    fn finish_open(&mut self) {
        // The Z-mount and D6 bodies record 14-bit rows in the packed two-in-
        // three-bytes layout, and their data size is the record: the plain
        // form keeps the Bayer layout, the three-colour one clears it. The
        // size's half-bytes-per-row division runs in the engine's int, then
        // the ceiling in float; the three-colour compare runs in float too.
        if self.make == "Nikon"
            && (self.model.starts_with('Z') || self.model.eq_ignore_ascii_case("D6"))
        {
            let packed_cells = (self.raw_width as i64 * 7 / 4) as f32 / 16.0;
            let packed_size = (packed_cells.ceil() as u32)
                .wrapping_mul(16)
                .wrapping_mul(self.raw_height as u32) as i64;
            if packed_size == self.data_size {
                self.loader = Some(SensorKind::Nikon14Bit);
            }
            let colour_cells = (self.raw_width as i64 * 21 / 4) as f32 / 16.0;
            let colour_size = colour_cells.ceil() * 16.0 * self.raw_height as f32;
            if colour_size == self.data_size as f32 {
                self.loader = Some(SensorKind::Nikon14Bit);
                self.filters = 0; // 3-color nefx image
            }
        }
        // The sRAW frame is a cropped sensor view: when its ratio is not one
        // of the sRAW shapes, replace it with the sensor size and the absolute
        // crop. The engine reads the crop record even when the tag was absent,
        // where its fields read -1, -1, 0, 0. The ratio divides as a float,
        // matching the engine's single-precision arithmetic.
        if self.make == "Canon"
            && self.loader == Some(SensorKind::CanonSraw)
            && self.raw_width > 0
        {
            let ratio = (self.raw_height as f32 / self.raw_width as f32) as f64;
            let fallback_area = CanonArea { l: -1, t: -1, r: 0, b: 0 };
            let crop = self.canon.default_crop_absolute.unwrap_or(fallback_area);
            if (ratio < 0.57 || ratio > 0.75)
                && self.canon.sensor_height > 1
                && self.canon.sensor_width > 1
            {
                self.raw_width = self.canon.sensor_width;
                self.left_margin = crop.l;
                self.width = crop.r - crop.l + 1;
                self.height = crop.b - crop.t + 1;
                self.raw_height = self.canon.sensor_height;
                self.top_margin = crop.t;
                self.load_flags |= 256; // reset width/height in the sRAW unpacker
                self.raw_pitch = 8 * self.raw_width as u32;
            } else if self.raw_width == 4032
                && self.raw_height == 3402
                && self.model.eq_ignore_ascii_case("EOS 80D")
            {
                // The 80D records 4032x3402 for its 4536x3024 sensor.
                self.raw_width = 4536;
                self.left_margin = 28;
                self.width = self.raw_width - self.left_margin;
                self.raw_height = 3024;
                self.top_margin = 8;
                self.height = self.raw_height - self.top_margin;
                self.load_flags |= 256;
                self.raw_pitch = 8 * self.raw_width as u32;
            }
        }
        // X-Trans compressed streams: with the unpacked loader still set,
        // the raw byte counts first pick the 14-bit packed layout, then the
        // block-compressed one from its 0x4953 header.
        if self.dng_version == 0
            && self.make == "Fujifilm"
            && self.loader == Some(SensorKind::Unpacked)
        {
            let unpacked_bytes = self.raw_width as i64 * self.raw_height as i64 * 2;
            if unpacked_bytes != self.data_size {
                let packed_bytes = self.raw_width as i64 * self.raw_height as i64 * 7 / 4;
                if packed_bytes == self.data_size {
                    self.loader = Some(SensorKind::Fuji14Bit);
                } else {
                    self.parse_fuji_compressed_header();
                }
            } else if self.model == "X-H2S" && self.fsize() < self.data_size + self.data_offset {
                // X-H2S bodies can record a wrong data size.
                self.parse_fuji_compressed_header();
            }
        }
        // The engine's aspect-rule pass over the body's inset records has no
        // effect this pass can see: the inset records reach no loader and the
        // metadata hand-off does not carry them.
        let mut adjust_margins = 0;
        if self.make == "Fujifilm" && self.filters == 9 {
            // X-Trans margins align to the pattern's 6x6 tile.
            let newtm = if self.top_margin % 6 != 0 {
                (self.top_margin / 6 + 1) * 6
            } else {
                self.top_margin
            };
            let newlm = if self.left_margin % 6 != 0 {
                (self.left_margin / 6 + 1) * 6
            } else {
                self.left_margin
            };
            if newtm != self.top_margin || newlm != self.left_margin {
                self.height -= newtm - self.top_margin;
                self.top_margin = newtm;
                self.width -= newlm - self.left_margin;
                self.left_margin = newlm;
                // The pattern re-copy is the plain one: the margins' rotation
                // happens in the loader, not here.
                self.xtrans = self.xtrans_abs;
            }
            adjust_margins = 6;
        } else if self.fuji_width == 0 && self.filters >= 1000 {
            // The odd-margin bayer rebuild that aligns the CFA with the
            // sensor. The engine's own Fuji X-Trans margin rule ran above and
            // needs `parse_fuji`, which never fires for CRW, CR2 and CR3
            // files.
            if self.top_margin % 2 != 0 || self.left_margin % 2 != 0 {
                let mut crop = [0usize; 2];
                if self.top_margin % 2 != 0 {
                    self.top_margin += 1;
                    self.height -= 1;
                    crop[1] = 1;
                }
                if self.left_margin % 2 != 0 {
                    self.left_margin += 1;
                    self.width -= 1;
                    crop[0] = 1;
                }
                let old_filters = self.filters;
                let mut rebuilt = 0u32;
                for square in 0..16usize {
                    let row = (square >> 1) + crop[1];
                    let column = (square & 1) + crop[0];
                    let colour = (old_filters
                        >> (((row << 1 & 14) + (column & 1)) << 1))
                        & 3;
                    rebuilt |= colour << (square * 2);
                }
                self.filters = rebuilt;
            }
            adjust_margins = 2;
        }
        if adjust_margins != 0 {
            // The engine's rounding pass over the inset records; like the
            // aspect rule above, it touches no fact this pass carries.
            if trace() {
                eprintln!("trace: finish_open adjust_margins {}", adjust_margins);
            }
        }
        // Sony: the pixel-shift composites carry their crop's own margin, and
        // the crop-mode frames of a few bodies whose sizes the directories do
        // not record keep their estimate.
        if self.make == "Sony" && self.dng_version == 0 {
            if self.loader == Some(SensorKind::SonyArq) {
                // A7RM4 16x, both APS-C and APS; A7RM3/M4 4x merge.
                self.width = self.raw_width as i32 - if self.raw_width > 12000 { 64 } else { 32 };
            }
            let a99 = self.sony.body_id == SONY_ID_SLT_A99;
            let lower = self.model.to_ascii_lowercase();
            let crop_pair = (lower.starts_with("ilce-7rm") || self.model.eq_ignore_ascii_case("ILCA-99M2"))
                && (self.raw_width == 5216 || self.raw_width == 6304);
            let a7r_crop = self.model.eq_ignore_ascii_case("ILCE-7R")
                && self.raw_width >= 4580
                && self.raw_width < 5020;
            let a7_crop = self.model.eq_ignore_ascii_case("ILCE-7") && self.raw_width == 3968;
            let a7m_crop = (lower.starts_with("ilce-7m") || self.model.eq_ignore_ascii_case("ILCE-9") || a99)
                && self.raw_width > 3750
                && self.raw_width < 4120;
            let a7s_crop = lower.starts_with("ilce-7s") && self.raw_width == 2816;
            if crop_pair || a7r_crop || a7_crop || a7m_crop || a7s_crop {
                self.width = self.raw_width as i32 - 32;
            }
        }
        // The oldest SuperCCD bodies carry double-wide rows; halving the raw
        // width moves them onto the dedicated loader.
        if self.dng_version == 0
            && self.make == "Fujifilm"
            && (self.model.starts_with("S20Pro") || self.model.starts_with("F700"))
        {
            self.raw_width /= 2;
            self.loader = Some(SensorKind::FujiF700S20);
        }
        // The packed rows the D810 and D4S write need their column-boundary
        // flag when the byte counts line up with three bytes per two samples.
        if self.loader == Some(SensorKind::Packed)
            && self.make == "Nikon"
            && self.load_flags == 0
            && (self.model.starts_with("D810") || self.model == "D4S")
            && self.data_size * 2 == self.raw_height as i64 * self.raw_width as i64 * 3
        {
            self.load_flags = 80;
        }
        // The 12-bit Sony records ship 14-bit-ranged metadata.
        if self.loader == Some(SensorKind::Packed) && self.make == "Sony" {
            if self.maximum > 4095 {
                self.maximum = 4095;
            }
            if self.black > 256 || self.cblack[0] > 256 {
                self.black /= 4;
                for level in &mut self.cblack[..4] {
                    *level /= 4;
                }
                let cells = (self.cblack[4] * self.cblack[5]).min(CBLACK_SIZE as u32 - 6) as usize;
                for level in self.cblack[6..6 + cells].iter_mut() {
                    *level /= 4;
                }
            }
        }
        // The engine's yuv loader is the Nikon sRAW in disguise: black goes,
        // the Bayer layout goes, and the pixel phase reinterprets the record
        // along with the s-curve table rebuilt over the gamma's head.
        if self.loader == Some(SensorKind::NikonYuv) {
            self.loader = Some(SensorKind::NikonSraw);
            self.yuv_gamma = true;
            self.black = 0;
            self.cblack.fill(0);
            self.filters = 0;
            self.tiff_samples = 3;
            self.colors = 3;
            self.rgb_cam = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        }
        // The 12-bit Nikon records ship 14-bit-ranged metadata.
        if (self.loader == Some(SensorKind::NikonPlain)
            || self.loader == Some(SensorKind::Packed)
            || self.loader == Some(SensorKind::NikonPaddedPacked))
            && self.make == "Nikon"
            && !self.model.starts_with("COOLPIX")
            && self.tiff_bps == 12
        {
            self.maximum = 4095;
            self.black /= 4;
            for level in &mut self.cblack[..4] {
                *level /= 4;
            }
            let cells = (self.cblack[4] * self.cblack[5]).min(CBLACK_SIZE as u32 - 6) as usize;
            for level in self.cblack[6..6 + cells].iter_mut() {
                *level /= 4;
            }
        }
        // The black levels of the Panasonic records: on the ordinary layout
        // they gain 15 rows of margin, then the smallest level becomes the
        // plane's black and the rest are differences against it.
        if self.loader == Some(SensorKind::Panasonic)
            && self.make == "Panasonic"
            && self.pana.black[0] != 0
            && self.pana.black[1] != 0
            && self.pana.black[2] != 0
        {
            if self.pana.encoding == 5 {
                self.zero_is_bad = false;
            }
            self.black = 0;
            let margin_rows = u32::from(self.pana.encoding == 4) * 15;
            self.cblack[0] = self.pana.black[0] + margin_rows;
            self.cblack[3] = self.pana.black[1] + margin_rows;
            self.cblack[1] = self.cblack[3];
            self.cblack[2] = self.pana.black[2] + margin_rows;
            let mut lowest = self.cblack[3];
            for channel in 0..3 {
                if lowest > self.cblack[channel] {
                    lowest = self.cblack[channel];
                }
            }
            for level in self.cblack[..4].iter_mut() {
                *level -= lowest;
            }
            self.black = lowest;
        }
        // The later Panasonic bodies: their encodings carry the row's own
        // packed layout, and the recorded size must match it. The open stage
        // takes the black levels back, loses the pattern gains and sets the
        // maximum from the largest LinearityLimit record.
        if self.loader == Some(SensorKind::Panasonic)
            && matches!(self.pana.encoding, 6 | 7 | 8)
        {
            self.cblack[0] = self.pana.black[0];
            self.cblack[1] = self.pana.black[1];
            self.cblack[2] = self.pana.black[2];
            self.cblack[3] = self.cblack[1];
            self.cblack[4] = 0;
            self.cblack[5] = 0;
            self.black = 0;
            let largest = self.pana.linear_max[0].max(self.pana.linear_max[1]).max(self.pana.linear_max[2]);
            self.maximum = largest;
            if self.pana.encoding == 6 {
                // The block count picks the layout: eleven samples a block
                // when the width divides evenly and the size matches, then
                // fourteen, and nothing else reads.
                let row_bytes_11 = self.raw_width / 11 * 16;
                let row_bytes_14 = self.raw_width / 14 * 16;
                let mut data_size = self.data_size;
                if data_size == 0 {
                    data_size = self.fsize() - self.data_offset;
                }
                let block_11 = self.raw_width % 11 == 0
                    && self.raw_height as i64 * row_bytes_11 as i64 == data_size;
                let block_14 = self.raw_width % 14 == 0
                    && self.raw_height as i64 * row_bytes_14 as i64 == data_size;
                if block_11 || block_14 {
                    self.loader = Some(SensorKind::PanasonicC6);
                } else {
                    self.refusal = Some("the camera raw data size does not match its recorded layout");
                }
            } else if self.pana.encoding == 7 {
                let pixels_per_block = if self.pana.bits == 14 { 9 } else { 10 };
                let row_bytes = self.raw_width / pixels_per_block * 16;
                if self.raw_width % pixels_per_block == 0
                    && self.raw_height as i64 * row_bytes as i64 == self.data_size
                {
                    self.loader = Some(SensorKind::PanasonicC7);
                } else {
                    self.refusal = Some("the camera raw data size does not match its recorded layout");
                }
            } else if self.pana.encoding == 8 {
                // The encoding-8 stream's stripe decoder is not ported; the
                // file keeps rawler's decode. The engine refuses only the
                // stripe-less corrupt forms, which rawler refuses as well.
                self.loader = None;
            }
        }
        // The engine reads the compression record's tone curve at open for
        // the bodies whose loader is `nikon_load_raw`; the loader's own
        // lookup reads back from it.
        if self.loader == Some(SensorKind::NikonPlain) {
            self.nikon_read_curve();
        }
    }

    /// The engine's `nikon_read_curve`: the compression record's tone curve
    /// replaces the identity over the entries it supplies, either read
    /// straight in or interpolated between its knees.
    fn nikon_read_curve(&mut self) {
        let mut curve = vec![0u16; 0x10000];
        for (index, value) in curve.iter_mut().enumerate() {
            *value = index as u16;
        }
        self.stream.seek(self.meta_offset.max(0) as u64);
        // A byte past the end keeps the -1 fgetc hands to a ushort.
        let ver0 = self.stream.getc().map_or(-1i16, i16::from) as u16;
        let ver1 = self.stream.getc().map_or(-1i16, i16::from) as u16;
        if ver0 == 0x49 || ver1 == 0x58 {
            self.stream.skip(2110);
        }
        let mut vpred = [0u16; 4];
        self.stream.read_shorts(&mut vpred);
        // The x86 shift truncates its count to five bits.
        let ceiling = 1i32.wrapping_shl(self.tiff_bps as u32) & 0x7fff;
        let mut step = ceiling;
        let mut max = ceiling;
        let csize = i32::from(self.stream.get2());
        if csize > 1 {
            step = max / (csize - 1);
        }
        if ver0 == 0x44 && (ver1 == 0x20 || (ver1 == 0x40 && step > 3)) && step > 0 {
            if ver1 == 0x40 {
                step /= 4;
                max /= 4;
            }
            for i in 0..csize {
                let index = i * step;
                if (index as usize) < curve.len() {
                    curve[index as usize] = self.stream.get2();
                }
            }
            for i in 0..max {
                let remainder = i % step;
                let left = i - remainder;
                curve[i as usize] = ((curve[left as usize] as i32 * (step - remainder)
                    + curve[(left + step) as usize] as i32 * remainder)
                    / step) as u16;
            }
        } else if ver0 != 0x46 && csize <= 0x4001 {
            self.stream.read_shorts(&mut curve[..csize as usize]);
        }
        self.nikon_curve = Some(curve);
    }

    // ===== handoff ==========================================================

    /// The metadata record the decoder phases expect, in the engine's
    /// initialization state: an identity curve and zeroed black levels.
    fn into_meta(self) -> CameraMeta {
        // The engine's curve is a 0x10000 identity fill; the Sony ladder
        // rewrites its head and the Nikon records rewrite a prefix.
        let mut curve = match self.nikon_curve {
            Some(curve) => curve,
            None => {
                let mut curve = self.curve_head.to_vec();
                curve.extend((4096u32..=0xffffu32).map(|value| value as u16));
                curve
            }
        };
        if self.yuv_gamma {
            // The compressed arm's table build overwrites every entry the
            // engine's gamma call reaches; the open stage's rebuild then runs
            // over its head (the sRAW loader reads the result back).
            curve = super::process::gamma_curve_table(1.0 / 2.4, 12.92, 1, 4095);
            rebuild_nikon_sraw_curve(&mut curve);
        }
        CameraMeta {
            make: self.make.clone(),
            model: self.model.clone(),
            colors: self.colors,
            filters: self.filters,
            xtrans: self.xtrans,
            xtrans_abs: self.xtrans_abs,
            raw_width: self.raw_width as u32,
            raw_height: self.raw_height as u32,
            width: self.width as u32,
            height: self.height as u32,
            left_margin: self.left_margin as u32,
            top_margin: self.top_margin as u32,
            pixel_aspect: self.pixel_aspect,
            fuji_width: self.fuji_width,
            tiff_bps: self.tiff_bps,
            tiff_samples: self.tiff_samples,
            data_offset: self.data_offset,
            data_size: self.data_size,
            strip_offset: self.strip_offset,
            meta_offset: self.meta_offset,
            order: self.stream.order,
            fuji_layout: self.fuji_layout as u32,
            tile_width: self.tile_width,
            tile_length: self.tile_length,
            curve,
            black: self.black,
            cblack: self.cblack,
            maximum: self.maximum,
            pre_mul: self.pre_mul,
            cam_mul: self.cam_mul,
            rgb_cam: self.rgb_cam,
            raw_color: self.raw_color,
            mask: self.mask,
            loader: self.loader.unwrap_or(SensorKind::CanonLosslessJpeg),
            cr2_slice: self.cr2_slice,
            sraw_mul: self.canon.sraw_mul,
            load_flags: self.load_flags,
            tiff_compress: self.tiff_compress,
            zero_is_bad: self.zero_is_bad,
            raw_pitch: self.raw_pitch,
            canon_unique_id: self.canon.unique_id,
            model2: self.model2.clone(),
            olympus: OlympusState {
                valid_bits: self.oly.valid_bits,
                tag_x640: self.oly.tag_x640,
                tag_x641: self.oly.tag_x641,
                tag_x642: self.oly.tag_x642,
                tag_x643: self.oly.tag_x643,
                tag_x644: self.oly.tag_x644,
                tag_x645: self.oly.tag_x645,
                tag_x646: self.oly.tag_x646,
                tag_x647: self.oly.tag_x647,
                tag_x648: self.oly.tag_x648,
                tag_x649: self.oly.tag_x649,
                tag_x650: self.oly.tag_x650,
                tag_x651: self.oly.tag_x651,
                tag_x652: self.oly.tag_x652,
                tag_x653: self.oly.tag_x653,
            },
            panasonic: PanasonicState { bits: self.pana.bits, encoding: self.pana.encoding },
            crx: {
                let chosen = &self.cr3.tracks[self.cr3.chosen_track];
                CameraCrxHeader {
                    version: chosen.version as u32,
                    f_width: chosen.f_width as u32,
                    f_height: chosen.f_height as u32,
                    tile_width: chosen.tile_width as u32,
                    tile_height: chosen.tile_height as u32,
                    n_planes: chosen.n_planes,
                    cfa_layout: chosen.cfa_layout,
                    enc_type: chosen.enc_type,
                    image_levels: chosen.image_levels,
                    n_bits: chosen.n_bits,
                    median_bits: chosen.median_bits,
                    mdat_header_size: chosen.mdat_header_size,
                }
            },
            fuji: FujiComprState {
                total_lines: self.fuji.total_lines,
                total_blocks: self.fuji.total_blocks,
                block_width: self.fuji.block_width,
                compr_bits: self.fuji.compr_bits,
                compr_raw_type: self.fuji.compr_raw_type,
                compr_lossless: self.fuji.compr_lossless,
            },
            strip_ifds: self
                .ifds
                .iter()
                .take(self.ifd_count)
                .map(|record| StripIfdFacts {
                    offset: record.offset,
                    rows_per_strip: record.rows_per_strip,
                    strip_offsets: record.strip_offsets.clone(),
                })
                .collect(),
            refusal: self.refusal,
        }
    }
}