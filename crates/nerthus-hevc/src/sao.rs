//! Sample adaptive offset (`sao.cc`): each CTB's offsets applied to a
//! copy of the deblocked picture.

use crate::params::CHROMA_MONO;
use crate::picture::Picture;
use crate::slice_header::SliceHeader;

fn sign(value: i32) -> i32 {
    value.signum()
}

/// `apply_sao_internal` for one component of one CTB.
#[allow(clippy::too_many_arguments)]
fn apply_sao_ctb(picture: &mut Picture, slices: &[SliceHeader], input: &[u16], x_ctb: i32, y_ctb: i32, c_idx: usize, ctb_width: i32, ctb_height: i32) {
    let info = picture.sao_info(x_ctb, y_ctb);
    let type_idx = (info.sao_type_idx >> (2 * c_idx)) & 3;
    if type_idx == 0 {
        return;
    }
    let sps = picture.sps.clone();
    let pps = picture.pps.clone();
    let bit_depth = sps.bit_depth(c_idx);
    let maximum = (1 << bit_depth) - 1;
    let x_c = x_ctb * ctb_width;
    let y_c = y_ctb * ctb_height;
    let width = picture.plane_width(c_idx);
    let height = picture.plane_height(c_idx);
    let stride = picture.stride(c_idx);

    // The CTB's slice is looked up at its component position as if luma.
    let Some(ctb_header) = slices.get(picture.slice_header_index(x_c, y_c)) else {
        return;
    };
    let ctb_slice_addr_rs = ctb_header.slice_addr_rs;
    let width_in_ctbs = sps.pic_width_in_ctbs_y;
    let shift_w = sps.chroma_shift_w(c_idx);
    let shift_h = sps.chroma_shift_h(c_idx);
    let ctb_shift_w = sps.log2_ctb_size_y - shift_w;
    let ctb_shift_h = sps.log2_ctb_size_y - shift_h;
    let block_width = if x_c + ctb_width > width { width - x_c } else { ctb_width };
    let block_height = if y_c + ctb_height > height { height - y_c } else { ctb_height };
    let extended_tests = picture.ctb_has_pcm_or_bypass(x_ctb, y_ctb);

    let skip = |picture: &Picture, i: i32, j: i32| -> bool {
        let x_luma = (x_c + i) << shift_w;
        let y_luma = (y_c + j) << shift_h;
        picture.cu_transquant_bypass(x_luma, y_luma) || (extended_tests && sps.pcm_loop_filter_disable_flag && picture.pcm_flag(x_luma, y_luma))
    };

    if type_idx == 2 {
        let class = (info.sao_eo_class >> (2 * c_idx)) & 3;
        let (h_pos, v_pos) = match class {
            0 => ([-1, 1], [0, 0]),
            1 => ([0, 0], [-1, 1]),
            2 => ([-1, 1], [-1, 1]),
            _ => ([1, -1], [-1, 1]),
        };
        let offsets = info.sao_offset_val[c_idx];
        let offset_by_edge = [offsets[0] as i32, offsets[1] as i32, 0, offsets[2] as i32, offsets[3] as i32];

        for j in 0..block_height {
            for i in 0..block_width {
                if skip(picture, i, j) {
                    continue;
                }
                let mut edge_zero = false;
                let boundary = i == 0 || j == 0 || i == block_width - 1 || j == block_height - 1;
                if boundary {
                    for k in 0..2 {
                        let x_s = x_c + i + h_pos[k];
                        let y_s = y_c + j + v_pos[k];
                        if x_s < 0 || y_s < 0 || x_s >= width || y_s >= height {
                            edge_zero = true;
                            break;
                        }
                        let Some(neighbour_header) = slices.get(picture.slice_header_index(x_s << shift_w, y_s << shift_h)) else {
                            return;
                        };
                        let neighbour_slice = neighbour_header.slice_addr_rs;
                        if neighbour_slice < ctb_slice_addr_rs {
                            let own = &slices[picture.slice_header_index((x_c + i) << shift_w, (y_c + j) << shift_h)];
                            if !own.slice_loop_filter_across_slices_enabled_flag {
                                edge_zero = true;
                                break;
                            }
                        }
                        if neighbour_slice > ctb_slice_addr_rs && !neighbour_header.slice_loop_filter_across_slices_enabled_flag {
                            edge_zero = true;
                            break;
                        }
                        let neighbour_tile = pps.tile_id_rs[((x_s >> ctb_shift_w) + (y_s >> ctb_shift_h) * width_in_ctbs) as usize];
                        let own_tile = pps.tile_id_rs[((x_c >> ctb_shift_w) + (y_c >> ctb_shift_h) * width_in_ctbs) as usize];
                        if !pps.loop_filter_across_tiles_enabled_flag && neighbour_tile != own_tile {
                            edge_zero = true;
                            break;
                        }
                    }
                }
                if edge_zero {
                    continue;
                }
                let index = ((y_c + j) * stride + x_c + i) as usize;
                let centre = input[index] as i32;
                let first = input[(index as isize + (h_pos[0] + v_pos[0] * stride) as isize) as usize] as i32;
                let second = input[(index as isize + (h_pos[1] + v_pos[1] * stride) as isize) as usize] as i32;
                let edge_index = sign(centre - first) + sign(centre - second);
                let offset = offset_by_edge[(edge_index + 2) as usize];
                picture.planes[c_idx][index] = (centre + offset).clamp(0, maximum) as u16;
            }
        }
    } else {
        let band_shift = bit_depth - 5;
        let left_class = info.sao_band_position[c_idx] as i32;
        let mut band_table = [0usize; 32];
        for k in 0..4 {
            band_table[((k + left_class) & 31) as usize] = k as usize + 1;
        }
        for j in 0..block_height {
            for i in 0..block_width {
                if extended_tests && skip(picture, i, j) {
                    continue;
                }
                let index = ((y_c + j) * stride + x_c + i) as usize;
                let pixel = (input[index] as i32).clamp(0, maximum);
                let band = band_table[(pixel >> band_shift) as usize];
                if band > 0 {
                    let offset = info.sao_offset_val[c_idx][band - 1] as i32;
                    picture.planes[c_idx][index] = (input[index] as i32 + offset).clamp(0, maximum) as u16;
                }
            }
        }
    }
}

