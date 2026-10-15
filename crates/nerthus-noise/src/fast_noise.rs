//! Scalar port of FastNoiseSIMD (Jordan Peck, MIT licence).
//!
//! This follows the `FastNoiseSIMD_CLib` build of the library. Every constant
//! and operation order here was checked against that binary so that a given
//! seed produces the same field as it does.

use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

const X_PRIME: i32 = 1619;
const Y_PRIME: i32 = 31337;
const Z_PRIME: i32 = 6971;
const HASH_MULTIPLIER: i32 = 60493;
const HASH_TO_FLOAT: f32 = 1.0 / 2_147_483_648.0;
const BIT10_MASK: i32 = 0x3ff;
const CELL_HALF_RANGE: f32 = 511.5;
const F3: f32 = 1.0 / 3.0;
const G3: f32 = 1.0 / 6.0;
const G33: f32 = -0.5;
const CUBIC_BOUNDING: f32 = 1.0 / (1.5 * 1.5 * 1.5);
const CELLULAR_INDEX_MAX: usize = 3;
const FAR_DISTANCE: f32 = 999_999.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    Value,
    ValueFractal,
    Perlin,
    PerlinFractal,
    Simplex,
    SimplexFractal,
    WhiteNoise,
    Cellular,
    Cubic,
    CubicFractal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractalType {
    Fbm,
    Billow,
    RigidMulti,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerturbType {
    None,
    Gradient,
    GradientFractal,
    Normalise,
    GradientNormalise,
    GradientFractalNormalise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellularDistanceFunction {
    Euclidean,
    Manhattan,
    Natural,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellularReturnType {
    CellValue,
    Distance,
    Distance2,
    Distance2Add,
    Distance2Sub,
    Distance2Mul,
    Distance2Div,
    NoiseLookup,
    Distance2Cave,
}

impl NoiseType {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Value,
            1 => Self::ValueFractal,
            2 => Self::Perlin,
            3 => Self::PerlinFractal,
            4 => Self::Simplex,
            6 => Self::WhiteNoise,
            7 => Self::Cellular,
            8 => Self::Cubic,
            9 => Self::CubicFractal,
            _ => Self::SimplexFractal,
        }
    }
}

impl FractalType {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Billow,
            2 => Self::RigidMulti,
            _ => Self::Fbm,
        }
    }
}

impl PerturbType {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Gradient,
            2 => Self::GradientFractal,
            3 => Self::Normalise,
            4 => Self::GradientNormalise,
            5 => Self::GradientFractalNormalise,
            _ => Self::None,
        }
    }
}

impl CellularDistanceFunction {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Manhattan,
            2 => Self::Natural,
            _ => Self::Euclidean,
        }
    }
}

impl CellularReturnType {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::CellValue,
            2 => Self::Distance2,
            3 => Self::Distance2Add,
            4 => Self::Distance2Sub,
            5 => Self::Distance2Mul,
            6 => Self::Distance2Div,
            7 => Self::NoiseLookup,
            8 => Self::Distance2Cave,
            _ => Self::Distance,
        }
    }
}

/// Settings captured for the cellular "noise lookup" return type.
#[derive(Clone, Copy, Debug)]
struct NoiseLookupSettings {
    noise_type: NoiseType,
    frequency: f32,
    fractal_type: FractalType,
    octaves: i32,
    lacunarity: f32,
    gain: f32,
    fractal_bounding: f32,
}

/// Perturb parameters resolved once per fill (mirrors `INIT_PERTURB_VALUES`).
#[derive(Clone, Copy, Debug)]
struct PerturbValues {
    amp: f32,
    frequency: f32,
    lacunarity: f32,
    gain: f32,
    normalise_length: f32,
}

/// Fractal parameters resolved once per fill.
#[derive(Clone, Copy, Debug)]
struct FractalValues {
    fractal_type: FractalType,
    octaves: i32,
    lacunarity: f32,
    gain: f32,
    bounding: f32,
}

#[derive(Clone, Debug)]
pub struct FastNoise {
    seed: i32,
    /// Whether to compute as the library's AVX2 build does (the build it
    /// runs on any current x86 PC): multiply-adds fused, rounded once, and
    /// `1 / sqrt(x)` from the approximate `RSQRTPS` instruction.
    avx2: bool,
    frequency: f32,
    noise_type: NoiseType,
    x_scale: f32,
    y_scale: f32,
    z_scale: f32,

    octaves: i32,
    lacunarity: f32,
    gain: f32,
    fractal_type: FractalType,
    fractal_bounding: f32,

    cellular_distance_function: CellularDistanceFunction,
    cellular_return_type: CellularReturnType,
    cellular_noise_lookup_type: NoiseType,
    cellular_noise_lookup_frequency: f32,
    cellular_distance_index0: usize,
    cellular_distance_index1: usize,
    cellular_jitter: f32,

    perturb_type: PerturbType,
    perturb_amp: f32,
    perturb_frequency: f32,
    perturb_octaves: i32,
    perturb_lacunarity: f32,
    perturb_gain: f32,
    perturb_fractal_bounding: f32,
    perturb_normalise_length: f32,
}

impl Default for FastNoise {
    fn default() -> Self {
        Self::new(1337)
    }
}

/// Whether new generators start in AVX2 mode ([`FastNoise::set_avx2`]).
static DEFAULT_AVX2: AtomicBool = AtomicBool::new(true);

/// Sets whether generators made from now on compute as the library's AVX2
/// build does (the default) or round every operation on its own, the way
/// its SSE2 code runs under an emulator that computes `RSQRTPS` exactly.
pub fn set_default_avx2(avx2: bool) {
    DEFAULT_AVX2.store(avx2, Ordering::Relaxed);
}

pub fn calculate_fractal_bounding(octaves: i32, gain: f32) -> f32 {
    let mut amp = gain;
    let mut amp_fractal = 1.0f32;
    let mut index = 1;
    while index < octaves {
        amp_fractal += amp;
        amp *= gain;
        index += 1;
    }
    1.0 / amp_fractal
}

impl FastNoise {
    pub fn new(seed: i32) -> Self {
        Self {
            seed,
            avx2: DEFAULT_AVX2.load(Ordering::Relaxed),
            frequency: 0.01,
            noise_type: NoiseType::SimplexFractal,
            x_scale: 1.0,
            y_scale: 1.0,
            z_scale: 1.0,
            octaves: 3,
            lacunarity: 2.0,
            gain: 0.5,
            fractal_type: FractalType::Fbm,
            fractal_bounding: calculate_fractal_bounding(3, 0.5),
            cellular_distance_function: CellularDistanceFunction::Euclidean,
            cellular_return_type: CellularReturnType::Distance,
            cellular_noise_lookup_type: NoiseType::Simplex,
            cellular_noise_lookup_frequency: 0.2,
            cellular_distance_index0: 0,
            cellular_distance_index1: 1,
            cellular_jitter: 0.45,
            perturb_type: PerturbType::None,
            perturb_amp: 1.0 / CELL_HALF_RANGE,
            perturb_frequency: 0.5,
            perturb_octaves: 3,
            perturb_lacunarity: 2.0,
            perturb_gain: 0.5,
            perturb_fractal_bounding: calculate_fractal_bounding(3, 0.5),
            perturb_normalise_length: 1.0,
        }
    }

    pub fn seed(&self) -> i32 {
        self.seed
    }

    pub fn set_seed(&mut self, seed: i32) {
        self.seed = seed;
    }

