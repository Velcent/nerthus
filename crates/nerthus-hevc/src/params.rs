//! Parameter sets as libde265 reads them (`vps.cc`, `vui.cc`, `sps.cc`,
//! `pps.cc`, `refpic.cc`): their syntax, the checks that make libde265
//! refuse one, and the values it derives.

use std::sync::Arc;

use crate::bitreader::{BitReader, UVLC_ERROR};
use crate::error::Error;
use crate::scan::scan_order;
use crate::util::ceil_div;

pub const MAX_VPS_SETS: i32 = 16;
pub const MAX_SPS_SETS: i32 = 16;
pub const MAX_PPS_SETS: i32 = 64;
pub const MAX_TEMPORAL_SUBLAYERS: i32 = 8;
pub const MAX_NUM_REF_PICS: usize = 16;
pub const MAX_NUM_LT_REF_PICS_SPS: usize = 32;
pub const MAX_TILE_COLUMNS: usize = 10;
pub const MAX_TILE_ROWS: usize = 10;

pub const CHROMA_MONO: i32 = 0;
pub const CHROMA_420: i32 = 1;
pub const CHROMA_422: i32 = 2;
pub const CHROMA_444: i32 = 3;

/// `READ_VLC`: an Exp-Golomb value, or the parameter error.
fn uvlc(reader: &mut BitReader) -> Result<i32, Error> {
    let value = reader.get_uvlc();
    if value == UVLC_ERROR {
        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
    }
    Ok(value)
}

/// `profile_tier_level::read`: only skipped over.
fn read_profile_tier_level(reader: &mut BitReader, max_sub_layers: i32) {
    let read_profile = |reader: &mut BitReader, profile: bool, level: bool| {
        if profile {
            reader.get_bits(2);
            reader.get_bits(1);
            reader.get_bits(5);
            for _ in 0..32 {
                reader.get_bits(1);
            }
            reader.get_bits(4);
            reader.skip_bits(44);
        }
        if level {
            reader.get_bits(8);
        }
    };
    read_profile(reader, true, true);
    let mut flags = Vec::new();
    for _ in 0..max_sub_layers - 1 {
        let profile = reader.get_bits(1) != 0;
        let level = reader.get_bits(1) != 0;
        flags.push((profile, level));
    }
    if max_sub_layers > 1 {
        for _ in max_sub_layers - 1..8 {
            reader.skip_bits(2);
        }
    }
    for (profile, level) in flags {
        read_profile(reader, profile, level);
    }
}

/// What libde265 uses of a video parameter set: the reorder buffer size
/// of its highest sub-layer.
#[derive(Clone, Copy, Debug)]
pub struct Vps {
    pub id: i32,
    pub max_num_reorder_pics: i32,
}

