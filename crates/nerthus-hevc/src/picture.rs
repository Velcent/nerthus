//! A picture being decoded (`de265_image`): its sample planes and the
//! per-block data the decoding and the loop filters share.

use std::sync::Arc;

use crate::params::{Pps, Sps};

pub const MODE_INTRA: u8 = 0;
pub const MODE_INTER: u8 = 1;

pub const PART_2NX2N: u8 = 0;
pub const PART_2NXN: u8 = 1;
pub const PART_NX2N: u8 = 2;
pub const PART_NXN: u8 = 3;
pub const PART_2NXNU: u8 = 4;
pub const PART_2NXND: u8 = 5;
pub const PART_NLX2N: u8 = 6;
pub const PART_NRX2N: u8 = 7;

pub const TU_FLAG_NONZERO_COEFF: u8 = 1 << 7;
pub const DEBLOCK_FLAG_VERTI: u8 = 1 << 4;
pub const DEBLOCK_FLAG_HORIZ: u8 = 1 << 5;
pub const DEBLOCK_PB_EDGE_VERTI: u8 = 1 << 6;
pub const DEBLOCK_PB_EDGE_HORIZ: u8 = 1 << 7;
pub const DEBLOCK_BS_MASK: u8 = 0x03;

/// `sao_info`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SaoInfo {
    pub sao_type_idx: u8,
    pub sao_eo_class: u8,
    pub sao_band_position: [u8; 3],
    pub sao_offset_val: [[i8; 4]; 3],
}

/// `CTB_info`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CtbInfo {
    pub slice_addr_rs: u16,
    pub slice_header_index: u16,
    pub sao: SaoInfo,
    pub has_pcm_or_cu_transquant_bypass: bool,
}

/// `CB_ref_info` (the bitfields' widths kept by masking).
#[derive(Clone, Copy, Debug, Default)]
pub struct CbInfo {
    pub log2_cb_size: u8,
    pub part_mode: u8,
    pub ct_depth: u8,
    pub pred_mode: u8,
    pub pcm_flag: u8,
    pub cu_transquant_bypass: u8,
    pub qp_y: i8,
}

/// `MetaDataArray`: one value per unit of `1 << log2_unit` samples.
#[derive(Clone, Debug, Default)]
pub struct MetaData<T: Clone + Default> {
    pub data: Vec<T>,
    pub log2_unit: i32,
    pub width: i32,
    pub height: i32,
}

impl<T: Clone + Default> MetaData<T> {
    fn new(width: i32, height: i32, log2_unit: i32) -> MetaData<T> {
        MetaData { data: vec![T::default(); (width * height).max(0) as usize], log2_unit, width, height }
    }

    pub fn index(&self, x: i32, y: i32) -> usize {
        ((x >> self.log2_unit) + (y >> self.log2_unit) * self.width) as usize
    }

    pub fn get(&self, x: i32, y: i32) -> &T {
        &self.data[self.index(x, y)]
    }

    pub fn get_mut(&mut self, x: i32, y: i32) -> &mut T {
        let index = self.index(x, y);
        &mut self.data[index]
    }

    pub fn clear(&mut self) {
        for value in self.data.iter_mut() {
            *value = T::default();
        }
    }
}

pub struct Picture {
    pub sps: Arc<Sps>,
    pub pps: Arc<Pps>,
    /// The planes, one sample a pixel whatever the bit depth.
    pub planes: [Vec<u16>; 3],
    pub width: i32,
    pub height: i32,
    pub chroma_width: i32,
    pub chroma_height: i32,
    /// `chroma_format` (0 mono, 1 4:2:0, 2 4:2:2, 3 4:4:4).
    pub chroma_format: i32,
    pub width_confwin: i32,
    pub height_confwin: i32,
    pub chroma_width_confwin: i32,
    pub chroma_height_confwin: i32,
    pub crop_left: i32,
    pub crop_top: i32,
    pub chroma_crop_left: i32,
    pub chroma_crop_top: i32,
    pub pic_output_flag: bool,

