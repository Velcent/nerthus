//! Show Skybox: the sky behind the terrain and the fog over it. The sky is
//! Eric Bruneton's precomputed atmospheric scattering
//! (`BrunetonsImprovedAtmosphere.Demo`: an Earth atmosphere with ozone,
//! the sun's own spectrum, white balance and an exposure of 8, seen from
//! wherever the camera is, a Unity unit being a kilometre) and fogs the
//! terrain with Unity's exponential fog (`FogDensity` × 0.05 a Unity unit)
//! in the colour of the sky at the horizon. Off, the camera clears to the
//! viewport background.
//!
//! Nerthus samples the same four-order Bruneton atmosphere, precomputed
//! into half-precision tables with Gaea's dimensions and atmosphere
//! parameters. A worker samples these by elevation and angle to the sun
//! from the camera's height and spreads the result over a dome, with the
//! sun's disc in front of it.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::terrain::ViewportSettings;
use crate::ViewportCamera;

#[path = "sky-atmosphere.rs"]
mod precomputed;

use precomputed::PrecomputedAtmosphere;

/// The planet's radius (`kBottomRadius`), in metres.
const BOTTOM_RADIUS: f32 = 6_360_000.0;
const MIE_PHASE_G: f32 = 0.8;
/// `kSunAngularRadius`, radians.
const SUN_ANGULAR_RADIUS: f32 = 0.004675;
/// The scene's `Demo.Exposure` (without luminance).
const EXPOSURE: f32 = 8.0;
/// The ground shown below the horizon (Bruneton's demo's dark sea).
const SHOWN_GROUND_ALBEDO: [f32; 3] = [0.0, 0.0, 0.04];
/// The wavelengths rendered without luminance (`kLambdaR`, `G`, `B`), nm.
const WAVELENGTHS: [f32; 3] = [680.0, 550.0, 440.0];
/// A Nerthus unit is 20 Unity units, a Unity unit a kilometre.
const METRES_PER_UNIT: f32 = 20_000.0;
/// The camera's height is kept within these, metres.
const LOWEST_CAMERA: f32 = 10.0;
const HIGHEST_CAMERA: f32 = 50_000.0;

/// The solar spectrum Bruneton's demo uses (W/m²/nm, 360 to 830 nm every
/// 10 nm).
const SOLAR_IRRADIANCE: [f32; 48] = [
    1.11776, 1.14259, 1.01249, 1.14716, 1.72765, 1.73054, 1.6887, 1.61253, 1.91198, 2.03474, 2.02042, 2.02212, 1.93377, 1.95809, 1.91686, 1.8298, 1.8685, 1.8931, 1.85149, 1.8504, 1.8341, 1.8345, 1.8147, 1.78158,
    1.7533, 1.6965, 1.68194, 1.64654, 1.6048, 1.52143, 1.55622, 1.5113, 1.474, 1.4482, 1.41018, 1.36775, 1.34188, 1.31429, 1.28303, 1.26758, 1.2367, 1.2082, 1.18737, 1.14683, 1.12362, 1.1058, 1.07124, 1.04992,
];
const SKY_ELEVATIONS: usize = 192;
const SKY_AZIMUTHS: usize = 48;
/// The dome's texture (azimuth by elevation) and its mesh.
const DOME_WIDTH: usize = 1024;
const DOME_HEIGHT: usize = 512;
const DOME_RADIUS: f32 = 500.0;
const SUN_DISTANCE: f32 = 400.0;
const DOME_SEGMENTS: (usize, usize) = (96, 48);

type Rgb = [f32; 3];

/// The Earth atmosphere (`Demo.Awake`) at the three wavelengths.
struct Atmosphere {
    rayleigh: Rgb,
    solar: Rgb,
    white_point: Rgb,
}

impl Atmosphere {
    fn earth() -> Self {
        let rayleigh = WAVELENGTHS.map(|lambda| 6.2031e-7 * (lambda / 1000.0).powi(-4));
        let solar = WAVELENGTHS.map(|lambda| SOLAR_IRRADIANCE[((lambda - 360.0) / 10.0) as usize]);
        // Demo.DoWhiteBalance: Model.ConvertSpectrumToLinearSrgb using
        // Bruneton's tabulated CIE 1931 matching functions, normalized by
        // the mean. An analytic fit introduces a visible colour shift.
        let white_point = [1.082_414_4, 0.967_555_9, 0.950_029_73];
        Self { rayleigh, solar, white_point }
    }
}

