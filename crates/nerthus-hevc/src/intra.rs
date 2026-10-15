//! Intra prediction (`intrapred.cc`, `intrapred.h`).

use crate::params::{Sps, CHROMA_444};
use crate::picture::{Picture, MODE_INTRA};

pub const INTRA_PLANAR: i32 = 0;
pub const INTRA_DC: i32 = 1;
pub const INTRA_ANGULAR_10: i32 = 10;
pub const INTRA_ANGULAR_26: i32 = 26;
pub const INTRA_ANGULAR_34: i32 = 34;

const MAX_INTRA_PRED_BLOCK_SIZE: i32 = 64;
/// Index of sample 0 in a border array (`2*MAX_INTRA_PRED_BLOCK_SIZE`).
const BORDER_CENTRE: i32 = 2 * MAX_INTRA_PRED_BLOCK_SIZE;
const BORDER_LENGTH: usize = (4 * MAX_INTRA_PRED_BLOCK_SIZE + 1) as usize;

const INTRA_PRED_ANGLE: [i32; 35] = [0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17, -13, -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32];
const INV_ANGLE: [i32; 15] = [-4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, -1638, -4096];

/// The border samples around a block, indexed from -2nT to 2nT.
struct Border {
    samples: [i32; BORDER_LENGTH],
}

impl Border {
    fn new() -> Border {
        Border { samples: [0; BORDER_LENGTH] }
    }

    fn get(&self, index: i32) -> i32 {
        self.samples[(index + BORDER_CENTRE) as usize]
    }

    fn set(&mut self, index: i32, value: i32) {
        self.samples[(index + BORDER_CENTRE) as usize] = value;
    }
}

/// `fillIntraPredModeCandidates` from the two neighbour modes.
fn candidates_from_neighbours(mode_a: i32, mode_b: i32) -> [i32; 3] {
    if mode_a == mode_b {
        if mode_a < 2 {
            return [INTRA_PLANAR, INTRA_DC, INTRA_ANGULAR_26];
        }
        let below = 2 + ((mode_a - 2 - 1 + 32) % 32);
        let above = 2 + ((mode_a - 2 + 1) % 32);
        return [mode_a, below, above];
    }
    let third = if mode_a != INTRA_PLANAR && mode_b != INTRA_PLANAR {
        INTRA_PLANAR
    } else if mode_a != INTRA_DC && mode_b != INTRA_DC {
        INTRA_DC
    } else {
        INTRA_ANGULAR_26
    };
    [mode_a, mode_b, third]
}

/// `fillIntraPredModeCandidates`.
pub fn intra_pred_mode_candidates(picture: &Picture, x: i32, y: i32, pu_index: i32, available_a: bool, available_b: bool) -> [i32; 3] {
    let sps = &picture.sps;
    let mode_a = if !available_a || picture.pred_mode(x - 1, y) != MODE_INTRA || picture.pcm_flag(x - 1, y) {
        INTRA_DC
    } else {
        picture.intra_pred_mode_at((pu_index - 1) as usize)
    };
    let ctb_top = (y >> sps.log2_ctb_size_y) << sps.log2_ctb_size_y;
    let mode_b = if !available_b || picture.pred_mode(x, y - 1) != MODE_INTRA || picture.pcm_flag(x, y - 1) || y - 1 < ctb_top {
        INTRA_DC
    } else {
        picture.intra_pred_mode_at((pu_index - sps.pic_width_in_min_pus) as usize)
    };
    candidates_from_neighbours(mode_a, mode_b)
}

/// `get_intra_scan_idx`.
pub fn intra_scan_idx(log2_trafo_size: i32, mode: i32, c_idx: usize, sps: &Sps) -> usize {
    let small_luma_or_444 = log2_trafo_size == 3 && (c_idx == 0 || sps.chroma_array_type == CHROMA_444);
    if log2_trafo_size == 2 || small_luma_or_444 {
        if (6..=14).contains(&mode) {
            return 2;
        }
        if (22..=30).contains(&mode) {
            return 1;
        }
    }
    0
}