    pub ctb_info: MetaData<CtbInfo>,
    pub cb_info: MetaData<CbInfo>,
    pub intra_pred_mode: MetaData<u8>,
    pub intra_pred_mode_c: MetaData<u8>,
    pub tu_info: MetaData<u8>,
    pub deblk_info: MetaData<u8>,
}

impl Picture {
    /// `alloc_image` with metadata: `None` when the conformance window
    /// covers the whole picture.
    pub fn new(sps: Arc<Sps>, pps: Arc<Pps>) -> Option<Picture> {
        let width = sps.pic_width_in_luma_samples;
        let height = sps.pic_height_in_luma_samples;
        let chroma_format = sps.chroma_format_idc;
        let (win_unit_x, win_unit_y) = match chroma_format {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        let (chroma_width, chroma_height) = match chroma_format {
            1 => ((width + 1) / 2, (height + 1) / 2),
            2 => ((width + 1) / 2, height),
            3 => (width, height),
            _ => (0, 0),
        };
        let left = sps.conf_win_left_offset;
        let right = sps.conf_win_right_offset;
        let top = sps.conf_win_top_offset;
        let bottom = sps.conf_win_bottom_offset;
        if (left + right) * win_unit_x >= width || (top + bottom) * win_unit_y >= height {
            return None;
        }
        let chroma_planes = if chroma_format == 0 { 0 } else { (chroma_width * chroma_height) as usize };
        let planes = [vec![0u16; (width * height) as usize], vec![0u16; chroma_planes], vec![0u16; chroma_planes]];
        Some(Picture {
            ctb_info: MetaData::new(sps.pic_width_in_ctbs_y, sps.pic_height_in_ctbs_y, sps.log2_ctb_size_y),
            cb_info: MetaData::new(sps.pic_width_in_min_cbs_y, sps.pic_height_in_min_cbs_y, sps.log2_min_cb_size_y),
            intra_pred_mode: MetaData::new(sps.pic_width_in_min_pus, sps.pic_height_in_min_pus, sps.log2_min_pu_size),
            intra_pred_mode_c: MetaData::new(sps.pic_width_in_min_pus, sps.pic_height_in_min_pus, sps.log2_min_pu_size),
            tu_info: MetaData::new(sps.pic_width_in_tbs_y, sps.pic_height_in_tbs_y, sps.log2_min_trafo_size),
            deblk_info: MetaData::new((width + 3) / 4, (height + 3) / 4, 2),
            planes,
            width,
            height,
            chroma_width,
            chroma_height,
            chroma_format,
            width_confwin: width - (left + right) * win_unit_x,
            height_confwin: height - (top + bottom) * win_unit_y,
            chroma_width_confwin: chroma_width - left - right,
            chroma_height_confwin: chroma_height - top - bottom,
            crop_left: left * win_unit_x,
            crop_top: top * win_unit_y,
            chroma_crop_left: left,
            chroma_crop_top: top,
            pic_output_flag: true,
            sps,
            pps,
        })
    }

    pub fn stride(&self, c_idx: usize) -> i32 {
        if c_idx == 0 { self.width } else { self.chroma_width }
    }

    pub fn plane_width(&self, c_idx: usize) -> i32 {
        if c_idx == 0 { self.width } else { self.chroma_width }
    }

    pub fn plane_height(&self, c_idx: usize) -> i32 {
        if c_idx == 0 { self.height } else { self.chroma_height }
    }

    pub fn bit_depth(&self, c_idx: usize) -> i32 {
        self.sps.bit_depth(c_idx)
    }

    pub fn set_sample(&mut self, c_idx: usize, x: i32, y: i32, value: i32) {
        let width = self.plane_width(c_idx);
        let height = self.plane_height(c_idx);
        if x < 0 || y < 0 || x >= width || y >= height {
            return;
        }
        self.planes[c_idx][(y * width + x) as usize] = value as u16;
    }

    /// `clear_metadata`.
    pub fn clear_metadata(&mut self) {
        self.cb_info.clear();
        self.ctb_info.clear();
        self.deblk_info.clear();
    }

    fn for_cb_block(&mut self, x: i32, y: i32, log2_width: i32, mut update: impl FnMut(&mut CbInfo)) {
        let unit = self.cb_info.log2_unit;
        let cb_x = x >> unit;
        let cb_y = y >> unit;
        let width = 1 << (log2_width - unit);
        for cby in cb_y..cb_y + width {
            for cbx in cb_x..cb_x + width {
                if cbx < self.cb_info.width && cby < self.cb_info.height {
                    let index = (cbx + cby * self.cb_info.width) as usize;
                    update(&mut self.cb_info.data[index]);
                }
            }
        }
    }

    pub fn set_pred_mode(&mut self, x: i32, y: i32, log2_width: i32, mode: u8) {
        self.for_cb_block(x, y, log2_width, |info| info.pred_mode = mode & 3);
    }

    pub fn pred_mode(&self, x: i32, y: i32) -> u8 {
        self.cb_info.get(x, y).pred_mode
    }

    pub fn set_pcm_flag(&mut self, x: i32, y: i32, log2_width: i32) {
        self.for_cb_block(x, y, log2_width, |info| info.pcm_flag = 1);
        self.ctb_info.get_mut(x, y).has_pcm_or_cu_transquant_bypass = true;
    }

    pub fn pcm_flag(&self, x: i32, y: i32) -> bool {
        self.cb_info.get(x, y).pcm_flag != 0
    }

    pub fn set_cu_transquant_bypass(&mut self, x: i32, y: i32, log2_width: i32) {
        self.for_cb_block(x, y, log2_width, |info| info.cu_transquant_bypass = 1);
        self.ctb_info.get_mut(x, y).has_pcm_or_cu_transquant_bypass = true;
    }

    pub fn cu_transquant_bypass(&self, x: i32, y: i32) -> bool {
        self.cb_info.get(x, y).cu_transquant_bypass != 0
    }

    pub fn set_log2_cb_size(&mut self, x: i32, y: i32, log2_cb_size: i32) {
        self.for_cb_block(x, y, log2_cb_size, |info| info.log2_cb_size = 0);
        self.cb_info.get_mut(x, y).log2_cb_size = (log2_cb_size & 7) as u8;
    }

    pub fn log2_cb_size(&self, x: i32, y: i32) -> i32 {
        self.cb_info.get(x, y).log2_cb_size as i32
    }

    pub fn set_part_mode(&mut self, x: i32, y: i32, mode: u8) {
        self.cb_info.get_mut(x, y).part_mode = mode & 7;
    }

    pub fn part_mode(&self, x: i32, y: i32) -> u8 {
        self.cb_info.get(x, y).part_mode
    }

    pub fn set_ct_depth(&mut self, x: i32, y: i32, log2_width: i32, depth: i32) {
        self.for_cb_block(x, y, log2_width, |info| info.ct_depth = (depth & 3) as u8);
    }

    pub fn ct_depth(&self, x: i32, y: i32) -> i32 {
        self.cb_info.get(x, y).ct_depth as i32
    }

    pub fn set_qpy(&mut self, x: i32, y: i32, log2_width: i32, qp: i32) {
        self.for_cb_block(x, y, log2_width, |info| info.qp_y = qp as i8);
    }

    pub fn qpy(&self, x: i32, y: i32) -> i32 {
        self.cb_info.get(x, y).qp_y as i32
    }

    pub fn set_split_transform_flag(&mut self, x: i32, y: i32, depth: i32) {
        *self.tu_info.get_mut(x, y) |= 1 << depth;
    }

    pub fn clear_split_transform_flags(&mut self, x: i32, y: i32, log2_cb_size: i32) {
        let unit = self.tu_info.log2_unit;
        let tu_x = x >> unit;
        let tu_y = y >> unit;
        let width = 1 << (log2_cb_size - unit);
        for tuy in tu_y..tu_y + width {
            for tux in tu_x..tu_x + width {
                if tux < self.tu_info.width && tuy < self.tu_info.height {
                    let index = (tux + tuy * self.tu_info.width) as usize;
                    self.tu_info.data[index] = 0;
                }
            }
        }
    }

    pub fn split_transform_flag(&self, x: i32, y: i32, depth: i32) -> bool {
        self.tu_info.get(x, y) & (1 << depth) != 0
    }

    pub fn set_nonzero_coefficient(&mut self, x: i32, y: i32, log2_size: i32) {
        let unit = self.tu_info.log2_unit;
        let tu_x = x >> unit;
        let tu_y = y >> unit;
        let width = 1 << (log2_size - unit);
        for tuy in tu_y..tu_y + width {
            for tux in tu_x..tu_x + width {
                if tux < self.tu_info.width && tuy < self.tu_info.height {
                    let index = (tux + tuy * self.tu_info.width) as usize;
                    self.tu_info.data[index] |= TU_FLAG_NONZERO_COEFF;
                }
            }
        }
    }

    pub fn nonzero_coefficient(&self, x: i32, y: i32) -> bool {
        self.tu_info.get(x, y) & TU_FLAG_NONZERO_COEFF != 0
    }

    /// `get_IntraPredMode` (a value past 34 reads as planar).
    pub fn intra_pred_mode(&self, x: i32, y: i32) -> i32 {
        let mode = *self.intra_pred_mode.get(x, y);
        if mode > 34 { 0 } else { mode as i32 }
    }

    pub fn intra_pred_mode_at(&self, index: usize) -> i32 {
        self.intra_pred_mode.data[index] as i32
    }

    pub fn set_intra_pred_mode(&mut self, pu_index: i32, log2_size: i32, mode: i32) {
        let size = 1 << (log2_size - self.intra_pred_mode.log2_unit);
        for y in 0..size {
            for x in 0..size {
                let index = (pu_index + x + y * self.intra_pred_mode.width) as usize;
                if index < self.intra_pred_mode.data.len() {
                    self.intra_pred_mode.data[index] = mode as u8;
                }
            }
        }
    }

    pub fn intra_pred_mode_c(&self, x: i32, y: i32) -> i32 {
        (self.intra_pred_mode_c.get(x, y) & 0x3f) as i32
    }

    pub fn is_intra_pred_mode_c_mode4(&self, x: i32, y: i32) -> bool {
        self.intra_pred_mode_c.get(x, y) & 0x80 != 0
    }

    pub fn set_intra_pred_mode_c(&mut self, x: i32, y: i32, log2_size: i32, mode: i32, mode4: bool) {
        let value = mode as u8 | if mode4 { 0x80 } else { 0 };
        let size = 1 << (log2_size - self.intra_pred_mode.log2_unit);
        let pu_index = (x >> self.sps.log2_min_pu_size) + (y >> self.sps.log2_min_pu_size) * self.sps.pic_width_in_min_pus;
        for dy in 0..size {
            for dx in 0..size {
                let index = (pu_index + dx + dy * self.intra_pred_mode_c.width) as usize;
                if index < self.intra_pred_mode_c.data.len() {
                    self.intra_pred_mode_c.data[index] = value;
                }
            }
        }
    }

    pub fn set_slice_addr_rs(&mut self, ctb_x: i32, ctb_y: i32, address: i32) {
        if ctb_x >= self.ctb_info.width || ctb_y >= self.ctb_info.height {
            return;
        }
        let index = (ctb_x + ctb_y * self.ctb_info.width) as usize;
        self.ctb_info.data[index].slice_addr_rs = address as u16;
    }

    pub fn slice_addr_rs(&self, ctb_x: i32, ctb_y: i32) -> i32 {
        self.ctb_info.data[(ctb_x + ctb_y * self.ctb_info.width) as usize].slice_addr_rs as i32
    }

    pub fn slice_addr_rs_at(&self, ctb_rs: i32) -> i32 {
        self.ctb_info.data[ctb_rs as usize].slice_addr_rs as i32
    }

    pub fn set_slice_header_index(&mut self, x: i32, y: i32, index: usize) {
        self.ctb_info.get_mut(x, y).slice_header_index = index as u16;
    }

    pub fn slice_header_index(&self, x: i32, y: i32) -> usize {
        self.ctb_info.get(x, y).slice_header_index as usize
    }

    pub fn slice_header_index_ctb(&self, ctb_x: i32, ctb_y: i32) -> usize {
        self.ctb_info.data[(ctb_x + ctb_y * self.ctb_info.width) as usize].slice_header_index as usize
    }

    pub fn sao_info(&self, ctb_x: i32, ctb_y: i32) -> SaoInfo {
        self.ctb_info.data[(ctb_x + ctb_y * self.ctb_info.width) as usize].sao
    }

    pub fn set_sao_info(&mut self, ctb_x: i32, ctb_y: i32, info: SaoInfo) {
        let index = (ctb_x + ctb_y * self.ctb_info.width) as usize;
        self.ctb_info.data[index].sao = info;
    }

    pub fn ctb_has_pcm_or_bypass(&self, ctb_x: i32, ctb_y: i32) -> bool {
        self.ctb_info.data[(ctb_x + ctb_y * self.ctb_info.width) as usize].has_pcm_or_cu_transquant_bypass
    }

    pub fn set_deblk_flags(&mut self, x: i32, y: i32, flags: u8) {
        let xd = x / 4;
        let yd = y / 4;
        if xd < self.deblk_info.width && yd < self.deblk_info.height {
            let index = (xd + yd * self.deblk_info.width) as usize;
            self.deblk_info.data[index] |= flags;
        }
    }

    pub fn deblk_flags(&self, x: i32, y: i32) -> u8 {
        self.deblk_info.data[(x / 4 + y / 4 * self.deblk_info.width) as usize]
    }

    pub fn set_deblk_bs(&mut self, x: i32, y: i32, bs: u8) {
        let index = (x / 4 + y / 4 * self.deblk_info.width) as usize;
        self.deblk_info.data[index] &= !DEBLOCK_BS_MASK;
        self.deblk_info.data[index] |= bs;
    }

    pub fn deblk_bs(&self, x: i32, y: i32) -> i32 {
        (self.deblk_info.data[(x / 4 + y / 4 * self.deblk_info.width) as usize] & DEBLOCK_BS_MASK) as i32
    }

    /// `available_zscan`.
    pub fn available_zscan(&self, x_curr: i32, y_curr: i32, x_n: i32, y_n: i32) -> bool {
        let sps = &self.sps;
        let pps = &self.pps;
        if x_n < 0 || y_n < 0 || x_n >= sps.pic_width_in_luma_samples || y_n >= sps.pic_height_in_luma_samples {
            return false;
        }
        let shift = sps.log2_min_trafo_size;
        let address_n = pps.min_tb_addr_zs[((x_n >> shift) + (y_n >> shift) * sps.pic_width_in_tbs_y) as usize];
        let address_curr = pps.min_tb_addr_zs[((x_curr >> shift) + (y_curr >> shift) * sps.pic_width_in_tbs_y) as usize];
        if address_n > address_curr {
            return false;
        }
        let ctb_shift = sps.log2_ctb_size_y;
        let (x_curr_ctb, y_curr_ctb) = (x_curr >> ctb_shift, y_curr >> ctb_shift);
        let (x_n_ctb, y_n_ctb) = (x_n >> ctb_shift, y_n >> ctb_shift);
        if self.slice_addr_rs(x_curr_ctb, y_curr_ctb) != self.slice_addr_rs(x_n_ctb, y_n_ctb) {
            return false;
        }
        let width = sps.pic_width_in_ctbs_y;
        pps.tile_id_rs[(x_curr_ctb + y_curr_ctb * width) as usize] == pps.tile_id_rs[(x_n_ctb + y_n_ctb * width) as usize]
    }

    /// `check_CTB_available`.
    pub fn check_ctb_available(&self, x_c: i32, y_c: i32, x_n: i32, y_n: i32) -> bool {
        let sps = &self.sps;
        if x_n < 0 || y_n < 0 || x_n >= sps.pic_width_in_luma_samples || y_n >= sps.pic_height_in_luma_samples {
            return false;
        }
        let current = (y_c >> sps.log2_ctb_size_y) * sps.pic_width_in_ctbs_y + (x_c >> sps.log2_ctb_size_y);
        let neighbour = (y_n >> sps.log2_ctb_size_y) * sps.pic_width_in_ctbs_y + (x_n >> sps.log2_ctb_size_y);
        if self.slice_addr_rs_at(current) != self.slice_addr_rs_at(neighbour) {
            return false;
        }
        self.pps.tile_id_rs[current as usize] == self.pps.tile_id_rs[neighbour as usize]
    }
}