    /// Computes as the library's AVX2 build does: `SIMDf_MUL_ADD`,
    /// `SIMDf_MUL_SUB` and `SIMDf_NMUL_ADD` fused, `SIMDf_INV_SQRT` the
    /// approximate reciprocal square root (Intel's table) and vector sets
    /// taken 8 positions at a time. Off, every operation is rounded on its
    /// own, `SIMDf_INV_SQRT` is `1 / sqrt(x)` and vector sets are taken 4
    /// positions at a time, as in the SSE builds. New generators start with
    /// [`set_default_avx2`]'s setting.
    pub fn set_avx2(&mut self, avx2: bool) {
        self.avx2 = avx2;
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }

    pub fn set_noise_type(&mut self, noise_type: NoiseType) {
        self.noise_type = noise_type;
    }

    pub fn set_axis_scales(&mut self, x_scale: f32, y_scale: f32, z_scale: f32) {
        self.x_scale = x_scale;
        self.y_scale = y_scale;
        self.z_scale = z_scale;
    }

    pub fn set_fractal_octaves(&mut self, octaves: i32) {
        self.octaves = octaves;
        self.fractal_bounding = calculate_fractal_bounding(self.octaves, self.gain);
    }

    pub fn set_fractal_lacunarity(&mut self, lacunarity: f32) {
        self.lacunarity = lacunarity;
    }

    pub fn set_fractal_gain(&mut self, gain: f32) {
        self.gain = gain;
        self.fractal_bounding = calculate_fractal_bounding(self.octaves, self.gain);
    }

    pub fn set_fractal_type(&mut self, fractal_type: FractalType) {
        self.fractal_type = fractal_type;
    }

    pub fn set_cellular_distance_function(&mut self, function: CellularDistanceFunction) {
        self.cellular_distance_function = function;
    }

    pub fn set_cellular_return_type(&mut self, return_type: CellularReturnType) {
        self.cellular_return_type = return_type;
    }

    pub fn set_cellular_noise_lookup_type(&mut self, noise_type: NoiseType) {
        self.cellular_noise_lookup_type = noise_type;
    }

    pub fn set_cellular_noise_lookup_frequency(&mut self, frequency: f32) {
        self.cellular_noise_lookup_frequency = frequency;
    }

    pub fn set_cellular_distance2_indices(&mut self, index0: i32, index1: i32) {
        let low = index0.min(index1).clamp(0, CELLULAR_INDEX_MAX as i32);
        let high = index0.max(index1).clamp(0, CELLULAR_INDEX_MAX as i32);
        self.cellular_distance_index0 = low as usize;
        self.cellular_distance_index1 = high as usize;
    }

    pub fn set_cellular_jitter(&mut self, jitter: f32) {
        self.cellular_jitter = jitter;
    }

    pub fn set_perturb_type(&mut self, perturb_type: PerturbType) {
        self.perturb_type = perturb_type;
    }

    /// Maximum distance a position can be perturbed (stored pre-divided like the original).
    pub fn set_perturb_amp(&mut self, amp: f32) {
        self.perturb_amp = amp * (1.0 / CELL_HALF_RANGE);
    }

    /// Writes the stored (already divided) perturb amplitude directly, as
    /// code that pokes the field instead of calling `SetPerturbAmp` does.
    pub fn set_perturb_amp_raw(&mut self, stored: f32) {
        self.perturb_amp = stored;
    }

    pub fn set_perturb_frequency(&mut self, frequency: f32) {
        self.perturb_frequency = frequency;
    }

    pub fn set_perturb_fractal_octaves(&mut self, octaves: i32) {
        self.perturb_octaves = octaves;
        self.perturb_fractal_bounding = calculate_fractal_bounding(self.perturb_octaves, self.perturb_gain);
    }

    pub fn set_perturb_fractal_lacunarity(&mut self, lacunarity: f32) {
        self.perturb_lacunarity = lacunarity;
    }

    pub fn set_perturb_fractal_gain(&mut self, gain: f32) {
        self.perturb_gain = gain;
        self.perturb_fractal_bounding = calculate_fractal_bounding(self.perturb_octaves, self.perturb_gain);
    }

    pub fn set_perturb_normalise_length(&mut self, length: f32) {
        self.perturb_normalise_length = length;
    }

    fn perturb_values(&self) -> PerturbValues {
        let normalise_length = self.perturb_normalise_length * self.frequency;
        // GradientFractal deliberately uses the main fractal bounding (matches the original).
        let fractal_amp = self.perturb_amp * self.fractal_bounding;
        let amp = match self.perturb_type {
            PerturbType::GradientFractal | PerturbType::GradientFractalNormalise => fractal_amp,
            _ => self.perturb_amp,
        };
        PerturbValues {
            amp,
            frequency: self.perturb_frequency,
            lacunarity: self.perturb_lacunarity,
            gain: self.perturb_gain,
            normalise_length,
        }
    }

    fn fractal_values(&self) -> FractalValues {
        FractalValues {
            fractal_type: self.fractal_type,
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
            bounding: self.fractal_bounding,
        }
    }

    #[inline(always)]
    fn apply_perturb_with<const AVX2: bool, const X86_NANS: bool>(&self, values: &PerturbValues, x: &mut f32, y: &mut f32, z: &mut f32) {
        let seed_base = self.seed.wrapping_sub(1);
        match self.perturb_type {
            PerturbType::None => {}
            PerturbType::Gradient => {
                gradient_perturb_single::<AVX2, X86_NANS>(seed_base, values.amp, values.frequency, x, y, z);
            }
            PerturbType::GradientFractal => {
                self.gradient_perturb_fractal::<AVX2, X86_NANS>(values, x, y, z);
            }
            PerturbType::Normalise => {
                normalise_position::<AVX2, X86_NANS>(values.normalise_length, x, y, z);
            }
            PerturbType::GradientNormalise => {
                gradient_perturb_single::<AVX2, X86_NANS>(seed_base, values.amp, values.frequency, x, y, z);
                normalise_position::<AVX2, X86_NANS>(values.normalise_length, x, y, z);
            }
            PerturbType::GradientFractalNormalise => {
                self.gradient_perturb_fractal::<AVX2, X86_NANS>(values, x, y, z);
                normalise_position::<AVX2, X86_NANS>(values.normalise_length, x, y, z);
            }
        }
    }

    #[inline(always)]
    fn gradient_perturb_fractal<const AVX2: bool, const X86_NANS: bool>(&self, values: &PerturbValues, x: &mut f32, y: &mut f32, z: &mut f32) {
        let mut seed = self.seed.wrapping_sub(1);
        let mut frequency = values.frequency;
        let mut amp = values.amp;
        gradient_perturb_single::<AVX2, X86_NANS>(seed, amp, frequency, x, y, z);
        let mut octave = 1;
        while octave < self.perturb_octaves {
            frequency *= values.lacunarity;
            seed = seed.wrapping_sub(1);
            amp *= values.gain;
            gradient_perturb_single::<AVX2, X86_NANS>(seed, amp, frequency, x, y, z);
            octave += 1;
        }
    }

