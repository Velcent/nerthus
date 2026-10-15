//! The CABAC decoding engine and its context models (`cabac.cc`,
//! `contextmodel.cc`).

use std::sync::Arc;

use crate::util::clip3;

const LPS_TABLE: [[u8; 4]; 64] = [
    [128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205], [116, 142, 169, 195], [111, 135, 160, 185],
    [105, 128, 152, 175], [100, 122, 144, 166], [95, 116, 137, 158], [90, 110, 130, 150], [85, 104, 123, 142], [81, 99, 117, 135],
    [77, 94, 111, 128], [73, 89, 105, 122], [69, 85, 100, 116], [66, 80, 95, 110], [62, 76, 90, 104], [59, 72, 86, 99],
    [56, 69, 81, 94], [53, 65, 77, 89], [51, 62, 73, 85], [48, 59, 69, 80], [46, 56, 66, 76], [43, 53, 63, 72],
    [41, 50, 59, 69], [39, 48, 56, 65], [37, 45, 54, 62], [35, 43, 51, 59], [33, 41, 48, 56], [32, 39, 46, 53],
    [30, 37, 43, 50], [29, 35, 41, 48], [27, 33, 39, 45], [26, 31, 37, 43], [24, 30, 35, 41], [23, 28, 33, 39],
    [22, 27, 32, 37], [21, 26, 30, 35], [20, 24, 29, 33], [19, 23, 27, 31], [18, 22, 26, 30], [17, 21, 25, 28],
    [16, 20, 23, 27], [15, 19, 22, 25], [14, 18, 21, 24], [14, 17, 20, 23], [13, 16, 19, 22], [12, 15, 18, 21],
    [12, 14, 17, 20], [11, 14, 16, 19], [11, 13, 15, 18], [10, 12, 15, 17], [10, 12, 14, 16], [9, 11, 13, 15],
    [9, 11, 12, 14], [8, 10, 12, 14], [8, 9, 11, 13], [7, 9, 11, 12], [7, 9, 10, 12], [7, 8, 10, 11],
    [6, 8, 9, 11], [6, 7, 9, 10], [6, 7, 8, 9], [2, 2, 2, 2],
];

const RENORM_TABLE: [u8; 32] = [6, 5, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];

const NEXT_STATE_MPS: [u8; 64] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38,
    39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

const NEXT_STATE_LPS: [u8; 64] = [
    0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, 13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24, 24, 25, 26, 26, 27, 27, 28,
    29, 29, 30, 30, 30, 31, 32, 32, 33, 33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

/// One context: its state and most probable symbol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextModel {
    pub mps: u8,
    pub state: u8,
}

pub const SAO_MERGE_FLAG: usize = 0;
pub const SAO_TYPE_IDX: usize = SAO_MERGE_FLAG + 1;
pub const SPLIT_CU_FLAG: usize = SAO_TYPE_IDX + 1;
pub const CU_SKIP_FLAG: usize = SPLIT_CU_FLAG + 3;
pub const PART_MODE: usize = CU_SKIP_FLAG + 3;
pub const PREV_INTRA_LUMA_PRED_FLAG: usize = PART_MODE + 4;
pub const INTRA_CHROMA_PRED_MODE: usize = PREV_INTRA_LUMA_PRED_FLAG + 1;
pub const CBF_LUMA: usize = INTRA_CHROMA_PRED_MODE + 1;
pub const CBF_CHROMA: usize = CBF_LUMA + 2;
pub const SPLIT_TRANSFORM_FLAG: usize = CBF_CHROMA + 4;
pub const CU_CHROMA_QP_OFFSET_FLAG: usize = SPLIT_TRANSFORM_FLAG + 3;
pub const CU_CHROMA_QP_OFFSET_IDX: usize = CU_CHROMA_QP_OFFSET_FLAG + 1;
pub const LAST_SIGNIFICANT_COEFFICIENT_X_PREFIX: usize = CU_CHROMA_QP_OFFSET_IDX + 1;
pub const LAST_SIGNIFICANT_COEFFICIENT_Y_PREFIX: usize = LAST_SIGNIFICANT_COEFFICIENT_X_PREFIX + 18;
pub const CODED_SUB_BLOCK_FLAG: usize = LAST_SIGNIFICANT_COEFFICIENT_Y_PREFIX + 18;
pub const SIGNIFICANT_COEFF_FLAG: usize = CODED_SUB_BLOCK_FLAG + 4;
pub const COEFF_ABS_LEVEL_GREATER1_FLAG: usize = SIGNIFICANT_COEFF_FLAG + 42 + 2;
pub const COEFF_ABS_LEVEL_GREATER2_FLAG: usize = COEFF_ABS_LEVEL_GREATER1_FLAG + 24;
pub const CU_QP_DELTA_ABS: usize = COEFF_ABS_LEVEL_GREATER2_FLAG + 6;
pub const TRANSFORM_SKIP_FLAG: usize = CU_QP_DELTA_ABS + 2;
pub const RDPCM_FLAG: usize = TRANSFORM_SKIP_FLAG + 2;
pub const RDPCM_DIR: usize = RDPCM_FLAG + 2;
pub const MERGE_FLAG: usize = RDPCM_DIR + 2;
pub const MERGE_IDX: usize = MERGE_FLAG + 1;
pub const PRED_MODE_FLAG: usize = MERGE_IDX + 1;
pub const ABS_MVD_GREATER01_FLAG: usize = PRED_MODE_FLAG + 1;
pub const MVP_LX_FLAG: usize = ABS_MVD_GREATER01_FLAG + 2;
pub const RQT_ROOT_CBF: usize = MVP_LX_FLAG + 1;
pub const REF_IDX_LX: usize = RQT_ROOT_CBF + 1;
pub const INTER_PRED_IDC: usize = REF_IDX_LX + 2;
pub const CU_TRANSQUANT_BYPASS_FLAG: usize = INTER_PRED_IDC + 5;
pub const LOG2_RES_SCALE_ABS_PLUS1: usize = CU_TRANSQUANT_BYPASS_FLAG + 1;
pub const RES_SCALE_SIGN_FLAG: usize = LOG2_RES_SCALE_ABS_PLUS1 + 8;
pub const TABLE_LENGTH: usize = RES_SCALE_SIGN_FLAG + 2;

