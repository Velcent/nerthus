//! The deblocking filter (`deblock.cc`) for intra pictures, where every
//! marked edge has boundary strength 2.

use crate::params::{CHROMA_420, CHROMA_MONO};
use crate::picture::{Picture, DEBLOCK_FLAG_HORIZ, DEBLOCK_FLAG_VERTI, DEBLOCK_PB_EDGE_HORIZ, DEBLOCK_PB_EDGE_VERTI, MODE_INTRA, PART_2NXN, PART_2NXND, PART_2NXNU, PART_NLX2N, PART_NRX2N, PART_NX2N, PART_NXN};
use crate::slice::table_8_22;
use crate::slice_header::SliceHeader;
use crate::util::clip3;

const TABLE_8_23_BETA: [i32; 52] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64];
const TABLE_8_23_TC: [i32; 54] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24];

fn clip_bit_depth(value: i32, bit_depth: i32) -> i32 {
    value.clamp(0, (1 << bit_depth) - 1)
}

/// `markTransformBlockBoundary`.
fn mark_transform_block_boundary(picture: &mut Picture, x0: i32, y0: i32, log2_trafo_size: i32, trafo_depth: i32, filter_left: u8, filter_top: u8) {
    if picture.split_transform_flag(x0, y0, trafo_depth) {
        let x1 = x0 + ((1 << log2_trafo_size) >> 1);
        let y1 = y0 + ((1 << log2_trafo_size) >> 1);
        mark_transform_block_boundary(picture, x0, y0, log2_trafo_size - 1, trafo_depth + 1, filter_left, filter_top);
        mark_transform_block_boundary(picture, x1, y0, log2_trafo_size - 1, trafo_depth + 1, DEBLOCK_FLAG_VERTI, filter_top);
        mark_transform_block_boundary(picture, x0, y1, log2_trafo_size - 1, trafo_depth + 1, filter_left, DEBLOCK_FLAG_HORIZ);
        mark_transform_block_boundary(picture, x1, y1, log2_trafo_size - 1, trafo_depth + 1, DEBLOCK_FLAG_VERTI, DEBLOCK_FLAG_HORIZ);
    } else {
        let size = 1 << log2_trafo_size;
        let mut k = 0;
        while k < size {
            picture.set_deblk_flags(x0, y0 + k, filter_left);
            k += 4;
        }
        let mut k = 0;
        while k < size {
            picture.set_deblk_flags(x0 + k, y0, filter_top);
            k += 4;
        }
    }
}

/// `markPredictionBlockBoundary`.
fn mark_prediction_block_boundary(picture: &mut Picture, x0: i32, y0: i32, log2_cb_size: i32) {
    let part_mode = picture.part_mode(x0, y0);
    let cb_size = 1 << log2_cb_size;
    let cb_size2 = 1 << (log2_cb_size - 1);
    let cb_size4 = 1 << (log2_cb_size - 2);
    for k in 0..cb_size {
        match part_mode {
            PART_NXN => {
                picture.set_deblk_flags(x0 + cb_size2, y0 + k, DEBLOCK_PB_EDGE_VERTI);
                picture.set_deblk_flags(x0 + k, y0 + cb_size2, DEBLOCK_PB_EDGE_HORIZ);
            }
            PART_NX2N => picture.set_deblk_flags(x0 + cb_size2, y0 + k, DEBLOCK_PB_EDGE_VERTI),
            PART_2NXN => picture.set_deblk_flags(x0 + k, y0 + cb_size2, DEBLOCK_PB_EDGE_HORIZ),
            PART_NLX2N => picture.set_deblk_flags(x0 + cb_size4, y0 + k, DEBLOCK_PB_EDGE_VERTI),
            PART_NRX2N => picture.set_deblk_flags(x0 + cb_size2 + cb_size4, y0 + k, DEBLOCK_PB_EDGE_VERTI),
            PART_2NXNU => picture.set_deblk_flags(x0 + k, y0 + cb_size4, DEBLOCK_PB_EDGE_HORIZ),
            PART_2NXND => picture.set_deblk_flags(x0 + k, y0 + cb_size2 + cb_size4, DEBLOCK_PB_EDGE_HORIZ),
            _ => {}
        }
    }
}

