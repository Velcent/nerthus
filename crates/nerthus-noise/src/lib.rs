//! Noise generation for Nerthus.
//!
//! `fast_noise` is a faithful port of FastNoiseSIMD, the library behind
//! almost every procedural primitive. `x86` emulates the x86 approximation
//! instructions (`RCPPS`, `RSQRTPS`) its AVX2 build and the native code
//! rely on.

pub mod fast_noise;
pub mod x86;
mod x86_tables;

pub use fast_noise::{
    calculate_fractal_bounding, set_default_avx2, CellularDistanceFunction, CellularReturnType, FastNoise, FractalType, NoiseType, PerturbType,
};