/// `video_parameter_set::read`.
pub fn read_vps(reader: &mut BitReader) -> Result<Vps, Error> {
    let id = reader.get_bits(4);
    if id >= MAX_VPS_SETS {
        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
    }
    reader.skip_bits(2);
    let max_layers = reader.get_bits(6) + 1;
    if max_layers > 63 {
        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
    }
    let max_sub_layers = reader.get_bits(3) + 1;
    if max_sub_layers >= MAX_TEMPORAL_SUBLAYERS {
        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
    }
    reader.get_bits(1);
    reader.skip_bits(16);
    read_profile_tier_level(reader, max_sub_layers);
    let ordering_info = reader.get_bits(1) != 0;
    let first = if ordering_info { 0 } else { max_sub_layers - 1 };
    let mut max_num_reorder_pics = 0;
    for _ in first..max_sub_layers {
        let max_dec_pic_buffering = reader.get_uvlc();
        let reorder = reader.get_uvlc();
        let max_latency_increase = reader.get_uvlc();
        if max_dec_pic_buffering == UVLC_ERROR || reorder == UVLC_ERROR || max_latency_increase == UVLC_ERROR {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        max_num_reorder_pics = reorder;
    }
    let vps = Vps { id, max_num_reorder_pics };
    let max_layer_id = reader.get_bits(6);
    let num_layer_sets = reader.get_uvlc();
    if num_layer_sets + 1 < 0 || num_layer_sets + 1 >= 1024 || num_layer_sets == UVLC_ERROR {
        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
    }
    let num_layer_sets = num_layer_sets + 1;
    for _ in 1..num_layer_sets {
        for _ in 0..=max_layer_id {
            reader.get_bits(1);
        }
    }
    let timing = reader.get_bits(1) != 0;
    if timing {
        reader.get_bits(32);
        reader.get_bits(32);
        let proportional = reader.get_bits(1) != 0;
        if proportional {
            reader.get_uvlc();
            let num_hrd = reader.get_uvlc();
            if !(0..1024).contains(&num_hrd) {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            if num_hrd > 0 {
                // libde265 returns after the first set's index.
                reader.get_uvlc();
                return Ok(vps);
            }
        }
    }
    reader.get_bits(1);
    Ok(vps)
}

/// The VUI values libheif reads back (`video_usability_information`).
#[derive(Clone, Debug)]
pub struct Vui {
    pub video_full_range_flag: bool,
    pub colour_primaries: i32,
    pub transfer_characteristics: i32,
    pub matrix_coeffs: i32,
}

impl Default for Vui {
    fn default() -> Vui {
        Vui { video_full_range_flag: false, colour_primaries: 2, transfer_characteristics: 2, matrix_coeffs: 2 }
    }
}

/// `video_usability_information::hrd_parameters`.
fn read_hrd_parameters(reader: &mut BitReader, max_sub_layers: i32) -> Result<(), Error> {
    let nal = reader.get_bits(1) != 0;
    let vcl = reader.get_bits(1) != 0;
    let mut sub_pic = false;
    if nal || vcl {
        sub_pic = reader.get_bits(1) != 0;
        if sub_pic {
            reader.get_bits(8);
            reader.get_bits(5);
            reader.get_bits(1);
            reader.get_bits(5);
        }
        reader.get_bits(4);
        reader.get_bits(4);
        if sub_pic {
            reader.get_bits(4);
        }
        reader.get_bits(5);
        reader.get_bits(5);
        reader.get_bits(5);
    }
    for _ in 0..max_sub_layers {
        let fixed_general = reader.get_bits(1) != 0;
        let fixed_within = if !fixed_general { reader.get_bits(1) != 0 } else { true };
        let mut low_delay = false;
        let mut cpb_count = 0;
        if fixed_within {
            uvlc(reader)?;
        } else {
            low_delay = reader.get_bits(1) != 0;
        }
        if !low_delay {
            cpb_count = uvlc(reader)?;
            if cpb_count > 31 {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
        }
        for nal_or_vcl in 0..2 {
            if (nal_or_vcl == 0 && nal) || (nal_or_vcl == 1 && vcl) {
                for _ in 0..=cpb_count {
                    uvlc(reader)?;
                    uvlc(reader)?;
                    if sub_pic {
                        uvlc(reader)?;
                        uvlc(reader)?;
                    }
                    reader.get_bits(1);
                }
            }
        }
    }
    Ok(())
}

/// `video_usability_information::read`.
fn read_vui(reader: &mut BitReader, max_sub_layers: i32) -> Result<Vui, Error> {
    let mut vui = Vui::default();
    if reader.get_bits(1) != 0 {
        let aspect_ratio_idc = reader.get_bits(8);
        if aspect_ratio_idc == 255 {
            reader.get_bits(16);
            reader.get_bits(16);
        }
    }
    if reader.get_bits(1) != 0 {
        reader.get_bits(1);
    }
    if reader.get_bits(1) != 0 {
        reader.get_bits(3);
        vui.video_full_range_flag = reader.get_bits(1) != 0;
        if reader.get_bits(1) != 0 {
            vui.colour_primaries = reader.get_bits(8);
            if vui.colour_primaries == 0 || vui.colour_primaries == 3 || vui.colour_primaries >= 11 {
                vui.colour_primaries = 2;
            }
            vui.transfer_characteristics = reader.get_bits(8);
            if vui.transfer_characteristics == 0 || vui.transfer_characteristics == 3 || vui.transfer_characteristics >= 18 {
                vui.transfer_characteristics = 2;
            }
            vui.matrix_coeffs = reader.get_bits(8);
            if vui.matrix_coeffs >= 11 {
                vui.matrix_coeffs = 2;
            }
        }
    }
    if reader.get_bits(1) != 0 {
        uvlc(reader)?;
        uvlc(reader)?;
    }
    reader.get_bits(1);
    reader.get_bits(1);
    reader.get_bits(1);
    if reader.get_bits(1) != 0 {
        for _ in 0..4 {
            uvlc(reader)?;
        }
    }
    if reader.get_bits(1) != 0 {
        reader.get_bits(32);
        reader.get_bits(32);
        if reader.get_bits(1) != 0 {
            uvlc(reader)?;
        }
        if reader.get_bits(1) != 0 {
            read_hrd_parameters(reader, max_sub_layers)?;
        }
    }
    if reader.get_bits(1) != 0 {
        reader.get_bits(1);
        reader.get_bits(1);
        reader.get_bits(1);
        for _ in 0..5 {
            uvlc(reader)?;
        }
    }
    Ok(vui)
}

/// A short-term reference picture set (`ref_pic_set`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RefPicSet {
    pub delta_poc_s0: [i16; MAX_NUM_REF_PICS],
    pub delta_poc_s1: [i16; MAX_NUM_REF_PICS],
    pub used_by_curr_pic_s0: [u8; MAX_NUM_REF_PICS],
    pub used_by_curr_pic_s1: [u8; MAX_NUM_REF_PICS],
    pub num_negative_pics: u8,
    pub num_positive_pics: u8,
    pub num_delta_pocs: u8,
    pub num_poc_total_curr_shortterm_only: u8,
}

impl RefPicSet {
    fn compute_derived_values(&mut self) {
        let mut total = 0;
        for i in 0..self.num_negative_pics as usize {
            if self.used_by_curr_pic_s0[i] != 0 {
                total += 1;
            }
        }
        for i in 0..self.num_positive_pics as usize {
            if self.used_by_curr_pic_s1[i] != 0 {
                total += 1;
            }
        }
        self.num_poc_total_curr_shortterm_only = total;
        self.num_delta_pocs = self.num_negative_pics.wrapping_add(self.num_positive_pics);
    }
}

/// `read_short_term_ref_pic_set`: false when the set is invalid.
pub fn read_short_term_ref_pic_set(reader: &mut BitReader, max_dec_pic_buffering: i32, out: &mut RefPicSet, index: usize, sets: &[RefPicSet], in_slice_header: bool) -> bool {
    let predicted = if index != 0 { reader.get_bits(1) != 0 } else { false };
    if predicted {
        let delta_index = if in_slice_header {
            let value = reader.get_uvlc();
            if value == UVLC_ERROR || value >= index as i32 {
                return false;
            }
            value as usize + 1
        } else {
            1
        };
        let source_index = index - delta_index;
        let sign = reader.get_bits(1);
        let abs_delta = reader.get_uvlc();
        if abs_delta == UVLC_ERROR {
            return false;
        }
        let abs_delta = abs_delta + 1;
        let delta_rps = if sign != 0 { -abs_delta } else { abs_delta };
        let source = sets[source_index];
        let count = source.num_delta_pocs as usize;
        let mut used = vec![0u8; count + 1];
        let mut use_delta = vec![0u8; count + 1];
        for j in 0..=count {
            used[j] = reader.get_bits(1) as u8;
            use_delta[j] = if used[j] != 0 { 1 } else { reader.get_bits(1) as u8 };
        }
        let negative = source.num_negative_pics as usize;
        let positive = source.num_positive_pics as usize;
        let mut i = 0usize;
        for j in (0..positive).rev() {
            let poc = source.delta_poc_s1[j] as i32 + delta_rps;
            if poc < 0 && use_delta[negative + j] != 0 {
                if i >= MAX_NUM_REF_PICS {
                    return false;
                }
                out.delta_poc_s0[i] = poc as i16;
                out.used_by_curr_pic_s0[i] = used[negative + j];
                i += 1;
            }
        }
        if delta_rps < 0 && use_delta[count] != 0 {
            if i >= MAX_NUM_REF_PICS {
                return false;
            }
            out.delta_poc_s0[i] = delta_rps as i16;
            out.used_by_curr_pic_s0[i] = used[count];
            i += 1;
        }
        for j in 0..negative {
            let poc = source.delta_poc_s0[j] as i32 + delta_rps;
            if poc < 0 && use_delta[j] != 0 {
                if i >= MAX_NUM_REF_PICS {
                    return false;
                }
                out.delta_poc_s0[i] = poc as i16;
                out.used_by_curr_pic_s0[i] = used[j];
                i += 1;
            }
        }
        out.num_negative_pics = i as u8;
        i = 0;
        for j in (0..negative).rev() {
            let poc = source.delta_poc_s0[j] as i32 + delta_rps;
            if poc > 0 && use_delta[j] != 0 {
                if i >= MAX_NUM_REF_PICS {
                    return false;
                }
                out.delta_poc_s1[i] = poc as i16;
                out.used_by_curr_pic_s1[i] = used[j];
                i += 1;
            }
        }
        if delta_rps > 0 && use_delta[count] != 0 {
            if i >= MAX_NUM_REF_PICS {
                return false;
            }
            out.delta_poc_s1[i] = delta_rps as i16;
            out.used_by_curr_pic_s1[i] = used[count];
            i += 1;
        }
        for j in 0..positive {
            let poc = source.delta_poc_s1[j] as i32 + delta_rps;
            if poc > 0 && use_delta[negative + j] != 0 {
                if i >= MAX_NUM_REF_PICS {
                    return false;
                }
                out.delta_poc_s1[i] = poc as i16;
                out.used_by_curr_pic_s1[i] = used[negative + j];
                i += 1;
            }
        }
        out.num_positive_pics = i as u8;
    } else {
        let negative = reader.get_uvlc();
        let positive = reader.get_uvlc();
        if negative == UVLC_ERROR || positive == UVLC_ERROR {
            return false;
        }
        if negative + positive > max_dec_pic_buffering {
            out.num_negative_pics = 0;
            out.num_positive_pics = 0;
            out.num_delta_pocs = 0;
            out.num_poc_total_curr_shortterm_only = 0;
            return false;
        }
        if negative > MAX_NUM_REF_PICS as i32 || positive > MAX_NUM_REF_PICS as i32 {
            return false;
        }
        out.num_negative_pics = negative as u8;
        out.num_positive_pics = positive as u8;
        let mut last = 0i32;
        for i in 0..negative as usize {
            let delta = reader.get_uvlc();
            if delta == UVLC_ERROR {
                return false;
            }
            let used = reader.get_bits(1) as u8;
            out.delta_poc_s0[i] = (last - (delta + 1)) as i16;
            out.used_by_curr_pic_s0[i] = used;
            last = out.delta_poc_s0[i] as i32;
        }
        last = 0;
        for i in 0..positive as usize {
            let delta = reader.get_uvlc();
            if delta == UVLC_ERROR {
                return false;
            }
            let used = reader.get_bits(1) as u8;
            out.delta_poc_s1[i] = (last + delta + 1) as i16;
            out.used_by_curr_pic_s1[i] = used;
            last = out.delta_poc_s1[i] as i32;
        }
    }
    out.compute_derived_values();
    true
}

/// `scaling_list_data`: the factors for 4x4 to 32x32 blocks, per matrix
/// (luma/Cb/Cr, intra then inter).
#[derive(Clone)]
pub struct ScalingList {
    pub size0: [[u8; 16]; 6],
    pub size1: [[u8; 64]; 6],
    pub size2: [[u8; 256]; 6],
    pub size3: [[u8; 1024]; 6],
}

impl Default for ScalingList {
    fn default() -> ScalingList {
        ScalingList { size0: [[0; 16]; 6], size1: [[0; 64]; 6], size2: [[0; 256]; 6], size3: [[0; 1024]; 6] }
    }
}

impl ScalingList {
    /// The factors for a `1 << log2` block, matrix `matrix_id`.
    pub fn factors(&self, log2: i32, matrix_id: usize) -> &[u8] {
        match log2 {
            2 => &self.size0[matrix_id],
            3 => &self.size1[matrix_id],
            4 => &self.size2[matrix_id],
            _ => &self.size3[matrix_id],
        }
    }
}

const DEFAULT_SCALING_LIST_4X4: [u8; 16] = [16; 16];
const DEFAULT_SCALING_LIST_8X8_INTRA: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, 17, 18, 18, 17, 18, 21, 19, 20, 21, 20, 19, 21, 24, 22, 22, 24, 24, 22, 22,
    24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31, 29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115,
];
const DEFAULT_SCALING_LIST_8X8_INTER: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 20, 20, 20, 20, 20, 20, 20, 24, 24, 24, 24, 24, 24, 24,
    24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28, 28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91,
];

