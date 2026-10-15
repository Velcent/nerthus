//! Slice segment data (`slice.cc`, `transform.cc`): the coding tree,
//! coding units, transform tree and residuals of intra pictures, with
//! the dequantisation and inverse transforms that rebuild the samples.

use std::sync::OnceLock;

use crate::bitreader::BitReader;
use crate::cabac::{self, CabacDecoder, ContextModel, ContextTable, TABLE_LENGTH};
use crate::intra::{decode_intra_prediction, intra_pred_mode_candidates, intra_scan_idx, INTRA_ANGULAR_10, INTRA_ANGULAR_26, INTRA_ANGULAR_34, INTRA_DC, INTRA_PLANAR};
use crate::params::{CHROMA_420, CHROMA_422, CHROMA_444, CHROMA_MONO};
use crate::picture::{Picture, SaoInfo, MODE_INTER, MODE_INTRA, PART_2NX2N, PART_NXN};
use crate::scan::{scan_order, scan_position};
use crate::slice_header::{SliceHeader, SLICE_TYPE_I};
use crate::transform;
use crate::util::clip3;

const TABLE_8_22: [i32; 13] = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37];
const LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 72];
const MAP_CHROMA_422: [i32; 35] = [0, 1, 2, 2, 2, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27, 27, 28, 28, 29, 29, 30, 31];
const CTX_IDX_MAP: [u8; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 99];
const MAX_PREFIX: i32 = 64;

/// `table8_22`: chroma QP from the luma-derived index (4:2:0).
pub fn table_8_22(qp_i: i32) -> i32 {
    if qp_i < 30 {
        qp_i
    } else if qp_i >= 43 {
        qp_i - 6
    } else {
        TABLE_8_22[(qp_i - 30) as usize]
    }
}

/// `ctxIdxLookup`: the significance context of each position, by block
/// size, luma/chroma, diagonal/other scan and the coded neighbours.
fn significance_lookup() -> &'static Vec<Vec<u8>> {
    static TABLE: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut tables = Vec::with_capacity(4 * 2 * 2 * 4);
        for log2w in 2..=5 {
            for c_idx in 0..2 {
                for scan_idx in 0..2 {
                    for prev_csbf in 0..4 {
                        let width = 1 << log2w;
                        let sub_block_width = width >> 2;
                        let mut table = vec![0u8; width * width];
                        for y_c in 0..width {
                            for x_c in 0..width {
                                let sig_ctx = if sub_block_width == 1 {
                                    CTX_IDX_MAP[(y_c << 2) + x_c] as i32
                                } else if x_c + y_c == 0 {
                                    0
                                } else {
                                    let x_s = x_c >> 2;
                                    let y_s = y_c >> 2;
                                    let x_p = x_c & 3;
                                    let y_p = y_c & 3;
                                    let mut context = match prev_csbf {
                                        0 => {
                                            if x_p + y_p >= 3 {
                                                0
                                            } else if x_p + y_p > 0 {
                                                1
                                            } else {
                                                2
                                            }
                                        }
                                        1 => match y_p {
                                            0 => 2,
                                            1 => 1,
                                            _ => 0,
                                        },
                                        2 => match x_p {
                                            0 => 2,
                                            1 => 1,
                                            _ => 0,
                                        },
                                        _ => 2,
                                    };
                                    if c_idx == 0 {
                                        if x_s + y_s > 0 {
                                            context += 3;
                                        }
                                        if sub_block_width == 2 {
                                            context += if scan_idx == 0 { 9 } else { 15 };
                                        } else {
                                            context += 21;
                                        }
                                    } else if sub_block_width == 2 {
                                        context += 9;
                                    } else {
                                        context += 12;
                                    }
                                    context
                                };
                                let increment = if c_idx == 0 { sig_ctx } else { 27 + sig_ctx };
                                table[x_c + (y_c << log2w)] = increment as u8;
                            }
                        }
                        tables.push(table);
                    }
                }
            }
        }
        tables
    })
}

fn significance_table(log2w: i32, c_idx: usize, scan_idx: usize, prev_csbf: usize) -> &'static [u8] {
    let index = (((log2w as usize * 2) + (c_idx != 0) as usize) * 2 + (scan_idx != 0) as usize) * 4 + prev_csbf;
    &significance_lookup()[index]
}

/// `map_chroma_pred_mode`.
fn map_chroma_pred_mode(intra_chroma_pred_mode: i32, intra_pred_mode: i32) -> i32 {
    if intra_chroma_pred_mode == 4 {
        return intra_pred_mode;
    }
    const CANDIDATES: [i32; 4] = [INTRA_PLANAR, INTRA_ANGULAR_26, INTRA_ANGULAR_10, INTRA_DC];
    let candidate = CANDIDATES[intra_chroma_pred_mode as usize];
    if candidate == intra_pred_mode { INTRA_ANGULAR_34 } else { candidate }
}

pub enum DecodeResult {
    EndOfSliceSegment,
    EndOfSubstream,
    Error,
}

/// `thread_context`: the state of decoding one substream.
pub struct ThreadContext<'a> {
    pub picture: &'a mut Picture,
    pub header: &'a SliceHeader,

    pub ctb_addr_in_rs: i32,
    pub ctb_addr_in_ts: i32,
    pub ctb_x: i32,
    pub ctb_y: i32,

    pub cabac: CabacDecoder,
    /// `ctx_model`, `None` when it holds no table.
    pub models: Option<Box<ContextTable>>,
    pub stat_coeff: [i32; 4],
    /// The models at the end of the segment, kept for a dependent one.
    pub stored_models: Option<Box<ContextTable>>,

    current_qg_x: i32,
    current_qg_y: i32,
    last_qpy_in_previous_qg: i32,
    pub current_qpy: i32,
    qp_y_prime: i32,
    qp_cb_prime: i32,
    qp_cr_prime: i32,

    is_cu_qp_delta_coded: bool,
    cu_qp_delta: i32,
    is_cu_chroma_qp_offset_coded: bool,
    cu_qp_offset_cb: i32,
    cu_qp_offset_cr: i32,

    cu_transquant_bypass_flag: bool,
    transform_skip_flag: [bool; 3],
    explicit_rdpcm_flag: bool,
    explicit_rdpcm_dir: bool,

    coeff_list: Box<[[i16; 1024]; 3]>,
    coeff_pos: Box<[[i16; 1024]; 3]>,
    n_coeff: [usize; 3],
    coeff_buf: Box<[i16; 1024]>,
    residual_luma: Box<[i32; 1024]>,
    res_scale_val: i32,
}