/// `intra_border_computer`: the border samples with their availability
/// and the substitution of the missing ones.
fn fill_border_samples(picture: &Picture, x_b: i32, y_b: i32, nt: i32, c_idx: usize, border: &mut Border) {
    let sps = &picture.sps;
    let pps = &picture.pps;
    let sub_width = if c_idx == 0 { 1 } else { sps.sub_width_c };
    let sub_height = if c_idx == 0 { 1 } else { sps.sub_height_c };

    let mut available_left = true;
    let mut available_top = true;
    let mut available_top_right = true;
    let mut available_top_left = true;

    let mut x_b_luma = x_b * sub_width;
    let mut y_b_luma = y_b * sub_height;
    if x_b_luma == 0 {
        available_left = false;
        available_top_left = false;
        x_b_luma = 0;
    }
    if y_b_luma == 0 {
        available_top = false;
        available_top_left = false;
        available_top_right = false;
        y_b_luma = 0;
    }
    if x_b_luma + nt * sub_width >= sps.pic_width_in_luma_samples {
        available_top_right = false;
    }

    let log2_ctb_size = sps.log2_ctb_size_y;
    let pic_width_in_ctbs = sps.pic_width_in_ctbs_y;
    let x_curr_ctb = x_b_luma >> log2_ctb_size;
    let y_curr_ctb = y_b_luma >> log2_ctb_size;
    let x_left_ctb = (x_b_luma - 1) >> log2_ctb_size;
    let x_right_ctb = (x_b_luma + nt * sub_width) >> log2_ctb_size;
    let y_top_ctb = (y_b_luma - 1) >> log2_ctb_size;

    let tile_id = |ctb_x: i32, ctb_y: i32| pps.tile_id_rs[(ctb_x + ctb_y * pic_width_in_ctbs) as usize];
    let current_slice = picture.slice_addr_rs(x_curr_ctb, y_curr_ctb);
    let current_tile = tile_id(x_curr_ctb, y_curr_ctb);
    let left_slice = if available_left { picture.slice_addr_rs(x_left_ctb, y_curr_ctb) } else { -1 };
    let top_slice = if available_top { picture.slice_addr_rs(x_curr_ctb, y_top_ctb) } else { -1 };
    let top_right_slice = if available_top_right { picture.slice_addr_rs(x_right_ctb, y_top_ctb) } else { -1 };
    let top_left_slice = if available_top_left { picture.slice_addr_rs(x_left_ctb, y_top_ctb) } else { -1 };
    let left_tile = if available_left { tile_id(x_left_ctb, y_curr_ctb) } else { -1 };
    let top_tile = if available_top { tile_id(x_curr_ctb, y_top_ctb) } else { -1 };
    let top_left_tile = if available_top_left { tile_id(x_left_ctb, y_top_ctb) } else { -1 };
    let top_right_tile = if available_top_right { tile_id(x_right_ctb, y_top_ctb) } else { -1 };

    if left_slice != current_slice || left_tile != current_tile {
        available_left = false;
    }
    if top_slice != current_slice || top_tile != current_tile {
        available_top = false;
    }
    if top_left_slice != current_slice || top_left_tile != current_tile {
        available_top_left = false;
    }
    if top_right_slice != current_slice || top_right_tile != current_tile {
        available_top_right = false;
    }

    let mut n_bottom = sps.pic_height_in_luma_samples - y_b * sub_height;
    n_bottom = (n_bottom + sub_height - 1) / sub_height;
    n_bottom = n_bottom.min(2 * nt);
    let mut n_right = sps.pic_width_in_luma_samples - x_b * sub_width;
    n_right = (n_right + sub_width - 1) / sub_width;
    n_right = n_right.min(2 * nt);

    let mut available = [false; BORDER_LENGTH];
    let mut n_avail = 0;
    let mut first_value = 0;

    let plane = &picture.planes[c_idx];
    let stride = picture.stride(c_idx);
    let sample = |x: i32, y: i32| -> i32 {
        let index = y * stride + x;
        if index < 0 || index as usize >= plane.len() {
            return 0;
        }
        plane[index as usize] as i32
    };

    let min_tb_address = |x: i32, y: i32| -> i32 {
        let index = (x >> sps.log2_min_trafo_size) + (y >> sps.log2_min_trafo_size) * sps.pic_width_in_tbs_y;
        pps.min_tb_addr_zs[index as usize]
    };
    let current_block_address = min_tb_address(x_b_luma, y_b_luma);
    let neighbour_available = |x_luma: i32, y_luma: i32| -> bool {
        let mut available_n = min_tb_address(x_luma, y_luma) <= current_block_address;
        if pps.constrained_intra_pred_flag && picture.pred_mode(x_luma, y_luma) != MODE_INTRA {
            available_n = false;
        }
        available_n
    };

    if available_left {
        let mut y = n_bottom - 1;
        while y >= 0 {
            if neighbour_available((x_b - 1) * sub_width, (y_b + y) * sub_height) {
                if n_avail == 0 {
                    first_value = sample(x_b - 1, y_b + y);
                }
                for i in 0..4 {
                    let index = -y + i - 1;
                    available[(index + BORDER_CENTRE) as usize] = true;
                    border.set(index, sample(x_b - 1, y_b + y - i));
                }
                n_avail += 4;
            }
            y -= 4;
        }
    }

    if available_top_left && neighbour_available((x_b - 1) * sub_width, (y_b - 1) * sub_height) {
        if n_avail == 0 {
            first_value = sample(x_b - 1, y_b - 1);
        }
        border.set(0, sample(x_b - 1, y_b - 1));
        available[BORDER_CENTRE as usize] = true;
        n_avail += 1;
    }

    let mut x = 0;
    while x < n_right {
        let border_available = if x < nt { available_top } else { available_top_right };
        if border_available && neighbour_available((x_b + x) * sub_width, (y_b - 1) * sub_height) {
            if n_avail == 0 {
                first_value = sample(x_b + x, y_b - 1);
            }
            for i in 0..4 {
                border.set(x + i + 1, sample(x_b + x + i, y_b - 1));
                available[(x + i + 1 + BORDER_CENTRE) as usize] = true;
            }
            n_avail += 4;
        }
        x += 4;
    }

    // reference_sample_substitution
    let bit_depth = picture.bit_depth(c_idx);
    if n_avail != 4 * nt + 1 {
        if n_avail == 0 {
            for i in -2 * nt..=2 * nt {
                border.set(i, 1 << (bit_depth - 1));
            }
        } else {
            if !available[(-2 * nt + BORDER_CENTRE) as usize] {
                border.set(-2 * nt, first_value);
            }
            for i in -2 * nt + 1..=2 * nt {
                if !available[(i + BORDER_CENTRE) as usize] {
                    let previous = border.get(i - 1);
                    border.set(i, previous);
                }
            }
        }
    }
}