/// `fill_scaling_factor`: a coded list in diagonal order spread over the
/// block (each coefficient covering 1, 2x2 or 4x4 positions).
fn fill_scaling_factor(factors: &mut [u8], list: &[u8], size_id: usize) {
    match size_id {
        0 => {
            let scan = scan_order(2, 0);
            for i in 0..16 {
                factors[scan[i].x as usize + 4 * scan[i].y as usize] = list[i];
            }
        }
        1 => {
            let scan = scan_order(3, 0);
            for i in 0..64 {
                factors[scan[i].x as usize + 8 * scan[i].y as usize] = list[i];
            }
        }
        _ => {
            let sub = if size_id == 2 { 2 } else { 4 };
            let width = 8 * sub;
            let scan = scan_order(3, 0);
            for i in 0..64 {
                for dy in 0..sub {
                    for dx in 0..sub {
                        let x = sub * scan[i].x as usize + dx;
                        let y = sub * scan[i].y as usize + dy;
                        factors[x + width * y] = list[i];
                    }
                }
            }
        }
    }
}

/// `set_default_scaling_lists` (which fills 32x32 matrices 0 and 1, not 0
/// and 3 as the coded lists are indexed).
pub fn default_scaling_lists() -> ScalingList {
    let mut lists = ScalingList::default();
    for matrix in 0..6 {
        fill_scaling_factor(&mut lists.size0[matrix], &DEFAULT_SCALING_LIST_4X4, 0);
    }
    for matrix in 0..3 {
        fill_scaling_factor(&mut lists.size1[matrix], &DEFAULT_SCALING_LIST_8X8_INTRA, 1);
        fill_scaling_factor(&mut lists.size1[matrix + 3], &DEFAULT_SCALING_LIST_8X8_INTER, 1);
    }
    for matrix in 0..3 {
        fill_scaling_factor(&mut lists.size2[matrix], &DEFAULT_SCALING_LIST_8X8_INTRA, 2);
        fill_scaling_factor(&mut lists.size2[matrix + 3], &DEFAULT_SCALING_LIST_8X8_INTER, 2);
    }
    fill_scaling_factor(&mut lists.size3[0], &DEFAULT_SCALING_LIST_8X8_INTRA, 3);
    fill_scaling_factor(&mut lists.size3[1], &DEFAULT_SCALING_LIST_8X8_INTER, 3);
    lists
}

/// `read_scaling_list`.
pub fn read_scaling_list(reader: &mut BitReader, lists: &mut ScalingList) -> Result<(), Error> {
    let mut dc_coeff = [[0i32; 6]; 4];
    for size_id in 0..4usize {
        let mut coded = [[0u8; 1024]; 6];
        let step = if size_id == 3 { 3 } else { 1 };
        let mut matrix_id = 0usize;
        while matrix_id < 6 {
            let mut dc_coef: i32;
            let pred_mode = reader.get_bits(1) != 0;
            if !pred_mode {
                let mut delta = reader.get_uvlc();
                if delta == UVLC_ERROR {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                if size_id == 3 {
                    delta *= 3;
                }
                if delta > matrix_id as i32 {
                    return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                }
                dc_coeff[size_id][matrix_id] = 16;
                dc_coef = 16;
                if delta == 0 {
                    if size_id == 0 {
                        coded[matrix_id][..16].copy_from_slice(&DEFAULT_SCALING_LIST_4X4);
                    } else if matrix_id < 3 {
                        coded[matrix_id][..64].copy_from_slice(&DEFAULT_SCALING_LIST_8X8_INTRA);
                    } else {
                        coded[matrix_id][..64].copy_from_slice(&DEFAULT_SCALING_LIST_8X8_INTER);
                    }
                } else {
                    let source = matrix_id - delta as usize;
                    let length = if size_id == 0 { 16 } else { 64 };
                    let copied = coded[source];
                    coded[matrix_id][..length].copy_from_slice(&copied[..length]);
                    dc_coef = dc_coeff[size_id][source];
                    dc_coeff[size_id][matrix_id] = dc_coeff[size_id][source];
                }
            } else {
                let mut next_coef = 8i32;
                let count = if size_id == 0 { 16 } else { 64 };
                if size_id > 1 {
                    dc_coef = reader.get_svlc();
                    if !(-7..=247).contains(&dc_coef) {
                        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                    }
                    dc_coef += 8;
                    next_coef = dc_coef;
                    dc_coeff[size_id][matrix_id] = dc_coef;
                } else {
                    dc_coef = 16;
                }
                for i in 0..count {
                    let delta = reader.get_svlc();
                    if !(-128..=127).contains(&delta) {
                        return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
                    }
                    next_coef = (next_coef + delta + 256) % 256;
                    coded[matrix_id][i] = next_coef as u8;
                }
            }
            match size_id {
                0 => fill_scaling_factor(&mut lists.size0[matrix_id], &coded[matrix_id], 0),
                1 => fill_scaling_factor(&mut lists.size1[matrix_id], &coded[matrix_id], 1),
                2 => {
                    fill_scaling_factor(&mut lists.size2[matrix_id], &coded[matrix_id], 2);
                    lists.size2[matrix_id][0] = dc_coef as u8;
                }
                _ => {
                    fill_scaling_factor(&mut lists.size3[matrix_id], &coded[matrix_id], 3);
                    lists.size3[matrix_id][0] = dc_coef as u8;
                }
            }
            matrix_id += step;
        }
    }
    // The 32x32 chroma matrices come from the 8x8 ones.
    let scan = scan_order(3, 0);
    for matrix in [1usize, 2, 4, 5] {
        for i in 0..64 {
            let x = scan[i].x as usize;
            let y = scan[i].y as usize;
            let value = lists.size1[matrix][y * 8 + x];
            for dy in 0..4 {
                for dx in 0..4 {
                    lists.size3[matrix][(4 * y + dy) * 32 + 4 * x + dx] = value;
                }
            }
        }
        lists.size3[matrix][0] = lists.size1[matrix][0];
    }
    Ok(())
}

/// `sps_range_extension`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpsRangeExtension {
    pub transform_skip_rotation_enabled_flag: bool,
    pub transform_skip_context_enabled_flag: bool,
    pub implicit_rdpcm_enabled_flag: bool,
    pub explicit_rdpcm_enabled_flag: bool,
    pub intra_smoothing_disabled_flag: bool,
    pub high_precision_offsets_enabled_flag: bool,
    pub persistent_rice_adaptation_enabled_flag: bool,
}