fn add(left: Rgb, right: Rgb) -> Rgb {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn scale(colour: Rgb, factor: f32) -> Rgb {
    colour.map(|channel| channel * factor)
}

fn multiply(left: Rgb, right: Rgb) -> Rgb {
    [left[0] * right[0], left[1] * right[1], left[2] * right[2]]
}

/// Rayleigh's phase function, and Cornette and Shanks' for Mie.
fn rayleigh_phase(nu: f32) -> f32 {
    3.0 / (16.0 * std::f32::consts::PI) * (1.0 + nu * nu)
}

fn mie_phase(nu: f32) -> f32 {
    let g = MIE_PHASE_G;
    let k = 3.0 / (8.0 * std::f32::consts::PI) * (1.0 - g * g) / (2.0 + g * g);
    k * (1.0 + nu * nu) / (1.0 + g * g - 2.0 * g * nu).max(1e-6).powf(1.5)
}

/// The tables that do not depend on the sun or the camera. Their expensive
/// precomputation is stored in the source (sky-atmosphere-data.rs);
/// loading and sampling them
/// happens on the sky worker, once per process.
struct Tables {
    atmosphere: Atmosphere,
    precomputed: PrecomputedAtmosphere,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(Tables::new)
}

impl Tables {
    fn new() -> Self {
        Self { atmosphere: Atmosphere::earth(), precomputed: PrecomputedAtmosphere::load() }
    }

    /// The reference's GetSkyRadiance, including its separate Rayleigh and
    /// Cornette-Shanks Mie phase functions and optional ground shading.
    fn sky_radiance(&self, radius: f32, view: Vec3, sun: Vec3, show_ground: bool) -> (Rgb, Rgb) {
        self.precomputed.sky_radiance(&self.atmosphere, radius, view, sun, show_ground)
    }

    /// The shader's last step: `pow(1 - exp(-radiance / white_point *
    /// exposure), 1 / 2.2)`.
    fn tone_map(&self, radiance: Rgb) -> Rgb {
        let mut colour = [0.0; 3];
        for channel in 0..3 {
            let exposed = radiance[channel] / self.atmosphere.white_point[channel] * EXPOSURE;
            colour[channel] = (1.0 - (-exposed).exp()).max(0.0).powf(1.0 / 2.2);
        }
        colour
    }
}

fn bilinear(table: &[Rgb], width: usize, height: usize, column: f32, row: f32) -> Rgb {
    let column = column.clamp(0.0, (width - 1) as f32);
    let row = row.clamp(0.0, (height - 1) as f32);
    let left = (column.floor() as usize).min(width.saturating_sub(2));
    let top = (row.floor() as usize).min(height.saturating_sub(2));
    let right = (left + 1).min(width - 1);
    let bottom = (top + 1).min(height - 1);
    let across = column - left as f32;
    let down = row - top as f32;
    let mut value = [0.0; 3];
    for channel in 0..3 {
        let upper = table[top * width + left][channel] * (1.0 - across) + table[top * width + right][channel] * across;
        let lower = table[bottom * width + left][channel] * (1.0 - across) + table[bottom * width + right][channel] * across;
        value[channel] = upper * (1.0 - down) + lower * down;
    }
    value
}

/// What the sky is worked out for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkyRequest {
    sun_azimuth: f32,
    sun_elevation: f32,
    /// The camera's height in metres.
    altitude: f32,
}

/// A finished sky: the dome's texture (sRGB bytes), the fog's colour and
/// the sun's disc, if it is above the horizon.
struct SkyImage {
    request: SkyRequest,
    pixels: Vec<u8>,
    fog: Rgb,
    sun: Option<Rgb>,
}

/// The sky by elevation (closer together at the horizon) and angle to
/// the sun's azimuth, from the camera's height.
struct SkyTable {
    sun_elevation: f32,
    altitude: f32,
    /// The horizon's dip below level, radians.
    dip: f32,
    colours: Vec<Rgb>,
}