/// `intra_prediction_sample_filtering`.
fn filter_border(sps: &Sps, border: &mut Border, nt: i32, c_idx: usize, mode: i32) {
    let filter = if mode == INTRA_DC || nt == 4 {
        false
    } else {
        let min_dist_ver_hor = (mode - 26).abs().min((mode - 10).abs());
        match nt {
            8 => min_dist_ver_hor > 7,
            16 => min_dist_ver_hor > 1,
            32 => min_dist_ver_hor > 0,
            _ => false,
        }
    };
    if !filter {
        return;
    }
    let threshold = 1 << (sps.bit_depth_luma - 5);
    let bi_int = sps.strong_intra_smoothing_enable_flag
        && c_idx == 0
        && nt == 32
        && (border.get(0) + border.get(64) - 2 * border.get(32)).abs() < threshold
        && (border.get(0) + border.get(-64) - 2 * border.get(-32)).abs() < threshold;

    let mut filtered = Border::new();
    filtered.set(-2 * nt, border.get(-2 * nt));
    filtered.set(2 * nt, border.get(2 * nt));
    if bi_int {
        let corner = border.get(0);
        filtered.set(0, corner);
        for i in 1..=63 {
            filtered.set(-i, corner + ((i * (border.get(-64) - corner) + 32) >> 6));
            filtered.set(i, corner + ((i * (border.get(64) - corner) + 32) >> 6));
        }
    } else {
        for i in -(2 * nt - 1)..=2 * nt - 1 {
            let value = (border.get(i + 1) + 2 * border.get(i) + border.get(i - 1) + 2) >> 2;
            filtered.set(i, value);
        }
    }
    for i in -2 * nt..=2 * nt {
        border.set(i, filtered.get(i));
    }
}

fn clip_bit_depth(value: i32, bit_depth: i32) -> i32 {
    value.clamp(0, (1 << bit_depth) - 1)
}