impl<'a> ThreadContext<'a> {
    pub fn new(picture: &'a mut Picture, header: &'a SliceHeader, cabac: CabacDecoder, ctb_addr_in_ts: i32) -> ThreadContext<'a> {
        let slice_qp_y = header.slice_qp_y;
        let mut context = ThreadContext {
            picture,
            header,
            ctb_addr_in_rs: 0,
            ctb_addr_in_ts,
            ctb_x: 0,
            ctb_y: 0,
            cabac,
            models: None,
            stat_coeff: [0; 4],
            stored_models: None,
            current_qg_x: -1,
            current_qg_y: -1,
            last_qpy_in_previous_qg: slice_qp_y,
            current_qpy: slice_qp_y,
            qp_y_prime: 0,
            qp_cb_prime: 0,
            qp_cr_prime: 0,
            is_cu_qp_delta_coded: false,
            cu_qp_delta: 0,
            is_cu_chroma_qp_offset_coded: false,
            cu_qp_offset_cb: 0,
            cu_qp_offset_cr: 0,
            cu_transquant_bypass_flag: false,
            transform_skip_flag: [false; 3],
            explicit_rdpcm_flag: false,
            explicit_rdpcm_dir: false,
            coeff_list: Box::new([[0; 1024]; 3]),
            coeff_pos: Box::new([[0; 1024]; 3]),
            n_coeff: [0; 3],
            coeff_buf: Box::new([0; 1024]),
            residual_luma: Box::new([0; 1024]),
            res_scale_val: 0,
        };
        context.init_thread_context();
        context
    }

    /// `init_thread_context`: the QP at the end of the previous slice.
    fn init_thread_context(&mut self) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let address = self.header.slice_segment_address;
        if address > 0 {
            let previous_ctb = pps.ctb_addr_ts_to_rs[(pps.ctb_addr_rs_to_ts[address as usize] - 1) as usize];
            let ctb_x = previous_ctb % sps.pic_width_in_ctbs_y;
            let ctb_y = previous_ctb / sps.pic_width_in_ctbs_y;
            let x = (((ctb_x + 1) << sps.log2_ctb_size_y) - 1).min(sps.pic_width_in_luma_samples - 1);
            let y = (((ctb_y + 1) << sps.log2_ctb_size_y) - 1).min(sps.pic_height_in_luma_samples - 1);
            self.current_qpy = self.picture.qpy(x, y);
        }
    }

    /// `setCtbAddrFromTS`: true past the end of the picture.
    pub fn set_ctb_addr_from_ts(&mut self) -> bool {
        let sps = &self.picture.sps;
        if self.ctb_addr_in_ts < sps.pic_size_in_ctbs_y {
            self.ctb_addr_in_rs = self.picture.pps.ctb_addr_ts_to_rs[self.ctb_addr_in_ts as usize];
            self.ctb_x = self.ctb_addr_in_rs % sps.pic_width_in_ctbs_y;
            self.ctb_y = self.ctb_addr_in_rs / sps.pic_width_in_ctbs_y;
            false
        } else {
            self.ctb_addr_in_rs = sps.pic_size_in_ctbs_y;
            self.ctb_x = self.ctb_addr_in_rs % sps.pic_width_in_ctbs_y;
            self.ctb_y = self.ctb_addr_in_rs / sps.pic_width_in_ctbs_y;
            true
        }
    }

    fn advance_ctb_addr(&mut self) -> bool {
        self.ctb_addr_in_ts += 1;
        self.set_ctb_addr_from_ts()
    }

    /// `initialize_CABAC_models`.
    pub fn initialize_models(&mut self) {
        let mut table = self.models.take().unwrap_or_else(|| Box::new([ContextModel::default(); TABLE_LENGTH]));
        cabac::initialize_models(&mut table, self.header.init_type as usize, self.header.slice_qp_y);
        self.models = Some(table);
        self.stat_coeff = [0; 4];
    }

    fn bit(&mut self, context: usize) -> i32 {
        let models = self.models.as_mut().expect("context models");
        self.cabac.decode_bit(&mut models[context])
    }

    fn bypass(&mut self) -> i32 {
        self.cabac.decode_bypass()
    }

    // --- syntax elements ---

    fn decode_sao_type_idx(&mut self) -> i32 {
        if self.bit(cabac::SAO_TYPE_IDX) == 0 {
            return 0;
        }
        if self.bypass() == 0 { 1 } else { 2 }
    }

    fn decode_split_cu_flag(&mut self, x0: i32, y0: i32, ct_depth: i32) -> i32 {
        let available_l = self.picture.check_ctb_available(x0, y0, x0 - 1, y0);
        let available_a = self.picture.check_ctb_available(x0, y0, x0, y0 - 1);
        let mut context = 0;
        if available_l && self.picture.ct_depth(x0 - 1, y0) > ct_depth {
            context += 1;
        }
        if available_a && self.picture.ct_depth(x0, y0 - 1) > ct_depth {
            context += 1;
        }
        self.bit(cabac::SPLIT_CU_FLAG + context)
    }

    fn decode_part_mode_intra(&mut self) -> u8 {
        if self.bit(cabac::PART_MODE) != 0 { PART_2NX2N } else { PART_NXN }
    }

    fn decode_intra_chroma_pred_mode(&mut self) -> i32 {
        if self.bit(cabac::INTRA_CHROMA_PRED_MODE) == 0 {
            return 4;
        }
        self.cabac.decode_fl_bypass(2) as i32
    }

    fn decode_cu_qp_delta_abs(&mut self) -> i32 {
        if self.bit(cabac::CU_QP_DELTA_ABS) == 0 {
            return 0;
        }
        let mut prefix = 1;
        for _ in 0..4 {
            if self.bit(cabac::CU_QP_DELTA_ABS + 1) == 0 {
                break;
            }
            prefix += 1;
        }
        if prefix == 5 {
            let value = self.cabac.decode_egk_bypass(0);
            return value.wrapping_add(5);
        }
        prefix
    }

    fn decode_last_significant_coeff_prefix(&mut self, log2_trafo_size: i32, c_idx: usize, base: usize) -> i32 {
        let maximum = (log2_trafo_size << 1) - 1;
        let (offset, shift) = if c_idx == 0 { (3 * (log2_trafo_size - 2) + ((log2_trafo_size - 1) >> 2), (log2_trafo_size + 1) >> 2) } else { (15, log2_trafo_size - 2) };
        for bin in 0..maximum {
            let context = base + (offset + (bin >> shift)) as usize;
            if self.bit(context) == 0 {
                return bin;
            }
        }
        maximum
    }

    fn decode_coeff_abs_level_remaining(&mut self, rice: i32) -> i32 {
        let mut prefix = -1;
        loop {
            prefix += 1;
            let codeword = self.bypass();
            if prefix > MAX_PREFIX {
                return 0;
            }
            if codeword == 0 {
                break;
            }
        }
        if prefix <= 3 {
            let codeword = self.cabac.decode_fl_bypass(rice) as i32;
            (prefix << rice).wrapping_add(codeword)
        } else {
            let codeword = self.cabac.decode_fl_bypass(prefix - 3 + rice) as i32;
            let base = 1i32.wrapping_shl((prefix - 3) as u32).wrapping_add(3 - 1);
            base.wrapping_shl(rice as u32).wrapping_add(codeword)
        }
    }

    // --- coding tree ---

    /// `read_sao`.
    fn read_sao(&mut self, x_ctb: i32, y_ctb: i32) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let width = sps.pic_width_in_ctbs_y;
        let mut info = SaoInfo::default();
        let mut merge_left = false;
        let mut merge_up = false;

        if x_ctb > 0 {
            let left_in_slice = self.ctb_addr_in_rs > self.header.slice_addr_rs;
            let left_in_tile = pps.tile_id_rs[(x_ctb + y_ctb * width) as usize] == pps.tile_id_rs[(x_ctb - 1 + y_ctb * width) as usize];
            if left_in_slice && left_in_tile {
                merge_left = self.bit(cabac::SAO_MERGE_FLAG) != 0;
            }
        }
        if y_ctb > 0 && !merge_left {
            let up_in_slice = self.ctb_addr_in_rs - width >= self.header.slice_addr_rs;
            let up_in_tile = pps.tile_id_rs[(x_ctb + y_ctb * width) as usize] == pps.tile_id_rs[(x_ctb + (y_ctb - 1) * width) as usize];
            if up_in_slice && up_in_tile {
                merge_up = self.bit(cabac::SAO_MERGE_FLAG) != 0;
            }
        }

        if !merge_up && !merge_left {
            let components = if sps.chroma_array_type == CHROMA_MONO { 1 } else { 3 };
            for c_idx in 0..components {
                let enabled = (self.header.slice_sao_luma_flag && c_idx == 0) || (self.header.slice_sao_chroma_flag && c_idx > 0);
                if !enabled {
                    continue;
                }
                let type_idx = match c_idx {
                    0 => {
                        let value = self.decode_sao_type_idx();
                        info.sao_type_idx = value as u8;
                        value
                    }
                    1 => {
                        let value = self.decode_sao_type_idx();
                        info.sao_type_idx |= (value << 2) as u8;
                        info.sao_type_idx |= (value << 4) as u8;
                        value
                    }
                    _ => ((info.sao_type_idx >> (2 * c_idx)) & 3) as i32,
                };
                if type_idx == 0 {
                    continue;
                }
                let bit_depth = self.picture.bit_depth(c_idx);
                let maximum = (1 << (bit_depth.min(10) - 5)) - 1;
                let mut offsets = [0i32; 4];
                for offset in offsets.iter_mut() {
                    *offset = self.cabac.decode_tu_bypass(maximum);
                }
                let mut signs = [0i32; 4];
                if type_idx == 1 {
                    for index in 0..4 {
                        if offsets[index] != 0 {
                            signs[index] = if self.bypass() != 0 { -1 } else { 1 };
                        }
                    }
                    info.sao_band_position[c_idx] = self.cabac.decode_fl_bypass(5) as u8;
                } else {
                    signs = [1, 1, -1, -1];
                    if c_idx == 0 {
                        info.sao_eo_class = self.cabac.decode_fl_bypass(2) as u8;
                    } else if c_idx == 1 {
                        let class = self.cabac.decode_fl_bypass(2) as u8;
                        info.sao_eo_class |= class << 2;
                        info.sao_eo_class |= class << 4;
                    }
                }
                let log2_offset_scale = if c_idx == 0 { pps.range_extension.log2_sao_offset_scale_luma } else { pps.range_extension.log2_sao_offset_scale_chroma };
                for index in 0..4 {
                    let scaled = (offsets[index] as i8 as i32) << log2_offset_scale;
                    info.sao_offset_val[c_idx][index] = signs[index].wrapping_mul(scaled) as i8;
                }
            }
            self.picture.set_sao_info(x_ctb, y_ctb, info);
        }
        if merge_left {
            let left = self.picture.sao_info(x_ctb - 1, y_ctb);
            self.picture.set_sao_info(x_ctb, y_ctb, left);
        }
        if merge_up {
            let up = self.picture.sao_info(x_ctb, y_ctb - 1);
            self.picture.set_sao_info(x_ctb, y_ctb, up);
        }
    }

    /// `read_coding_tree_unit`.
    fn read_coding_tree_unit(&mut self) {
        let sps = self.picture.sps.clone();
        let x_ctb = self.ctb_addr_in_rs % sps.pic_width_in_ctbs_y;
        let y_ctb = self.ctb_addr_in_rs / sps.pic_width_in_ctbs_y;
        let x_pixels = x_ctb << sps.log2_ctb_size_y;
        let y_pixels = y_ctb << sps.log2_ctb_size_y;
        self.picture.set_slice_addr_rs(x_ctb, y_ctb, self.header.slice_addr_rs);
        self.picture.set_slice_header_index(x_pixels, y_pixels, self.header.slice_index);
        if self.header.slice_sao_luma_flag || self.header.slice_sao_chroma_flag {
            self.read_sao(x_ctb, y_ctb);
        }
        self.read_coding_quadtree(x_pixels, y_pixels, sps.log2_ctb_size_y, 0);
    }

    /// `read_coding_quadtree`.
    fn read_coding_quadtree(&mut self, x0: i32, y0: i32, log2_cb_size: i32, ct_depth: i32) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let size = 1 << log2_cb_size;
        let inside = x0 + size <= sps.pic_width_in_luma_samples && y0 + size <= sps.pic_height_in_luma_samples;
        let split = if inside && log2_cb_size > sps.log2_min_cb_size_y {
            self.decode_split_cu_flag(x0, y0, ct_depth) != 0
        } else {
            log2_cb_size > sps.log2_min_cb_size_y
        };

        if pps.cu_qp_delta_enabled_flag && log2_cb_size >= pps.log2_min_cu_qp_delta_size {
            self.is_cu_qp_delta_coded = false;
            self.cu_qp_delta = 0;
        }
        if self.header.cu_chroma_qp_offset_enabled_flag && log2_cb_size >= pps.log2_min_cu_chroma_qp_offset_size {
            self.is_cu_chroma_qp_offset_coded = false;
        }

        if split {
            let x1 = x0 + (1 << (log2_cb_size - 1));
            let y1 = y0 + (1 << (log2_cb_size - 1));
            self.read_coding_quadtree(x0, y0, log2_cb_size - 1, ct_depth + 1);
            if x1 < sps.pic_width_in_luma_samples {
                self.read_coding_quadtree(x1, y0, log2_cb_size - 1, ct_depth + 1);
            }
            if y1 < sps.pic_height_in_luma_samples {
                self.read_coding_quadtree(x0, y1, log2_cb_size - 1, ct_depth + 1);
            }
            if x1 < sps.pic_width_in_luma_samples && y1 < sps.pic_height_in_luma_samples {
                self.read_coding_quadtree(x1, y1, log2_cb_size - 1, ct_depth + 1);
            }
        } else {
            self.picture.set_ct_depth(x0, y0, log2_cb_size, ct_depth);
            self.read_coding_unit(x0, y0, log2_cb_size);
        }
    }

    /// `read_pcm_samples`.
    fn read_pcm_samples(&mut self, x0: i32, y0: i32, log2_cb_size: i32) {
        let data = self.cabac.shared_data();
        let start = self.cabac.position.min(self.cabac.end);
        let bytes = &data[start..self.cabac.end];
        let mut reader = BitReader::new(bytes);
        self.read_pcm_component(&mut reader, x0, y0, log2_cb_size, 0);
        if self.picture.sps.chroma_array_type != CHROMA_MONO {
            self.read_pcm_component(&mut reader, x0, y0, log2_cb_size, 1);
            self.read_pcm_component(&mut reader, x0, y0, log2_cb_size, 2);
        }
        reader.prepare_for_cabac();
        self.cabac.position = start + reader.position();
        self.cabac.start();
    }

    fn read_pcm_component(&mut self, reader: &mut BitReader, x0: i32, y0: i32, log2_cb_size: i32, c_idx: usize) {
        let sps = self.picture.sps.clone();
        let mut width = 1 << log2_cb_size;
        let mut height = 1 << log2_cb_size;
        let (mut x, mut y) = (x0, y0);
        let (pcm_bits, bit_depth) = if c_idx > 0 {
            width /= sps.sub_width_c;
            height /= sps.sub_height_c;
            x /= sps.sub_width_c;
            y /= sps.sub_height_c;
            (sps.pcm_sample_bit_depth_chroma, sps.bit_depth_c)
        } else {
            (sps.pcm_sample_bit_depth_luma, sps.bit_depth_y)
        };
        let shift = (bit_depth - pcm_bits).max(0);
        for dy in 0..height {
            for dx in 0..width {
                let value = reader.get_bits(pcm_bits);
                self.picture.set_sample(c_idx, x + dx, y + dy, value.wrapping_shl(shift as u32));
            }
        }
    }

    /// `read_coding_unit` (intra slices).
    fn read_coding_unit(&mut self, x0: i32, y0: i32, log2_cb_size: i32) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();

        self.picture.set_log2_cb_size(x0, y0, log2_cb_size);
        self.picture.clear_split_transform_flags(x0, y0, log2_cb_size);
        let cb_size = 1 << log2_cb_size;

        self.decode_quantization_parameters(x0, y0, x0, y0);

        if pps.transquant_bypass_enable_flag {
            let bypass = self.bit(cabac::CU_TRANSQUANT_BYPASS_FLAG) != 0;
            self.cu_transquant_bypass_flag = bypass;
            if bypass {
                self.picture.set_cu_transquant_bypass(x0, y0, log2_cb_size);
            }
        } else {
            self.cu_transquant_bypass_flag = false;
        }

        // Inter slices are refused before decoding, so every CU is intra.
        let pred_mode = MODE_INTRA;
        self.picture.set_pred_mode(x0, y0, log2_cb_size, pred_mode);

        let mut intra_split = false;
        let part_mode = if log2_cb_size == sps.log2_min_cb_size_y {
            let mode = self.decode_part_mode_intra();
            if mode == PART_NXN {
                intra_split = true;
            }
            mode
        } else {
            PART_2NX2N
        };
        self.picture.set_part_mode(x0, y0, part_mode);

        let mut pcm = false;
        let pcm_size_allowed = log2_cb_size >= sps.log2_min_ipcm_cb_size_y && log2_cb_size <= sps.log2_max_ipcm_cb_size_y;
        if part_mode == PART_2NX2N && sps.pcm_enabled_flag && pcm_size_allowed {
            pcm = self.cabac.decode_term_bit() != 0;
        }

        if pcm {
            self.picture.set_pcm_flag(x0, y0, log2_cb_size);
            self.read_pcm_samples(x0, y0, log2_cb_size);
            return;
        }

        self.read_intra_modes(x0, y0, log2_cb_size, part_mode);

        let max_trafo_depth = sps.max_transform_hierarchy_depth_intra + intra_split as i32;
        let initial_chroma_cbf = if sps.chroma_array_type == CHROMA_MONO { 0 } else { 1 };
        let tree = TransformTree { x_base: x0, y_base: y0, x_cu_base: x0, y_cu_base: y0, max_trafo_depth, intra_split };
        self.read_transform_tree(&tree, x0, y0, log2_cb_size, 0, 0, initial_chroma_cbf, initial_chroma_cbf);
        let _ = cb_size;
    }

    /// The luma and chroma intra prediction modes of a coding unit.
    fn read_intra_modes(&mut self, x0: i32, y0: i32, log2_cb_size: i32, part_mode: u8) {
        let sps = self.picture.sps.clone();
        let cb_size = 1 << log2_cb_size;
        let pb_offset = if part_mode == PART_NXN { cb_size / 2 } else { cb_size };
        let log2_intra_pred_size = if part_mode == PART_NXN { log2_cb_size - 1 } else { log2_cb_size };

        let mut prev_intra_luma_pred_flag = [false; 4];
        let mut count = 0;
        let mut j = 0;
        while j < cb_size {
            let mut i = 0;
            while i < cb_size {
                prev_intra_luma_pred_flag[count] = self.bit(cabac::PREV_INTRA_LUMA_PRED_FLAG) != 0;
                count += 1;
                i += pb_offset;
            }
            j += pb_offset;
        }

        let available_a0 = self.picture.check_ctb_available(x0, y0, x0 - 1, y0);
        let available_b0 = self.picture.check_ctb_available(x0, y0, x0, y0 - 1);

        let mut index = 0;
        let mut j = 0;
        while j < cb_size {
            let mut i = 0;
            while i < cb_size {
                let mut mpm_idx = 0;
                let mut rem_mode = 0;
                if prev_intra_luma_pred_flag[index] {
                    mpm_idx = self.cabac.decode_tu_bypass(2);
                } else {
                    rem_mode = self.cabac.decode_fl_bypass(5) as i32;
                }
                let x = x0 + i;
                let y = y0 + j;
                let available_a = available_a0 || i > 0;
                let available_b = available_b0 || j > 0;
                let pu_index = (x >> sps.log2_min_pu_size) + (y >> sps.log2_min_pu_size) * sps.pic_width_in_min_pus;
                let mut candidates = intra_pred_mode_candidates(self.picture, x, y, pu_index, available_a, available_b);
                let mode = if prev_intra_luma_pred_flag[index] {
                    candidates[mpm_idx as usize]
                } else {
                    if candidates[0] > candidates[1] {
                        candidates.swap(0, 1);
                    }
                    if candidates[0] > candidates[2] {
                        candidates.swap(0, 2);
                    }
                    if candidates[1] > candidates[2] {
                        candidates.swap(1, 2);
                    }
                    let mut mode = rem_mode;
                    for candidate in candidates {
                        if mode >= candidate {
                            mode += 1;
                        }
                    }
                    mode
                };
                self.picture.set_intra_pred_mode(pu_index, log2_intra_pred_size, mode);
                index += 1;
                i += pb_offset;
            }
            j += pb_offset;
        }

        if sps.chroma_array_type == CHROMA_444 {
            let mut j = 0;
            while j < cb_size {
                let mut i = 0;
                while i < cb_size {
                    let x = x0 + i;
                    let y = y0 + j;
                    let chroma_mode = self.decode_intra_chroma_pred_mode();
                    let luma_mode = self.picture.intra_pred_mode(x, y);
                    let mode_c = map_chroma_pred_mode(chroma_mode, luma_mode);
                    self.picture.set_intra_pred_mode_c(x, y, log2_intra_pred_size, mode_c, chroma_mode == 4);
                    i += pb_offset;
                }
                j += pb_offset;
            }
        } else if sps.chroma_array_type != CHROMA_MONO {
            let chroma_mode = self.decode_intra_chroma_pred_mode();
            let luma_mode = self.picture.intra_pred_mode(x0, y0);
            let mut mode_c = map_chroma_pred_mode(chroma_mode, luma_mode);
            if sps.chroma_array_type == CHROMA_422 {
                mode_c = MAP_CHROMA_422[mode_c as usize];
            }
            self.picture.set_intra_pred_mode_c(x0, y0, log2_cb_size, mode_c, chroma_mode == 4);
        }
    }

    /// `read_transform_tree`.
    #[allow(clippy::too_many_arguments)]
    fn read_transform_tree(&mut self, tree: &TransformTree, x0: i32, y0: i32, log2_trafo_size: i32, trafo_depth: i32, blk_idx: i32, parent_cbf_cb: i32, parent_cbf_cr: i32) {
        let sps = self.picture.sps.clone();
        let readable = log2_trafo_size <= sps.log2_max_trafo_size && log2_trafo_size > sps.log2_min_trafo_size && trafo_depth < tree.max_trafo_depth && !(tree.intra_split && trafo_depth == 0);
        let split = if readable {
            let context = (5 - log2_trafo_size) as usize;
            self.bit(cabac::SPLIT_TRANSFORM_FLAG + context) != 0
        } else {
            // Intra CUs never take the inter split.
            log2_trafo_size > sps.log2_max_trafo_size || (tree.intra_split && trafo_depth == 0)
        };
        if split {
            self.picture.set_split_transform_flag(x0, y0, trafo_depth);
        }

        let mut cbf_cb = -1;
        let mut cbf_cr = -1;
        let chroma_present = (log2_trafo_size > 2 && sps.chroma_array_type != CHROMA_MONO) || sps.chroma_array_type == CHROMA_444;
        if chroma_present {
            let second_bit = sps.chroma_array_type == CHROMA_422 && (!split || log2_trafo_size == 3);
            if parent_cbf_cb != 0 {
                cbf_cb = self.bit(cabac::CBF_CHROMA + trafo_depth as usize);
                if second_bit {
                    cbf_cb |= self.bit(cabac::CBF_CHROMA + trafo_depth as usize) << 1;
                }
            }
            if parent_cbf_cr != 0 {
                cbf_cr = self.bit(cabac::CBF_CHROMA + trafo_depth as usize);
                if second_bit {
                    cbf_cr |= self.bit(cabac::CBF_CHROMA + trafo_depth as usize) << 1;
                }
            }
        }
        if cbf_cb < 0 {
            cbf_cb = if trafo_depth > 0 && log2_trafo_size == 2 { parent_cbf_cb } else { 0 };
        }
        if cbf_cr < 0 {
            cbf_cr = if trafo_depth > 0 && log2_trafo_size == 2 { parent_cbf_cr } else { 0 };
        }

        if split {
            let x1 = x0 + (1 << (log2_trafo_size - 1));
            let y1 = y0 + (1 << (log2_trafo_size - 1));
            let child = TransformTree { x_base: x0, y_base: y0, ..*tree };
            self.read_transform_tree(&child, x0, y0, log2_trafo_size - 1, trafo_depth + 1, 0, cbf_cb, cbf_cr);
            self.read_transform_tree(&child, x1, y0, log2_trafo_size - 1, trafo_depth + 1, 1, cbf_cb, cbf_cr);
            self.read_transform_tree(&child, x0, y1, log2_trafo_size - 1, trafo_depth + 1, 2, cbf_cb, cbf_cr);
            self.read_transform_tree(&child, x1, y1, log2_trafo_size - 1, trafo_depth + 1, 3, cbf_cb, cbf_cr);
        } else {
            let context = (trafo_depth == 0) as usize;
            let cbf_luma = self.bit(cabac::CBF_LUMA + context);
            self.read_transform_unit(tree, x0, y0, log2_trafo_size, blk_idx, cbf_luma, cbf_cb, cbf_cr);
        }
    }

    fn read_cross_comp_pred(&mut self, c_idx_minus1: usize) {
        let mut log2_res_scale_abs_plus1 = 0;
        for bin in 0..4 {
            let context = cabac::LOG2_RES_SCALE_ABS_PLUS1 + 4 * c_idx_minus1 + bin;
            if self.bit(context) == 0 {
                break;
            }
            log2_res_scale_abs_plus1 += 1;
        }
        self.res_scale_val = if log2_res_scale_abs_plus1 != 0 {
            let sign = self.bit(cabac::RES_SCALE_SIGN_FLAG + c_idx_minus1);
            (1 << (log2_res_scale_abs_plus1 - 1)) * (1 - 2 * sign)
        } else {
            0
        };
    }

    /// `read_transform_unit`.
    #[allow(clippy::too_many_arguments)]
    fn read_transform_unit(&mut self, tree: &TransformTree, x0: i32, y0: i32, log2_trafo_size: i32, blk_idx: i32, cbf_luma: i32, cbf_cb: i32, cbf_cr: i32) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let chroma_array_type = sps.chroma_array_type;
        let log2_trafo_size_c = (if chroma_array_type == CHROMA_444 { log2_trafo_size } else { log2_trafo_size - 1 }).max(2);
        let cbf_chroma = cbf_cb | cbf_cr;

        self.transform_skip_flag = [false; 3];
        self.explicit_rdpcm_flag = false;

        let pred_mode = self.picture.pred_mode(x0, y0);
        let (x_cu_base, y_cu_base) = (tree.x_cu_base, tree.y_cu_base);

        if cbf_luma != 0 || cbf_chroma != 0 {
            let mut decode_quant_parameters = false;
            if pps.cu_qp_delta_enabled_flag && !self.is_cu_qp_delta_coded {
                let delta_abs = self.decode_cu_qp_delta_abs();
                let mut delta_sign = 0;
                if delta_abs != 0 {
                    delta_sign = self.bypass();
                }
                self.is_cu_qp_delta_coded = true;
                self.cu_qp_delta = delta_abs.wrapping_mul(1 - 2 * delta_sign);
                decode_quant_parameters = true;
            }
            if self.header.cu_chroma_qp_offset_enabled_flag && cbf_chroma != 0 && !self.cu_transquant_bypass_flag && !self.is_cu_chroma_qp_offset_coded {
                let offset_flag = self.bit(cabac::CU_CHROMA_QP_OFFSET_FLAG) != 0;
                let mut offset_idx = 0;
                if offset_flag && pps.range_extension.chroma_qp_offset_list_len > 1 {
                    offset_idx = self.bit(cabac::CU_CHROMA_QP_OFFSET_IDX) as usize;
                }
                self.is_cu_chroma_qp_offset_coded = true;
                if offset_flag {
                    self.cu_qp_offset_cb = pps.range_extension.cb_qp_offset_list[offset_idx] as i32;
                    self.cu_qp_offset_cr = pps.range_extension.cr_qp_offset_list[offset_idx] as i32;
                } else {
                    self.cu_qp_offset_cb = 0;
                    self.cu_qp_offset_cr = 0;
                }
                decode_quant_parameters = true;
            }
            if decode_quant_parameters {
                self.decode_quantization_parameters(x0, y0, x_cu_base, y_cu_base);
            }
        }

        let nt = 1 << log2_trafo_size;
        let nt_c = 1 << log2_trafo_size_c;
        let sub_width_c = sps.sub_width_c;
        let sub_height_c = sps.sub_height_c;

        self.res_scale_val = 0;
        if cbf_luma != 0 {
            self.residual_coding(x0, y0, log2_trafo_size, 0);
        }
        self.decode_tu(x0, y0, x_cu_base, y_cu_base, nt, 0, pred_mode, cbf_luma != 0);

        if log2_trafo_size > 2 || chroma_array_type == CHROMA_444 {
            let cross_component = pps.range_extension.cross_component_prediction_enabled_flag && cbf_luma != 0 && (pred_mode == MODE_INTER || self.picture.is_intra_pred_mode_c_mode4(x0, y0));

            for c_idx in 1..3usize {
                let cbf = if c_idx == 1 { cbf_cb } else { cbf_cr };
                if cross_component {
                    self.read_cross_comp_pred(c_idx - 1);
                } else {
                    self.res_scale_val = 0;
                }
                if cbf & 1 != 0 {
                    self.residual_coding(x0, y0, log2_trafo_size_c, c_idx);
                }
                if sps.chroma_array_type != CHROMA_MONO {
                    self.decode_tu(x0 / sub_width_c, y0 / sub_height_c, x_cu_base / sub_width_c, y_cu_base / sub_height_c, nt_c, c_idx, pred_mode, cbf & 1 != 0);
                }
                if chroma_array_type == CHROMA_422 {
                    let y_offset = 1 << log2_trafo_size_c;
                    if cbf & 2 != 0 {
                        self.residual_coding(x0, y0 + y_offset * sub_height_c, log2_trafo_size_c, c_idx);
                    }
                    self.decode_tu(x0 / sub_width_c, y0 / sub_height_c + y_offset, x_cu_base / sub_width_c, y_cu_base / sub_height_c + y_offset, nt_c, c_idx, pred_mode, cbf & 2 != 0);
                }
            }
        } else if blk_idx == 3 {
            let (x_base, y_base) = (tree.x_base, tree.y_base);
            for c_idx in 1..3usize {
                let cbf = if c_idx == 1 { cbf_cb } else { cbf_cr };
                if cbf & 1 != 0 {
                    self.residual_coding(x_base, y_base, log2_trafo_size, c_idx);
                }
                if sps.chroma_array_type != CHROMA_MONO {
                    self.decode_tu(x_base / sub_width_c, y_base / sub_height_c, x_cu_base / sub_width_c, y_cu_base / sub_height_c, nt, c_idx, pred_mode, cbf & 1 != 0);
                }
                if cbf & 2 != 0 {
                    let y_offset = if c_idx == 1 { 1 << log2_trafo_size } else { 1 << log2_trafo_size_c };
                    self.residual_coding(x_base, y_base + y_offset, log2_trafo_size, c_idx);
                }
                if chroma_array_type == CHROMA_422 {
                    self.decode_tu(x_base / sub_width_c, y_base / sub_height_c + (1 << log2_trafo_size), x_cu_base / sub_width_c, y_cu_base / sub_height_c, nt, c_idx, pred_mode, cbf & 2 != 0);
                }
            }
        }
    }

    /// `residual_coding`: fills the coefficient list of `c_idx`.
    fn residual_coding(&mut self, x0: i32, y0: i32, log2_trafo_size: i32, c_idx: usize) {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let pred_mode = self.picture.pred_mode(x0, y0);

        if c_idx == 0 {
            self.picture.set_nonzero_coefficient(x0, y0, log2_trafo_size);
        }

        self.transform_skip_flag[c_idx] = if pps.transform_skip_enabled_flag && !self.cu_transquant_bypass_flag && log2_trafo_size <= pps.log2_max_transform_skip_size {
            let context = if c_idx == 0 { 0 } else { 1 };
            self.bit(cabac::TRANSFORM_SKIP_FLAG + context) != 0
        } else {
            false
        };

        self.explicit_rdpcm_flag = false;
        if pred_mode == MODE_INTER && sps.range_extension.explicit_rdpcm_enabled_flag && (self.transform_skip_flag[c_idx] || self.cu_transquant_bypass_flag) {
            let context = if c_idx != 0 { 1 } else { 0 };
            self.explicit_rdpcm_flag = self.bit(cabac::RDPCM_FLAG + context) != 0;
            if self.explicit_rdpcm_flag {
                self.explicit_rdpcm_dir = self.bit(cabac::RDPCM_DIR + context) != 0;
            }
        }

        let skip_or_bypass = self.transform_skip_flag[c_idx] || self.cu_transquant_bypass_flag;
        let sb_type = if c_idx == 0 { 2 } else { 0 } + skip_or_bypass as usize;

        let x_prefix = self.decode_last_significant_coeff_prefix(log2_trafo_size, c_idx, cabac::LAST_SIGNIFICANT_COEFFICIENT_X_PREFIX);
        let y_prefix = self.decode_last_significant_coeff_prefix(log2_trafo_size, c_idx, cabac::LAST_SIGNIFICANT_COEFFICIENT_Y_PREFIX);
        let mut last_x = x_prefix;
        if x_prefix > 3 {
            let bits = (x_prefix >> 1) - 1;
            let suffix = self.cabac.decode_fl_bypass(bits) as i32;
            last_x = ((2 + (x_prefix & 1)) << bits) + suffix;
        }
        let mut last_y = y_prefix;
        if y_prefix > 3 {
            let bits = (y_prefix >> 1) - 1;
            let suffix = self.cabac.decode_fl_bypass(bits) as i32;
            last_y = ((2 + (y_prefix & 1)) << bits) + suffix;
        }

        let scan_idx = if pred_mode == MODE_INTRA {
            let mode = if c_idx == 0 { self.picture.intra_pred_mode(x0, y0) } else { self.picture.intra_pred_mode_c(x0, y0) };
            intra_scan_idx(log2_trafo_size, mode, c_idx, &sps)
        } else {
            0
        };
        if scan_idx == 2 {
            std::mem::swap(&mut last_x, &mut last_y);
        }

        let scan_sub = scan_order((log2_trafo_size - 2) as usize, scan_idx);
        let scan_pos = scan_order(2, scan_idx);
        let last = scan_position(last_x as usize, last_y as usize, scan_idx, log2_trafo_size as usize);
        let last_scan_pos = last.scan_pos as i32;
        let last_sub_block = last.sub_block as i32;

        let sub_block_width = 1usize << (log2_trafo_size - 2);
        let mut neighbours = [0u8; 64];
        let mut c1 = 1;
        let mut first_sub_block = true;
        let mut last_sub_block_greater1_ctx = 0;
        let coeff_stride = 1 << log2_trafo_size;
        let mut last_greater1_ctx = 0;
        let mut last_greater1_flag = 0;
        let mut last_ctx_set = 0;

        let transform_skip_context = sps.range_extension.transform_skip_context_enabled_flag && skip_or_bypass;
        let sign_hiding_off = self.cu_transquant_bypass_flag || self.explicit_rdpcm_flag;
        let intra_mode = if c_idx == 0 { self.picture.intra_pred_mode(x0, y0) } else { self.picture.intra_pred_mode_c(x0, y0) };
        let implicit_rdpcm = pred_mode == MODE_INTRA && sps.range_extension.implicit_rdpcm_enabled_flag && self.transform_skip_flag[c_idx] && (intra_mode == 10 || intra_mode == 26);

        self.n_coeff[c_idx] = 0;

        let mut i = last_sub_block;
        while i >= 0 {
            let sub_block = scan_sub[i as usize];
            let (s_x, s_y) = (sub_block.x as usize, sub_block.y as usize);
            let mut infer_sb_dc_sig_coeff = false;
            let mut coded = false;
            if i < last_sub_block && i > 0 {
                let flags = neighbours[s_x + s_y * sub_block_width];
                let csbf_ctx = ((flags & 1) | (flags >> 1)) as usize;
                let context = csbf_ctx + if c_idx != 0 { 2 } else { 0 };
                coded = self.bit(cabac::CODED_SUB_BLOCK_FLAG + context) != 0;
                infer_sb_dc_sig_coeff = true;
            } else if i == 0 || i == last_sub_block {
                coded = true;
            }
            if coded {
                if s_x > 0 {
                    neighbours[s_x - 1 + s_y * sub_block_width] |= 1;
                }
                if s_y > 0 {
                    neighbours[s_x + (s_y - 1) * sub_block_width] |= 2;
                }
            }

            let mut coeff_value = [0i32; 16];
            let mut coeff_scan_pos = [0i32; 16];
            let mut coeff_sign = [0i32; 16];
            let mut coeff_has_max_base_level = [false; 16];
            let mut count = 0usize;

            if coded {
                let x_s0 = (s_x << 2) as i32;
                let y_s0 = (s_y << 2) as i32;
                let prev_csbf = neighbours[s_x + s_y * sub_block_width] as usize;
                let table = significance_table(log2_trafo_size - 2, c_idx, scan_idx, prev_csbf);
                let last_coeff = if i == last_sub_block { last_scan_pos - 1 } else { 15 };
                if i == last_sub_block {
                    coeff_value[count] = 1;
                    coeff_has_max_base_level[count] = true;
                    coeff_scan_pos[count] = last_scan_pos;
                    count += 1;
                }
                let mut n = last_coeff;
                while n > 0 {
                    let position = scan_pos[n as usize];
                    let x_c = x_s0 + position.x as i32;
                    let y_c = y_s0 + position.y as i32;
                    let context = if transform_skip_context { if c_idx == 0 { 42 } else { 16 + 27 } } else { table[(x_c + (y_c << log2_trafo_size)) as usize] as usize };
                    if self.bit(cabac::SIGNIFICANT_COEFF_FLAG + context) != 0 {
                        coeff_value[count] = 1;
                        coeff_has_max_base_level[count] = true;
                        coeff_scan_pos[count] = n;
                        count += 1;
                        infer_sb_dc_sig_coeff = false;
                    }
                    n -= 1;
                }
                if last_coeff >= 0 {
                    if !infer_sb_dc_sig_coeff {
                        let context = if transform_skip_context { if c_idx == 0 { 42 } else { 16 + 27 } } else { table[(x_s0 + (y_s0 << log2_trafo_size)) as usize] as usize };
                        if self.bit(cabac::SIGNIFICANT_COEFF_FLAG + context) != 0 {
                            coeff_value[count] = 1;
                            coeff_has_max_base_level[count] = true;
                            coeff_scan_pos[count] = 0;
                            count += 1;
                        }
                    } else {
                        coeff_value[count] = 1;
                        coeff_has_max_base_level[count] = true;
                        coeff_scan_pos[count] = 0;
                        count += 1;
                    }
                }
            }

            if count > 0 {
                let mut ctx_set = if i == 0 || c_idx > 0 { 0 } else { 2 };
                if c1 == 0 {
                    ctx_set += 1;
                }
                c1 = 1;

                let mut new_last_greater1_scan_pos = -1i32;
                let greater1_count = count.min(8);
                for c in 0..greater1_count {
                    // `decode_coeff_abs_level_greater1` (the context set is
                    // the HM one passed in).
                    let mut greater1_ctx;
                    if c == 0 {
                        let previous = if first_sub_block { 1 } else { last_sub_block_greater1_ctx };
                        let _ = previous;
                        greater1_ctx = 1;
                    } else {
                        greater1_ctx = last_greater1_ctx;
                        if greater1_ctx > 0 {
                            if last_greater1_flag == 1 {
                                greater1_ctx = 0;
                            } else {
                                greater1_ctx += 1;
                            }
                        }
                    }
                    let mut context = (ctx_set * 4 + greater1_ctx.min(3)) as usize;
                    if c_idx > 0 {
                        context += 16;
                    }
                    let flag = self.bit(cabac::COEFF_ABS_LEVEL_GREATER1_FLAG + context);
                    last_greater1_ctx = greater1_ctx;
                    last_greater1_flag = flag;
                    last_ctx_set = ctx_set;

                    if flag != 0 {
                        coeff_value[c] += 1;
                        c1 = 0;
                        if new_last_greater1_scan_pos == -1 {
                            new_last_greater1_scan_pos = c as i32;
                        }
                    } else {
                        coeff_has_max_base_level[c] = false;
                        if c1 < 3 && c1 > 0 {
                            c1 += 1;
                        }
                    }
                }
                first_sub_block = false;
                last_sub_block_greater1_ctx = last_greater1_ctx;

                if new_last_greater1_scan_pos != -1 {
                    let mut context = last_ctx_set as usize;
                    if c_idx > 0 {
                        context += 4;
                    }
                    let flag = self.bit(cabac::COEFF_ABS_LEVEL_GREATER2_FLAG + context);
                    coeff_value[new_last_greater1_scan_pos as usize] += flag;
                    coeff_has_max_base_level[new_last_greater1_scan_pos as usize] = flag != 0;
                }

                let sign_hidden = if sign_hiding_off || implicit_rdpcm { false } else { coeff_scan_pos[0] - coeff_scan_pos[count - 1] > 3 };
                for sign in coeff_sign.iter_mut().take(count - 1) {
                    *sign = self.bypass();
                }
                if !pps.sign_data_hiding_flag || !sign_hidden {
                    coeff_sign[count - 1] = self.bypass();
                } else {
                    coeff_sign[count - 1] = 0;
                }

                let persistent_rice = sps.range_extension.persistent_rice_adaptation_enabled_flag;
                let mut sum_abs_level = 0i32;
                let mut rice = if persistent_rice { self.stat_coeff[sb_type] / 4 } else { 0 };
                let mut first_with_remaining = true;

                for n in 0..count {
                    let base_level = coeff_value[n];
                    let mut remaining = 0;
                    if coeff_has_max_base_level[n] {
                        remaining = self.decode_coeff_abs_level_remaining(rice);
                        let threshold = 3i32.wrapping_mul(1i32.wrapping_shl(rice as u32));
                        if base_level.wrapping_add(remaining) > threshold {
                            rice += 1;
                            if !persistent_rice && rice > 4 {
                                rice = 4;
                            }
                        }
                        if persistent_rice && first_with_remaining {
                            let stat = self.stat_coeff[sb_type];
                            if remaining >= (3 << (stat / 4)) {
                                self.stat_coeff[sb_type] += 1;
                            } else if 2 * remaining < (1 << (stat / 4)) && stat > 0 {
                                self.stat_coeff[sb_type] -= 1;
                            }
                        }
                        first_with_remaining = false;
                    }

                    let mut current = base_level.wrapping_add(remaining) as i16;
                    if coeff_sign[n] != 0 {
                        current = current.wrapping_neg();
                    }
                    if pps.sign_data_hiding_flag && sign_hidden {
                        sum_abs_level = sum_abs_level.wrapping_add(current as i32);
                        if n == count - 1 && (sum_abs_level & 1) != 0 {
                            current = current.wrapping_neg();
                        }
                    }

                    let position = scan_pos[coeff_scan_pos[n] as usize];
                    let x_c = (s_x << 2) as i32 + position.x as i32;
                    let y_c = (s_y << 2) as i32 + position.y as i32;
                    let slot = self.n_coeff[c_idx];
                    self.coeff_list[c_idx][slot] = current;
                    self.coeff_pos[c_idx][slot] = (x_c + y_c * coeff_stride) as i16;
                    self.n_coeff[c_idx] += 1;
                }
            }
            i -= 1;
        }
    }

    /// `decode_TU`.
    #[allow(clippy::too_many_arguments)]
    fn decode_tu(&mut self, x0: i32, y0: i32, x_cu_base: i32, y_cu_base: i32, nt: i32, c_idx: usize, pred_mode: u8, cbf: bool) {
        let sps = self.picture.sps.clone();
        let mut residual_dpcm = 0;
        if pred_mode == MODE_INTRA {
            let mut mode = if c_idx == 0 { self.picture.intra_pred_mode(x0, y0) } else { self.picture.intra_pred_mode_c(x0 * sps.sub_width_c, y0 * sps.sub_height_c) };
            if !(0..35).contains(&mode) {
                mode = INTRA_DC;
            }
            decode_intra_prediction(self.picture, x0, y0, mode, nt, c_idx);
            let skip_or_bypass = self.cu_transquant_bypass_flag || self.transform_skip_flag[c_idx];
            if sps.range_extension.implicit_rdpcm_enabled_flag && skip_or_bypass && (mode == 10 || mode == 26) {
                residual_dpcm = if mode == 26 { 2 } else { 1 };
            }
        } else if self.explicit_rdpcm_flag {
            residual_dpcm = if self.explicit_rdpcm_dir { 2 } else { 1 };
        }

        if cbf {
            self.scale_coefficients(x0, y0, x_cu_base, y_cu_base, nt, c_idx, self.transform_skip_flag[c_idx], pred_mode == MODE_INTRA, residual_dpcm);
        } else if c_idx != 0 && self.res_scale_val != 0 {
            self.n_coeff[c_idx] = 0;
            self.scale_coefficients(x0, y0, x_cu_base, y_cu_base, nt, c_idx, self.transform_skip_flag[c_idx], pred_mode == MODE_INTRA, 0);
        }
    }

    /// `decode_quantization_parameters`.
    fn decode_quantization_parameters(&mut self, x_c: i32, y_c: i32, x_cu_base: i32, y_cu_base: i32) {
        let _ = (x_c, y_c);
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();

        let qg_mask = (1 << pps.log2_min_cu_qp_delta_size) - 1;
        let x_qg = x_cu_base - (x_cu_base & qg_mask);
        let y_qg = y_cu_base - (y_cu_base & qg_mask);
        if x_qg != self.current_qg_x || y_qg != self.current_qg_y {
            self.last_qpy_in_previous_qg = self.current_qpy;
            self.current_qg_x = x_qg;
            self.current_qg_y = y_qg;
        }

        let ctb_mask = (1 << sps.log2_ctb_size_y) - 1;
        let first_in_ctb_row = x_qg == 0 && (y_qg & ctb_mask) == 0;
        let first_ctb_in_slice = self.header.slice_addr_rs;
        let slice_start_x = (first_ctb_in_slice % sps.pic_width_in_ctbs_y) * sps.ctb_size_y;
        let slice_start_y = (first_ctb_in_slice / sps.pic_width_in_ctbs_y) * sps.ctb_size_y;
        let first_qg_in_slice = slice_start_x == x_qg && slice_start_y == y_qg;
        let mut first_qg_in_tile = false;
        if pps.tiles_enabled_flag && (x_qg & ctb_mask) == 0 && (y_qg & ctb_mask) == 0 {
            first_qg_in_tile = pps.is_tile_start_ctb(x_qg >> sps.log2_ctb_size_y, y_qg >> sps.log2_ctb_size_y);
        }

        let mut qp_y_pred = if first_qg_in_slice || first_qg_in_tile || (first_in_ctb_row && pps.entropy_coding_sync_enabled_flag) { self.header.slice_qp_y } else { self.last_qpy_in_previous_qg };

        let ctb_shift = 2 * (sps.log2_ctb_size_y - sps.log2_min_trafo_size);
        let qp_y_a = if self.picture.available_zscan(x_qg, y_qg, x_qg - 1, y_qg) {
            let index = ((x_qg - 1) >> sps.log2_min_trafo_size) + (y_qg >> sps.log2_min_trafo_size) * sps.pic_width_in_tbs_y;
            let ctb_address_a = pps.min_tb_addr_zs[index as usize] >> ctb_shift;
            if ctb_address_a == self.ctb_addr_in_ts { self.picture.qpy(x_qg - 1, y_qg) } else { qp_y_pred }
        } else {
            qp_y_pred
        };
        let qp_y_b = if self.picture.available_zscan(x_qg, y_qg, x_qg, y_qg - 1) {
            let index = (x_qg >> sps.log2_min_trafo_size) + ((y_qg - 1) >> sps.log2_min_trafo_size) * sps.pic_width_in_tbs_y;
            let ctb_address_b = pps.min_tb_addr_zs[index as usize] >> ctb_shift;
            if ctb_address_b == self.ctb_addr_in_ts { self.picture.qpy(x_qg, y_qg - 1) } else { qp_y_pred }
        } else {
            qp_y_pred
        };
        qp_y_pred = (qp_y_a + qp_y_b + 1) >> 1;

        let qp_bd_offset_y = sps.qp_bd_offset_y;
        let qp_y = ((qp_y_pred + self.cu_qp_delta + 52 + 2 * qp_bd_offset_y) % (52 + qp_bd_offset_y)) - qp_bd_offset_y;
        self.qp_y_prime = (qp_y + qp_bd_offset_y).max(0);

        let qp_bd_offset_c = sps.qp_bd_offset_c;
        let qp_i_cb = clip3(-qp_bd_offset_c, 57, qp_y + pps.pic_cb_qp_offset + self.header.slice_cb_qp_offset + self.cu_qp_offset_cb);
        let qp_i_cr = clip3(-qp_bd_offset_c, 57, qp_y + pps.pic_cr_qp_offset + self.header.slice_cr_qp_offset + self.cu_qp_offset_cr);
        let (qp_cb, qp_cr) = if sps.chroma_array_type == CHROMA_420 { (table_8_22(qp_i_cb), table_8_22(qp_i_cr)) } else { (qp_i_cb, qp_i_cr) };
        self.qp_cb_prime = (qp_cb + qp_bd_offset_c).max(0);
        self.qp_cr_prime = (qp_cr + qp_bd_offset_c).max(0);

        let log2_cb_size = self.picture.log2_cb_size(x_cu_base, y_cu_base).max(3);
        self.picture.set_qpy(x_cu_base, y_cu_base, log2_cb_size, qp_y);
        self.current_qpy = qp_y;
    }

    /// `cross_comp_pred`.
    fn cross_comp_pred(&self, residual: &mut [i32], nt: usize) {
        let sps = &self.picture.sps;
        for index in 0..nt * nt {
            let luma = (self.residual_luma[index] << sps.bit_depth_c) >> sps.bit_depth_y;
            residual[index] += (self.res_scale_val * luma) >> 3;
        }
    }

    fn add_residual(&mut self, c_idx: usize, x: i32, y: i32, nt: usize, residual: &[i32]) {
        let bit_depth = self.picture.bit_depth(c_idx);
        let maximum = (1 << bit_depth) - 1;
        let stride = self.picture.stride(c_idx);
        let plane = &mut self.picture.planes[c_idx];
        for dy in 0..nt as i32 {
            for dx in 0..nt as i32 {
                let index = (y + dy) * stride + x + dx;
                if index < 0 || index as usize >= plane.len() {
                    continue;
                }
                let value = plane[index as usize] as i32 + residual[(dy * nt as i32 + dx) as usize];
                plane[index as usize] = value.clamp(0, maximum) as u16;
            }
        }
    }

    /// `scale_coefficients_internal`.
    #[allow(clippy::too_many_arguments)]
    fn scale_coefficients(&mut self, x_t: i32, y_t: i32, x0: i32, y0: i32, nt: i32, c_idx: usize, transform_skip: bool, intra: bool, rdpcm_mode: i32) {
        let _ = (x0, y0);
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let qp = match c_idx {
            0 => self.qp_y_prime,
            1 => self.qp_cb_prime,
            _ => self.qp_cr_prime,
        };
        let size = nt as usize;
        let bit_depth = sps.bit_depth(c_idx);
        // The pred-mode lookup takes the component's coordinates as luma ones.
        let cu_pred_mode_intra = self.picture.pred_mode(x_t, y_t) == MODE_INTRA;
        let rotate = sps.range_extension.transform_skip_rotation_enabled_flag && nt == 4 && cu_pred_mode_intra;

        if self.cu_transquant_bypass_flag {
            for index in 0..self.n_coeff[c_idx] {
                let position = self.coeff_pos[c_idx][index] as usize;
                self.coeff_buf[position] = self.coeff_list[c_idx][index];
            }
            if rotate {
                transform::rotate_coefficients(&mut self.coeff_buf[..], size);
            }
            let mut residual = vec![0i32; size * size];
            match rdpcm_mode {
                2 => transform::transform_bypass_rdpcm_v(&mut residual, &self.coeff_buf[..], size),
                1 => transform::transform_bypass_rdpcm_h(&mut residual, &self.coeff_buf[..], size),
                _ => transform::transform_bypass(&mut residual, &self.coeff_buf[..], size),
            }
            self.finish_explicit_residual(c_idx, x_t, y_t, size, residual);
            if rotate {
                self.coeff_buf[..size * size].fill(0);
            }
        } else {
            let mut bd_shift = bit_depth + nt.trailing_zeros() as i32 - 5;
            if !sps.scaling_list_enable_flag {
                bd_shift -= 4;
                let offset = 1i32.wrapping_shl((bd_shift - 1) as u32);
                let fact = LEVEL_SCALE[(qp % 6) as usize].wrapping_shl((qp / 6) as u32);
                for index in 0..self.n_coeff[c_idx] {
                    let coefficient = self.coeff_list[c_idx][index] as i32;
                    let scaled = coefficient.wrapping_mul(fact).wrapping_add(offset) >> bd_shift;
                    let position = self.coeff_pos[c_idx][index] as usize;
                    self.coeff_buf[position] = scaled.clamp(-32768, 32767) as i16;
                }
            } else {
                let offset = 1i64 << (bd_shift - 1);
                let mut matrix_id = c_idx;
                if nt == 32 {
                    matrix_id = 0;
                }
                if !intra {
                    if nt < 32 {
                        matrix_id += 3;
                    } else {
                        matrix_id += 1;
                    }
                }
                let factors = pps.scaling_list.factors(nt.trailing_zeros() as i32, matrix_id);
                for index in 0..self.n_coeff[c_idx] {
                    let position = self.coeff_pos[c_idx][index] as usize;
                    let m = factors[position] as i32;
                    let fact = (m * LEVEL_SCALE[(qp % 6) as usize]).wrapping_shl((qp / 6) as u32) as i64;
                    let coefficient = self.coeff_list[c_idx][index] as i64;
                    let scaled = (coefficient * fact + offset) >> bd_shift;
                    self.coeff_buf[position] = scaled.clamp(-32768, 32767) as i16;
                }
            }

            if transform_skip {
                let bd_shift = (20 - bit_depth).max(0);
                let ts_shift = 5 + nt.trailing_zeros() as i32;
                if rotate {
                    transform::rotate_coefficients(&mut self.coeff_buf[..], size);
                }
                let mut residual = vec![0i32; size * size];
                match rdpcm_mode {
                    2 => transform::rdpcm_v(&mut residual, &self.coeff_buf[..], size, ts_shift, bd_shift),
                    1 => transform::rdpcm_h(&mut residual, &self.coeff_buf[..], size, ts_shift, bd_shift),
                    _ => transform::transform_skip_residual(&mut residual, &self.coeff_buf[..], size, ts_shift, bd_shift),
                }
                self.finish_explicit_residual(c_idx, x_t, y_t, size, residual);
                if rotate {
                    self.coeff_buf[..size * size].fill(0);
                }
            } else {
                let dst = nt == 4 && c_idx == 0 && cu_pred_mode_intra;
                if pps.range_extension.cross_component_prediction_enabled_flag {
                    let mut residual = vec![0i32; size * size];
                    let bd_shift = 20 - bit_depth;
                    if dst {
                        transform::idst_4x4(&mut residual, &self.coeff_buf[..], bd_shift, 15);
                    } else {
                        transform::idct(&mut residual, size, &self.coeff_buf[..], bd_shift, 15);
                    }
                    self.finish_explicit_residual(c_idx, x_t, y_t, size, residual);
                } else {
                    let residual = if dst { transform::dst_4x4_residual(&self.coeff_buf[..], bit_depth).to_vec() } else { transform::idct_residual(&self.coeff_buf[..], size, bit_depth) };
                    self.add_residual(c_idx, x_t, y_t, size, &residual);
                }
            }
        }

        for index in 0..self.n_coeff[c_idx] {
            let position = self.coeff_pos[c_idx][index] as usize;
            self.coeff_buf[position] = 0;
        }
    }

    /// The residual of the bypass, skip and explicit paths: kept for
    /// cross-component prediction (luma), predicted from luma (chroma),
    /// then added.
    fn finish_explicit_residual(&mut self, c_idx: usize, x: i32, y: i32, size: usize, mut residual: Vec<i32>) {
        if c_idx == 0 {
            self.residual_luma[..size * size].copy_from_slice(&residual);
        } else if self.res_scale_val != 0 {
            self.cross_comp_pred(&mut residual, size);
        }
        self.add_residual(c_idx, x, y, size, &residual);
    }

    // --- substreams ---

    /// `decode_substream`.
    pub fn decode_substream(&mut self, wpp_models: &mut [Option<Box<ContextTable>>], first_independent_substream: bool) -> DecodeResult {
        let sps = self.picture.sps.clone();
        let pps = self.picture.pps.clone();
        let ctb_width = sps.pic_width_in_ctbs_y;
        let start_ctb_y = self.ctb_y;

        let resumes_row = !first_independent_substream || self.ctb_y != start_ctb_y;
        if resumes_row && pps.entropy_coding_sync_enabled_flag && self.ctb_y >= 1 && self.ctb_x == 0 {
            if sps.pic_width_in_ctbs_y > 1 {
                let row = (self.ctb_y - 1) as usize;
                if row >= wpp_models.len() {
                    return DecodeResult::Error;
                }
                self.models = wpp_models[row].take();
            } else {
                self.initialize_models();
            }
        }

        loop {
            let ctb_x = self.ctb_x;
            let ctb_y = self.ctb_y;
            if (ctb_x + ctb_y * ctb_width) as usize >= pps.ctb_addr_rs_to_ts.len() {
                return DecodeResult::Error;
            }
            if ctb_x >= sps.pic_width_in_ctbs_y || ctb_y >= sps.pic_height_in_ctbs_y {
                return DecodeResult::Error;
            }
            if self.models.is_none() {
                return DecodeResult::Error;
            }

            self.read_coding_tree_unit();

            if pps.entropy_coding_sync_enabled_flag && ctb_x == 1 && ctb_y < sps.pic_height_in_ctbs_y - 1 {
                if wpp_models.len() <= ctb_y as usize {
                    return DecodeResult::Error;
                }
                wpp_models[ctb_y as usize] = self.models.clone();
            }

            let end_of_slice_segment = self.cabac.decode_term_bit() != 0;
            if end_of_slice_segment && pps.dependent_slice_segments_enabled_flag {
                self.stored_models = self.models.clone();
            }

            let last_ctb_y = self.ctb_y;
            let end_of_picture = self.advance_ctb_addr();
            if end_of_picture && !end_of_slice_segment {
                return DecodeResult::Error;
            }
            if end_of_slice_segment {
                return DecodeResult::EndOfSliceSegment;
            }

            let mut end_of_sub_stream = false;
            if pps.tiles_enabled_flag {
                let current = self.ctb_addr_in_ts as usize;
                end_of_sub_stream |= pps.tile_id[current] != pps.tile_id[current - 1];
            }
            if pps.entropy_coding_sync_enabled_flag {
                end_of_sub_stream |= last_ctb_y != self.ctb_y;
            }
            if end_of_sub_stream {
                if self.cabac.decode_term_bit() == 0 {
                    return DecodeResult::Error;
                }
                self.cabac.start();
                return DecodeResult::EndOfSubstream;
            }
        }
    }
}

/// The positions a transform tree's units refer back to.
#[derive(Clone, Copy)]
struct TransformTree {
    x_base: i32,
    y_base: i32,
    x_cu_base: i32,
    y_cu_base: i32,
    max_trafo_depth: i32,
    intra_split: bool,
}

/// True when the slice can be decoded here: intra slices only.
pub fn is_supported_slice(header: &SliceHeader) -> bool {
    header.slice_type == SLICE_TYPE_I
}