pub type ContextTable = [ContextModel; TABLE_LENGTH];

const SPLIT_CU_FLAG_INIT: [[i32; 3]; 3] = [[139, 141, 157], [107, 139, 126], [107, 139, 126]];
const CU_SKIP_FLAG_INIT: [[i32; 3]; 2] = [[197, 185, 201], [197, 185, 201]];
const PART_MODE_INIT: [i32; 9] = [184, 154, 139, 154, 154, 154, 139, 154, 154];
const PREV_INTRA_LUMA_PRED_FLAG_INIT: [i32; 3] = [184, 154, 183];
const INTRA_CHROMA_PRED_MODE_INIT: [i32; 3] = [63, 152, 152];
const CBF_LUMA_INIT: [i32; 4] = [111, 141, 153, 111];
const CBF_CHROMA_INIT: [i32; 12] = [94, 138, 182, 154, 149, 107, 167, 154, 149, 92, 167, 154];
const SPLIT_TRANSFORM_FLAG_INIT: [i32; 9] = [153, 138, 138, 124, 138, 94, 224, 167, 122];
const LAST_SIGNIFICANT_COEFFICIENT_PREFIX_INIT: [i32; 54] = [
    110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63, 125, 110, 94, 110, 95, 79, 125, 111, 110, 78, 110,
    111, 111, 95, 94, 108, 123, 108, 125, 110, 124, 110, 95, 94, 125, 111, 111, 79, 125, 126, 111, 111, 79, 108, 123, 93,
];
const CODED_SUB_BLOCK_FLAG_INIT: [i32; 12] = [91, 171, 134, 141, 121, 140, 61, 154, 121, 140, 61, 154];
const SIGNIFICANT_COEFF_FLAG_INIT: [[i32; 42]; 3] = [
    [
        111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125,
        140, 139, 182, 182, 152, 136, 152, 136, 153, 136, 139, 111, 136, 139, 111,
    ],
    [
        155, 154, 139, 153, 139, 123, 123, 63, 153, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154,
        170, 153, 123, 123, 107, 121, 107, 121, 167, 151, 183, 140, 151, 183, 140,
    ],
    [
        170, 154, 139, 153, 139, 123, 123, 63, 124, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154, 166, 183, 140, 136, 153, 154,
        170, 153, 138, 138, 122, 121, 122, 121, 167, 151, 183, 140, 151, 183, 140,
    ],
];
const SIGNIFICANT_COEFF_FLAG_SKIPMODE_INIT: [[i32; 2]; 3] = [[141, 111], [140, 140], [140, 140]];
const COEFF_ABS_LEVEL_GREATER1_FLAG_INIT: [i32; 72] = [
    140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152, 140, 179, 166, 182, 140, 227, 122, 197, 154, 196, 196, 167,
    154, 152, 167, 182, 182, 134, 149, 136, 153, 121, 136, 137, 169, 194, 166, 167, 154, 167, 137, 182, 154, 196, 167, 167, 154, 152, 167, 182,
    182, 134, 149, 136, 153, 121, 136, 122, 169, 208, 166, 167, 154, 152, 167, 182,
];
const COEFF_ABS_LEVEL_GREATER2_FLAG_INIT: [i32; 18] = [138, 153, 136, 167, 152, 152, 107, 167, 91, 122, 107, 167, 107, 167, 91, 107, 107, 167];
const SAO_MERGE_FLAG_INIT: [i32; 3] = [153, 153, 153];
const SAO_TYPE_IDX_INIT: [i32; 3] = [200, 185, 160];
const CU_QP_DELTA_ABS_INIT: [i32; 2] = [154, 154];
const TRANSFORM_SKIP_FLAG_INIT: [i32; 2] = [139, 139];
const MERGE_FLAG_INIT: [i32; 2] = [110, 154];
const MERGE_IDX_INIT: [i32; 2] = [122, 137];
const PRED_MODE_FLAG_INIT: [i32; 2] = [149, 134];
const ABS_MVD_GREATER01_FLAG_INIT: [i32; 4] = [140, 198, 169, 198];
const MVP_LX_FLAG_INIT: [i32; 1] = [168];
const RQT_ROOT_CBF_INIT: [i32; 1] = [79];
const REF_IDX_LX_INIT: [i32; 2] = [153, 153];
const INTER_PRED_IDC_INIT: [i32; 5] = [95, 79, 63, 31, 31];
const CU_TRANSQUANT_BYPASS_FLAG_INIT: [i32; 3] = [154, 154, 154];

