//! The slice segment header as libde265 reads it (`slice.cc`).

use std::sync::Arc;

use crate::bitreader::{BitReader, UVLC_ERROR};
use crate::cabac::ContextTable;
use crate::error::Error;
use crate::params::{read_short_term_ref_pic_set, Pps, RefPicSet, CHROMA_MONO, MAX_NUM_REF_PICS};
use crate::util::{ceil_log2, clip3};

pub const SLICE_TYPE_B: i32 = 0;
pub const SLICE_TYPE_P: i32 = 1;
pub const SLICE_TYPE_I: i32 = 2;

pub const NAL_UNIT_RASL_N: i32 = 8;
pub const NAL_UNIT_RASL_R: i32 = 9;
pub const NAL_UNIT_BLA_W_LP: i32 = 16;
pub const NAL_UNIT_IDR_W_RADL: i32 = 19;
pub const NAL_UNIT_IDR_N_LP: i32 = 20;
pub const NAL_UNIT_RESERVED_IRAP_VCL23: i32 = 23;

pub fn is_rap_pic(nal_unit_type: i32) -> bool {
    (NAL_UNIT_BLA_W_LP..=NAL_UNIT_RESERVED_IRAP_VCL23).contains(&nal_unit_type)
}

pub fn is_rasl(nal_unit_type: i32) -> bool {
    nal_unit_type == NAL_UNIT_RASL_N || nal_unit_type == NAL_UNIT_RASL_R
}

#[derive(Clone)]
pub struct SliceHeader {
    pub slice_index: usize,
    pub pps: Arc<Pps>,

    pub first_slice_segment_in_pic_flag: bool,
    pub no_output_of_prior_pics_flag: bool,
    pub slice_pic_parameter_set_id: i32,
    pub dependent_slice_segment_flag: bool,
    pub slice_segment_address: i32,

    pub slice_type: i32,
    pub pic_output_flag: bool,
    pub colour_plane_id: i32,
    pub slice_pic_order_cnt_lsb: i32,
    pub short_term_ref_pic_set_sps_flag: bool,
    pub slice_ref_pic_set: RefPicSet,
    pub short_term_ref_pic_set_idx: i32,
    pub num_long_term_sps: i32,
    pub num_long_term_pics: i32,
    pub lt_idx_sps: [u8; MAX_NUM_REF_PICS],
    pub poc_lsb_lt: [i32; MAX_NUM_REF_PICS],
    pub used_by_curr_pic_lt_flag: [u8; MAX_NUM_REF_PICS],
    pub delta_poc_msb_present_flag: [u8; MAX_NUM_REF_PICS],
    pub delta_poc_msb_cycle_lt: [i32; MAX_NUM_REF_PICS],

    pub slice_temporal_mvp_enabled_flag: bool,
    pub slice_sao_luma_flag: bool,
    pub slice_sao_chroma_flag: bool,

    pub num_ref_idx_active_override_flag: bool,
    pub num_ref_idx_l0_active: i32,
    pub num_ref_idx_l1_active: i32,
    pub ref_pic_list_modification_flag_l0: bool,
    pub ref_pic_list_modification_flag_l1: bool,
    pub list_entry_l0: [u8; 16],
    pub list_entry_l1: [u8; 16],
    pub mvd_l1_zero_flag: bool,
    pub cabac_init_flag: bool,
    pub collocated_from_l0_flag: bool,
    pub collocated_ref_idx: i32,

    pub luma_log2_weight_denom: u8,
    pub chroma_log2_weight_denom: u8,
    pub luma_weight_flag: [[u8; 16]; 2],
    pub chroma_weight_flag: [[u8; 16]; 2],
    pub luma_weight: [[i16; 16]; 2],
    pub luma_offset: [[i8; 16]; 2],
    pub chroma_weight: [[[i16; 2]; 16]; 2],
    pub chroma_offset: [[[i8; 2]; 16]; 2],

