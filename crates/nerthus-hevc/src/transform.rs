//! Inverse transforms and residual helpers (`fallback-dct.cc`).

const MAT_DST: [[i32; 4]; 4] = [[29, 55, 74, 84], [74, 74, 0, -74], [84, -29, -74, 55], [55, -84, 74, -29]];

#[rustfmt::skip]
const MAT_DCT: [[i8; 32]; 32] = [
    [64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64],
    [90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4, -4, -13, -22, -31, -38, -46, -54, -61, -67, -73, -78, -82, -85, -88, -90, -90],
    [90, 87, 80, 70, 57, 43, 25, 9, -9, -25, -43, -57, -70, -80, -87, -90, -90, -87, -80, -70, -57, -43, -25, -9, 9, 25, 43, 57, 70, 80, 87, 90],
    [90, 82, 67, 46, 22, -4, -31, -54, -73, -85, -90, -88, -78, -61, -38, -13, 13, 38, 61, 78, 88, 90, 85, 73, 54, 31, 4, -22, -46, -67, -82, -90],
    [89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89, 89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89],
    [88, 67, 31, -13, -54, -82, -90, -78, -46, -4, 38, 73, 90, 85, 61, 22, -22, -61, -85, -90, -73, -38, 4, 46, 78, 90, 82, 54, 13, -31, -67, -88],
    [87, 57, 9, -43, -80, -90, -70, -25, 25, 70, 90, 80, 43, -9, -57, -87, -87, -57, -9, 43, 80, 90, 70, 25, -25, -70, -90, -80, -43, 9, 57, 87],
    [85, 46, -13, -67, -90, -73, -22, 38, 82, 88, 54, -4, -61, -90, -78, -31, 31, 78, 90, 61, 4, -54, -88, -82, -38, 22, 73, 90, 67, 13, -46, -85],
    [83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83],
    [82, 22, -54, -90, -61, 13, 78, 85, 31, -46, -90, -67, 4, 73, 88, 38, -38, -88, -73, -4, 67, 90, 46, -31, -85, -78, -13, 61, 90, 54, -22, -82],
    [80, 9, -70, -87, -25, 57, 90, 43, -43, -90, -57, 25, 87, 70, -9, -80, -80, -9, 70, 87, 25, -57, -90, -43, 43, 90, 57, -25, -87, -70, 9, 80],
    [78, -4, -82, -73, 13, 85, 67, -22, -88, -61, 31, 90, 54, -38, -90, -46, 46, 90, 38, -54, -90, -31, 61, 88, 22, -67, -85, -13, 73, 82, 4, -78],
    [75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75, 75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75],
    [73, -31, -90, -22, 78, 67, -38, -90, -13, 82, 61, -46, -88, -4, 85, 54, -54, -85, 4, 88, 46, -61, -82, 13, 90, 38, -67, -78, 22, 90, 31, -73],
    [70, -43, -87, 9, 90, 25, -80, -57, 57, 80, -25, -90, -9, 87, 43, -70, -70, 43, 87, -9, -90, -25, 80, 57, -57, -80, 25, 90, 9, -87, -43, 70],
    [67, -54, -78, 38, 85, -22, -90, 4, 90, 13, -88, -31, 82, 46, -73, -61, 61, 73, -46, -82, 31, 88, -13, -90, -4, 90, 22, -85, -38, 78, 54, -67],
    [64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64],
    [61, -73, -46, 82, 31, -88, -13, 90, -4, -90, 22, 85, -38, -78, 54, 67, -67, -54, 78, 38, -85, -22, 90, 4, -90, 13, 88, -31, -82, 46, 73, -61],
    [57, -80, -25, 90, -9, -87, 43, 70, -70, -43, 87, 9, -90, 25, 80, -57, -57, 80, 25, -90, 9, 87, -43, -70, 70, 43, -87, -9, 90, -25, -80, 57],
    [54, -85, -4, 88, -46, -61, 82, 13, -90, 38, 67, -78, -22, 90, -31, -73, 73, 31, -90, 22, 78, -67, -38, 90, -13, -82, 61, 46, -88, 4, 85, -54],
    [50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50, 50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50],
    [46, -90, 38, 54, -90, 31, 61, -88, 22, 67, -85, 13, 73, -82, 4, 78, -78, -4, 82, -73, -13, 85, -67, -22, 88, -61, -31, 90, -54, -38, 90, -46],
    [43, -90, 57, 25, -87, 70, 9, -80, 80, -9, -70, 87, -25, -57, 90, -43, -43, 90, -57, -25, 87, -70, -9, 80, -80, 9, 70, -87, 25, 57, -90, 43],
    [38, -88, 73, -4, -67, 90, -46, -31, 85, -78, 13, 61, -90, 54, 22, -82, 82, -22, -54, 90, -61, -13, 78, -85, 31, 46, -90, 67, 4, -73, 88, -38],
    [36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36],
    [31, -78, 90, -61, 4, 54, -88, 82, -38, -22, 73, -90, 67, -13, -46, 85, -85, 46, 13, -67, 90, -73, 22, 38, -82, 88, -54, -4, 61, -90, 78, -31],
    [25, -70, 90, -80, 43, 9, -57, 87, -87, 57, -9, -43, 80, -90, 70, -25, -25, 70, -90, 80, -43, -9, 57, -87, 87, -57, 9, 43, -80, 90, -70, 25],
    [22, -61, 85, -90, 73, -38, -4, 46, -78, 90, -82, 54, -13, -31, 67, -88, 88, -67, 31, 13, -54, 82, -90, 78, -46, 4, 38, -73, 90, -85, 61, -22],
    [18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18, 18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18],
    [13, -38, 61, -78, 88, -90, 85, -73, 54, -31, 4, 22, -46, 67, -82, 90, -90, 82, -67, 46, -22, -4, 31, -54, 73, -85, 90, -88, 78, -61, 38, -13],
    [9, -25, 43, -57, 70, -80, 87, -90, 90, -87, 80, -70, 57, -43, 25, -9, -9, 25, -43, 57, -70, 80, -87, 90, -90, 87, -80, 70, -57, 43, -25, 9],
    [4, -13, 22, -31, 38, -46, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90, 90, -90, 88, -85, 82, -78, 73, -67, 61, -54, 46, -38, 31, -22, 13, -4],
];

