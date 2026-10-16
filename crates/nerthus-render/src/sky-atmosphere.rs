//! Four-order Bruneton lookup tables, with the parameters and texture
//! packing used by Gaea's viewport. See tools/sky-atmosphere for their
//! reproducible precomputation and the independent reference comparison.

use std::io::Read;

use bevy::prelude::Vec3;
use flate2::read::ZlibDecoder;
use half::f16;

use super::{add, mie_phase, multiply, rayleigh_phase, scale, Atmosphere, Rgb, SHOWN_GROUND_ALBEDO, SUN_ANGULAR_RADIUS};

const TRANSMITTANCE_WIDTH: usize = 256;
const TRANSMITTANCE_HEIGHT: usize = 64;
const SCATTERING_HEIGHTS: usize = 32;
const SCATTERING_VIEWS: usize = 128;
const SCATTERING_SUNS: usize = 32;
const SCATTERING_ANGLES: usize = 8;
const SCATTERING_WIDTH: usize = SCATTERING_SUNS * SCATTERING_ANGLES;
const IRRADIANCE_WIDTH: usize = 64;
const IRRADIANCE_HEIGHT: usize = 16;

// Model.LengthUnitInMeters is 1000. Computing the lookup geometry in
// kilometres also preserves the original shaders' floating-point precision.
const BOTTOM_RADIUS: f32 = 6360.0;
const TOP_RADIUS: f32 = 6420.0;
// Demo.UseHalfPrecision selects a maximum solar zenith angle of 102 degrees.
const MU_S_MIN: f32 = -0.207_911_74;
const TABLE_BYTES: &[u8] = include_bytes!("../assets/sky-atmosphere.bin.zlib");

type Rgba = [f32; 4];

pub(super) struct PrecomputedAtmosphere {
    transmittance: Vec<Rgba>,
    scattering: Vec<Rgba>,
    irradiance: Vec<Rgba>,
}

impl PrecomputedAtmosphere {
    pub(super) fn load() -> Self {
        let transmittance_count = TRANSMITTANCE_WIDTH * TRANSMITTANCE_HEIGHT;
        let scattering_count = SCATTERING_WIDTH * SCATTERING_VIEWS * SCATTERING_HEIGHTS;
        let irradiance_count = IRRADIANCE_WIDTH * IRRADIANCE_HEIGHT;
        let expected_size = 8 + (transmittance_count + scattering_count + irradiance_count) * 8;
        let mut decoded = Vec::with_capacity(expected_size);
        let mut decoder = ZlibDecoder::new(TABLE_BYTES);
        decoder.read_to_end(&mut decoded).expect("the bundled atmosphere tables decompress");
        assert_eq!(decoded.len(), expected_size, "the bundled atmosphere tables have their declared dimensions");
        assert_eq!(&decoded[..8], b"NTHSKY01", "the bundled atmosphere tables have the supported format");

        let mut texels = decoded[8..].chunks_exact(8);
        let transmittance = read_texels(&mut texels, transmittance_count);
        let scattering = read_texels(&mut texels, scattering_count);
        let irradiance = read_texels(&mut texels, irradiance_count);
        Self { transmittance, scattering, irradiance }
    }

    pub(super) fn sky_radiance(&self, atmosphere: &Atmosphere, radius_metres: f32, view: Vec3, sun: Vec3, show_ground: bool) -> (Rgb, Rgb) {
        let radius = (radius_metres / 1000.0).clamp(BOTTOM_RADIUS, TOP_RADIUS);
        let mu = view.y.clamp(-1.0, 1.0);
        let mu_sun = sun.y.clamp(-1.0, 1.0);
        let nu = view.dot(sun).clamp(-1.0, 1.0);
        let ground = distance_to_ground(radius, mu);
        let combined = self.combined_scattering(radius, mu, mu_sun, nu, ground.is_some());
        let mut radiance = [0.0; 3];
        let mie_ratio = if combined[0] > 0.0 { combined[3] / combined[0] } else { 0.0 };
        for channel in 0..3 {
            // GetExtrapolatedSingleMieScattering reconstructs the three Mie
            // channels from the red channel in the combined texture's alpha.
            // The demo's Mie coefficient is constant across wavelengths.
            let mie = combined[channel] * mie_ratio * atmosphere.rayleigh[0] / atmosphere.rayleigh[channel];
            radiance[channel] = combined[channel] * rayleigh_phase(nu) + mie * mie_phase(nu);
        }

        if let Some(distance) = ground {
            if show_ground {
                let point = Vec3::new(0.0, radius, 0.0) + view * distance;
                let mu_ground_sun = point.normalize().dot(sun).clamp(-1.0, 1.0);
                let direct = self.transmittance_to_sun(BOTTOM_RADIUS, mu_ground_sun);
                let indirect = self.ground_irradiance(mu_ground_sun);
                let direct_irradiance = multiply(atmosphere.solar, direct);
                let sunlight = scale(direct_irradiance, mu_ground_sun.max(0.0));
                let irradiance = add(sunlight, indirect);
                let reflected_irradiance = multiply(irradiance, SHOWN_GROUND_ALBEDO);
                let reflected = scale(reflected_irradiance, 1.0 / std::f32::consts::PI);
                let mu_ground_view = (radius * mu + distance) / BOTTOM_RADIUS;
                let from_ground = self.transmittance(BOTTOM_RADIUS, -mu_ground_view);
                let from_camera = self.transmittance(radius, -mu);
                let mut segment = [0.0; 3];
                for channel in 0..3 {
                    segment[channel] = (from_ground[channel] / from_camera[channel]).min(1.0);
                }
                radiance = add(radiance, multiply(segment, reflected));
            }
            (radiance, [0.0; 3])
        } else {
            (radiance, self.transmittance(radius, mu))
        }
    }