    /// Evaluates the configured noise at an already-scaled position (after perturbation).
    #[inline(always)]
    fn evaluate_with<const AVX2: bool>(&self, fractal: &FractalValues, lookup: &NoiseLookupSettings, x: f32, y: f32, z: f32) -> f32 {
        let seed = self.seed;
        match self.noise_type {
            NoiseType::Value => value_single::<AVX2>(seed, x, y, z),
            NoiseType::ValueFractal => fractal_single::<AVX2>(fractal, seed, x, y, z, value_single::<AVX2>),
            NoiseType::Perlin => perlin_single::<AVX2>(seed, x, y, z),
            NoiseType::PerlinFractal => fractal_single::<AVX2>(fractal, seed, x, y, z, perlin_single::<AVX2>),
            NoiseType::Simplex => simplex_single::<AVX2>(seed, x, y, z),
            NoiseType::SimplexFractal => fractal_single::<AVX2>(fractal, seed, x, y, z, simplex_single::<AVX2>),
            NoiseType::Cubic => cubic_single::<AVX2>(seed, x, y, z),
            NoiseType::CubicFractal => fractal_single::<AVX2>(fractal, seed, x, y, z, cubic_single::<AVX2>),
            NoiseType::Cellular => self.cellular_single::<AVX2>(lookup, seed, x, y, z),
            NoiseType::WhiteNoise => white_noise_single(seed, x, y, z),
        }
    }

    #[inline(always)]
    fn cellular_single<const AVX2: bool>(&self, lookup: &NoiseLookupSettings, seed: i32, x: f32, y: f32, z: f32) -> f32 {
        let jitter = self.cellular_jitter;
        let function = self.cellular_distance_function;
        let index0 = self.cellular_distance_index0;
        let index1 = self.cellular_distance_index1;
        match self.cellular_return_type {
            CellularReturnType::CellValue => cellular_value_single::<AVX2>(function, seed, x, y, z, jitter),
            CellularReturnType::Distance => cellular_distance_single::<AVX2>(function, seed, x, y, z, jitter),
            CellularReturnType::Distance2
            | CellularReturnType::Distance2Add
            | CellularReturnType::Distance2Sub
            | CellularReturnType::Distance2Mul
            | CellularReturnType::Distance2Div => {
                let distances = cellular_distances_single::<AVX2>(function, seed, x, y, z, jitter, index1);
                combine_distance2(self.cellular_return_type, distances[index0], distances[index1])
            }
            CellularReturnType::Distance2Cave => {
                let first_distances = cellular_distances_single::<AVX2>(function, seed, x, y, z, jitter, index1);
                let first = first_distances[index0] / first_distances[index1];
                let shifted_seed = seed.wrapping_add(1);
                let second_distances = cellular_distances_single::<AVX2>(function, shifted_seed, x + 0.5, y + 0.5, z + 0.5, jitter, index1);
                let second = second_distances[index0] / second_distances[index1];
                simd_min(first, second)
            }
            CellularReturnType::NoiseLookup => cellular_lookup_single::<AVX2>(function, seed, x, y, z, jitter, lookup),
        }
    }

    fn lookup_settings(&self) -> NoiseLookupSettings {
        NoiseLookupSettings {
            noise_type: self.cellular_noise_lookup_type,
            frequency: self.cellular_noise_lookup_frequency,
            fractal_type: self.fractal_type,
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
            fractal_bounding: self.fractal_bounding,
        }
    }

    /// Fills `noise_set` exactly like `FastNoiseSIMD::FillNoiseSet`.
    ///
    /// Layout is x-major: `index = (ix * y_size + iy) * z_size + iz`.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_noise_set(
        &self,
        noise_set: &mut [f32],
        x_start: i32,
        y_start: i32,
        z_start: i32,
        x_size: usize,
        y_size: usize,
        z_size: usize,
        scale_modifier: f32,
    ) {
        assert!(noise_set.len() >= x_size * y_size * z_size);
        let row_len = z_size;
        let rows = &mut noise_set[..x_size * y_size * z_size];
        let context = FillContext::new(self, scale_modifier);

        rows.par_chunks_mut(row_len).enumerate().for_each(|(row_index, row)| {
            let ix = (row_index / y_size) as i32;
            let iy = (row_index % y_size) as i32;
            let x = x_start + ix;
            let y = y_start + iy;
            context.fill_row(self, x, y, z_start, row);
        });
    }

    /// Convenience wrapper returning a freshly allocated set.
    #[allow(clippy::too_many_arguments)]
    pub fn get_noise_set(
        &self,
        x_start: i32,
        y_start: i32,
        z_start: i32,
        x_size: usize,
        y_size: usize,
        z_size: usize,
        scale_modifier: f32,
    ) -> Vec<f32> {
        let mut values = vec![0.0f32; x_size * y_size * z_size];
        self.fill_noise_set(&mut values, x_start, y_start, z_start, x_size, y_size, z_size, scale_modifier);
        values
    }

    /// Samples one lattice position the same way `fill_noise_set` would.
    pub fn sample(&self, x: i32, y: i32, z: i32, scale_modifier: f32) -> f32 {
        let context = FillContext::new(self, scale_modifier);
        context.sample(self, x, y, z)
    }

    /// Samples a continuous (already frequency-scaled) position. Used by vector sets.
    pub fn sample_position(&self, x: f32, y: f32, z: f32) -> f32 {
        let context = FillContext::new(self, 1.0);
        let scaling = PositionScaling {
            frequency: [1.0; 3],
            offset: [0.0; 3],
            scaled: false,
        };
        let mut value = [0.0f32];
        context.fill_positions(self, &scaling, &[[x, y, z]], &mut value);
        value[0]
    }

    /// `FillNoiseSet(noiseSet, vectorSet, xOffset, yOffset, zOffset)`: evaluates the
    /// noise at arbitrary positions. Positions are scaled by frequency and axis
    /// scales (no scale modifier), offsets likewise, then perturbed. The
    /// library works through the set a vector at a time (8 positions in the
    /// AVX2 build, 4 in the SSE builds) and leaves the perturbation out of
    /// the last, partial vector, so the positions after the last whole
    /// vector are not perturbed.
    pub fn fill_vector_set(&self, positions: &[[f32; 3]], offset: [f32; 3]) -> Vec<f32> {
        self.fill_vector_set_padded(positions, positions.len(), offset)
    }

    /// [`Self::fill_vector_set`] for a set of `set_length` positions of
    /// which only the first `positions.len()` are wanted (a set built from a
    /// longer buffer): the last, partial vector is that of the whole set, so
    /// a long enough set perturbs every wanted position.
    pub fn fill_vector_set_padded(&self, positions: &[[f32; 3]], set_length: usize, offset: [f32; 3]) -> Vec<f32> {
        let x_frequency = self.frequency * self.x_scale;
        let y_frequency = self.frequency * self.y_scale;
        let z_frequency = self.frequency * self.z_scale;
        let x_offset = offset[0] * x_frequency;
        let y_offset = offset[1] * y_frequency;
        let z_offset = offset[2] * z_frequency;
        let context = FillContext::new(self, 1.0);
        let scaling = PositionScaling {
            frequency: [x_frequency, y_frequency, z_frequency],
            offset: [x_offset, y_offset, z_offset],
            scaled: true,
        };
        let mut values = vec![0.0f32; positions.len()];

        let vector_size = if self.avx2 { 8 } else { 4 };
        let whole_vectors = set_length - set_length % vector_size;
        let perturbed = whole_vectors.min(positions.len());
        let (vector_positions, remainder_positions) = positions.split_at(perturbed);
        let (vector_values, remainder_values) = values.split_at_mut(perturbed);
        context.fill_positions(self, &scaling, vector_positions, vector_values);

        let mut unperturbed = self.clone();
        unperturbed.perturb_type = PerturbType::None;
        context.fill_positions(&unperturbed, &scaling, remainder_positions, remainder_values);
        values
    }

    /// 2D maps are sampled as the x = 0 slice of the 3D set: rows on y, columns on z.
    /// This fills a `rows x columns` row-major map that way.
    pub fn fill_slice(&self, rows: usize, columns: usize, row_offset: i32, column_offset: i32, scale_modifier: f32) -> Vec<f32> {
        let mut values = vec![0.0f32; rows * columns];
        let context = FillContext::new(self, scale_modifier);
        values.par_chunks_mut(columns).enumerate().for_each(|(row, out_row)| {
            let y = row as i32 + row_offset;
            context.fill_row(self, 0, y, column_offset, out_row);
        });
        values
    }
}