    pub five_minus_max_num_merge_cand: i32,
    pub slice_qp_delta: i32,
    pub slice_cb_qp_offset: i32,
    pub slice_cr_qp_offset: i32,
    pub cu_chroma_qp_offset_enabled_flag: bool,

    pub deblocking_filter_override_flag: bool,
    pub slice_deblocking_filter_disabled_flag: bool,
    pub slice_beta_offset: i32,
    pub slice_tc_offset: i32,
    pub slice_loop_filter_across_slices_enabled_flag: bool,

    pub num_entry_point_offsets: i32,
    pub offset_len: i32,
    pub entry_point_offset: Vec<i32>,
    pub slice_segment_header_extension_length: i32,

    pub slice_qp_y: i32,
    pub init_type: i32,
    pub slice_addr_rs: i32,
    pub max_num_merge_cand: i32,
    pub curr_rps_idx: i32,
    pub curr_rps: RefPicSet,
    pub num_poc_total_curr: i32,

    /// The CABAC models at the end of the segment, for a dependent one.
    pub ctx_model_storage: Option<Box<ContextTable>>,
    pub ctx_model_storage_defined: bool,

    /// Set when a check drops the header part-way (with a warning).
    skipped: bool,
}

/// What reading a header came to: a header to decode, or one libde265
/// drops (`continueDecoding` false) with a warning but no error.
pub enum ReadOutcome {
    Decode(Box<SliceHeader>),
    Skip,
}

impl SliceHeader {
    /// `reset`.
    fn new(pps: Arc<Pps>) -> SliceHeader {
        SliceHeader {
            slice_index: 0,
            pps,
            first_slice_segment_in_pic_flag: false,
            no_output_of_prior_pics_flag: false,
            slice_pic_parameter_set_id: 0,
            dependent_slice_segment_flag: false,
            slice_segment_address: 0,
            slice_type: 0,
            pic_output_flag: false,
            colour_plane_id: 0,
            slice_pic_order_cnt_lsb: 0,
            short_term_ref_pic_set_sps_flag: false,
            slice_ref_pic_set: RefPicSet::default(),
            short_term_ref_pic_set_idx: 0,
            num_long_term_sps: 0,
            num_long_term_pics: 0,
            lt_idx_sps: [0; MAX_NUM_REF_PICS],
            poc_lsb_lt: [0; MAX_NUM_REF_PICS],
            used_by_curr_pic_lt_flag: [0; MAX_NUM_REF_PICS],
            delta_poc_msb_present_flag: [0; MAX_NUM_REF_PICS],
            delta_poc_msb_cycle_lt: [0; MAX_NUM_REF_PICS],
            slice_temporal_mvp_enabled_flag: false,
            slice_sao_luma_flag: false,
            slice_sao_chroma_flag: false,
            num_ref_idx_active_override_flag: false,
            num_ref_idx_l0_active: 0,
            num_ref_idx_l1_active: 0,
            ref_pic_list_modification_flag_l0: false,
            ref_pic_list_modification_flag_l1: false,
            list_entry_l0: [0; 16],
            list_entry_l1: [0; 16],
            mvd_l1_zero_flag: false,
            cabac_init_flag: false,
            collocated_from_l0_flag: false,
            collocated_ref_idx: 0,
            luma_log2_weight_denom: 0,
            chroma_log2_weight_denom: 0,
            luma_weight_flag: [[0; 16]; 2],
            chroma_weight_flag: [[0; 16]; 2],
            luma_weight: [[0; 16]; 2],
            luma_offset: [[0; 16]; 2],
            chroma_weight: [[[0; 2]; 16]; 2],
            chroma_offset: [[[0; 2]; 16]; 2],
            five_minus_max_num_merge_cand: 0,
            slice_qp_delta: 0,
            slice_cb_qp_offset: 0,
            slice_cr_qp_offset: 0,
            cu_chroma_qp_offset_enabled_flag: false,
            deblocking_filter_override_flag: false,
            slice_deblocking_filter_disabled_flag: false,
            slice_beta_offset: 0,
            slice_tc_offset: 0,
            slice_loop_filter_across_slices_enabled_flag: false,
            num_entry_point_offsets: 0,
            offset_len: 0,
            entry_point_offset: Vec::new(),
            slice_segment_header_extension_length: 0,
            slice_qp_y: 0,
            init_type: 0,
            slice_addr_rs: 0,
            max_num_merge_cand: 0,
            curr_rps_idx: 0,
            curr_rps: RefPicSet::default(),
            num_poc_total_curr: 0,
            ctx_model_storage: None,
            ctx_model_storage_defined: false,
            skipped: false,
        }
    }