/// `decode_intra_prediction`: predicts the block at (`x_b0`, `y_b0`) in
/// the component's plane.
pub fn decode_intra_prediction(picture: &mut Picture, x_b0: i32, y_b0: i32, mode: i32, nt: i32, c_idx: usize) {
    let mut border = Border::new();
    fill_border_samples(picture, x_b0, y_b0, nt, c_idx, &mut border);

    let sps = picture.sps.clone();
    if !sps.range_extension.intra_smoothing_disabled_flag && (c_idx == 0 || sps.chroma_array_type == CHROMA_444) {
        filter_border(&sps, &mut border, nt, c_idx, mode);
    }

    let mut prediction = vec![0i32; (nt * nt) as usize];
    let log2_nt = nt.trailing_zeros() as i32;
    match mode {
        INTRA_PLANAR => {
            for y in 0..nt {
                for x in 0..nt {
                    let value = (nt - 1 - x) * border.get(-1 - y) + (x + 1) * border.get(1 + nt) + (nt - 1 - y) * border.get(1 + x) + (y + 1) * border.get(-1 - nt) + nt;
                    prediction[(x + y * nt) as usize] = value >> (log2_nt + 1);
                }
            }
        }
        INTRA_DC => {
            let mut dc_value = 0;
            for i in 0..nt {
                dc_value += border.get(i + 1);
                dc_value += border.get(-i - 1);
            }
            dc_value += nt;
            dc_value >>= log2_nt + 1;
            for value in prediction.iter_mut() {
                *value = dc_value;
            }
            if c_idx == 0 && nt < 32 {
                prediction[0] = (border.get(-1) + 2 * dc_value + border.get(1) + 2) >> 2;
                for x in 1..nt {
                    prediction[x as usize] = (border.get(x + 1) + 3 * dc_value + 2) >> 2;
                }
                for y in 1..nt {
                    prediction[(y * nt) as usize] = (border.get(-y - 1) + 3 * dc_value + 2) >> 2;
                }
            }
        }
        _ => {
            let bit_depth = picture.bit_depth(c_idx);
            let disable_boundary_filter = sps.range_extension.implicit_rdpcm_enabled_flag && picture.cu_transquant_bypass(x_b0, y_b0);
            predict_angular(&mut prediction, bit_depth, disable_boundary_filter, mode, nt, c_idx, &border);
        }
    }

    let stride = picture.stride(c_idx);
    let plane = &mut picture.planes[c_idx];
    for y in 0..nt {
        for x in 0..nt {
            let index = (y_b0 + y) * stride + x_b0 + x;
            if index >= 0 && (index as usize) < plane.len() {
                plane[index as usize] = prediction[(x + y * nt) as usize] as u16;
            }
        }
    }
}

/// `intra_prediction_angular`.
fn predict_angular(prediction: &mut [i32], bit_depth: i32, disable_boundary_filter: bool, mode: i32, nt: i32, c_idx: usize, border: &Border) {
    let mut reference = Border::new();
    let angle = INTRA_PRED_ANGLE[mode as usize];

    if mode >= 18 {
        for x in 0..=nt {
            reference.set(x, border.get(x));
        }
        if angle < 0 {
            let inv_angle = INV_ANGLE[(mode - 11) as usize];
            if (nt * angle) >> 5 < -1 {
                for x in (nt * angle) >> 5..=-1 {
                    reference.set(x, border.get(-((x * inv_angle + 128) >> 8)));
                }
            }
        } else {
            for x in nt + 1..=2 * nt {
                reference.set(x, border.get(x));
            }
        }
        for y in 0..nt {
            let index = ((y + 1) * angle) >> 5;
            let fraction = ((y + 1) * angle) & 31;
            for x in 0..nt {
                let value = if fraction != 0 {
                    ((32 - fraction) * reference.get(x + index + 1) + fraction * reference.get(x + index + 2) + 16) >> 5
                } else {
                    reference.get(x + index + 1)
                };
                prediction[(x + y * nt) as usize] = value;
            }
        }
        if mode == INTRA_ANGULAR_26 && c_idx == 0 && nt < 32 && !disable_boundary_filter {
            for y in 0..nt {
                let value = border.get(1) + ((border.get(-1 - y) - border.get(0)) >> 1);
                prediction[(y * nt) as usize] = clip_bit_depth(value, bit_depth);
            }
        }
    } else {
        for x in 0..=nt {
            reference.set(x, border.get(-x));
        }
        if angle < 0 {
            let inv_angle = INV_ANGLE[(mode - 11) as usize];
            if (nt * angle) >> 5 < -1 {
                for x in (nt * angle) >> 5..=-1 {
                    reference.set(x, border.get((x * inv_angle + 128) >> 8));
                }
            }
        } else {
            for x in nt + 1..=2 * nt {
                reference.set(x, border.get(-x));
            }
        }
        for x in 0..nt {
            let index = ((x + 1) * angle) >> 5;
            let fraction = ((x + 1) * angle) & 31;
            for y in 0..nt {
                let value = if fraction != 0 {
                    ((32 - fraction) * reference.get(y + index + 1) + fraction * reference.get(y + index + 2) + 16) >> 5
                } else {
                    reference.get(y + index + 1)
                };
                prediction[(x + y * nt) as usize] = value;
            }
        }
        if mode == INTRA_ANGULAR_10 && c_idx == 0 && nt < 32 && !disable_boundary_filter {
            for x in 0..nt {
                let value = border.get(-1) + ((border.get(1 + x) - border.get(0)) >> 1);
                prediction[x as usize] = clip_bit_depth(value, bit_depth);
            }
        }
    }
}