    fn transmittance(&self, radius: f32, mu: f32) -> Rgb {
        let horizon = atmosphere_horizon();
        let rho = (radius * radius - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
        let distance = distance_to_top(radius, mu);
        let nearest = TOP_RADIUS - radius;
        let farthest = rho + horizon;
        let x_mu = (distance - nearest) / (farthest - nearest);
        let x_r = rho / horizon;
        let column = x_mu * (TRANSMITTANCE_WIDTH - 1) as f32;
        let row = x_r * (TRANSMITTANCE_HEIGHT - 1) as f32;
        let sample = sample_texture(&self.transmittance, [TRANSMITTANCE_WIDTH, TRANSMITTANCE_HEIGHT, 1], [column, row, 0.0]);
        [sample[0], sample[1], sample[2]]
    }

    fn transmittance_to_sun(&self, radius: f32, mu_sun: f32) -> Rgb {
        let sin_horizon = BOTTOM_RADIUS / radius;
        let cos_horizon = -(1.0 - sin_horizon * sin_horizon).max(0.0).sqrt();
        let edge = sin_horizon * SUN_ANGULAR_RADIUS;
        let t = ((mu_sun - cos_horizon + edge) / (2.0 * edge)).clamp(0.0, 1.0);
        let visible = t * t * (3.0 - 2.0 * t);
        scale(self.transmittance(radius, mu_sun), visible)
    }

    fn combined_scattering(&self, radius: f32, mu: f32, mu_sun: f32, nu: f32, ground: bool) -> Rgba {
        let coordinates = scattering_coordinates(radius, mu, mu_sun, nu, ground);
        let angle = coordinates[3];
        let first_angle = angle.floor();
        let fraction = angle - first_angle;
        let dimensions = [SCATTERING_WIDTH, SCATTERING_VIEWS, SCATTERING_HEIGHTS];
        let first_coordinates = [first_angle * SCATTERING_SUNS as f32 + coordinates[0], coordinates[1], coordinates[2]];
        let next_coordinates = [(first_angle + 1.0) * SCATTERING_SUNS as f32 + coordinates[0], coordinates[1], coordinates[2]];
        let first = sample_texture(&self.scattering, dimensions, first_coordinates);
        let next = sample_texture(&self.scattering, dimensions, next_coordinates);
        std::array::from_fn(|channel| first[channel] * (1.0 - fraction) + next[channel] * fraction)
    }

    fn ground_irradiance(&self, mu_sun: f32) -> Rgb {
        let column = (mu_sun + 1.0) * 0.5 * (IRRADIANCE_WIDTH - 1) as f32;
        let sample = sample_texture(&self.irradiance, [IRRADIANCE_WIDTH, IRRADIANCE_HEIGHT, 1], [column, 0.0, 0.0]);
        [sample[0], sample[1], sample[2]]
    }
}

fn read_texels(texels: &mut std::slice::ChunksExact<'_, u8>, count: usize) -> Vec<Rgba> {
    let mut values = Vec::with_capacity(count);
    for texel in texels.take(count) {
        let value = std::array::from_fn(|channel| {
            let offset = channel * 2;
            let bits = u16::from_le_bytes([texel[offset], texel[offset + 1]]);
            f16::from_bits(bits).to_f32()
        });
        values.push(value);
    }
    values
}

fn atmosphere_horizon() -> f32 {
    (TOP_RADIUS * TOP_RADIUS - BOTTOM_RADIUS * BOTTOM_RADIUS).sqrt()
}

fn distance_to_top(radius: f32, mu: f32) -> f32 {
    let discriminant = radius * radius * (mu * mu - 1.0) + TOP_RADIUS * TOP_RADIUS;
    (-radius * mu + discriminant.max(0.0).sqrt()).max(0.0)
}

fn distance_to_ground(radius: f32, mu: f32) -> Option<f32> {
    let discriminant = radius * radius * (mu * mu - 1.0) + BOTTOM_RADIUS * BOTTOM_RADIUS;
    (mu < 0.0 && discriminant >= 0.0).then(|| (-radius * mu - discriminant.sqrt()).max(0.0))
}

/// Continuous texel indices for the four atmospheric variables. The two
/// halves of the view axis are sampled separately at the geometric horizon,
/// preventing interpolation through the planet.
fn scattering_coordinates(radius: f32, mu: f32, mu_sun: f32, nu: f32, ground: bool) -> [f32; 4] {
    let horizon = atmosphere_horizon();
    let rho = (radius * radius - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
    let height = rho / horizon * (SCATTERING_HEIGHTS - 1) as f32;
    let half_views = SCATTERING_VIEWS / 2;
    let radius_mu = radius * mu;
    let discriminant = radius_mu * radius_mu - radius * radius + BOTTOM_RADIUS * BOTTOM_RADIUS;
    let view = if ground {
        let distance = -radius_mu - discriminant.max(0.0).sqrt();
        let nearest = radius - BOTTOM_RADIUS;
        let range = rho - nearest;
        let fraction = if range == 0.0 { 0.0 } else { (distance - nearest) / range };
        (1.0 - fraction.clamp(0.0, 1.0)) * (half_views - 1) as f32
    } else {
        let distance = -radius_mu + (discriminant + horizon * horizon).max(0.0).sqrt();
        let nearest = TOP_RADIUS - radius;
        let farthest = rho + horizon;
        let fraction = (distance - nearest) / (farthest - nearest);
        half_views as f32 + fraction.clamp(0.0, 1.0) * (half_views - 1) as f32
    };

    let nearest = TOP_RADIUS - BOTTOM_RADIUS;
    let range = horizon - nearest;
    let distance = distance_to_top(BOTTOM_RADIUS, mu_sun);
    let a = (distance - nearest) / range;
    // This is the older Bruneton parameterization used by Gaea's shader,
    // rather than the revised mapping in the current upstream demo.
    let a_max = -2.0 * MU_S_MIN * BOTTOM_RADIUS / range;
    let sun_fraction = (1.0 - a / a_max).max(0.0) / (1.0 + a);
    let sun = sun_fraction.clamp(0.0, 1.0) * (SCATTERING_SUNS - 1) as f32;
    let angle = (nu + 1.0) * 0.5 * (SCATTERING_ANGLES - 1) as f32;
    [sun, view, height, angle]
}

fn sample_texture(table: &[Rgba], dimensions: [usize; 3], coordinates: [f32; 3]) -> Rgba {
    let mut lower = [0; 3];
    let mut upper = [0; 3];
    let mut fraction = [0.0; 3];
    for axis in 0..3 {
        let coordinate = coordinates[axis].clamp(0.0, (dimensions[axis] - 1) as f32);
        lower[axis] = coordinate.floor() as usize;
        upper[axis] = (lower[axis] + 1).min(dimensions[axis] - 1);
        fraction[axis] = coordinate - lower[axis] as f32;
    }
    let mut result = [0.0; 4];
    for z in 0..2 {
        let layer = if z == 0 { lower[2] } else { upper[2] };
        let weight_z = if z == 0 { 1.0 - fraction[2] } else { fraction[2] };
        for y in 0..2 {
            let row = if y == 0 { lower[1] } else { upper[1] };
            let weight_y = if y == 0 { 1.0 - fraction[1] } else { fraction[1] };
            for x in 0..2 {
                let column = if x == 0 { lower[0] } else { upper[0] };
                let weight_x = if x == 0 { 1.0 - fraction[0] } else { fraction[0] };
                let weight = weight_x * weight_y * weight_z;
                let index = column + dimensions[0] * (row + dimensions[1] * layer);
                for channel in 0..4 {
                    result[channel] += table[index][channel] * weight;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_angle_interpolation_preserves_sun_and_horizon_boundaries() {
        let up = scattering_coordinates(BOTTOM_RADIUS, 1.0, 1.0, 1.0, false);
        let down = scattering_coordinates(BOTTOM_RADIUS, -1.0, 1.0, -1.0, true);
        assert_eq!(up, [31.0, 64.0, 0.0, 7.0]);
        assert_eq!(down, [31.0, 63.0, 0.0, 0.0]);
        let below_sunset = scattering_coordinates(BOTTOM_RADIUS, 0.0, -1.0, 0.0, false);
        assert_eq!(below_sunset[0], 0.0);
        assert_eq!(below_sunset[1], 127.0);
    }

    #[test]
    fn table_contains_all_four_scattering_orders_and_finite_half_precision_values() {
        let tables = PrecomputedAtmosphere::load();
        assert_eq!(tables.scattering.len(), 1_048_576);
        for table in [&tables.transmittance, &tables.scattering, &tables.irradiance] {
            assert!(table.iter().flatten().all(|value| value.is_finite() && *value >= 0.0));
        }
        assert!(tables.transmittance.iter().all(|texel| texel[..3].iter().all(|value| *value <= 1.0)));

        // A directional sample from the independently executed reference
        // shaders. Single scattering alone is [0.31934, 0.63086, 0.93945];
        // the colour-dependent additional orders must be present in RGB,
        // while alpha continues to hold the first-order Mie component.
        let index = 244 + SCATTERING_WIDTH * (100 + SCATTERING_VIEWS * 5);
        let expected = [0.338_134_77, 0.690_917_97, 1.133_789_1, 0.029_373_169];
        for channel in 0..4 {
            assert!((tables.scattering[index][channel] - expected[channel]).abs() < 0.002);
        }
    }

    #[test]
    fn sky_radiance_matches_independent_reference_shaders() {
        // Reference GPU samples cover noon, the forward-scattering sunset
        // glow, the anti-solar sky, a camera at 40 km and twilight. The
        // generator also emits these samples without using this sampler.
        let cases = [
            (1000.0, 60.0, 90.0, 0.0, [0.003_842_635_5, 0.009_873_482, 0.024_350_744], [0.967_773_44, 0.924_316_4, 0.886_230_47]),
            (1000.0, 5.0, 2.0, 0.0, [0.254_997_22, 0.238_926_89, 0.193_836_84], [0.578_125, 0.283_447_27, 0.105_102_54]),
            (1000.0, 5.0, 20.0, 180.0, [0.008_009_693, 0.017_405_36, 0.039_456_416], [0.910_156_25, 0.797_851_56, 0.705_566_4]),
            (40_000.0, 33.0, 0.0, 90.0, [0.000_493_595, 0.001_512_203_8, 0.004_371_321], [0.993_652_34, 0.984_863_3, 0.969_238_3]),
            (2000.0, -5.0, 10.0, 0.0, [0.000_892_981_6, 0.000_826_156, 0.001_753_533_7], [0.854_980_47, 0.679_199_2, 0.557_617_2]),
        ];
        let tables = super::super::tables();
        for (altitude, sun_degrees, view_degrees, azimuth_degrees, expected_radiance, expected_transmittance) in cases {
            let sun_elevation = f32::to_radians(sun_degrees);
            let view_elevation = f32::to_radians(view_degrees);
            let azimuth = f32::to_radians(azimuth_degrees);
            let sun = Vec3::new(sun_elevation.cos(), sun_elevation.sin(), 0.0);
            let view = Vec3::new(view_elevation.cos() * azimuth.cos(), view_elevation.sin(), view_elevation.cos() * azimuth.sin());
            let radius = super::super::BOTTOM_RADIUS + altitude;
            let (radiance, transmittance) = tables.sky_radiance(radius, view, sun, false);
            for channel in 0..3 {
                assert!((radiance[channel] - expected_radiance[channel]).abs() < 0.000_3, "altitude={altitude}, sun={sun_degrees}, channel={channel}: {radiance:?} versus {expected_radiance:?}");
                assert!((transmittance[channel] - expected_transmittance[channel]).abs() < 0.000_6, "altitude={altitude}, sun={sun_degrees}, channel={channel}: {transmittance:?} versus {expected_transmittance:?}");
            }
        }
    }

    #[test]
    fn horizon_and_sun_angular_edges_remain_finite() {
        let tables = super::super::tables();
        for altitude in [10.0, 1000.0, 50_000.0] {
            let radius = super::super::BOTTOM_RADIUS + altitude;
            let horizon = -(super::super::BOTTOM_RADIUS / radius).acos();
            for offset in [-0.000_1, 0.0, 0.000_1] {
                let elevation = horizon + offset;
                let view = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
                let sun = Vec3::new(1.0, 0.0, 0.0);
                let (radiance, transmittance) = tables.sky_radiance(radius, view, sun, true);
                assert!(radiance.iter().all(|value| value.is_finite() && *value >= 0.0));
                assert!(transmittance.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
            }
        }
        let atmosphere = &tables.precomputed;
        let hidden = atmosphere.transmittance_to_sun(BOTTOM_RADIUS, -SUN_ANGULAR_RADIUS);
        let emerging = atmosphere.transmittance_to_sun(BOTTOM_RADIUS, 0.0);
        let risen = atmosphere.transmittance_to_sun(BOTTOM_RADIUS, SUN_ANGULAR_RADIUS);
        assert_eq!(hidden, [0.0; 3]);
        for channel in 0..3 {
            assert!(emerging[channel] > 0.0 && risen[channel] > emerging[channel]);
        }
    }
}