    /// `slice_segment_header::read`.
    pub fn read(reader: &mut BitReader, nal_unit_type: i32, get_pps: impl Fn(i32) -> Option<Arc<Pps>>, previous: Option<&SliceHeader>) -> Result<ReadOutcome, Error> {
        let first_slice_segment_in_pic_flag = reader.get_bits(1) != 0;
        let mut no_output_of_prior_pics_flag = false;
        if is_rap_pic(nal_unit_type) {
            no_output_of_prior_pics_flag = reader.get_bits(1) != 0;
        }
        let pps_id = reader.get_uvlc();
        if !(0..64).contains(&pps_id) {
            return Ok(ReadOutcome::Skip);
        }
        let Some(pps) = get_pps(pps_id) else {
            return Ok(ReadOutcome::Skip);
        };
        let sps = pps.sps.clone();

        let mut header = SliceHeader::new(pps);
        header.first_slice_segment_in_pic_flag = first_slice_segment_in_pic_flag;
        header.no_output_of_prior_pics_flag = no_output_of_prior_pics_flag;
        header.slice_pic_parameter_set_id = pps_id;

        if !first_slice_segment_in_pic_flag {
            let dependent = if header.pps.dependent_slice_segments_enabled_flag { reader.get_bits(1) != 0 } else { false };
            let address_bits = ceil_log2(sps.pic_size_in_ctbs_y);
            let slice_segment_address = reader.get_bits(address_bits);
            if dependent {
                if slice_segment_address == 0 {
                    return Ok(ReadOutcome::Skip);
                }
                let Some(previous) = previous else {
                    return Err(Error::NO_INITIAL_SLICE_HEADER);
                };
                header = previous.clone();
                header.first_slice_segment_in_pic_flag = false;
            }
            header.dependent_slice_segment_flag = dependent;
            header.slice_segment_address = slice_segment_address;
        }

        if header.slice_segment_address < 0 || header.slice_segment_address >= sps.pic_size_in_ctbs_y {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }

        if !header.dependent_slice_segment_flag {
            header.read_independent_fields(reader, nal_unit_type)?;
            if header.skipped {
                return Ok(ReadOutcome::Skip);
            }
        }

        let pps = header.pps.clone();
        if pps.tiles_enabled_flag || pps.entropy_coding_sync_enabled_flag {
            header.num_entry_point_offsets = reader.get_uvlc();
            if header.num_entry_point_offsets == UVLC_ERROR {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            if pps.entropy_coding_sync_enabled_flag {
                let first_ctb_row = header.slice_segment_address / sps.pic_width_in_ctbs_y;
                let last_ctb_row = first_ctb_row + header.num_entry_point_offsets;
                if last_ctb_row >= sps.pic_height_in_ctbs_y {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
            }
            if pps.tiles_enabled_flag && header.num_entry_point_offsets > pps.num_tile_columns * pps.num_tile_rows {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            header.entry_point_offset.resize(header.num_entry_point_offsets as usize, 0);
            if header.num_entry_point_offsets > 0 {
                header.offset_len = reader.get_uvlc();
                if header.offset_len == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                header.offset_len += 1;
                if header.offset_len > 32 {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                for index in 0..header.num_entry_point_offsets as usize {
                    let offset = reader.get_bits(header.offset_len).wrapping_add(1);
                    header.entry_point_offset[index] = offset;
                    if index > 0 {
                        header.entry_point_offset[index] = offset.wrapping_add(header.entry_point_offset[index - 1]);
                    }
                }
            }
        } else {
            header.num_entry_point_offsets = 0;
        }

        if pps.slice_segment_header_extension_present_flag {
            header.slice_segment_header_extension_length = reader.get_uvlc();
            if header.slice_segment_header_extension_length == UVLC_ERROR || header.slice_segment_header_extension_length > 1000 {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            for _ in 0..header.slice_segment_header_extension_length {
                reader.get_bits(8);
            }
        }

        header.compute_derived_values();
        Ok(ReadOutcome::Decode(Box::new(header)))
    }

    fn read_independent_fields(&mut self, reader: &mut BitReader, nal_unit_type: i32) -> Result<(), Error> {
        let pps = self.pps.clone();
        let sps = pps.sps.clone();
        for _ in 0..pps.num_extra_slice_header_bits {
            reader.skip_bits(1);
        }

        self.slice_type = reader.get_uvlc();
        if self.slice_type > 2 || self.slice_type == UVLC_ERROR {
            // WARNING_SLICEHEADER_INVALID
            self.skipped = true;
            return Ok(());
        }

        self.pic_output_flag = if pps.output_flag_present_flag { reader.get_bits(1) != 0 } else { true };
        if sps.separate_colour_plane_flag {
            self.colour_plane_id = reader.get_bits(2);
        }

        self.slice_pic_order_cnt_lsb = 0;
        self.short_term_ref_pic_set_sps_flag = false;
        let mut num_lt_pics = 0;
        if nal_unit_type != NAL_UNIT_IDR_W_RADL && nal_unit_type != NAL_UNIT_IDR_N_LP {
            self.slice_pic_order_cnt_lsb = reader.get_bits(sps.log2_max_pic_order_cnt_lsb);
            self.short_term_ref_pic_set_sps_flag = reader.get_bits(1) != 0;
            let set_count = sps.ref_pic_sets.len();
            let max_dec_pic_buffering = sps.max_dec_pic_buffering[(sps.max_sub_layers - 1) as usize];
            if !self.short_term_ref_pic_set_sps_flag {
                let mut set = RefPicSet::default();
                read_short_term_ref_pic_set(reader, max_dec_pic_buffering, &mut set, set_count, &sps.ref_pic_sets, true);
                self.slice_ref_pic_set = set;
                self.curr_rps_idx = set_count as i32;
                self.curr_rps = set;
            } else {
                let bits = ceil_log2(set_count as i32);
                self.short_term_ref_pic_set_idx = if bits > 0 { reader.get_bits(bits) } else { 0 };
                if self.short_term_ref_pic_set_idx >= set_count as i32 {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                self.curr_rps_idx = self.short_term_ref_pic_set_idx;
                self.curr_rps = sps.ref_pic_sets[self.curr_rps_idx as usize];
            }

            if sps.long_term_ref_pics_present_flag {
                if sps.num_long_term_ref_pics_sps > 0 {
                    self.num_long_term_sps = reader.get_uvlc();
                    if self.num_long_term_sps == UVLC_ERROR {
                        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                    }
                } else {
                    self.num_long_term_sps = 0;
                }
                self.num_long_term_pics = reader.get_uvlc();
                if self.num_long_term_pics == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                let reference_count = self.num_long_term_sps + self.num_long_term_pics + self.curr_rps.num_negative_pics as i32 + self.curr_rps.num_positive_pics as i32;
                if reference_count > max_dec_pic_buffering {
                    // WARNING_MAX_NUM_REF_PICS_EXCEEDED
                    self.skipped = true;
                    return Ok(());
                }
                for index in 0..(self.num_long_term_sps + self.num_long_term_pics) as usize {
                    // The arrays hold 16; a longer list writes past them in C.
                    let slot = index.min(MAX_NUM_REF_PICS - 1);
                    let used_by_curr_pic = if (index as i32) < self.num_long_term_sps {
                        let bits = ceil_log2(sps.num_long_term_ref_pics_sps);
                        self.lt_idx_sps[slot] = reader.get_bits(bits) as u8;
                        if self.lt_idx_sps[slot] as i32 >= sps.num_long_term_ref_pics_sps {
                            // NON_EXISTING_LT_REFERENCE_CANDIDATE_IN_SLICE_HEADER
                            self.skipped = true;
                            return Ok(());
                        }
                        sps.used_by_curr_pic_lt_sps_flag[self.lt_idx_sps[slot] as usize] != 0
                    } else {
                        self.poc_lsb_lt[slot] = reader.get_bits(sps.log2_max_pic_order_cnt_lsb);
                        self.used_by_curr_pic_lt_flag[slot] = reader.get_bits(1) as u8;
                        self.used_by_curr_pic_lt_flag[slot] != 0
                    };
                    if used_by_curr_pic {
                        num_lt_pics += 1;
                    }
                    self.delta_poc_msb_present_flag[slot] = reader.get_bits(1) as u8;
                    if self.delta_poc_msb_present_flag[slot] != 0 {
                        self.delta_poc_msb_cycle_lt[slot] = reader.get_uvlc();
                        if self.delta_poc_msb_cycle_lt[slot] == UVLC_ERROR {
                            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                        }
                    } else {
                        self.delta_poc_msb_cycle_lt[slot] = 0;
                    }
                }
            } else {
                self.num_long_term_sps = 0;
                self.num_long_term_pics = 0;
            }

            self.slice_temporal_mvp_enabled_flag = if sps.temporal_mvp_enabled_flag { reader.get_bits(1) != 0 } else { false };
        } else {
            self.slice_pic_order_cnt_lsb = 0;
            self.num_long_term_sps = 0;
            self.num_long_term_pics = 0;
        }

        if sps.sample_adaptive_offset_enabled_flag {
            self.slice_sao_luma_flag = reader.get_bits(1) != 0;
            self.slice_sao_chroma_flag = if sps.chroma_array_type != CHROMA_MONO { reader.get_bits(1) != 0 } else { false };
        } else {
            self.slice_sao_luma_flag = false;
            self.slice_sao_chroma_flag = false;
        }

        self.num_ref_idx_l0_active = 0;
        self.num_ref_idx_l1_active = 0;
        if self.slice_type == SLICE_TYPE_P || self.slice_type == SLICE_TYPE_B {
            self.read_inter_fields(reader, num_lt_pics)?;
            if self.skipped {
                return Ok(());
            }
        }

        self.slice_qp_delta = reader.get_svlc();
        if self.slice_qp_delta == UVLC_ERROR {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        if pps.pps_slice_chroma_qp_offsets_present_flag {
            self.slice_cb_qp_offset = reader.get_svlc();
            if self.slice_cb_qp_offset == UVLC_ERROR {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            self.slice_cr_qp_offset = reader.get_svlc();
            if self.slice_cr_qp_offset == UVLC_ERROR {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
        } else {
            self.slice_cb_qp_offset = 0;
            self.slice_cr_qp_offset = 0;
        }

        if pps.range_extension.chroma_qp_offset_list_enabled_flag {
            self.cu_chroma_qp_offset_enabled_flag = reader.get_bits(1) != 0;
        }

        self.deblocking_filter_override_flag = if pps.deblocking_filter_override_enabled_flag { reader.get_bits(1) != 0 } else { false };
        self.slice_beta_offset = pps.beta_offset;
        self.slice_tc_offset = pps.tc_offset;
        if self.deblocking_filter_override_flag {
            self.slice_deblocking_filter_disabled_flag = reader.get_bits(1) != 0;
            if !self.slice_deblocking_filter_disabled_flag {
                self.slice_beta_offset = reader.get_svlc();
                if self.slice_beta_offset == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                self.slice_beta_offset *= 2;
                self.slice_tc_offset = reader.get_svlc();
                if self.slice_tc_offset == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                self.slice_tc_offset *= 2;
            }
        } else {
            self.slice_deblocking_filter_disabled_flag = pps.pic_disable_deblocking_filter_flag;
        }

        let any_loop_filter = self.slice_sao_luma_flag || self.slice_sao_chroma_flag || !self.slice_deblocking_filter_disabled_flag;
        if pps.pps_loop_filter_across_slices_enabled_flag && any_loop_filter {
            self.slice_loop_filter_across_slices_enabled_flag = reader.get_bits(1) != 0;
        } else {
            self.slice_loop_filter_across_slices_enabled_flag = pps.pps_loop_filter_across_slices_enabled_flag;
        }
        Ok(())
    }

    fn read_inter_fields(&mut self, reader: &mut BitReader, num_lt_pics: i32) -> Result<(), Error> {
        let pps = self.pps.clone();
        self.num_ref_idx_active_override_flag = reader.get_bits(1) != 0;
        if self.num_ref_idx_active_override_flag {
            self.num_ref_idx_l0_active = reader.get_uvlc();
            if self.num_ref_idx_l0_active == UVLC_ERROR {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            self.num_ref_idx_l0_active += 1;
            if self.slice_type == SLICE_TYPE_B {
                self.num_ref_idx_l1_active = reader.get_uvlc();
                if self.num_ref_idx_l1_active == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                self.num_ref_idx_l1_active += 1;
            }
        } else {
            self.num_ref_idx_l0_active = pps.num_ref_idx_l0_default_active;
            self.num_ref_idx_l1_active = pps.num_ref_idx_l1_default_active;
        }
        if self.num_ref_idx_l0_active > 16 || self.num_ref_idx_l1_active > 16 {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }

        self.num_poc_total_curr = self.curr_rps.num_poc_total_curr_shortterm_only as i32 + num_lt_pics;
        if pps.lists_modification_present_flag && self.num_poc_total_curr > 1 {
            let bits = ceil_log2(self.num_poc_total_curr);
            self.ref_pic_list_modification_flag_l0 = reader.get_bits(1) != 0;
            if self.ref_pic_list_modification_flag_l0 {
                for index in 0..self.num_ref_idx_l0_active as usize {
                    self.list_entry_l0[index] = reader.get_bits(bits) as u8;
                }
            }
            if self.slice_type == SLICE_TYPE_B {
                self.ref_pic_list_modification_flag_l1 = reader.get_bits(1) != 0;
                if self.ref_pic_list_modification_flag_l1 {
                    for index in 0..self.num_ref_idx_l1_active as usize {
                        self.list_entry_l1[index] = reader.get_bits(bits) as u8;
                    }
                }
            } else {
                self.ref_pic_list_modification_flag_l1 = false;
            }
        } else {
            self.ref_pic_list_modification_flag_l0 = false;
            self.ref_pic_list_modification_flag_l1 = false;
        }

        if self.slice_type == SLICE_TYPE_B {
            self.mvd_l1_zero_flag = reader.get_bits(1) != 0;
        }
        self.cabac_init_flag = if pps.cabac_init_present_flag { reader.get_bits(1) != 0 } else { false };

        if self.slice_temporal_mvp_enabled_flag {
            self.collocated_from_l0_flag = if self.slice_type == SLICE_TYPE_B { reader.get_bits(1) != 0 } else { true };
            let from_l0_with_choice = self.collocated_from_l0_flag && self.num_ref_idx_l0_active > 1;
            let from_l1_with_choice = !self.collocated_from_l0_flag && self.num_ref_idx_l1_active > 1;
            if from_l0_with_choice || from_l1_with_choice {
                self.collocated_ref_idx = reader.get_uvlc();
                if self.collocated_ref_idx == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
            } else {
                self.collocated_ref_idx = 0;
            }
            let past_l0 = self.collocated_from_l0_flag && self.collocated_ref_idx >= self.num_ref_idx_l0_active;
            let past_l1 = !self.collocated_from_l0_flag && self.collocated_ref_idx >= self.num_ref_idx_l1_active;
            if past_l0 || past_l1 {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
        }

        let weighted_p = pps.weighted_pred_flag && self.slice_type == SLICE_TYPE_P;
        let weighted_b = pps.weighted_bipred_flag && self.slice_type == SLICE_TYPE_B;
        if (weighted_p || weighted_b) && !self.read_pred_weight_table(reader) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }

        self.five_minus_max_num_merge_cand = reader.get_uvlc();
        if self.five_minus_max_num_merge_cand == UVLC_ERROR {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        self.max_num_merge_cand = 5 - self.five_minus_max_num_merge_cand;
        Ok(())
    }

    /// `read_pred_weight_table`.
    fn read_pred_weight_table(&mut self, reader: &mut BitReader) -> bool {
        let sps = self.pps.sps.clone();
        let high_precision = sps.range_extension.high_precision_offsets_enabled_flag;
        let offset_half_range_y = 1 << if high_precision { sps.bit_depth_y - 1 } else { 7 };
        let offset_half_range_c = 1 << if high_precision { sps.bit_depth_c - 1 } else { 7 };

        let value = reader.get_uvlc();
        self.luma_log2_weight_denom = value as u8;
        if !(0..=7).contains(&value) {
            return false;
        }
        if sps.chroma_format_idc != 0 {
            let value = reader.get_svlc() + self.luma_log2_weight_denom as i32;
            if !(0..=7).contains(&value) {
                return false;
            }
            self.chroma_log2_weight_denom = value as u8;
        }

        for list in 0..2 {
            if list == 1 && self.slice_type != SLICE_TYPE_B {
                continue;
            }
            let count = if list == 0 { self.num_ref_idx_l0_active } else { self.num_ref_idx_l1_active } as usize;
            for index in 0..count {
                self.luma_weight_flag[list][index] = reader.get_bits(1) as u8;
            }
            if sps.chroma_format_idc != 0 {
                for index in 0..count {
                    self.chroma_weight_flag[list][index] = reader.get_bits(1) as u8;
                }
            }
            for index in 0..count {
                if self.luma_weight_flag[list][index] != 0 {
                    let delta = reader.get_svlc();
                    if !(-128..=127).contains(&delta) {
                        return false;
                    }
                    self.luma_weight[list][index] = ((1 << self.luma_log2_weight_denom) + delta) as i16;
                    let offset = reader.get_svlc();
                    if offset < -offset_half_range_y || offset > offset_half_range_y - 1 {
                        return false;
                    }
                    self.luma_offset[list][index] = offset as i8;
                } else {
                    self.luma_weight[list][index] = 1 << self.luma_log2_weight_denom;
                    self.luma_offset[list][index] = 0;
                }
                if self.chroma_weight_flag[list][index] != 0 {
                    for component in 0..2 {
                        let delta = reader.get_svlc();
                        if !(-128..=127).contains(&delta) {
                            return false;
                        }
                        let weight = (1 << self.chroma_log2_weight_denom) + delta;
                        self.chroma_weight[list][index][component] = weight as i16;
                        let delta_offset = reader.get_svlc();
                        if delta_offset < -4 * offset_half_range_c || delta_offset > 4 * offset_half_range_c - 1 {
                            return false;
                        }
                        let predicted = (offset_half_range_c * weight) >> self.chroma_log2_weight_denom;
                        let offset = clip3(-offset_half_range_c, offset_half_range_c - 1, offset_half_range_c + delta_offset - predicted);
                        self.chroma_offset[list][index][component] = offset as i8;
                    }
                } else {
                    for component in 0..2 {
                        self.chroma_weight[list][index][component] = 1 << self.chroma_log2_weight_denom;
                        self.chroma_offset[list][index][component] = 0;
                    }
                }
            }
        }
        true
    }

    /// `compute_derived_values`.
    fn compute_derived_values(&mut self) {
        self.slice_qp_y = self.pps.pic_init_qp + self.slice_qp_delta;
        self.init_type = match self.slice_type {
            SLICE_TYPE_I => 0,
            SLICE_TYPE_P => self.cabac_init_flag as i32 + 1,
            SLICE_TYPE_B => 2 - self.cabac_init_flag as i32,
            _ => self.init_type,
        };
        self.max_num_merge_cand = 5 - self.five_minus_max_num_merge_cand;
    }
}