fn clip_i16(value: i32) -> i32 {
    value.clamp(-32768, 32767)
}

/// The DST's vertical pass with the clip `transform_4x4_luma_add` and
/// `transform_idst_4x4` share.
fn dst_vertical(coeffs: &[i16], minimum: i32, maximum: i32) -> [[i32; 4]; 4] {
    let mut g = [[0i32; 4]; 4];
    for c in 0..4 {
        for i in 0..4 {
            let mut sum = 0i32;
            for j in 0..4 {
                sum += MAT_DST[j][i] * coeffs[c + j * 4] as i32;
            }
            g[i][c] = ((sum + 64) >> 7).clamp(minimum, maximum);
        }
    }
    g
}

/// `transform_4x4_luma_add_{8,16}`: the residual, clipped to 16 bits.
pub fn dst_4x4_residual(coeffs: &[i16], bit_depth: i32) -> [i32; 16] {
    let post_shift = 20 - bit_depth;
    let rounding = 1 << (post_shift - 1);
    let g = dst_vertical(coeffs, -32768, 32767);
    let mut residual = [0i32; 16];
    for y in 0..4 {
        for i in 0..4 {
            let mut sum = 0i32;
            for j in 0..4 {
                sum += MAT_DST[j][i] * g[y][j];
            }
            residual[y * 4 + i] = clip_i16((sum + rounding) >> post_shift);
        }
    }
    residual
}

/// `transform_idst_4x4`.
pub fn idst_4x4(residual: &mut [i32], coeffs: &[i16], bd_shift: i32, max_coeff_bits: i32) {
    let coeff_max = (1 << max_coeff_bits) - 1;
    let coeff_min = -(1 << max_coeff_bits);
    let rounding = 1 << (bd_shift - 1);
    let g = dst_vertical(coeffs, coeff_min, coeff_max);
    for y in 0..4 {
        for i in 0..4 {
            let mut sum = 0i32;
            for j in 0..4 {
                sum += MAT_DST[j][i] * g[y][j];
            }
            residual[y * 4 + i] = (sum + rounding) >> bd_shift;
        }
    }
}

/// The IDCT's vertical pass (`g` is row-major, as in C).
fn idct_vertical(coeffs: &[i16], nt: usize, minimum: i32, maximum: i32) -> Vec<i32> {
    let fact = 32 / nt;
    let mut g = vec![0i32; nt * nt];
    for c in 0..nt {
        for i in 0..nt {
            let mut sum = 0i32;
            for j in 0..nt {
                sum += MAT_DCT[fact * j][i] as i32 * coeffs[c + j * nt] as i32;
            }
            g[c + i * nt] = ((sum + 64) >> 7).clamp(minimum, maximum);
        }
    }
    g
}