/// `set_initValue`.
fn init_value(qp: i32, init: i32) -> ContextModel {
    let slope = init >> 4;
    let intersection = init & 0xf;
    let m = slope * 5 - 45;
    let n = (intersection << 3) - 16;
    let pre_state = clip3(1, 126, ((m * clip3(0, 51, qp)) >> 4) + n);
    let mps = if pre_state <= 63 { 0 } else { 1 };
    let state = if mps != 0 { pre_state - 64 } else { 63 - pre_state };
    ContextModel { mps, state: state as u8 }
}

fn init_contexts(table: &mut ContextTable, start: usize, qp: i32, values: &[i32]) {
    for (i, &value) in values.iter().enumerate() {
        table[start + i] = init_value(qp, value);
    }
}

fn init_constant(table: &mut ContextTable, start: usize, qp: i32, value: i32, count: usize) {
    let model = init_value(qp, value);
    for entry in table[start..start + count].iter_mut() {
        *entry = model;
    }
}

/// `initialize_CABAC_models`: inter contexts are only set for P and B
/// slices; an I slice leaves them as they were.
pub fn initialize_models(table: &mut ContextTable, init_type: usize, qp: i32) {
    if init_type > 0 {
        init_contexts(table, CU_SKIP_FLAG, qp, &CU_SKIP_FLAG_INIT[init_type - 1]);
        init_contexts(table, PRED_MODE_FLAG, qp, &PRED_MODE_FLAG_INIT[init_type - 1..init_type]);
        init_contexts(table, MERGE_FLAG, qp, &MERGE_FLAG_INIT[init_type - 1..init_type]);
        init_contexts(table, MERGE_IDX, qp, &MERGE_IDX_INIT[init_type - 1..init_type]);
        init_contexts(table, INTER_PRED_IDC, qp, &INTER_PRED_IDC_INIT);
        init_contexts(table, REF_IDX_LX, qp, &REF_IDX_LX_INIT);
        let mvd_start = if init_type == 1 { 0 } else { 2 };
        init_contexts(table, ABS_MVD_GREATER01_FLAG, qp, &ABS_MVD_GREATER01_FLAG_INIT[mvd_start..mvd_start + 2]);
        init_contexts(table, MVP_LX_FLAG, qp, &MVP_LX_FLAG_INIT);
        init_contexts(table, RQT_ROOT_CBF, qp, &RQT_ROOT_CBF_INIT);
        init_constant(table, RDPCM_FLAG, qp, 139, 2);
        init_constant(table, RDPCM_DIR, qp, 139, 2);
    }
    init_contexts(table, SPLIT_CU_FLAG, qp, &SPLIT_CU_FLAG_INIT[init_type]);
    let part_start = if init_type != 2 { init_type } else { 5 };
    init_contexts(table, PART_MODE, qp, &PART_MODE_INIT[part_start..part_start + 4]);
    init_contexts(table, PREV_INTRA_LUMA_PRED_FLAG, qp, &PREV_INTRA_LUMA_PRED_FLAG_INIT[init_type..init_type + 1]);
    init_contexts(table, INTRA_CHROMA_PRED_MODE, qp, &INTRA_CHROMA_PRED_MODE_INIT[init_type..init_type + 1]);
    let cbf_start = if init_type == 0 { 0 } else { 2 };
    init_contexts(table, CBF_LUMA, qp, &CBF_LUMA_INIT[cbf_start..cbf_start + 2]);
    init_contexts(table, CBF_CHROMA, qp, &CBF_CHROMA_INIT[init_type * 4..init_type * 4 + 4]);
    init_contexts(table, SPLIT_TRANSFORM_FLAG, qp, &SPLIT_TRANSFORM_FLAG_INIT[init_type * 3..init_type * 3 + 3]);
    let prefix = &LAST_SIGNIFICANT_COEFFICIENT_PREFIX_INIT[init_type * 18..init_type * 18 + 18];
    init_contexts(table, LAST_SIGNIFICANT_COEFFICIENT_X_PREFIX, qp, prefix);
    init_contexts(table, LAST_SIGNIFICANT_COEFFICIENT_Y_PREFIX, qp, prefix);
    init_contexts(table, CODED_SUB_BLOCK_FLAG, qp, &CODED_SUB_BLOCK_FLAG_INIT[init_type * 4..init_type * 4 + 4]);
    init_contexts(table, SIGNIFICANT_COEFF_FLAG, qp, &SIGNIFICANT_COEFF_FLAG_INIT[init_type]);
    init_contexts(table, SIGNIFICANT_COEFF_FLAG + 42, qp, &SIGNIFICANT_COEFF_FLAG_SKIPMODE_INIT[init_type]);
    init_contexts(table, COEFF_ABS_LEVEL_GREATER1_FLAG, qp, &COEFF_ABS_LEVEL_GREATER1_FLAG_INIT[init_type * 24..init_type * 24 + 24]);
    init_contexts(table, COEFF_ABS_LEVEL_GREATER2_FLAG, qp, &COEFF_ABS_LEVEL_GREATER2_FLAG_INIT[init_type * 6..init_type * 6 + 6]);
    init_contexts(table, SAO_MERGE_FLAG, qp, &SAO_MERGE_FLAG_INIT[init_type..init_type + 1]);
    init_contexts(table, SAO_TYPE_IDX, qp, &SAO_TYPE_IDX_INIT[init_type..init_type + 1]);
    init_contexts(table, CU_QP_DELTA_ABS, qp, &CU_QP_DELTA_ABS_INIT);
    init_contexts(table, TRANSFORM_SKIP_FLAG, qp, &TRANSFORM_SKIP_FLAG_INIT);
    init_contexts(table, CU_TRANSQUANT_BYPASS_FLAG, qp, &CU_TRANSQUANT_BYPASS_FLAG_INIT[init_type..init_type + 1]);
    init_constant(table, LOG2_RES_SCALE_ABS_PLUS1, qp, 154, 8);
    init_constant(table, RES_SCALE_SIGN_FLAG, qp, 154, 2);
    init_constant(table, CU_CHROMA_QP_OFFSET_FLAG, qp, 154, 1);
    init_constant(table, CU_CHROMA_QP_OFFSET_IDX, qp, 154, 1);
}