/// `apply_sample_adaptive_offset`.
pub fn apply_sample_adaptive_offset(picture: &mut Picture, slices: &[SliceHeader]) {
    let sps = picture.sps.clone();
    if !sps.sample_adaptive_offset_enabled_flag {
        return;
    }
    let ctb_size = 1 << sps.log2_ctb_size_y;
    let components = if sps.chroma_array_type == CHROMA_MONO { 1 } else { 3 };
    let inputs: Vec<Vec<u16>> = (0..components).map(|c_idx| picture.planes[c_idx].clone()).collect();
    for y_ctb in 0..sps.pic_height_in_ctbs_y {
        for x_ctb in 0..sps.pic_width_in_ctbs_y {
            let Some(header) = slices.get(picture.slice_header_index_ctb(x_ctb, y_ctb)) else {
                break;
            };
            let (luma, chroma) = (header.slice_sao_luma_flag, header.slice_sao_chroma_flag);
            if luma {
                apply_sao_ctb(picture, slices, &inputs[0], x_ctb, y_ctb, 0, ctb_size, ctb_size);
            }
            if chroma && components == 3 {
                let width = ctb_size / sps.sub_width_c;
                let height = ctb_size / sps.sub_height_c;
                apply_sao_ctb(picture, slices, &inputs[1], x_ctb, y_ctb, 1, width, height);
                apply_sao_ctb(picture, slices, &inputs[2], x_ctb, y_ctb, 2, width, height);
            }
        }
    }
}