impl SkyTable {
    fn new(sun_elevation: f32, altitude: f32) -> Self {
        let tables = tables();
        let radius = BOTTOM_RADIUS + altitude;
        let dip = (BOTTOM_RADIUS / radius).clamp(-1.0, 1.0).acos();
        let elevation = sun_elevation.to_radians();
        let sun = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let mut colours = Vec::with_capacity(SKY_ELEVATIONS * SKY_AZIMUTHS);
        for row in 0..SKY_ELEVATIONS {
            let view_elevation = table_elevation(row, dip);
            for column in 0..SKY_AZIMUTHS {
                let relative = column as f32 / (SKY_AZIMUTHS - 1) as f32 * std::f32::consts::PI;
                let view = Vec3::new(view_elevation.cos() * relative.cos(), view_elevation.sin(), view_elevation.cos() * relative.sin());
                let (radiance, _) = tables.sky_radiance(radius, view, sun, true);
                colours.push(tables.tone_map(radiance));
            }
        }
        Self { sun_elevation, altitude, dip, colours }
    }

    /// The colour looking at `elevation` (radians), `relative` (radians,
    /// 0 to π) round from the sun.
    fn colour(&self, elevation: f32, relative: f32) -> Rgb {
        let row = table_row(elevation, self.dip);
        let column = relative / std::f32::consts::PI * (SKY_AZIMUTHS - 1) as f32;
        bilinear(&self.colours, SKY_AZIMUTHS, SKY_ELEVATIONS, column, row)
    }
}

/// The dome's elevation samples bunch round the horizon, `dip` below level.
/// This mapping resamples the precomputed atmosphere for the dome texture.
fn table_elevation(row: usize, dip: f32) -> f32 {
    let t = row as f32 / (SKY_ELEVATIONS - 1) as f32 * 2.0 - 1.0;
    let half_pi = std::f32::consts::FRAC_PI_2;
    if t >= 0.0 { -dip + (half_pi + dip) * t * t } else { -dip - (half_pi - dip) * t * t }
}

fn table_row(elevation: f32, dip: f32) -> f32 {
    let half_pi = std::f32::consts::FRAC_PI_2;
    let t = if elevation >= -dip { ((elevation + dip) / (half_pi + dip)).max(0.0).sqrt() } else { -((-dip - elevation) / (half_pi - dip)).max(0.0).sqrt() };
    (t + 1.0) / 2.0 * (SKY_ELEVATIONS - 1) as f32
}