/// `CABAC_decoder` over a slice's bytes.
#[derive(Clone, Debug, Default)]
pub struct CabacDecoder {
    data: Arc<[u8]>,
    /// Where `bitstream_curr` is.
    pub position: usize,
    /// Where `bitstream_end` is.
    pub end: usize,
    range: u32,
    value: u32,
    bits_needed: i16,
}

impl CabacDecoder {
    /// `init_CABAC_decoder`: the decoder over `data[start..end]`.
    pub fn new(data: Arc<[u8]>, start: usize, end: usize) -> CabacDecoder {
        CabacDecoder { data, position: start, end, range: 0, value: 0, bits_needed: 0 }
    }

    pub fn shared_data(&self) -> Arc<[u8]> {
        self.data.clone()
    }

    /// `init_CABAC_decoder_2`.
    pub fn start(&mut self) {
        let length = self.end as isize - self.position as isize;
        self.range = 510;
        self.bits_needed = 8;
        self.value = 0;
        if length > 0 {
            self.value = (self.data[self.position] as u32) << 8;
            self.position += 1;
            self.bits_needed -= 8;
        }
        if length > 1 {
            self.value |= self.data[self.position] as u32;
            self.position += 1;
            self.bits_needed -= 8;
        }
    }

    fn next_byte(&mut self) -> Option<u32> {
        if self.position < self.end {
            let byte = self.data[self.position] as u32;
            self.position += 1;
            Some(byte)
        } else {
            None
        }
    }