/// `derive_edgeFlags_CTBRow`.
fn derive_edge_flags_ctb_row(picture: &mut Picture, slices: &[SliceHeader], ctb_y: i32) -> bool {
    let sps = picture.sps.clone();
    let pps = picture.pps.clone();
    let min_cb_size = sps.min_cb_size_y;
    let ctb_mask = (1 << sps.log2_ctb_size_y) - 1;
    let ctb_shift = sps.log2_ctb_size_y;
    let width_in_ctbs = sps.pic_width_in_ctbs_y;
    let mut enabled = false;

    let cb_y_start = (ctb_y << sps.log2_ctb_size_y) >> sps.log2_min_cb_size_y;
    let cb_y_end = (((ctb_y + 1) << sps.log2_ctb_size_y) >> sps.log2_min_cb_size_y).min(sps.pic_height_in_min_cbs_y);
    for cb_y in cb_y_start..cb_y_end {
        for cb_x in 0..sps.pic_width_in_min_cbs_y {
            let log2_cb_size = picture.cb_info.data[(cb_x + cb_y * picture.cb_info.width) as usize].log2_cb_size as i32;
            if log2_cb_size == 0 {
                continue;
            }
            let x0 = cb_x * min_cb_size;
            let y0 = cb_y * min_cb_size;
            let x0_ctb = x0 >> ctb_shift;
            let y0_ctb = y0 >> ctb_shift;
            let Some(header) = slices.get(picture.slice_header_index(x0, y0)) else {
                return false;
            };

            let mut filter_left = if x0 == 0 { 0 } else { DEBLOCK_FLAG_VERTI };
            let mut filter_top = if y0 == 0 { 0 } else { DEBLOCK_FLAG_HORIZ };
            if x0 != 0 && (x0 & ctb_mask) == 0 {
                let left = slices.get(picture.slice_header_index(x0 - 1, y0));
                let other_slice = left.is_some_and(|left| header.slice_addr_rs != left.slice_addr_rs);
                let other_tile = pps.tile_id_rs[(x0_ctb + y0_ctb * width_in_ctbs) as usize] != pps.tile_id_rs[(((x0 - 1) >> ctb_shift) + y0_ctb * width_in_ctbs) as usize];
                if (!header.slice_loop_filter_across_slices_enabled_flag && other_slice) || (!pps.loop_filter_across_tiles_enabled_flag && other_tile) {
                    filter_left = 0;
                }
            }
            if y0 != 0 && (y0 & ctb_mask) == 0 {
                let top = slices.get(picture.slice_header_index(x0, y0 - 1));
                let other_slice = top.is_some_and(|top| header.slice_addr_rs != top.slice_addr_rs);
                let other_tile = pps.tile_id_rs[(x0_ctb + y0_ctb * width_in_ctbs) as usize] != pps.tile_id_rs[(x0_ctb + ((y0 - 1) >> ctb_shift) * width_in_ctbs) as usize];
                if (!header.slice_loop_filter_across_slices_enabled_flag && other_slice) || (!pps.loop_filter_across_tiles_enabled_flag && other_tile) {
                    filter_top = 0;
                }
            }

            if !header.slice_deblocking_filter_disabled_flag {
                enabled = true;
                mark_transform_block_boundary(picture, x0, y0, log2_cb_size, 0, filter_left, filter_top);
                mark_prediction_block_boundary(picture, x0, y0, log2_cb_size);
            }
        }
    }
    enabled
}