/// The dome's texture, the fog's colour and the sun's disc for a request,
/// reusing the table when only the sun's azimuth changed.
fn build_sky(request: SkyRequest, table: &mut Option<SkyTable>) -> SkyImage {
    let reusable = table.as_ref().is_some_and(|built| built.sun_elevation == request.sun_elevation && built.altitude == request.altitude);
    if !reusable {
        *table = Some(SkyTable::new(request.sun_elevation, request.altitude));
    }
    let sky = table.as_ref().expect("the table was just built");
    let sun_azimuth = request.sun_azimuth.to_radians();
    let mut pixels = Vec::with_capacity(DOME_WIDTH * DOME_HEIGHT * 4);
    for y in 0..DOME_HEIGHT {
        let elevation = std::f32::consts::FRAC_PI_2 - (y as f32 + 0.5) / DOME_HEIGHT as f32 * std::f32::consts::PI;
        for x in 0..DOME_WIDTH {
            let azimuth = (x as f32 + 0.5) / DOME_WIDTH as f32 * std::f32::consts::TAU;
            let offset = (azimuth - sun_azimuth).rem_euclid(std::f32::consts::TAU);
            let relative = if offset > std::f32::consts::PI { std::f32::consts::TAU - offset } else { offset };
            let colour = sky.colour(elevation, relative);
            for channel in colour {
                pixels.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            pixels.push(255);
        }
    }
    // The fog takes the sky just above the horizon, all the way round.
    let mut fog = [0.0; 3];
    let samples = 16;
    for step in 0..samples {
        let relative = (step as f32 + 0.5) / samples as f32 * std::f32::consts::PI;
        fog = add(fog, sky.colour(-sky.dip + 0.035, relative));
    }
    let fog = scale(fog, 1.0 / samples as f32);
    let sun = sun_colour(request, sky.dip);
    SkyImage { request, pixels, fog, sun }
}

/// The sun's disc as the camera sees it (`radiance + transmittance *
/// GetSolarRadiance()`), or none once it is below the horizon.
fn sun_colour(request: SkyRequest, dip: f32) -> Option<Rgb> {
    let elevation = request.sun_elevation.to_radians();
    if elevation < -dip - SUN_ANGULAR_RADIUS {
        return None;
    }
    let tables = tables();
    let radius = BOTTOM_RADIUS + request.altitude;
    let sun = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
    let (radiance, transmittance) = tables.sky_radiance(radius, sun, sun, false);
    let solid_angle = std::f32::consts::PI * SUN_ANGULAR_RADIUS * SUN_ANGULAR_RADIUS;
    let solar = scale(tables.atmosphere.solar, 1.0 / solid_angle);
    let seen = add(radiance, multiply(transmittance, solar));
    Some(tables.tone_map(seen))
}

/// The thread that works skies out, the newest request first. The web
/// build has no threads: there the requests wait until `finished` works
/// out the newest.
#[derive(Resource)]
pub(crate) struct SkyWorker {
    requests: Sender<SkyRequest>,
    results: std::sync::Mutex<Receiver<SkyImage>>,
    #[cfg(target_arch = "wasm32")]
    waiting: std::sync::Mutex<(Receiver<SkyRequest>, Sender<SkyImage>, Option<SkyTable>)>,
}

impl Default for SkyWorker {
    fn default() -> Self {
        let (request_sender, request_receiver) = channel::<SkyRequest>();
        let (result_sender, result_receiver) = channel::<SkyImage>();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let spawned = thread::Builder::new().name("nerthus-sky".to_string()).spawn(move || {
                let mut table = None;
                while let Ok(request) = request_receiver.recv() {
                    if !build_newest(request, &request_receiver, &result_sender, &mut table) {
                        return;
                    }
                }
            });
            if let Err(error) = spawned {
                error!("cannot start the sky thread: {error}");
            }
        }
        Self {
            requests: request_sender,
            results: std::sync::Mutex::new(result_receiver),
            #[cfg(target_arch = "wasm32")]
            waiting: std::sync::Mutex::new((request_receiver, result_sender, None)),
        }
    }
}

impl SkyWorker {
    /// The newest sky finished since the last call.
    fn finished(&self) -> Option<SkyImage> {
        #[cfg(target_arch = "wasm32")]
        if let Ok(mut waiting) = self.waiting.lock() {
            let (requests, results, table) = &mut *waiting;
            if let Ok(request) = requests.try_recv() {
                build_newest(request, requests, results, table);
            }
        }
        self.results.lock().ok().and_then(|results| results.try_iter().last())
    }
}

/// Works out the sky for `request`, or for the newest of the requests that
/// came after it; false once nothing takes the skies.
fn build_newest(mut request: SkyRequest, requests: &Receiver<SkyRequest>, results: &Sender<SkyImage>, table: &mut Option<SkyTable>) -> bool {
    while let Ok(newer) = requests.try_recv() {
        request = newer;
    }
    let image = build_sky(request, table);
    results.send(image).is_ok()
}

/// The sky's state between frames.
#[derive(Resource, Default)]
pub(crate) struct SkyState {
    requested: Option<SkyRequest>,
    shown: Option<SkyRequest>,
    fog: Option<Rgb>,
    sun: Option<Rgb>,
}

#[derive(Component)]
pub(crate) struct SkyDome;

#[derive(Component)]
pub(crate) struct SunDisc;