fn idct_horizontal(g: &[i32], nt: usize, residual: &mut [i32], bd_shift: i32) {
    let fact = 32 / nt;
    let rounding = 1 << (bd_shift - 1);
    for y in 0..nt {
        for i in 0..nt {
            let mut sum = 0i32;
            for j in 0..nt {
                sum += MAT_DCT[fact * j][i] as i32 * g[y * nt + j];
            }
            residual[y * nt + i] = (sum + rounding) >> bd_shift;
        }
    }
}

/// `transform_idct_add`: the residual (the second pass is not clipped).
pub fn idct_residual(coeffs: &[i16], nt: usize, bit_depth: i32) -> Vec<i32> {
    let g = idct_vertical(coeffs, nt, -32768, 32767);
    let mut residual = vec![0i32; nt * nt];
    idct_horizontal(&g, nt, &mut residual, 20 - bit_depth);
    residual
}

/// `transform_idct_fallback`.
pub fn idct(residual: &mut [i32], nt: usize, coeffs: &[i16], bd_shift: i32, max_coeff_bits: i32) {
    let coeff_max = (1 << max_coeff_bits) - 1;
    let coeff_min = -(1 << max_coeff_bits);
    let g = idct_vertical(coeffs, nt, coeff_min, coeff_max);
    idct_horizontal(&g, nt, residual, bd_shift);
}

/// `transform_skip_residual_fallback`.
pub fn transform_skip_residual(residual: &mut [i32], coeffs: &[i16], nt: usize, ts_shift: i32, bd_shift: i32) {
    let rounding = 1i32.wrapping_shl((bd_shift - 1) as u32);
    for index in 0..nt * nt {
        let value = (coeffs[index] as i32).wrapping_shl(ts_shift as u32);
        residual[index] = value.wrapping_add(rounding) >> bd_shift;
    }
}

/// `rdpcm_v_fallback`.
pub fn rdpcm_v(residual: &mut [i32], coeffs: &[i16], nt: usize, ts_shift: i32, bd_shift: i32) {
    let rounding = 1i32.wrapping_shl((bd_shift - 1) as u32);
    for x in 0..nt {
        let mut sum = 0i32;
        for y in 0..nt {
            let value = (coeffs[x + y * nt] as i32).wrapping_shl(ts_shift as u32);
            sum = sum.wrapping_add(value.wrapping_add(rounding) >> bd_shift);
            residual[y * nt + x] = sum;
        }
    }
}

/// `rdpcm_h_fallback`.
pub fn rdpcm_h(residual: &mut [i32], coeffs: &[i16], nt: usize, ts_shift: i32, bd_shift: i32) {
    let rounding = 1i32.wrapping_shl((bd_shift - 1) as u32);
    for y in 0..nt {
        let mut sum = 0i32;
        for x in 0..nt {
            let value = (coeffs[x + y * nt] as i32).wrapping_shl(ts_shift as u32);
            sum = sum.wrapping_add(value.wrapping_add(rounding) >> bd_shift);
            residual[y * nt + x] = sum;
        }
    }
}

/// `transform_bypass_fallback`.
pub fn transform_bypass(residual: &mut [i32], coeffs: &[i16], nt: usize) {
    for index in 0..nt * nt {
        residual[index] = coeffs[index] as i32;
    }
}

/// `transform_bypass_rdpcm_v_fallback`.
pub fn transform_bypass_rdpcm_v(residual: &mut [i32], coeffs: &[i16], nt: usize) {
    for x in 0..nt {
        let mut sum = 0i32;
        for y in 0..nt {
            sum = sum.wrapping_add(coeffs[x + y * nt] as i32);
            residual[y * nt + x] = sum;
        }
    }
}

/// `transform_bypass_rdpcm_h_fallback`.
pub fn transform_bypass_rdpcm_h(residual: &mut [i32], coeffs: &[i16], nt: usize) {
    for y in 0..nt {
        let mut sum = 0i32;
        for x in 0..nt {
            sum = sum.wrapping_add(coeffs[x + y * nt] as i32);
            residual[y * nt + x] = sum;
        }
    }
}

/// `rotate_coefficients_fallback`.
pub fn rotate_coefficients(coeffs: &mut [i16], nt: usize) {
    for y in 0..nt / 2 {
        for x in 0..nt {
            coeffs.swap(y * nt + x, (nt - 1 - y) * nt + nt - 1 - x);
        }
    }
}