    /// `decode_CABAC_bit`.
    pub fn decode_bit(&mut self, model: &mut ContextModel) -> i32 {
        let decoded;
        // The range stays in 256..=510 for any stream; the mask only keeps
        // a broken one from indexing out of the table.
        let range_index = ((self.range >> 6).wrapping_sub(4) & 3) as usize;
        let lps = LPS_TABLE[(model.state & 63) as usize][range_index] as u32;
        self.range -= lps;
        let scaled_range = self.range << 7;
        if self.value < scaled_range {
            decoded = model.mps as i32;
            model.state = NEXT_STATE_MPS[model.state as usize];
            if scaled_range < (256 << 7) {
                self.range = scaled_range >> 6;
                self.value <<= 1;
                self.bits_needed += 1;
                if self.bits_needed == 0 {
                    self.bits_needed = -8;
                    if let Some(byte) = self.next_byte() {
                        self.value |= byte;
                    }
                }
            }
        } else {
            self.value -= scaled_range;
            let bits = RENORM_TABLE[(lps >> 3) as usize] as u32;
            self.value <<= bits;
            self.range = lps << bits;
            decoded = 1 - model.mps as i32;
            if model.state == 0 {
                model.mps = 1 - model.mps;
            }
            model.state = NEXT_STATE_LPS[model.state as usize];
            self.bits_needed += bits as i16;
            if self.bits_needed >= 0 {
                if let Some(byte) = self.next_byte() {
                    self.value |= byte << self.bits_needed;
                }
                self.bits_needed -= 8;
            }
        }
        decoded
    }

    /// `decode_CABAC_term_bit`.
    pub fn decode_term_bit(&mut self) -> i32 {
        self.range -= 2;
        let scaled_range = self.range << 7;
        if self.value >= scaled_range {
            return 1;
        }
        if scaled_range < (256 << 7) {
            self.range = scaled_range >> 6;
            self.value = self.value.wrapping_mul(2);
            self.bits_needed += 1;
            if self.bits_needed == 0 {
                self.bits_needed = -8;
                if let Some(byte) = self.next_byte() {
                    self.value = self.value.wrapping_add(byte);
                }
            }
        }
        0
    }

    /// `decode_CABAC_bypass`.
    pub fn decode_bypass(&mut self) -> i32 {
        self.value <<= 1;
        self.bits_needed += 1;
        if self.bits_needed >= 0 {
            self.bits_needed = -8;
            if let Some(byte) = self.next_byte() {
                self.value |= byte;
            }
        }
        let scaled_range = self.range << 7;
        if self.value >= scaled_range {
            self.value -= scaled_range;
            1
        } else {
            0
        }
    }

    pub fn decode_tu_bypass(&mut self, max: i32) -> i32 {
        for i in 0..max {
            if self.decode_bypass() == 0 {
                return i;
            }
        }
        max
    }

    /// `decode_CABAC_FL_bypass_parallel`.
    fn decode_fl_bypass_parallel(&mut self, bits: i32) -> i32 {
        self.value <<= bits as u32;
        self.bits_needed += bits as i16;
        if self.bits_needed >= 0
            && let Some(byte) = self.next_byte()
        {
            let input = (byte as i32) << self.bits_needed;
            self.bits_needed -= 8;
            self.value |= input as u32;
        }
        let scaled_range = self.range << 7;
        let mut value = (self.value / scaled_range) as i32;
        if value >= (1 << bits) {
            value = (1 << bits) - 1;
        }
        self.value = self.value.wrapping_sub((value as u32).wrapping_mul(scaled_range));
        value
    }

    /// `decode_CABAC_FL_bypass`.
    pub fn decode_fl_bypass(&mut self, bits: i32) -> u32 {
        if bits <= 8 {
            if bits == 0 {
                return 0;
            }
            return self.decode_fl_bypass_parallel(bits) as u32;
        }
        let mut value = self.decode_fl_bypass_parallel(8) as u32;
        for _ in 0..bits - 8 {
            value <<= 1;
            value |= self.decode_bypass() as u32;
        }
        value
    }

    /// `decode_CABAC_EGk_bypass`.
    pub fn decode_egk_bypass(&mut self, k: i32) -> i32 {
        let mut base = 0i32;
        let mut n = k;
        loop {
            if self.decode_bypass() == 0 {
                break;
            }
            base = base.wrapping_add(1i32.wrapping_shl(n as u32));
            n += 1;
            if n == k + 32 {
                return 0;
            }
        }
        let suffix = self.decode_fl_bypass(n) as i32;
        base.wrapping_add(suffix)
    }
}