/// `derive_boundaryStrength` for intra pictures.
fn derive_boundary_strength(picture: &mut Picture, vertical: bool, y_start: i32, y_end: i32, x_start: i32, x_end: i32) {
    let (x_increment, y_increment) = if vertical { (2, 1) } else { (1, 2) };
    let (x_offset, y_offset) = if vertical { (1, 0) } else { (0, 1) };
    let edge_mask = if vertical { DEBLOCK_FLAG_VERTI | DEBLOCK_PB_EDGE_VERTI } else { DEBLOCK_FLAG_HORIZ | DEBLOCK_PB_EDGE_HORIZ };
    let x_end = x_end.min(picture.deblk_info.width);
    let y_end = y_end.min(picture.deblk_info.height);
    for y in (y_start..y_end).step_by(y_increment as usize) {
        for x in (x_start..x_end).step_by(x_increment as usize) {
            let x_di = x << 2;
            let y_di = y << 2;
            let mut strength = 0;
            if picture.deblk_flags(x_di, y_di) & edge_mask != 0 {
                let p_intra = picture.pred_mode(x_di - x_offset, y_di - y_offset) == MODE_INTRA;
                let q_intra = picture.pred_mode(x_di, y_di) == MODE_INTRA;
                if p_intra || q_intra {
                    strength = 2;
                } else if picture.deblk_flags(x_di, y_di) & (if vertical { DEBLOCK_FLAG_VERTI } else { DEBLOCK_FLAG_HORIZ }) != 0 && (picture.nonzero_coefficient(x_di, y_di) || picture.nonzero_coefficient(x_di - x_offset, y_di - y_offset)) {
                    strength = 1;
                }
            }
            picture.set_deblk_bs(x_di, y_di, strength);
        }
    }
}