/// Per-fill resolved state shared by all samples.
struct FillContext {
    x_frequency: f32,
    y_frequency: f32,
    z_frequency: f32,
    perturb: PerturbValues,
    fractal: FractalValues,
    lookup: NoiseLookupSettings,
}

impl FillContext {
    fn new(noise: &FastNoise, scale_modifier: f32) -> Self {
        let scale = scale_modifier * noise.frequency;
        Self {
            x_frequency: scale * noise.x_scale,
            y_frequency: scale * noise.y_scale,
            z_frequency: scale * noise.z_scale,
            perturb: noise.perturb_values(),
            fractal: noise.fractal_values(),
            lookup: noise.lookup_settings(),
        }
    }

    fn sample(&self, noise: &FastNoise, x: i32, y: i32, z: i32) -> f32 {
        let mut value = [0.0f32];
        self.fill_row(noise, x, y, z, &mut value);
        value[0]
    }

    /// Samples (`x`, `y`, `z_start + i`) into `row[i]`.
    fn fill_row(&self, noise: &FastNoise, x: i32, y: i32, z_start: i32, row: &mut [f32]) {
        if !noise.avx2 {
            self.fill_row_with::<false>(noise, x, y, z_start, row);
            return;
        }
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("fma") {
            // SAFETY: the CPU has the FMA instructions the function is compiled for.
            unsafe { self.fill_row_fma(noise, x, y, z_start, row) };
            return;
        }
        self.fill_row_with::<true>(noise, x, y, z_start, row);
    }

    /// The AVX2-mode row loop compiled with FMA instructions, so fused
    /// multiply-adds are instructions rather than library calls.
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "fma")]
    unsafe fn fill_row_fma(&self, noise: &FastNoise, x: i32, y: i32, z_start: i32, row: &mut [f32]) {
        self.fill_row_with::<true>(noise, x, y, z_start, row);
    }

    #[inline(always)]
    fn fill_row_with<const AVX2: bool>(&self, noise: &FastNoise, x: i32, y: i32, z_start: i32, row: &mut [f32]) {
        for (iz, out) in row.iter_mut().enumerate() {
            let z = z_start + iz as i32;
            *out = self.sample_with::<AVX2>(noise, x, y, z);
        }
    }

    #[inline(always)]
    fn sample_with<const AVX2: bool>(&self, noise: &FastNoise, x: i32, y: i32, z: i32) -> f32 {
        if noise.noise_type == NoiseType::WhiteNoise {
            // White noise ignores frequency and hashes the integer lattice directly.
            let xi = x.wrapping_mul(X_PRIME);
            let yi = y.wrapping_mul(Y_PRIME);
            let zi = z.wrapping_mul(Z_PRIME);
            return val_coord(noise.seed, xi, yi, zi);
        }
        let mut xf = x as f32 * self.x_frequency;
        let mut yf = y as f32 * self.y_frequency;
        let mut zf = z as f32 * self.z_frequency;
        noise.apply_perturb_with::<AVX2, false>(&self.perturb, &mut xf, &mut yf, &mut zf);
        noise.evaluate_with::<AVX2>(&self.fractal, &self.lookup, xf, yf, zf)
    }

    /// Samples continuous positions into `values`.
    fn fill_positions(&self, noise: &FastNoise, scaling: &PositionScaling, positions: &[[f32; 3]], values: &mut [f32]) {
        if !noise.avx2 {
            self.fill_positions_with::<false>(noise, scaling, positions, values);
            return;
        }
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("fma") {
            // SAFETY: the CPU has the FMA instructions the function is compiled for.
            unsafe { self.fill_positions_fma(noise, scaling, positions, values) };
            return;
        }
        self.fill_positions_with::<true>(noise, scaling, positions, values);
    }

    /// [`Self::fill_positions_with`] in AVX2 mode compiled with FMA
    /// instructions.
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "fma")]
    unsafe fn fill_positions_fma(&self, noise: &FastNoise, scaling: &PositionScaling, positions: &[[f32; 3]], values: &mut [f32]) {
        self.fill_positions_with::<true>(noise, scaling, positions, values);
    }

    #[inline(always)]
    fn fill_positions_with<const AVX2: bool>(&self, noise: &FastNoise, scaling: &PositionScaling, positions: &[[f32; 3]], values: &mut [f32]) {
        if EMULATE_X86_NANS && noise.noise_type == NoiseType::WhiteNoise {
            self.fill_positions_nans::<AVX2, true>(noise, scaling, positions, values);
        } else {
            self.fill_positions_nans::<AVX2, false>(noise, scaling, positions, values);
        }
    }

    /// The position loop, with x86's NaN rules where a NaN can arise when
    /// `X86_NANS` (see [`EMULATE_X86_NANS`]).
    #[inline(always)]
    fn fill_positions_nans<const AVX2: bool, const X86_NANS: bool>(&self, noise: &FastNoise, scaling: &PositionScaling, positions: &[[f32; 3]], values: &mut [f32]) {
        for (position, out) in positions.iter().zip(values.iter_mut()) {
            let [mut xf, mut yf, mut zf] = *position;
            if scaling.scaled {
                xf = nan_mul_add::<AVX2, X86_NANS>(xf, scaling.frequency[0], scaling.offset[0]);
                yf = nan_mul_add::<AVX2, X86_NANS>(yf, scaling.frequency[1], scaling.offset[1]);
                zf = nan_mul_add::<AVX2, X86_NANS>(zf, scaling.frequency[2], scaling.offset[2]);
            }
            noise.apply_perturb_with::<AVX2, X86_NANS>(&self.perturb, &mut xf, &mut yf, &mut zf);
            *out = noise.evaluate_with::<AVX2>(&self.fractal, &self.lookup, xf, yf, zf);
        }
    }
}

/// How a vector set's positions are scaled before sampling: `position ·
/// frequency + offset` per axis (`FillNoiseSet(…, vectorSet, …)`), or not
/// at all.
struct PositionScaling {
    frequency: [f32; 3],
    offset: [f32; 3],
    scaled: bool,
}

// ---------------------------------------------------------------------------
// Primitive helpers
// ---------------------------------------------------------------------------

/// `SIMDi_CONVERT_TO_INT` (`CVTPS2DQ`) with the default rounding mode:
/// to nearest, ties to even.
#[inline(always)]
fn round_to_int(value: f32) -> i32 {
    convert_to_int(value.round_ties_even())
}

/// `SIMDi_CONVERT_TO_INT` of a whole value: `CVTPS2DQ` gives `i32::MIN`
/// for NaN and for values outside the `i32` range.
#[inline(always)]
fn convert_to_int(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) { value as i32 } else { i32::MIN }
}