/// `seq_parameter_set`, with its derived values.
#[derive(Clone)]
pub struct Sps {
    pub id: i32,
    pub video_parameter_set_id: i32,
    pub max_sub_layers: i32,
    pub chroma_format_idc: i32,
    pub separate_colour_plane_flag: bool,
    pub pic_width_in_luma_samples: i32,
    pub pic_height_in_luma_samples: i32,
    pub conf_win_left_offset: i32,
    pub conf_win_right_offset: i32,
    pub conf_win_top_offset: i32,
    pub conf_win_bottom_offset: i32,
    pub bit_depth_luma: i32,
    pub bit_depth_chroma: i32,
    pub log2_max_pic_order_cnt_lsb: i32,
    pub max_dec_pic_buffering: [i32; 7],
    pub log2_min_luma_coding_block_size: i32,
    pub log2_diff_max_min_luma_coding_block_size: i32,
    pub log2_min_transform_block_size: i32,
    pub log2_diff_max_min_transform_block_size: i32,
    pub max_transform_hierarchy_depth_inter: i32,
    pub max_transform_hierarchy_depth_intra: i32,
    pub scaling_list_enable_flag: bool,
    pub scaling_list: ScalingList,
    pub sample_adaptive_offset_enabled_flag: bool,
    pub pcm_enabled_flag: bool,
    pub pcm_sample_bit_depth_luma: i32,
    pub pcm_sample_bit_depth_chroma: i32,
    pub log2_min_pcm_luma_coding_block_size: i32,
    pub log2_diff_max_min_pcm_luma_coding_block_size: i32,
    pub pcm_loop_filter_disable_flag: bool,
    pub ref_pic_sets: Vec<RefPicSet>,
    pub long_term_ref_pics_present_flag: bool,
    pub num_long_term_ref_pics_sps: i32,
    pub used_by_curr_pic_lt_sps_flag: [u8; MAX_NUM_LT_REF_PICS_SPS],
    pub temporal_mvp_enabled_flag: bool,
    pub strong_intra_smoothing_enable_flag: bool,
    pub vui: Vui,
    pub range_extension: SpsRangeExtension,

    pub chroma_array_type: i32,
    pub sub_width_c: i32,
    pub sub_height_c: i32,
    pub win_unit_x: i32,
    pub win_unit_y: i32,
    pub bit_depth_y: i32,
    pub bit_depth_c: i32,
    pub qp_bd_offset_y: i32,
    pub qp_bd_offset_c: i32,
    pub log2_min_cb_size_y: i32,
    pub log2_ctb_size_y: i32,
    pub min_cb_size_y: i32,
    pub ctb_size_y: i32,
    pub pic_width_in_min_cbs_y: i32,
    pub pic_height_in_min_cbs_y: i32,
    pub pic_width_in_ctbs_y: i32,
    pub pic_height_in_ctbs_y: i32,
    pub pic_size_in_min_cbs_y: i32,
    pub pic_size_in_ctbs_y: i32,
    pub log2_min_trafo_size: i32,
    pub log2_max_trafo_size: i32,
    pub log2_min_pu_size: i32,
    pub pic_width_in_min_pus: i32,
    pub pic_height_in_min_pus: i32,
    pub pic_width_in_tbs_y: i32,
    pub pic_height_in_tbs_y: i32,
    pub pic_size_in_tbs_y: i32,
    pub log2_min_ipcm_cb_size_y: i32,
    pub log2_max_ipcm_cb_size_y: i32,
}

const SUB_WIDTH_C: [i32; 4] = [1, 2, 2, 1];
const SUB_HEIGHT_C: [i32; 4] = [1, 2, 1, 1];

impl Sps {
    pub fn bit_depth(&self, c_idx: usize) -> i32 {
        if c_idx == 0 { self.bit_depth_y } else { self.bit_depth_c }
    }

    pub fn chroma_shift_w(&self, c_idx: usize) -> i32 {
        if c_idx != 0 { self.sub_width_c - 1 } else { 0 }
    }

    pub fn chroma_shift_h(&self, c_idx: usize) -> i32 {
        if c_idx != 0 { self.sub_height_c - 1 } else { 0 }
    }