/// `edge_filtering_luma_internal`.
fn edge_filtering_luma(picture: &mut Picture, slices: &[SliceHeader], vertical: bool, y_start: i32, y_end: i32, x_start: i32, x_end: i32) {
    let sps = picture.sps.clone();
    let (x_increment, y_increment) = if vertical { (2, 1) } else { (1, 2) };
    let stride = picture.stride(0);
    let bit_depth = sps.bit_depth_y;
    let x_end = x_end.min(picture.deblk_info.width);
    let y_end = y_end.min(picture.deblk_info.height);

    for y in (y_start..y_end).step_by(y_increment as usize) {
        for x in (x_start..x_end).step_by(x_increment as usize) {
            let x_di = x << 2;
            let y_di = y << 2;
            let strength = picture.deblk_bs(x_di, y_di);
            if strength > 0 {
                let origin = (y_di * stride + x_di) as isize;
                let step_across = if vertical { 1 } else { stride as isize };
                let step_along = if vertical { stride as isize } else { 1 };
                let plane = &picture.planes[0];
                let sample = |k: isize, i: isize| -> i32 { plane[(origin + k * step_along + i * step_across) as usize] as i32 };
                let mut q = [[0i32; 4]; 4];
                let mut p = [[0i32; 4]; 4];
                for k in 0..4 {
                    for i in 0..4 {
                        q[k][i] = sample(k as isize, i as isize);
                        p[k][i] = sample(k as isize, -(i as isize) - 1);
                    }
                }

                let qp_q = picture.qpy(x_di, y_di);
                let qp_p = if vertical { picture.qpy(x_di - 1, y_di) } else { picture.qpy(x_di, y_di - 1) };
                let qp_l = (qp_q + qp_p + 1) >> 1;
                let Some(header) = slices.get(picture.slice_header_index(x_di, y_di)) else {
                    continue;
                };
                let q_beta = clip3(0, 51, qp_l + header.slice_beta_offset);
                let beta = TABLE_8_23_BETA[q_beta as usize] * (1 << (bit_depth - 8));
                let q_tc = clip3(0, 53, qp_l + 2 * (strength - 1) + header.slice_tc_offset);
                let tc = TABLE_8_23_TC[q_tc as usize] * (1 << (bit_depth - 8));

                let mut d_e = 0;
                let mut d_ep = 0;
                let mut d_eq = 0;
                let dp0 = (p[0][2] - 2 * p[0][1] + p[0][0]).abs();
                let dp3 = (p[3][2] - 2 * p[3][1] + p[3][0]).abs();
                let dq0 = (q[0][2] - 2 * q[0][1] + q[0][0]).abs();
                let dq3 = (q[3][2] - 2 * q[3][1] + q[3][0]).abs();
                let dpq0 = dp0 + dq0;
                let dpq3 = dp3 + dq3;
                let dp = dp0 + dp3;
                let dq = dq0 + dq3;
                let d = dpq0 + dpq3;
                if d < beta {
                    let d_sam0 = 2 * dpq0 < (beta >> 2) && (p[0][3] - p[0][0]).abs() + (q[0][0] - q[0][3]).abs() < (beta >> 3) && (p[0][0] - q[0][0]).abs() < ((5 * tc + 1) >> 1);
                    let d_sam3 = 2 * dpq3 < (beta >> 2) && (p[3][3] - p[3][0]).abs() + (q[3][0] - q[3][3]).abs() < (beta >> 3) && (p[3][0] - q[3][0]).abs() < ((5 * tc + 1) >> 1);
                    d_e = if d_sam0 && d_sam3 { 2 } else { 1 };
                    if dp < ((beta + (beta >> 1)) >> 3) {
                        d_ep = 1;
                    }
                    if dq < ((beta + (beta >> 1)) >> 3) {
                        d_eq = 1;
                    }
                }

                if d_e != 0 {
                    let (x_p, y_p) = if vertical { (x_di - 1, y_di) } else { (x_di, y_di - 1) };
                    let pcm_off = sps.pcm_loop_filter_disable_flag;
                    let filter_p = !(pcm_off && picture.pcm_flag(x_p, y_p)) && !picture.cu_transquant_bypass(x_p, y_p);
                    let filter_q = !(pcm_off && picture.pcm_flag(x_di, y_di)) && !picture.cu_transquant_bypass(x_di, y_di);

                    let plane = &mut picture.planes[0];
                    let mut write = |k: usize, i: isize, value: i32| {
                        plane[(origin + k as isize * step_along + i * step_across) as usize] = value as u16;
                    };
                    for k in 0..4 {
                        let [p0, p1, p2, p3] = p[k];
                        let [q0, q1, q2, q3] = q[k];
                        if d_e == 2 {
                            let p_new = [
                                clip3(p0 - 2 * tc, p0 + 2 * tc, (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3),
                                clip3(p1 - 2 * tc, p1 + 2 * tc, (p2 + p1 + p0 + q0 + 2) >> 2),
                                clip3(p2 - 2 * tc, p2 + 2 * tc, (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3),
                            ];
                            let q_new = [
                                clip3(q0 - 2 * tc, q0 + 2 * tc, (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3),
                                clip3(q1 - 2 * tc, q1 + 2 * tc, (p0 + q0 + q1 + q2 + 2) >> 2),
                                clip3(q2 - 2 * tc, q2 + 2 * tc, (p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3),
                            ];
                            for i in 0..3 {
                                if filter_p {
                                    write(k, -(i as isize) - 1, p_new[i]);
                                }
                                if filter_q {
                                    write(k, i as isize, q_new[i]);
                                }
                            }
                        } else {
                            let mut delta = (9 * (q0 - p0) - 3 * (q1 - p1) + 8) >> 4;
                            if delta.abs() < tc * 10 {
                                delta = clip3(-tc, tc, delta);
                                if filter_p {
                                    write(k, -1, clip_bit_depth(p0 + delta, bit_depth));
                                }
                                if filter_q {
                                    write(k, 0, clip_bit_depth(q0 - delta, bit_depth));
                                }
                                if d_ep == 1 && filter_p {
                                    let delta_p = clip3(-(tc >> 1), tc >> 1, (((p2 + p0 + 1) >> 1) - p1 + delta) >> 1);
                                    write(k, -2, clip_bit_depth(p1 + delta_p, bit_depth));
                                }
                                if d_eq == 1 && filter_q {
                                    let delta_q = clip3(-(tc >> 1), tc >> 1, (((q2 + q0 + 1) >> 1) - q1 - delta) >> 1);
                                    write(k, 1, clip_bit_depth(q1 + delta_q, bit_depth));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// `edge_filtering_chroma_internal`.
fn edge_filtering_chroma(picture: &mut Picture, slices: &[SliceHeader], vertical: bool, y_start: i32, y_end: i32, x_start: i32, x_end: i32) {
    let sps = picture.sps.clone();
    let pps = picture.pps.clone();
    let sub_width_c = sps.sub_width_c;
    let sub_height_c = sps.sub_height_c;
    let (mut x_increment, mut y_increment) = if vertical { (2, 1) } else { (1, 2) };
    x_increment *= sub_width_c;
    y_increment *= sub_height_c;
    let stride = picture.stride(1);
    let x_end = x_end.min(picture.deblk_info.width);
    let y_end = y_end.min(picture.deblk_info.height);
    let bit_depth = sps.bit_depth_c;

    for y in (y_start..y_end).step_by(y_increment as usize) {
        for x in (x_start..x_end).step_by(x_increment as usize) {
            let x_di = x << (3 - sub_width_c);
            let y_di = y << (3 - sub_height_c);
            let strength = picture.deblk_bs(x_di * sub_width_c, y_di * sub_height_c);
            if strength > 1 {
                for plane_index in 0..2usize {
                    let qp_offset = if plane_index == 0 { pps.pic_cb_qp_offset } else { pps.pic_cr_qp_offset };
                    let origin = (y_di * stride + x_di) as isize;
                    let step_across = if vertical { 1 } else { stride as isize };
                    let step_along = if vertical { stride as isize } else { 1 };
                    let mut p = [[0i32; 4]; 2];
                    let mut q = [[0i32; 4]; 2];
                    {
                        let plane = &picture.planes[plane_index + 1];
                        for i in 0..2 {
                            for k in 0..4 {
                                q[i][k] = plane[(origin + k as isize * step_along + i as isize * step_across) as usize] as i32;
                                p[i][k] = plane[(origin + k as isize * step_along - (i as isize + 1) * step_across) as usize] as i32;
                            }
                        }
                    }

                    let x_luma = sub_width_c * x_di;
                    let y_luma = sub_height_c * y_di;
                    let qp_q = picture.qpy(x_luma, y_luma);
                    let qp_p = if vertical { picture.qpy(x_luma - 1, y_luma) } else { picture.qpy(x_luma, y_luma - 1) };
                    let qp_i = ((qp_q + qp_p + 1) >> 1) + qp_offset;
                    let qp_c = if sps.chroma_array_type == CHROMA_420 { table_8_22(qp_i) } else { qp_i.min(51) };
                    let Some(header) = slices.get(picture.slice_header_index(x_luma, y_luma)) else {
                        continue;
                    };
                    let q_index = clip3(0, 53, qp_c + 2 * (strength - 1) + header.slice_tc_offset);
                    let tc = TABLE_8_23_TC[q_index as usize] * (1 << (bit_depth - 8));

                    let (x_p, y_p) = if vertical { (x_luma - 1, y_luma) } else { (x_luma, y_luma - 1) };
                    let pcm_off = sps.pcm_loop_filter_disable_flag;
                    let filter_p = !(pcm_off && picture.pcm_flag(x_p, y_p)) && !picture.cu_transquant_bypass(x_p, y_p);
                    let filter_q = !(pcm_off && picture.pcm_flag(x_luma, y_luma)) && !picture.cu_transquant_bypass(x_luma, y_luma);

                    let plane = &mut picture.planes[plane_index + 1];
                    for k in 0..4 {
                        let delta = clip3(-tc, tc, (((q[0][k] - p[0][k]) * 4) + p[1][k] - q[1][k] + 4) >> 3);
                        let base = origin + k as isize * step_along;
                        if filter_p {
                            plane[(base - step_across) as usize] = clip_bit_depth(p[0][k] + delta, bit_depth) as u16;
                        }
                        if filter_q {
                            plane[base as usize] = clip_bit_depth(q[0][k] - delta, bit_depth) as u16;
                        }
                    }
                }
            }
        }
    }
}

/// The deblocking filter over the whole picture: every CTB row's
/// vertical edges, then every row's horizontal ones.
pub fn apply_deblocking_filter(picture: &mut Picture, slices: &[SliceHeader]) {
    let sps = picture.sps.clone();
    let deblock_size = sps.ctb_size_y / 4;
    let width = picture.deblk_info.width;
    let height = picture.deblk_info.height;
    let mut row_enabled = vec![false; sps.pic_height_in_ctbs_y as usize];
    for pass in 0..2 {
        let vertical = pass == 0;
        for ctb_y in 0..sps.pic_height_in_ctbs_y {
            let first = ctb_y * deblock_size;
            let last = ((ctb_y + 1) * deblock_size).min(height);
            if vertical {
                row_enabled[ctb_y as usize] = derive_edge_flags_ctb_row(picture, slices, ctb_y);
            }
            if !row_enabled[ctb_y as usize] {
                continue;
            }
            derive_boundary_strength(picture, vertical, first, last, 0, width);
            edge_filtering_luma(picture, slices, vertical, first, last, 0, width);
            if sps.chroma_array_type != CHROMA_MONO {
                edge_filtering_chroma(picture, slices, vertical, first, last, 0, width);
            }
        }
    }
}