/// `SIMDf_MIN(a, b)` (`MINPS`): `b` when either is NaN.
#[inline(always)]
fn simd_min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `SIMDf_MAX(a, b)` (`MAXPS`): `b` when either is NaN.
#[inline(always)]
fn simd_max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `a * b + c` (`SIMDf_MUL_ADD`): rounded once in the AVX2 build.
#[inline(always)]
fn mul_add<const AVX2: bool>(a: f32, b: f32, c: f32) -> f32 {
    if AVX2 { a.mul_add(b, c) } else { a * b + c }
}

/// `a * b - c` (`SIMDf_MUL_SUB`): rounded once in the AVX2 build.
#[inline(always)]
fn mul_sub<const AVX2: bool>(a: f32, b: f32, c: f32) -> f32 {
    if AVX2 { a.mul_add(b, -c) } else { a * b - c }
}

/// `c - a * b` (`SIMDf_NMUL_ADD`): rounded once in the AVX2 build.
#[inline(always)]
fn nmul_add<const AVX2: bool>(a: f32, b: f32, c: f32) -> f32 {
    if AVX2 { (-a).mul_add(b, c) } else { c - a * b }
}

/// Whether white noise at arbitrary positions follows x86's NaN rules
/// in software. White noise hashes a position's bits, and a NaN that an
/// operation creates is `0xFFC00000` on x86 but `0x7FC00000` on ARM, so
/// positions that become NaN while they are scaled or perturbed (an
/// infinity times zero, an infinity minus itself) would hash differently.
/// x86 hardware already behaves that way.
const EMULATE_X86_NANS: bool = !cfg!(target_arch = "x86_64");

/// `a * b`, with x86's NaN rules when `X86_NANS`.
#[inline(always)]
fn nan_mul<const X86_NANS: bool>(a: f32, b: f32) -> f32 {
    if X86_NANS { crate::x86::mulss(a, b) } else { a * b }
}

/// `a - b`, with x86's NaN rules when `X86_NANS`.
#[inline(always)]
fn nan_sub<const X86_NANS: bool>(a: f32, b: f32) -> f32 {
    if X86_NANS { crate::x86::subss(a, b) } else { a - b }
}

/// [`mul_add`], with x86's NaN rules when `X86_NANS`.
#[inline(always)]
fn nan_mul_add<const AVX2: bool, const X86_NANS: bool>(a: f32, b: f32, c: f32) -> f32 {
    if !X86_NANS {
        return mul_add::<AVX2>(a, b, c);
    }
    if AVX2 {
        crate::x86::fmadd(a, b, c)
    } else {
        let product = crate::x86::mulss(a, b);
        crate::x86::addss(product, c)
    }
}

/// `1 / sqrt(x)` (`SIMDf_INV_SQRT`): the AVX2 build's `RSQRTPS`
/// approximation.
#[inline(always)]
fn inv_sqrt<const AVX2: bool>(x: f32) -> f32 {
    if AVX2 { crate::x86::rsqrt(x) } else { 1.0 / x.sqrt() }
}

#[inline(always)]
fn interp_quintic<const AVX2: bool>(t: f32) -> f32 {
    let inner = mul_add::<AVX2>(mul_sub::<AVX2>(t, 6.0, 15.0), t, 10.0);
    inner * t * t * t
}

#[inline(always)]
fn lerp<const AVX2: bool>(a: f32, b: f32, t: f32) -> f32 {
    mul_add::<AVX2>(b - a, t, a)
}

#[inline(always)]
fn cubic_lerp<const AVX2: bool>(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let p = (d - c) - (a - b);
    let linear = mul_add::<AVX2>(t, c - a, b);
    let quadratic = mul_add::<AVX2>(t, t * ((a - b) - p), linear);
    mul_add::<AVX2>(t, t * (t * p), quadratic)
}

#[inline(always)]
fn hash(seed: i32, x: i32, y: i32, z: i32) -> i32 {
    let mut value = seed ^ x;
    value ^= y;
    value ^= z;
    value = value.wrapping_mul(value).wrapping_mul(HASH_MULTIPLIER).wrapping_mul(value);
    (value >> 13) ^ value
}

#[inline(always)]
fn hash_hb(seed: i32, x: i32, y: i32, z: i32) -> i32 {
    let mut value = seed ^ x;
    value ^= y;
    value ^= z;
    value.wrapping_mul(value).wrapping_mul(HASH_MULTIPLIER).wrapping_mul(value)
}

#[inline(always)]
fn val_coord(seed: i32, x: i32, y: i32, z: i32) -> f32 {
    hash_hb(seed, x, y, z) as f32 * HASH_TO_FLOAT
}

#[inline(always)]
fn grad_coord(seed: i32, xi: i32, yi: i32, zi: i32, x: f32, y: f32, z: f32) -> f32 {
    let value = hash(seed, xi, yi, zi);
    let selector = value & 13;
    let u = if selector < 8 { x } else { y };
    let v = if selector < 2 {
        y
    } else if selector == 12 {
        x
    } else {
        z
    };
    let u = if value & 1 != 0 { -u } else { u };
    let v = if value & 2 != 0 { -v } else { v };
    u + v
}

// ---------------------------------------------------------------------------
// Noise kernels
// ---------------------------------------------------------------------------

#[inline(always)]
fn value_single<const AVX2: bool>(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xs = x.floor();
    let ys = y.floor();
    let zs = z.floor();

    let x0 = convert_to_int(xs).wrapping_mul(X_PRIME);
    let y0 = convert_to_int(ys).wrapping_mul(Y_PRIME);
    let z0 = convert_to_int(zs).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let tx = interp_quintic::<AVX2>(x - xs);
    let ty = interp_quintic::<AVX2>(y - ys);
    let tz = interp_quintic::<AVX2>(z - zs);

    let a = lerp::<AVX2>(val_coord(seed, x0, y0, z0), val_coord(seed, x1, y0, z0), tx);
    let b = lerp::<AVX2>(val_coord(seed, x0, y1, z0), val_coord(seed, x1, y1, z0), tx);
    let c = lerp::<AVX2>(val_coord(seed, x0, y0, z1), val_coord(seed, x1, y0, z1), tx);
    let d = lerp::<AVX2>(val_coord(seed, x0, y1, z1), val_coord(seed, x1, y1, z1), tx);

    let near = lerp::<AVX2>(a, b, ty);
    let far = lerp::<AVX2>(c, d, ty);
    lerp::<AVX2>(near, far, tz)
}

#[inline(always)]
pub(crate) fn perlin_single<const AVX2: bool>(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xs = x.floor();
    let ys = y.floor();
    let zs = z.floor();

    let x0 = convert_to_int(xs).wrapping_mul(X_PRIME);
    let y0 = convert_to_int(ys).wrapping_mul(Y_PRIME);
    let z0 = convert_to_int(zs).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let xf0 = x - xs;
    let yf0 = y - ys;
    let zf0 = z - zs;
    let xf1 = xf0 - 1.0;
    let yf1 = yf0 - 1.0;
    let zf1 = zf0 - 1.0;

    let tx = interp_quintic::<AVX2>(xf0);
    let ty = interp_quintic::<AVX2>(yf0);
    let tz = interp_quintic::<AVX2>(zf0);

    let a = lerp::<AVX2>(grad_coord(seed, x0, y0, z0, xf0, yf0, zf0), grad_coord(seed, x1, y0, z0, xf1, yf0, zf0), tx);
    let b = lerp::<AVX2>(grad_coord(seed, x0, y1, z0, xf0, yf1, zf0), grad_coord(seed, x1, y1, z0, xf1, yf1, zf0), tx);
    let c = lerp::<AVX2>(grad_coord(seed, x0, y0, z1, xf0, yf0, zf1), grad_coord(seed, x1, y0, z1, xf1, yf0, zf1), tx);
    let d = lerp::<AVX2>(grad_coord(seed, x0, y1, z1, xf0, yf1, zf1), grad_coord(seed, x1, y1, z1, xf1, yf1, zf1), tx);

    let near = lerp::<AVX2>(a, b, ty);
    let far = lerp::<AVX2>(c, d, ty);
    lerp::<AVX2>(near, far, tz)
}