    /// `seq_parameter_set::read`.
    pub fn read(reader: &mut BitReader) -> Result<Sps, Error> {
        let video_parameter_set_id = reader.get_bits(4);
        let max_sub_layers = reader.get_bits(3) + 1;
        if max_sub_layers > 7 {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        reader.get_bits(1);
        read_profile_tier_level(reader, max_sub_layers);
        let id = uvlc(reader)?;
        if id >= MAX_SPS_SETS {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let chroma_format_idc = uvlc(reader)?;
        let separate_colour_plane_flag = if chroma_format_idc == 3 { reader.get_bits(1) != 0 } else { false };
        if !(0..=3).contains(&chroma_format_idc) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let width = uvlc(reader)?;
        let height = uvlc(reader)?;
        if width == 0 || height == 0 || width > 65535 || height > 65535 {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let mut window = [0i32; 4];
        if reader.get_bits(1) != 0 {
            for value in window.iter_mut() {
                *value = uvlc(reader)?;
            }
        }
        let bit_depth_luma = uvlc(reader)? + 8;
        let bit_depth_chroma = uvlc(reader)? + 8;
        if bit_depth_luma > 16 || bit_depth_chroma > 16 {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let log2_max_pic_order_cnt_lsb = uvlc(reader)? + 4;
        if !(4..=16).contains(&log2_max_pic_order_cnt_lsb) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let ordering_info = reader.get_bits(1) != 0;
        let first = if ordering_info { 0 } else { max_sub_layers - 1 };
        let mut max_dec_pic_buffering = [0i32; 7];
        for i in first..max_sub_layers {
            let value = reader.get_uvlc();
            if value == UVLC_ERROR || value + 1 > MAX_NUM_REF_PICS as i32 {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            max_dec_pic_buffering[i as usize] = value + 1;
            // sps_max_num_reorder_pics and sps_max_latency_increase_plus1
            uvlc(reader)?;
            uvlc(reader)?;
        }
        if ordering_info {
            // libde265 copies the top layer's values down when they were
            // all sent (the inverse of the standard's inference).
            let top = (max_sub_layers - 1) as usize;
            for i in 0..top {
                max_dec_pic_buffering[i] = max_dec_pic_buffering[top];
            }
        }
        let log2_min_luma_coding_block_size = uvlc(reader)? + 3;
        let log2_diff_max_min_luma_coding_block_size = uvlc(reader)?;
        let log2_min_transform_block_size = uvlc(reader)? + 2;
        let log2_diff_max_min_transform_block_size = uvlc(reader)?;
        let max_transform_hierarchy_depth_inter = uvlc(reader)?;
        let max_transform_hierarchy_depth_intra = uvlc(reader)?;
        if log2_min_luma_coding_block_size > 6
            || log2_min_luma_coding_block_size + log2_diff_max_min_luma_coding_block_size > 6
            || log2_min_transform_block_size > 5
            || log2_min_transform_block_size + log2_diff_max_min_transform_block_size > 5
        {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let scaling_list_enable_flag = reader.get_bits(1) != 0;
        let mut scaling_list = ScalingList::default();
        if scaling_list_enable_flag {
            if reader.get_bits(1) != 0 {
                read_scaling_list(reader, &mut scaling_list)?;
            } else {
                scaling_list = default_scaling_lists();
            }
        }
        // amp_enabled_flag (inter only)
        reader.skip_bits(1);
        let sample_adaptive_offset_enabled_flag = reader.get_bits(1) != 0;
        let pcm_enabled_flag = reader.get_bits(1) != 0;
        let mut pcm = (0, 0, 0, 0, false);
        if pcm_enabled_flag {
            let luma = reader.get_bits(4) + 1;
            let chroma = reader.get_bits(4) + 1;
            let min = uvlc(reader)? + 3;
            let diff = uvlc(reader)?;
            let loop_filter = reader.get_bits(1) != 0;
            if luma > bit_depth_luma || chroma > bit_depth_chroma {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            pcm = (luma, chroma, min, diff, loop_filter);
        }
        let num_short_term = uvlc(reader)?;
        if !(0..=64).contains(&num_short_term) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        let mut ref_pic_sets = vec![RefPicSet::default(); num_short_term as usize];
        let max_buffering = max_dec_pic_buffering[(max_sub_layers - 1) as usize];
        for i in 0..num_short_term as usize {
            let mut set = ref_pic_sets[i];
            if !read_short_term_ref_pic_set(reader, max_buffering, &mut set, i, &ref_pic_sets, false) {
                return Err(Error::WARNING_SPS_HEADER_INVALID);
            }
            ref_pic_sets[i] = set;
        }
        let long_term_ref_pics_present_flag = reader.get_bits(1) != 0;
        let mut num_long_term_ref_pics_sps = 0;
        let mut used_by_curr_pic_lt_sps_flag = [0u8; MAX_NUM_LT_REF_PICS_SPS];
        if long_term_ref_pics_present_flag {
            num_long_term_ref_pics_sps = uvlc(reader)?;
            if num_long_term_ref_pics_sps > MAX_NUM_LT_REF_PICS_SPS as i32 {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            }
            for i in 0..num_long_term_ref_pics_sps as usize {
                // lt_ref_pic_poc_lsb_sps
                reader.skip_bits(log2_max_pic_order_cnt_lsb);
                used_by_curr_pic_lt_sps_flag[i] = reader.get_bits(1) as u8;
            }
        }
        let temporal_mvp_enabled_flag = reader.get_bits(1) != 0;
        let strong_intra_smoothing_enable_flag = reader.get_bits(1) != 0;
        let mut vui = Vui::default();
        if reader.get_bits(1) != 0 {
            vui = read_vui(reader, max_sub_layers)?;
        }
        let mut range_extension = SpsRangeExtension::default();
        let extension_present = reader.get_bits(1) != 0;
        let mut range_extension_flag = false;
        if extension_present {
            range_extension_flag = reader.get_bits(1) != 0;
            reader.get_bits(1);
            reader.get_bits(6);
        }
        if range_extension_flag {
            range_extension.transform_skip_rotation_enabled_flag = reader.get_bits(1) != 0;
            range_extension.transform_skip_context_enabled_flag = reader.get_bits(1) != 0;
            range_extension.implicit_rdpcm_enabled_flag = reader.get_bits(1) != 0;
            range_extension.explicit_rdpcm_enabled_flag = reader.get_bits(1) != 0;
            // extended_precision_processing_flag: libde265 ignores it.
            reader.skip_bits(1);
            range_extension.intra_smoothing_disabled_flag = reader.get_bits(1) != 0;
            range_extension.high_precision_offsets_enabled_flag = reader.get_bits(1) != 0;
            range_extension.persistent_rice_adaptation_enabled_flag = reader.get_bits(1) != 0;
            // cabac_bypass_alignment_enabled_flag: libde265 ignores it.
            reader.skip_bits(1);
        }
        let mut sps = Sps {
            id,
            video_parameter_set_id,
            max_sub_layers,
            chroma_format_idc,
            separate_colour_plane_flag,
            pic_width_in_luma_samples: width,
            pic_height_in_luma_samples: height,
            conf_win_left_offset: window[0],
            conf_win_right_offset: window[1],
            conf_win_top_offset: window[2],
            conf_win_bottom_offset: window[3],
            bit_depth_luma,
            bit_depth_chroma,
            log2_max_pic_order_cnt_lsb,
            max_dec_pic_buffering,
            log2_min_luma_coding_block_size,
            log2_diff_max_min_luma_coding_block_size,
            log2_min_transform_block_size,
            log2_diff_max_min_transform_block_size,
            max_transform_hierarchy_depth_inter,
            max_transform_hierarchy_depth_intra,
            scaling_list_enable_flag,
            scaling_list,
            sample_adaptive_offset_enabled_flag,
            pcm_enabled_flag,
            pcm_sample_bit_depth_luma: pcm.0,
            pcm_sample_bit_depth_chroma: pcm.1,
            log2_min_pcm_luma_coding_block_size: pcm.2,
            log2_diff_max_min_pcm_luma_coding_block_size: pcm.3,
            pcm_loop_filter_disable_flag: pcm.4,
            ref_pic_sets,
            long_term_ref_pics_present_flag,
            num_long_term_ref_pics_sps,
            used_by_curr_pic_lt_sps_flag,
            temporal_mvp_enabled_flag,
            strong_intra_smoothing_enable_flag,
            vui,
            range_extension,
            chroma_array_type: 0,
            sub_width_c: 1,
            sub_height_c: 1,
            win_unit_x: 1,
            win_unit_y: 1,
            bit_depth_y: 8,
            bit_depth_c: 8,
            qp_bd_offset_y: 0,
            qp_bd_offset_c: 0,
            log2_min_cb_size_y: 0,
            log2_ctb_size_y: 0,
            min_cb_size_y: 0,
            ctb_size_y: 0,
            pic_width_in_min_cbs_y: 0,
            pic_height_in_min_cbs_y: 0,
            pic_width_in_ctbs_y: 0,
            pic_height_in_ctbs_y: 0,
            pic_size_in_min_cbs_y: 0,
            pic_size_in_ctbs_y: 0,
            log2_min_trafo_size: 0,
            log2_max_trafo_size: 0,
            log2_min_pu_size: 0,
            pic_width_in_min_pus: 0,
            pic_height_in_min_pus: 0,
            pic_width_in_tbs_y: 0,
            pic_height_in_tbs_y: 0,
            pic_size_in_tbs_y: 0,
            log2_min_ipcm_cb_size_y: 0,
            log2_max_ipcm_cb_size_y: 0,
        };
        sps.compute_derived_values()?;
        Ok(sps)
    }

    /// `seq_parameter_set::compute_derived_values` (without sanitising).
    fn compute_derived_values(&mut self) -> Result<(), Error> {
        let format = self.chroma_format_idc as usize;
        self.sub_width_c = SUB_WIDTH_C[format];
        self.sub_height_c = SUB_HEIGHT_C[format];
        self.chroma_array_type = if self.separate_colour_plane_flag { 0 } else { self.chroma_format_idc };
        if self.chroma_array_type == 0 {
            self.win_unit_x = 1;
            self.win_unit_y = 1;
        } else {
            self.win_unit_x = SUB_WIDTH_C[format];
            self.win_unit_y = SUB_HEIGHT_C[format];
        }
        self.bit_depth_y = self.bit_depth_luma;
        self.qp_bd_offset_y = 6 * (self.bit_depth_luma - 8);
        self.bit_depth_c = self.bit_depth_chroma;
        self.qp_bd_offset_c = 6 * (self.bit_depth_chroma - 8);
        self.log2_min_cb_size_y = self.log2_min_luma_coding_block_size;
        self.log2_ctb_size_y = self.log2_min_cb_size_y + self.log2_diff_max_min_luma_coding_block_size;
        self.min_cb_size_y = 1 << self.log2_min_cb_size_y;
        self.ctb_size_y = 1 << self.log2_ctb_size_y;
        self.pic_width_in_min_cbs_y = ceil_div(self.pic_width_in_luma_samples, self.min_cb_size_y);
        self.pic_width_in_ctbs_y = ceil_div(self.pic_width_in_luma_samples, self.ctb_size_y);
        self.pic_height_in_min_cbs_y = ceil_div(self.pic_height_in_luma_samples, self.min_cb_size_y);
        self.pic_height_in_ctbs_y = ceil_div(self.pic_height_in_luma_samples, self.ctb_size_y);
        self.pic_size_in_min_cbs_y = self.pic_width_in_min_cbs_y * self.pic_height_in_min_cbs_y;
        self.pic_size_in_ctbs_y = self.pic_width_in_ctbs_y * self.pic_height_in_ctbs_y;
        self.log2_min_trafo_size = self.log2_min_transform_block_size;
        self.log2_max_trafo_size = self.log2_min_transform_block_size + self.log2_diff_max_min_transform_block_size;
        if self.max_transform_hierarchy_depth_inter > self.log2_ctb_size_y - self.log2_min_trafo_size {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        if self.max_transform_hierarchy_depth_intra > self.log2_ctb_size_y - self.log2_min_trafo_size {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        self.log2_min_pu_size = self.log2_min_cb_size_y - 1;
        self.pic_width_in_min_pus = self.pic_width_in_ctbs_y << (self.log2_ctb_size_y - self.log2_min_pu_size);
        self.pic_height_in_min_pus = self.pic_height_in_ctbs_y << (self.log2_ctb_size_y - self.log2_min_pu_size);
        self.log2_min_ipcm_cb_size_y = self.log2_min_pcm_luma_coding_block_size;
        self.log2_max_ipcm_cb_size_y = self.log2_min_pcm_luma_coding_block_size + self.log2_diff_max_min_pcm_luma_coding_block_size;
        self.pic_width_in_tbs_y = self.pic_width_in_ctbs_y << (self.log2_ctb_size_y - self.log2_min_trafo_size);
        self.pic_height_in_tbs_y = self.pic_height_in_ctbs_y << (self.log2_ctb_size_y - self.log2_min_trafo_size);
        self.pic_size_in_tbs_y = self.pic_width_in_tbs_y * self.pic_height_in_tbs_y;
        if self.pic_width_in_luma_samples % self.min_cb_size_y != 0 || self.pic_height_in_luma_samples % self.min_cb_size_y != 0 {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        if self.log2_min_trafo_size > self.log2_min_cb_size_y {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        if self.log2_max_trafo_size > self.log2_ctb_size_y.min(5) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        if !(8..=16).contains(&self.bit_depth_y) || !(8..=16).contains(&self.bit_depth_c) {
            return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
        }
        Ok(())
    }
}

/// `pps_range_extension`.
#[derive(Clone, Copy, Debug)]
pub struct PpsRangeExtension {
    pub log2_max_transform_skip_block_size: i32,
    pub cross_component_prediction_enabled_flag: bool,
    pub chroma_qp_offset_list_enabled_flag: bool,
    pub diff_cu_chroma_qp_offset_depth: i32,
    pub chroma_qp_offset_list_len: i32,
    pub cb_qp_offset_list: [i8; 6],
    pub cr_qp_offset_list: [i8; 6],
    pub log2_sao_offset_scale_luma: i32,
    pub log2_sao_offset_scale_chroma: i32,
}

impl Default for PpsRangeExtension {
    fn default() -> PpsRangeExtension {
        PpsRangeExtension {
            log2_max_transform_skip_block_size: 2,
            cross_component_prediction_enabled_flag: false,
            chroma_qp_offset_list_enabled_flag: false,
            diff_cu_chroma_qp_offset_depth: 0,
            chroma_qp_offset_list_len: 0,
            cb_qp_offset_list: [0; 6],
            cr_qp_offset_list: [0; 6],
            log2_sao_offset_scale_luma: 0,
            log2_sao_offset_scale_chroma: 0,
        }
    }
}

/// `pic_parameter_set`, with its derived tile and scan tables.
#[derive(Clone)]
pub struct Pps {
    pub sps: Arc<Sps>,
    pub id: i32,
    pub seq_parameter_set_id: i32,
    pub dependent_slice_segments_enabled_flag: bool,
    pub sign_data_hiding_flag: bool,
    pub cabac_init_present_flag: bool,
    pub num_ref_idx_l0_default_active: i32,
    pub num_ref_idx_l1_default_active: i32,
    pub pic_init_qp: i32,
    pub constrained_intra_pred_flag: bool,
    pub transform_skip_enabled_flag: bool,
    pub cu_qp_delta_enabled_flag: bool,
    pub diff_cu_qp_delta_depth: i32,
    pub pic_cb_qp_offset: i32,
    pub pic_cr_qp_offset: i32,
    pub pps_slice_chroma_qp_offsets_present_flag: bool,
    pub weighted_pred_flag: bool,
    pub weighted_bipred_flag: bool,
    pub output_flag_present_flag: bool,
    pub transquant_bypass_enable_flag: bool,
    pub entropy_coding_sync_enabled_flag: bool,
    pub tiles_enabled_flag: bool,
    pub num_tile_columns: i32,
    pub num_tile_rows: i32,
    pub uniform_spacing_flag: bool,
    pub loop_filter_across_tiles_enabled_flag: bool,
    pub pps_loop_filter_across_slices_enabled_flag: bool,
    pub deblocking_filter_override_enabled_flag: bool,
    pub pic_disable_deblocking_filter_flag: bool,
    pub beta_offset: i32,
    pub tc_offset: i32,
    pub scaling_list: ScalingList,
    pub lists_modification_present_flag: bool,
    pub num_extra_slice_header_bits: i32,
    pub slice_segment_header_extension_present_flag: bool,
    pub range_extension: PpsRangeExtension,

    pub log2_min_cu_qp_delta_size: i32,
    pub log2_min_cu_chroma_qp_offset_size: i32,
    pub log2_max_transform_skip_size: i32,
    pub col_width: [i32; MAX_TILE_COLUMNS],
    pub row_height: [i32; MAX_TILE_ROWS],
    pub col_bd: [i32; MAX_TILE_COLUMNS + 1],
    pub row_bd: [i32; MAX_TILE_ROWS + 1],
    pub ctb_addr_rs_to_ts: Vec<i32>,
    pub ctb_addr_ts_to_rs: Vec<i32>,
    pub tile_id: Vec<i32>,
    pub tile_id_rs: Vec<i32>,
    pub min_tb_addr_zs: Vec<i32>,
}

impl Pps {
    /// `is_tile_start_CTB`.
    pub fn is_tile_start_ctb(&self, ctb_x: i32, ctb_y: i32) -> bool {
        if !self.tiles_enabled_flag {
            return ctb_x == 0 && ctb_y == 0;
        }
        for column in 0..self.num_tile_columns as usize {
            if self.col_bd[column] == ctb_x {
                for row in 0..self.num_tile_rows as usize {
                    if self.row_bd[row] == ctb_y {
                        return true;
                    }
                }
                return false;
            }
        }
        false
    }

    /// `pic_parameter_set::read`: `None` when libde265 refuses it (which it
    /// only warns about). `get_sps` gives the active SPS of an id.
    pub fn read(reader: &mut BitReader, get_sps: impl Fn(i32) -> Option<Arc<Sps>>) -> Option<Pps> {
        let id = reader.get_uvlc();
        if id >= MAX_PPS_SETS || id == UVLC_ERROR {
            return None;
        }
        let sps_id = reader.get_uvlc();
        if sps_id >= MAX_SPS_SETS || sps_id == UVLC_ERROR {
            return None;
        }
        let dependent_slice_segments_enabled_flag = reader.get_bits(1) != 0;
        let output_flag_present_flag = reader.get_bits(1) != 0;
        let num_extra_slice_header_bits = reader.get_bits(3);
        let sign_data_hiding_flag = reader.get_bits(1) != 0;
        let cabac_init_present_flag = reader.get_bits(1) != 0;
        let l0 = reader.get_uvlc();
        if l0 == UVLC_ERROR {
            return None;
        }
        let l1 = reader.get_uvlc();
        if l1 == UVLC_ERROR {
            return None;
        }
        let sps = get_sps(sps_id)?;
        let pic_init_qp = reader.get_svlc();
        if pic_init_qp == UVLC_ERROR {
            return None;
        }
        let constrained_intra_pred_flag = reader.get_bits(1) != 0;
        let transform_skip_enabled_flag = reader.get_bits(1) != 0;
        let cu_qp_delta_enabled_flag = reader.get_bits(1) != 0;
        let mut diff_cu_qp_delta_depth = 0;
        if cu_qp_delta_enabled_flag {
            diff_cu_qp_delta_depth = reader.get_uvlc();
            if diff_cu_qp_delta_depth == UVLC_ERROR {
                return None;
            }
        }
        let pic_cb_qp_offset = reader.get_svlc();
        if pic_cb_qp_offset == UVLC_ERROR {
            return None;
        }
        let pic_cr_qp_offset = reader.get_svlc();
        if pic_cr_qp_offset == UVLC_ERROR {
            return None;
        }
        let pps_slice_chroma_qp_offsets_present_flag = reader.get_bits(1) != 0;
        let weighted_pred_flag = reader.get_bits(1) != 0;
        let weighted_bipred_flag = reader.get_bits(1) != 0;
        let transquant_bypass_enable_flag = reader.get_bits(1) != 0;
        let tiles_enabled_flag = reader.get_bits(1) != 0;
        let entropy_coding_sync_enabled_flag = reader.get_bits(1) != 0;
        let mut num_tile_columns = 1;
        let mut num_tile_rows = 1;
        let mut uniform_spacing_flag = true;
        let mut loop_filter_across_tiles_enabled_flag = false;
        let mut col_width = [0i32; MAX_TILE_COLUMNS];
        let mut row_height = [0i32; MAX_TILE_ROWS];
        if tiles_enabled_flag {
            num_tile_columns = reader.get_uvlc();
            if num_tile_columns == UVLC_ERROR || num_tile_columns + 1 > MAX_TILE_COLUMNS as i32 {
                return None;
            }
            num_tile_columns += 1;
            num_tile_rows = reader.get_uvlc();
            if num_tile_rows == UVLC_ERROR || num_tile_rows + 1 > MAX_TILE_ROWS as i32 {
                return None;
            }
            num_tile_rows += 1;
            uniform_spacing_flag = reader.get_bits(1) != 0;
            if !uniform_spacing_flag {
                let mut last_width = sps.pic_width_in_ctbs_y;
                let mut last_height = sps.pic_height_in_ctbs_y;
                for i in 0..(num_tile_columns - 1) as usize {
                    let value = reader.get_uvlc();
                    if value == UVLC_ERROR {
                        return None;
                    }
                    col_width[i] = value + 1;
                    last_width -= col_width[i];
                }
                if last_width <= 0 {
                    return None;
                }
                col_width[(num_tile_columns - 1) as usize] = last_width;
                for i in 0..(num_tile_rows - 1) as usize {
                    let value = reader.get_uvlc();
                    if value == UVLC_ERROR {
                        return None;
                    }
                    row_height[i] = value + 1;
                    last_height -= row_height[i];
                }
                if last_height <= 0 {
                    return None;
                }
                row_height[(num_tile_rows - 1) as usize] = last_height;
            }
            loop_filter_across_tiles_enabled_flag = reader.get_bits(1) != 0;
        }
        let mut beta_offset = 0;
        let mut tc_offset = 0;
        let pps_loop_filter_across_slices_enabled_flag = reader.get_bits(1) != 0;
        let control_present = reader.get_bits(1) != 0;
        let mut deblocking_filter_override_enabled_flag = false;
        let mut pic_disable_deblocking_filter_flag = false;
        if control_present {
            deblocking_filter_override_enabled_flag = reader.get_bits(1) != 0;
            pic_disable_deblocking_filter_flag = reader.get_bits(1) != 0;
            if !pic_disable_deblocking_filter_flag {
                beta_offset = reader.get_svlc();
                if beta_offset == UVLC_ERROR {
                    return None;
                }
                beta_offset *= 2;
                tc_offset = reader.get_svlc();
                if tc_offset == UVLC_ERROR {
                    return None;
                }
                tc_offset *= 2;
            }
        }
        let scaling_present = reader.get_bits(1) != 0;
        if !sps.scaling_list_enable_flag && scaling_present {
            return None;
        }
        let mut scaling_list = sps.scaling_list.clone();
        if scaling_present && read_scaling_list(reader, &mut scaling_list).is_err() {
            return None;
        }
        let lists_modification_present_flag = reader.get_bits(1) != 0;
        let merge_level = reader.get_uvlc();
        if merge_level == UVLC_ERROR {
            return None;
        }
        let log2_parallel_merge_level = merge_level + 2;
        if log2_parallel_merge_level - 2 > sps.log2_min_luma_coding_block_size - 3 + 1 + sps.log2_diff_max_min_luma_coding_block_size {
            return None;
        }
        let slice_segment_header_extension_present_flag = reader.get_bits(1) != 0;
        let mut range_extension = PpsRangeExtension::default();
        if reader.get_bits(1) != 0 {
            let range_flag = reader.get_bits(1) != 0;
            reader.get_bits(1);
            reader.get_bits(6);
            if range_flag {
                range_extension = read_pps_range_extension(reader, &sps, transform_skip_enabled_flag)?;
            }
        }
        let mut pps = Pps {
            sps,
            id,
            seq_parameter_set_id: sps_id,
            dependent_slice_segments_enabled_flag,
            sign_data_hiding_flag,
            cabac_init_present_flag,
            num_ref_idx_l0_default_active: l0 + 1,
            num_ref_idx_l1_default_active: l1 + 1,
            pic_init_qp: pic_init_qp + 26,
            constrained_intra_pred_flag,
            transform_skip_enabled_flag,
            cu_qp_delta_enabled_flag,
            diff_cu_qp_delta_depth,
            pic_cb_qp_offset,
            pic_cr_qp_offset,
            pps_slice_chroma_qp_offsets_present_flag,
            weighted_pred_flag,
            weighted_bipred_flag,
            output_flag_present_flag,
            transquant_bypass_enable_flag,
            entropy_coding_sync_enabled_flag,
            tiles_enabled_flag,
            num_tile_columns,
            num_tile_rows,
            uniform_spacing_flag,
            loop_filter_across_tiles_enabled_flag,
            pps_loop_filter_across_slices_enabled_flag,
            deblocking_filter_override_enabled_flag,
            pic_disable_deblocking_filter_flag,
            beta_offset,
            tc_offset,
            scaling_list,
            lists_modification_present_flag,
            num_extra_slice_header_bits,
            slice_segment_header_extension_present_flag,
            range_extension,
            log2_min_cu_qp_delta_size: 0,
            log2_min_cu_chroma_qp_offset_size: 0,
            log2_max_transform_skip_size: 0,
            col_width,
            row_height,
            col_bd: [0; MAX_TILE_COLUMNS + 1],
            row_bd: [0; MAX_TILE_ROWS + 1],
            ctb_addr_rs_to_ts: Vec::new(),
            ctb_addr_ts_to_rs: Vec::new(),
            tile_id: Vec::new(),
            tile_id_rs: Vec::new(),
            min_tb_addr_zs: Vec::new(),
        };
        pps.set_derived_values();
        Some(pps)
    }

    /// `pic_parameter_set::set_derived_values`.
    fn set_derived_values(&mut self) {
        let sps = self.sps.clone();
        self.log2_min_cu_qp_delta_size = sps.log2_ctb_size_y - self.diff_cu_qp_delta_depth;
        self.log2_min_cu_chroma_qp_offset_size = sps.log2_ctb_size_y - self.range_extension.diff_cu_chroma_qp_offset_depth;
        self.log2_max_transform_skip_size = self.range_extension.log2_max_transform_skip_block_size;
        let columns = self.num_tile_columns as usize;
        let rows = self.num_tile_rows as usize;
        if self.uniform_spacing_flag {
            for i in 0..columns {
                let start = i as i32 * sps.pic_width_in_ctbs_y / columns as i32;
                let end = (i as i32 + 1) * sps.pic_width_in_ctbs_y / columns as i32;
                self.col_width[i] = end - start;
            }
            for i in 0..rows {
                let start = i as i32 * sps.pic_height_in_ctbs_y / rows as i32;
                let end = (i as i32 + 1) * sps.pic_height_in_ctbs_y / rows as i32;
                self.row_height[i] = end - start;
            }
        }
        self.col_bd[0] = 0;
        for i in 0..columns {
            self.col_bd[i + 1] = self.col_bd[i] + self.col_width[i];
        }
        self.row_bd[0] = 0;
        for i in 0..rows {
            self.row_bd[i + 1] = self.row_bd[i] + self.row_height[i];
        }
        let size = sps.pic_size_in_ctbs_y as usize;
        self.ctb_addr_rs_to_ts = vec![0; size];
        self.ctb_addr_ts_to_rs = vec![0; size];
        self.tile_id = vec![0; size];
        self.tile_id_rs = vec![0; size];
        self.min_tb_addr_zs = vec![0; sps.pic_size_in_tbs_y as usize];
        let width = sps.pic_width_in_ctbs_y;
        for ctb_addr_rs in 0..size as i32 {
            let tb_x = ctb_addr_rs % width;
            let tb_y = ctb_addr_rs / width;
            let mut tile_x = -1i32;
            let mut tile_y = -1i32;
            for i in 0..columns {
                if tb_x >= self.col_bd[i] {
                    tile_x = i as i32;
                }
            }
            for j in 0..rows {
                if tb_y >= self.row_bd[j] {
                    tile_y = j as i32;
                }
            }
            let mut address = 0;
            for i in 0..tile_x as usize {
                address += self.row_height[tile_y as usize] * self.col_width[i];
            }
            for j in 0..tile_y as usize {
                address += width * self.row_height[j];
            }
            address += (tb_y - self.row_bd[tile_y as usize]) * self.col_width[tile_x as usize];
            address += tb_x - self.col_bd[tile_x as usize];
            self.ctb_addr_rs_to_ts[ctb_addr_rs as usize] = address;
            self.ctb_addr_ts_to_rs[address as usize] = ctb_addr_rs;
        }
        let mut tile_index = 0;
        for j in 0..rows {
            for i in 0..columns {
                for y in self.row_bd[j]..self.row_bd[j + 1] {
                    for x in self.col_bd[i]..self.col_bd[i + 1] {
                        let rs = (y * width + x) as usize;
                        let ts = self.ctb_addr_rs_to_ts[rs] as usize;
                        self.tile_id[ts] = tile_index;
                        self.tile_id_rs[rs] = tile_index;
                    }
                }
                tile_index += 1;
            }
        }
        let shift = sps.log2_ctb_size_y - sps.log2_min_trafo_size;
        for y in 0..sps.pic_height_in_tbs_y {
            for x in 0..sps.pic_width_in_tbs_y {
                let tb_x = (x << sps.log2_min_trafo_size) >> sps.log2_ctb_size_y;
                let tb_y = (y << sps.log2_min_trafo_size) >> sps.log2_ctb_size_y;
                let ctb_addr_rs = width * tb_y + tb_x;
                let mut value = self.ctb_addr_rs_to_ts[ctb_addr_rs as usize] << (shift * 2);
                for i in 0..shift {
                    let m = 1 << i;
                    value += (if m & x != 0 { m * m } else { 0 }) + (if m & y != 0 { 2 * m * m } else { 0 });
                }
                self.min_tb_addr_zs[(x + y * sps.pic_width_in_tbs_y) as usize] = value;
            }
        }
    }
}

/// `pps_range_extension::read`.
fn read_pps_range_extension(reader: &mut BitReader, sps: &Sps, transform_skip_enabled: bool) -> Option<PpsRangeExtension> {
    let mut extension = PpsRangeExtension::default();
    if transform_skip_enabled {
        // libde265 checks the value but keeps it either way.
        let value = reader.get_uvlc();
        extension.log2_max_transform_skip_block_size = value.wrapping_add(2) as u8 as i32;
    }
    extension.cross_component_prediction_enabled_flag = reader.get_bits(1) != 0;
    extension.chroma_qp_offset_list_enabled_flag = reader.get_bits(1) != 0;
    if extension.chroma_qp_offset_list_enabled_flag {
        let depth = reader.get_uvlc();
        if depth == UVLC_ERROR || depth > sps.log2_diff_max_min_luma_coding_block_size {
            return None;
        }
        extension.diff_cu_chroma_qp_offset_depth = depth;
        let length = reader.get_uvlc();
        if length == UVLC_ERROR || length > 5 {
            return None;
        }
        extension.chroma_qp_offset_list_len = length + 1;
        for i in 0..extension.chroma_qp_offset_list_len as usize {
            let cb = reader.get_svlc();
            if cb == UVLC_ERROR || !(-12..=12).contains(&cb) {
                return None;
            }
            extension.cb_qp_offset_list[i] = cb as i8;
            let cr = reader.get_svlc();
            if cr == UVLC_ERROR || !(-12..=12).contains(&cr) {
                return None;
            }
            extension.cr_qp_offset_list[i] = cr as i8;
        }
    }
    let luma = reader.get_uvlc();
    if luma == UVLC_ERROR || luma > 0.max(sps.bit_depth_y - 10) {
        return None;
    }
    extension.log2_sao_offset_scale_luma = luma;
    let chroma = reader.get_uvlc();
    if chroma == UVLC_ERROR || chroma > 0.max(sps.bit_depth_c - 10) {
        return None;
    }
    extension.log2_sao_offset_scale_chroma = chroma;
    Some(extension)
}