pub(crate) fn spawn_sky(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let dome_material = StandardMaterial {
        base_color: Color::srgb(0.53, 0.63, 0.74),
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    };
    let dome = (SkyDome, Mesh3d(meshes.add(dome_mesh())), MeshMaterial3d(materials.add(dome_material)), Transform::default(), Visibility::Hidden, NotShadowCaster);
    commands.spawn(dome);
    let sun_material = StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    };
    let disc = Circle::new(1.0);
    let sun = (SunDisc, Mesh3d(meshes.add(disc)), MeshMaterial3d(materials.add(sun_material)), Transform::default(), Visibility::Hidden, NotShadowCaster);
    commands.spawn(sun);
}

/// A sphere round the origin seen from inside, its texture running round
/// from north (−Z) clockwise and down from the zenith.
fn dome_mesh() -> Mesh {
    let (around, down) = DOME_SEGMENTS;
    let mut positions = Vec::with_capacity((around + 1) * (down + 1));
    let mut normals = Vec::with_capacity((around + 1) * (down + 1));
    let mut uvs = Vec::with_capacity((around + 1) * (down + 1));
    for row in 0..=down {
        let v = row as f32 / down as f32;
        let elevation = std::f32::consts::FRAC_PI_2 - v * std::f32::consts::PI;
        for column in 0..=around {
            let u = column as f32 / around as f32;
            let azimuth = u * std::f32::consts::TAU;
            let direction = sky_direction(azimuth, elevation);
            positions.push((direction * DOME_RADIUS).to_array());
            normals.push((-direction).to_array());
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity(around * down * 6);
    for row in 0..down {
        for column in 0..around {
            let corner = (row * (around + 1) + column) as u32;
            let below = corner + (around + 1) as u32;
            indices.extend_from_slice(&[corner, corner + 1, below, corner + 1, below + 1, below]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The direction at `azimuth` (clockwise from north, −Z) and `elevation`,
/// radians.
fn sky_direction(azimuth: f32, elevation: f32) -> Vec3 {
    Vec3::new(azimuth.sin() * elevation.cos(), elevation.sin(), -azimuth.cos() * elevation.cos())
}

/// The dome and the sun's disc as the sky moves them.
type DomeQuery<'a> = (&'a mut Transform, &'a mut Visibility, &'a MeshMaterial3d<StandardMaterial>);

/// Shows the sky and fog while Show Skybox is on: asks for a new sky when
/// the sun or the camera's height moved, takes in finished ones, keeps
/// the dome and the sun round the camera and the fog on it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_sky(
    mut commands: Commands,
    settings: Res<ViewportSettings>,
    worker: Res<SkyWorker>,
    mut state: ResMut<SkyState>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Single<(Entity, &GlobalTransform, Option<&DistanceFog>), With<ViewportCamera>>,
    mut dome: Single<DomeQuery, (With<SkyDome>, Without<SunDisc>)>,
    mut sun: Single<DomeQuery, (With<SunDisc>, Without<SkyDome>)>,
) {
    let (camera_entity, camera_transform, fog) = *camera;
    let eye = camera_transform.translation();
    if !settings.skybox {
        *dome.1 = Visibility::Hidden;
        *sun.1 = Visibility::Hidden;
        if fog.is_some() {
            commands.entity(camera_entity).remove::<DistanceFog>();
        }
        return;
    }

    // A new sky once the camera's height moves by a fiftieth.
    let altitude = (eye.y * METRES_PER_UNIT).clamp(LOWEST_CAMERA, HIGHEST_CAMERA);
    let steps = (altitude.ln() / 1.02f32.ln()).round();
    let altitude = 1.02f32.powf(steps);
    let request = SkyRequest { sun_azimuth: settings.sun_azimuth, sun_elevation: settings.sun_elevation, altitude };
    if state.requested != Some(request) {
        let _ = worker.requests.send(request);
        state.requested = Some(request);
    }
    let finished = worker.finished();
    if let Some(image) = finished {
        let extent = Extent3d { width: DOME_WIDTH as u32, height: DOME_HEIGHT as u32, depth_or_array_layers: 1 };
        let mut texture = Image::new(extent, TextureDimension::D2, image.pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        let sampler = ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        };
        texture.sampler = ImageSampler::Descriptor(sampler);
        if let Some(mut material) = materials.get_mut(&dome.2.0) {
            material.base_color_texture = Some(images.add(texture));
            material.base_color = Color::WHITE;
        }
        if let (Some(colour), Some(mut material)) = (image.sun, materials.get_mut(&sun.2.0)) {
            material.base_color = Color::srgb(colour[0], colour[1], colour[2]);
        }
        state.shown = Some(image.request);
        state.fog = Some(image.fog);
        state.sun = image.sun;
    }

    *dome.1 = Visibility::Visible;
    dome.0.translation = eye;
    let elevation = settings.sun_elevation.to_radians();
    let direction = sky_direction(settings.sun_azimuth.to_radians(), elevation);
    let disc_centre = eye + direction * SUN_DISTANCE;
    *sun.1 = if state.sun.is_some() { Visibility::Visible } else { Visibility::Hidden };
    let disc_radius = SUN_DISTANCE * SUN_ANGULAR_RADIUS.tan();
    *sun.0 = Transform::from_translation(disc_centre).looking_at(eye, Vec3::Y).with_scale(Vec3::splat(disc_radius));

    // `SetFog(FogDensity * 0.1)`, halved, a Unity unit being a twentieth
    // of Nerthus's: `FogDensity` a unit.
    let colour = state.fog.unwrap_or([0.53, 0.63, 0.74]);
    let wanted = (settings.fog_density > 0.0).then(|| DistanceFog {
        color: Color::srgb(colour[0], colour[1], colour[2]),
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::Exponential { density: settings.fog_density },
    });
    match wanted {
        Some(wanted) => {
            let same = fog.is_some_and(|fog| fog.color == wanted.color && matches!(fog.falloff, FogFalloff::Exponential { density } if density == settings.fog_density));
            if !same {
                commands.entity(camera_entity).insert(wanted);
            }
        }
        None => {
            if fog.is_some() {
                commands.entity(camera_entity).remove::<DistanceFog>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_elevation_coordinates_invert() {
        for row in [0, 57, 95, 96, 150, SKY_ELEVATIONS - 1] {
            let elevation = table_elevation(row, 0.05);
            assert!((table_row(elevation, 0.05) - row as f32).abs() < 1e-3);
        }
    }

    /// `NERTHUS_SKY_PPM=<folder>` writes the dome's texture for a few suns
    /// and heights there, to look at.
    #[test]
    fn domes_can_be_written_out() {
        let Ok(folder) = std::env::var("NERTHUS_SKY_PPM") else {
            return;
        };
        let mut table = None;
        for (elevation, altitude) in [(33.0, 1000.0), (5.0, 1000.0), (33.0, 40_000.0), (1.0, 2000.0)] {
            let request = SkyRequest { sun_azimuth: 45.0, sun_elevation: elevation, altitude };
            let image = build_sky(request, &mut table);
            let mut ppm = format!("P6\n{DOME_WIDTH} {DOME_HEIGHT}\n255\n").into_bytes();
            for pixel in image.pixels.chunks(4) {
                ppm.extend_from_slice(&pixel[..3]);
            }
            let path = std::path::Path::new(&folder).join(format!("sky-{elevation}-{altitude}.ppm"));
            std::fs::write(path, ppm).expect("the sky is written");
        }
    }

    #[test]
    fn the_noon_sky_is_blue_and_the_sunset_glows() {
        let noon = SkyTable::new(60.0, 1000.0);
        let zenith = noon.colour(std::f32::consts::FRAC_PI_2 - 0.01, 0.0);
        assert!(zenith[2] > zenith[1] && zenith[1] > zenith[0], "{zenith:?}");
        // The exposure leaves the sun's disc white even this low.
        let request = SkyRequest { sun_azimuth: 0.0, sun_elevation: 1.0, altitude: 1000.0 };
        let sun = sun_colour(request, noon.dip).expect("the sun is up");
        assert!(sun.iter().all(|channel| *channel > 0.99), "{sun:?}");
        // The horizon under a setting sun is warm.
        let sunset = SkyTable::new(1.0, 2000.0);
        let glow = sunset.colour(-sunset.dip + 0.01, 0.0);
        assert!(glow[0] > glow[2], "{glow:?}");
        let white = Atmosphere::earth().white_point;
        assert!((white[0] + white[1] + white[2] - 3.0).abs() < 1e-4);
    }
}