#[inline(always)]
fn simplex_single<const AVX2: bool>(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let f = F3 * ((x + y) + z);
    let x0f = (x + f).floor();
    let y0f = (y + f).floor();
    let z0f = (z + f).floor();

    let i = convert_to_int(x0f).wrapping_mul(X_PRIME);
    let j = convert_to_int(y0f).wrapping_mul(Y_PRIME);
    let k = convert_to_int(z0f).wrapping_mul(Z_PRIME);

    let g = G3 * ((x0f + y0f) + z0f);
    let x0 = x - (x0f - g);
    let y0 = y - (y0f - g);
    let z0 = z - (z0f - g);

    let x0_ge_y0 = x0 >= y0;
    let y0_ge_z0 = y0 >= z0;
    let x0_ge_z0 = x0 >= z0;

    let i1 = x0_ge_y0 && x0_ge_z0;
    let j1 = !x0_ge_y0 && y0_ge_z0;
    let k1 = !x0_ge_z0 && !y0_ge_z0;

    let i2 = x0_ge_y0 || x0_ge_z0;
    let j2 = !x0_ge_y0 || y0_ge_z0;
    let k2 = !(x0_ge_z0 && y0_ge_z0);

    let x1 = (x0 - if i1 { 1.0 } else { 0.0 }) + G3;
    let y1 = (y0 - if j1 { 1.0 } else { 0.0 }) + G3;
    let z1 = (z0 - if k1 { 1.0 } else { 0.0 }) + G3;
    let x2 = (x0 - if i2 { 1.0 } else { 0.0 }) + F3;
    let y2 = (y0 - if j2 { 1.0 } else { 0.0 }) + F3;
    let z2 = (z0 - if k2 { 1.0 } else { 0.0 }) + F3;
    let x3 = x0 + G33;
    let y3 = y0 + G33;
    let z3 = z0 + G33;

    let t0 = nmul_add::<AVX2>(z0, z0, nmul_add::<AVX2>(y0, y0, nmul_add::<AVX2>(x0, x0, 0.6)));
    let t1 = nmul_add::<AVX2>(z1, z1, nmul_add::<AVX2>(y1, y1, nmul_add::<AVX2>(x1, x1, 0.6)));
    let t2 = nmul_add::<AVX2>(z2, z2, nmul_add::<AVX2>(y2, y2, nmul_add::<AVX2>(x2, x2, 0.6)));
    let t3 = nmul_add::<AVX2>(z3, z3, nmul_add::<AVX2>(y3, y3, nmul_add::<AVX2>(x3, x3, 0.6)));

    let mut sum = 0.0f32;
    if t3 >= 0.0 {
        let t = t3 * t3;
        let gradient = grad_coord(seed, i.wrapping_add(X_PRIME), j.wrapping_add(Y_PRIME), k.wrapping_add(Z_PRIME), x3, y3, z3);
        sum = t * t * gradient;
    }
    if t2 >= 0.0 {
        let t = t2 * t2;
        let ii = if i2 { i.wrapping_add(X_PRIME) } else { i };
        let jj = if j2 { j.wrapping_add(Y_PRIME) } else { j };
        let kk = if k2 { k.wrapping_add(Z_PRIME) } else { k };
        sum += t * t * grad_coord(seed, ii, jj, kk, x2, y2, z2);
    }
    if t1 >= 0.0 {
        let t = t1 * t1;
        let ii = if i1 { i.wrapping_add(X_PRIME) } else { i };
        let jj = if j1 { j.wrapping_add(Y_PRIME) } else { j };
        let kk = if k1 { k.wrapping_add(Z_PRIME) } else { k };
        sum += t * t * grad_coord(seed, ii, jj, kk, x1, y1, z1);
    }
    if t0 >= 0.0 {
        let t = t0 * t0;
        sum += t * t * grad_coord(seed, i, j, k, x0, y0, z0);
    }
    32.0 * sum
}

#[inline(always)]
fn cubic_single<const AVX2: bool>(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xf1 = x.floor();
    let yf1 = y.floor();
    let zf1 = z.floor();

    let x1 = convert_to_int(xf1).wrapping_mul(X_PRIME);
    let y1 = convert_to_int(yf1).wrapping_mul(Y_PRIME);
    let z1 = convert_to_int(zf1).wrapping_mul(Z_PRIME);

    let xs_list = [x1.wrapping_sub(X_PRIME), x1, x1.wrapping_add(X_PRIME), x1.wrapping_add(X_PRIME.wrapping_mul(2))];
    let ys_list = [y1.wrapping_sub(Y_PRIME), y1, y1.wrapping_add(Y_PRIME), y1.wrapping_add(Y_PRIME.wrapping_mul(2))];
    let zs_list = [z1.wrapping_sub(Z_PRIME), z1, z1.wrapping_add(Z_PRIME), z1.wrapping_add(Z_PRIME.wrapping_mul(2))];

    let tx = x - xf1;
    let ty = y - yf1;
    let tz = z - zf1;

    let mut planes = [0.0f32; 4];
    for (plane_index, &zc) in zs_list.iter().enumerate() {
        let mut lines = [0.0f32; 4];
        for (line_index, &yc) in ys_list.iter().enumerate() {
            let a = val_coord(seed, xs_list[0], yc, zc);
            let b = val_coord(seed, xs_list[1], yc, zc);
            let c = val_coord(seed, xs_list[2], yc, zc);
            let d = val_coord(seed, xs_list[3], yc, zc);
            lines[line_index] = cubic_lerp::<AVX2>(a, b, c, d, tx);
        }
        planes[plane_index] = cubic_lerp::<AVX2>(lines[0], lines[1], lines[2], lines[3], ty);
    }
    let value = cubic_lerp::<AVX2>(planes[0], planes[1], planes[2], planes[3], tz);
    value * CUBIC_BOUNDING
}

#[inline(always)]
fn white_noise_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xb = x.to_bits() as i32;
    let yb = y.to_bits() as i32;
    let zb = z.to_bits() as i32;
    let xi = (xb ^ (xb >> 16)).wrapping_mul(X_PRIME);
    let yi = (yb ^ (yb >> 16)).wrapping_mul(Y_PRIME);
    let zi = (zb ^ (zb >> 16)).wrapping_mul(Z_PRIME);
    val_coord(seed, xi, yi, zi)
}

#[inline(always)]
fn fractal_single<const AVX2: bool>(
    fractal: &FractalValues,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    kernel: impl Fn(i32, f32, f32, f32) -> f32,
) -> f32 {
    match fractal.fractal_type {
        FractalType::Fbm => fbm_single::<AVX2>(fractal, seed, x, y, z, kernel),
        FractalType::Billow => billow_single::<AVX2>(fractal, seed, x, y, z, kernel),
        FractalType::RigidMulti => rigid_multi_single::<AVX2>(fractal, seed, x, y, z, kernel),
    }
}

#[inline(always)]
fn fbm_single<const AVX2: bool>(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: impl Fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = kernel(seed_f, xf, yf, zf);
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        result = mul_add::<AVX2>(kernel(seed_f, xf, yf, zf), amp, result);
        octave += 1;
    }
    result * fractal.bounding
}

#[inline(always)]
fn billow_single<const AVX2: bool>(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: impl Fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = mul_sub::<AVX2>(kernel(seed_f, xf, yf, zf).abs(), 2.0, 1.0);
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        let octave_value = mul_sub::<AVX2>(kernel(seed_f, xf, yf, zf).abs(), 2.0, 1.0);
        result = mul_add::<AVX2>(octave_value, amp, result);
        octave += 1;
    }
    result * fractal.bounding
}

#[inline(always)]
fn rigid_multi_single<const AVX2: bool>(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: impl Fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = 1.0 - kernel(seed_f, xf, yf, zf).abs();
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        let ridge = 1.0 - kernel(seed_f, xf, yf, zf).abs();
        result = nmul_add::<AVX2>(ridge, amp, result);
        octave += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Cellular
// ---------------------------------------------------------------------------

/// Distance used by the CellValue and NoiseLookup returns (Natural = E + M).
#[inline(always)]
fn cellular_distance<const AVX2: bool>(function: CellularDistanceFunction, x: f32, y: f32, z: f32) -> f32 {
    match function {
        CellularDistanceFunction::Euclidean => length_squared::<AVX2>(x, y, z),
        CellularDistanceFunction::Manhattan => x.abs() + y.abs() + z.abs(),
        CellularDistanceFunction::Natural => {
            let euclidean = length_squared::<AVX2>(x, y, z);
            let manhattan = x.abs() + y.abs() + z.abs();
            euclidean + manhattan
        }
    }
}

/// `x² + (y² + z²)` the way the library sums it.
#[inline(always)]
fn length_squared<const AVX2: bool>(x: f32, y: f32, z: f32) -> f32 {
    mul_add::<AVX2>(x, x, mul_add::<AVX2>(y, y, z * z))
}

/// Distance used by the Distance/Distance2/Cave returns. FastNoiseSIMD
/// redefines Natural as E * M before instantiating these.
#[inline(always)]
fn cellular_distance_product<const AVX2: bool>(function: CellularDistanceFunction, x: f32, y: f32, z: f32) -> f32 {
    match function {
        CellularDistanceFunction::Natural => {
            let euclidean = length_squared::<AVX2>(x, y, z);
            let manhattan = x.abs() + y.abs() + z.abs();
            euclidean * manhattan
        }
        _ => cellular_distance::<AVX2>(function, x, y, z),
    }
}

/// The feature point's direction inside one cell and the factor that
/// scales it to the jitter length.
#[inline(always)]
fn cell_direction<const AVX2: bool>(value: i32, jitter: f32) -> (f32, f32, f32, f32) {
    let xd = (value & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let yd = ((value >> 10) & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let zd = ((value >> 20) & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let length_sq = length_squared::<AVX2>(xd, yd, zd);
    let inv_mag = jitter * inv_sqrt::<AVX2>(length_sq);
    (xd, yd, zd, inv_mag)
}

/// Visits the 27 candidate cells around (x, y, z) in the original iteration
/// order, with each feature point's offset in its cell and its offset from
/// (x, y, z).
#[inline(always)]
fn for_each_cell<const AVX2: bool>(seed: i32, x: f32, y: f32, z: f32, jitter: f32, mut visit: impl FnMut(f32, f32, f32, i32, (f32, f32, f32), (f32, f32, f32))) {
    let xc_start = round_to_int(x).wrapping_sub(1);
    let yc_start = round_to_int(y).wrapping_sub(1);
    let zc_start = round_to_int(z).wrapping_sub(1);

    let mut xcf = xc_start as f32 - x;
    let ycf_base = yc_start as f32 - y;
    let zcf_base = zc_start as f32 - z;

    let mut xc = xc_start.wrapping_mul(X_PRIME);
    let yc_base = yc_start.wrapping_mul(Y_PRIME);
    let zc_base = zc_start.wrapping_mul(Z_PRIME);

    let mut cell_x = xc_start as f32;
    for _ in 0..3 {
        let mut ycf = ycf_base;
        let mut yc = yc_base;
        let mut cell_y = yc_start as f32;
        for _ in 0..3 {
            let mut zcf = zcf_base;
            let mut zc = zc_base;
            let mut cell_z = zc_start as f32;
            for _ in 0..3 {
                let value = hash_hb(seed, xc, yc, zc);
                let (xd, yd, zd, inv_mag) = cell_direction::<AVX2>(value, jitter);
                let offset = (xd * inv_mag, yd * inv_mag, zd * inv_mag);
                let delta = (mul_add::<AVX2>(xd, inv_mag, xcf), mul_add::<AVX2>(yd, inv_mag, ycf), mul_add::<AVX2>(zd, inv_mag, zcf));
                visit(cell_x, cell_y, cell_z, value, offset, delta);
                zcf += 1.0;
                zc = zc.wrapping_add(Z_PRIME);
                cell_z += 1.0;
            }
            ycf += 1.0;
            yc = yc.wrapping_add(Y_PRIME);
            cell_y += 1.0;
        }
        xcf += 1.0;
        xc = xc.wrapping_add(X_PRIME);
        cell_x += 1.0;
    }
}

#[inline(always)]
fn cellular_value_single<const AVX2: bool>(function: CellularDistanceFunction, seed: i32, x: f32, y: f32, z: f32, jitter: f32) -> f32 {
    let mut distance = FAR_DISTANCE;
    let mut cell_value = 0.0f32;
    for_each_cell::<AVX2>(seed, x, y, z, jitter, |_, _, _, value, _, delta| {
        let new_distance = cellular_distance::<AVX2>(function, delta.0, delta.1, delta.2);
        let new_value = value as f32 * HASH_TO_FLOAT;
        if new_distance < distance {
            cell_value = new_value;
        }
        distance = simd_min(new_distance, distance);
    });
    cell_value
}

#[inline(always)]
fn cellular_distance_single<const AVX2: bool>(function: CellularDistanceFunction, seed: i32, x: f32, y: f32, z: f32, jitter: f32) -> f32 {
    let mut distance = FAR_DISTANCE;
    for_each_cell::<AVX2>(seed, x, y, z, jitter, |_, _, _, _, _, delta| {
        let new_distance = cellular_distance_product::<AVX2>(function, delta.0, delta.1, delta.2);
        distance = simd_min(distance, new_distance);
    });
    distance
}

#[inline(always)]
fn cellular_distances_single<const AVX2: bool>(
    function: CellularDistanceFunction,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    jitter: f32,
    index1: usize,
) -> [f32; CELLULAR_INDEX_MAX + 1] {
    let mut distances = [FAR_DISTANCE; CELLULAR_INDEX_MAX + 1];
    for_each_cell::<AVX2>(seed, x, y, z, jitter, |_, _, _, _, _, delta| {
        let new_distance = cellular_distance_product::<AVX2>(function, delta.0, delta.1, delta.2);
        let mut index = index1;
        while index > 0 {
            let keep_lower = simd_min(distances[index], new_distance);
            distances[index] = simd_max(keep_lower, distances[index - 1]);
            index -= 1;
        }
        distances[0] = simd_min(distances[0], new_distance);
    });
    distances
}

#[inline(always)]
fn combine_distance2(return_type: CellularReturnType, distance: f32, distance2: f32) -> f32 {
    match return_type {
        CellularReturnType::Distance2 => distance2,
        CellularReturnType::Distance2Add => distance + distance2,
        CellularReturnType::Distance2Sub => distance2 - distance,
        CellularReturnType::Distance2Mul => distance * distance2,
        CellularReturnType::Distance2Div => distance / distance2,
        _ => distance2,
    }
}

#[inline(always)]
fn cellular_lookup_single<const AVX2: bool>(
    function: CellularDistanceFunction,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    jitter: f32,
    lookup: &NoiseLookupSettings,
) -> f32 {
    let mut distance = FAR_DISTANCE;
    let mut cell = (0.0f32, 0.0f32, 0.0f32);
    for_each_cell::<AVX2>(seed, x, y, z, jitter, |cell_x, cell_y, cell_z, _, offset, _| {
        // Measured from the cell's own position, without fusing: far from
        // the origin `cell - x` keeps its rounding where the other cellular
        // functions' running offset does not.
        let delta_x = offset.0 + (cell_x - x);
        let delta_y = offset.1 + (cell_y - y);
        let delta_z = offset.2 + (cell_z - z);
        let new_distance = cellular_distance::<AVX2>(function, delta_x, delta_y, delta_z);
        if new_distance < distance {
            cell = (offset.0 + cell_x, offset.1 + cell_y, offset.2 + cell_z);
        }
        distance = simd_min(new_distance, distance);
    });

    let xf = cell.0 * lookup.frequency;
    let yf = cell.1 * lookup.frequency;
    let zf = cell.2 * lookup.frequency;
    let fractal = FractalValues {
        fractal_type: lookup.fractal_type,
        octaves: lookup.octaves,
        lacunarity: lookup.lacunarity,
        gain: lookup.gain,
        bounding: lookup.fractal_bounding,
    };
    match lookup.noise_type {
        NoiseType::Value => value_single::<AVX2>(seed, xf, yf, zf),
        NoiseType::ValueFractal => fractal_single::<AVX2>(&fractal, seed, xf, yf, zf, value_single::<AVX2>),
        NoiseType::Perlin => perlin_single::<AVX2>(seed, xf, yf, zf),
        NoiseType::PerlinFractal => fractal_single::<AVX2>(&fractal, seed, xf, yf, zf, perlin_single::<AVX2>),
        NoiseType::Simplex => simplex_single::<AVX2>(seed, xf, yf, zf),
        NoiseType::SimplexFractal => fractal_single::<AVX2>(&fractal, seed, xf, yf, zf, simplex_single::<AVX2>),
        NoiseType::Cubic => cubic_single::<AVX2>(seed, xf, yf, zf),
        NoiseType::CubicFractal => fractal_single::<AVX2>(&fractal, seed, xf, yf, zf, cubic_single::<AVX2>),
        NoiseType::WhiteNoise | NoiseType::Cellular => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Perturbation
// ---------------------------------------------------------------------------

#[inline(always)]
fn gradient_perturb_single<const AVX2: bool, const X86_NANS: bool>(seed: i32, amp: f32, frequency: f32, x: &mut f32, y: &mut f32, z: &mut f32) {
    let xf = nan_mul::<X86_NANS>(*x, frequency);
    let yf = nan_mul::<X86_NANS>(*y, frequency);
    let zf = nan_mul::<X86_NANS>(*z, frequency);

    let xs = xf.floor();
    let ys = yf.floor();
    let zs = zf.floor();

    let x0 = convert_to_int(xs).wrapping_mul(X_PRIME);
    let y0 = convert_to_int(ys).wrapping_mul(Y_PRIME);
    let z0 = convert_to_int(zs).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let tx = interp_quintic::<AVX2>(nan_sub::<X86_NANS>(xf, xs));
    let ty = interp_quintic::<AVX2>(nan_sub::<X86_NANS>(yf, ys));
    let tz = interp_quintic::<AVX2>(nan_sub::<X86_NANS>(zf, zs));

    let corners = [
        hash_hb(seed, x0, y0, z0),
        hash_hb(seed, x1, y0, z0),
        hash_hb(seed, x0, y1, z0),
        hash_hb(seed, x1, y1, z0),
        hash_hb(seed, x0, y0, z1),
        hash_hb(seed, x1, y0, z1),
        hash_hb(seed, x0, y1, z1),
        hash_hb(seed, x1, y1, z1),
    ];

    let mut offsets = [0.0f32; 3];
    for (axis, offset) in offsets.iter_mut().enumerate() {
        let shift = axis as u32 * 10;
        let mut values = [0.0f32; 8];
        for (value, corner) in values.iter_mut().zip(corners.iter()) {
            let field = (corner >> shift) & BIT10_MASK;
            *value = field as f32 - CELL_HALF_RANGE;
        }
        let a = lerp::<AVX2>(values[0], values[1], tx);
        let b = lerp::<AVX2>(values[2], values[3], tx);
        let c = lerp::<AVX2>(values[4], values[5], tx);
        let d = lerp::<AVX2>(values[6], values[7], tx);
        let near = lerp::<AVX2>(a, b, ty);
        let far = lerp::<AVX2>(c, d, ty);
        *offset = lerp::<AVX2>(near, far, tz);
    }

    *x = nan_mul_add::<AVX2, X86_NANS>(offsets[0], amp, *x);
    *y = nan_mul_add::<AVX2, X86_NANS>(offsets[1], amp, *y);
    *z = nan_mul_add::<AVX2, X86_NANS>(offsets[2], amp, *z);
}

#[inline(always)]
fn normalise_position<const AVX2: bool, const X86_NANS: bool>(length: f32, x: &mut f32, y: &mut f32, z: &mut f32) {
    let length_sq = length_squared::<AVX2>(*x, *y, *z);
    let inv_mag = nan_mul::<X86_NANS>(length, inv_sqrt::<AVX2>(length_sq));
    *x = nan_mul::<X86_NANS>(*x, inv_mag);
    *y = nan_mul::<X86_NANS>(*y, inv_mag);
    *z = nan_mul::<X86_NANS>(*z, inv_mag);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractal_bounding_matches_reference() {
        let bounding = calculate_fractal_bounding(3, 0.5);
        assert!((bounding - 1.0 / 1.75).abs() < 1e-7);
    }

    #[test]
    fn perlin_is_zero_on_lattice() {
        let value = perlin_single::<false>(1337, 3.0, 4.0, 5.0);
        assert_eq!(value, 0.0);
    }

    #[test]
    fn white_noise_hashes_x86_nans() {
        // At frequency zero an infinite position scales to a NaN, which
        // x86 makes negative on every platform.
        let mut noise = FastNoise::new(7);
        noise.set_noise_type(NoiseType::WhiteNoise);
        noise.set_frequency(0.0);
        let positions = [[f32::INFINITY, 1.0, 2.0]];
        let values = noise.fill_vector_set(&positions, [0.0, 0.0, 0.0]);
        let expected = white_noise_single(7, f32::from_bits(0xFFC0_0000), 0.0, 0.0);
        assert_eq!(values[0].to_bits(), expected.to_bits());
    }

    #[test]
    fn fill_matches_single_sample() {
        let mut noise = FastNoise::new(42);
        noise.set_noise_type(NoiseType::PerlinFractal);
        noise.set_perturb_type(PerturbType::GradientFractal);
        let set = noise.get_noise_set(0, 3, 0, 1, 1, 16, 1.0);
        let single = noise.sample(0, 3, 7, 1.0);
        assert_eq!(set[7], single);
    }
}
