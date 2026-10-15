//! File loading: raw float and 16-bit heightfields, and images through
//! Magick.NET.
//!
//! An image is read straight from ImageMagick's pixel cache, so its
//! quirks reach the map: an 8-bit grey PNG (a palette image to IM), a grey
//! PNG with alpha or a grey image with one extra channel gives a map
//! interleaving its grey with its index, alpha or extra channel, taken
//! from the top half of the image; colour images give their red channel
//! unless kept as colour.

use std::path::Path;

use nerthus_engine::{Map, Resolution};
use nerthus_image::colorspace_transform::ToSrgb;
use nerthus_image::{quanta, Colorspace, Image};
use nerthus_io::pixel_cache::{read_pixel_cache, CacheColorspace, ImageType, PixelCache, PixelChannel};

/// `1f / Quantum.Max`: a map is divided by a scalar as a multiplication
/// by its reciprocal.
const QUANTUM_SCALE: f32 = 1.0 / 65535.0;

/// Whether a channel is one of the image's colours (grey, red, green,
/// blue or black).
fn is_colour(channel: PixelChannel) -> bool {
    !matches!(channel, PixelChannel::Index | PixelChannel::Alpha | PixelChannel::Meta)
}

/// The grey or red, green and blue channels of the cache as an `Image` for
/// IM's colour operations.
fn colour_image(cache: &PixelCache) -> Image {
    let kept: Vec<usize> = (0..cache.channels.len()).filter(|&index| is_colour(cache.channels[index])).collect();
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * kept.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &index in &kept {
            data.push(pixel[index]);
        }
    }
    let mut image = Image::from_data(cache.width, cache.height, kept.len(), data);
    // Caches in other colorspaces are taken to sRGB before they get here.
    image.colorspace = match cache.colorspace {
        CacheColorspace::Rgb | CacheColorspace::ScRgb => Colorspace::LinearRgb,
        CacheColorspace::Gray => Colorspace::Gray,
        CacheColorspace::LinearGray => Colorspace::LinearGray,
        _ => Colorspace::SRgb,
    };
    image
}

/// The channels a colorspace transform keeps as they are: alpha and meta
/// channels, in order, with their samples. The transform resyncs the
/// cache, so alpha IM no longer counts goes.
fn extra_channels(cache: &PixelCache) -> Vec<(PixelChannel, Vec<f32>)> {
    let stride = cache.channels.len();
    let mut extras = Vec::new();
    for (position, channel) in cache.channels.iter().enumerate() {
        if !matches!(channel, PixelChannel::Alpha | PixelChannel::Meta) {
            continue;
        }
        if *channel == PixelChannel::Alpha && !cache.alpha_trait {
            continue;
        }
        let samples: Vec<f32> = cache.data.chunks_exact(stride).map(|pixel| pixel[position]).collect();
        extras.push((*channel, samples));
    }
    extras
}

/// A cache of new colours (`colours`, `colour_channels` per pixel) and the
/// extra channels after them. `SetImageColorspace` types a grey result
/// Grayscale; others keep the undefined type writing pixels left.
fn with_extras(colours: &[f32], colour_channels: Vec<PixelChannel>, extras: Vec<(PixelChannel, Vec<f32>)>, model: &PixelCache, colorspace: CacheColorspace, gamma: f64) -> PixelCache {
    let colour_count = colour_channels.len();
    let mut channels = colour_channels;
    channels.extend(extras.iter().map(|(channel, _)| *channel));
    let mut data = Vec::with_capacity(model.width * model.height * channels.len());
    for (index, pixel) in colours.chunks_exact(colour_count).enumerate() {
        data.extend_from_slice(pixel);
        for (_, samples) in &extras {
            data.push(samples[index]);
        }
    }
    let mut cache = PixelCache::new(model.width, model.height, channels, data, colorspace, gamma);
    if matches!(colorspace, CacheColorspace::Gray | CacheColorspace::LinearGray) {
        cache.image_type = ImageType::Grayscale;
    }
    cache
}

/// `SyncImage` then `SetImageStorageClass(DirectClass)`, which every
/// colorspace transform starts with on a PseudoClass image: each pixel
/// takes its colormap entry's colour (an index past the colormap, the
/// first entry's; a grey image's grey, the entry's blue) and the index
/// channel goes.
fn sync_image(cache: &PixelCache) -> PixelCache {
    let Some(index_position) = cache.channels.iter().position(|channel| *channel == PixelChannel::Index) else {
        return cache.clone();
    };
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * (stride - 1));
    for pixel in cache.data.chunks_exact(stride) {
        let index = pixel[index_position];
        let in_range = index >= 0.0 && (index as usize) < cache.colormap.len();
        let entry = if in_range { cache.colormap.get(index as usize) } else { cache.colormap.first() };
        for (position, channel) in cache.channels.iter().enumerate() {
            let value = match (channel, entry) {
                (PixelChannel::Index, _) => continue,
                (PixelChannel::Gray | PixelChannel::Blue, Some(entry)) => entry[2] as f32,
                (PixelChannel::Red, Some(entry)) => entry[0] as f32,
                (PixelChannel::Green, Some(entry)) => entry[1] as f32,
                (PixelChannel::Alpha, Some(entry)) => entry[3] as f32,
                _ => pixel[position],
            };
            data.push(value);
        }
    }
    let mut channels = cache.channels.clone();
    channels.remove(index_position);
    PixelCache::new(cache.width, cache.height, channels, data, cache.colorspace, cache.gamma)
}

/// The cache after a transform to grey: `DirectClass` (no index), grey
/// plus the extra channels.
fn from_gray_image(image: Image, extras: Vec<(PixelChannel, Vec<f32>)>, model: &PixelCache, colorspace: CacheColorspace, gamma: f64) -> PixelCache {
    with_extras(&image.data, vec![PixelChannel::Gray], extras, model, colorspace, gamma)
}

/// `TransformsRGBImage` (IM 7.1.2): each pixel's colours to sRGB (a CMYK
/// image's black goes), alpha and meta channels kept.
fn transform_to_srgb(cache: &PixelCache) -> PixelCache {
    let conversion = ToSrgb::new(cache.colorspace, &cache.transform_settings);
    let stride = cache.channels.len();
    let colour_positions: Vec<usize> = (0..stride).filter(|&index| is_colour(cache.channels[index])).collect();
    let mut colours = Vec::with_capacity(cache.width * cache.height * 3);
    let mut pixel_colours = Vec::with_capacity(colour_positions.len());
    for pixel in cache.data.chunks_exact(stride) {
        pixel_colours.clear();
        pixel_colours.extend(colour_positions.iter().map(|&position| pixel[position]));
        colours.extend(conversion.convert(&pixel_colours));
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    with_extras(&colours, channels, extra_channels(cache), cache, CacheColorspace::SRgb, cache.gamma)
}

/// The cache as `TransformImageColorspace` leaves it for a transform out
/// of sRGB: sRGB and Transparent caches as they are (IM transforms
/// straight from them), grey ones too (the grey transforms take them
/// from there), every other colorspace through `TransformsRGBImage`.
fn to_srgb(cache: &PixelCache) -> PixelCache {
    match cache.colorspace {
        CacheColorspace::SRgb | CacheColorspace::Transparent | CacheColorspace::Gray | CacheColorspace::LinearGray => cache.clone(),
        _ => transform_to_srgb(cache),
    }
}

/// `TransformImageColorspace` to LinearGRAY (through sRGB, IM 7.1.2).
fn to_linear_gray(cache: &PixelCache) -> PixelCache {
    let synced = sync_image(cache);
    let cache = to_srgb(&synced);
    let extras = extra_channels(&cache);
    let image = colour_image(&cache);
    let linear = quanta::to_linear_gray(&image);
    from_gray_image(linear, extras, &cache, CacheColorspace::LinearGray, 1.0)
}

/// `TransformImageColorspace` from LinearGRAY to GRAY.
fn linear_to_gray(cache: &PixelCache) -> PixelCache {
    let synced = sync_image(cache);
    let extras = extra_channels(&synced);
    let image = colour_image(&synced);
    let gray = quanta::linear_to_gray(&image);
    from_gray_image(gray, extras, &synced, CacheColorspace::Gray, synced.gamma)
}

/// `ScaleQuantumToMap` (Q16 HDRI): a quantum rounded to a map index.
fn scale_quantum_to_map(quantum: f32) -> usize {
    if quantum >= 65535.0 {
        return 65535;
    }
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    (quantum + 0.5) as usize
}

/// `GammaImage`'s `gamma_map`: each map index raised to 1/`gamma`, back
/// as a quantum (`ScaleMapToQuantum`).
fn gamma_map(gamma: f64) -> Vec<f32> {
    let reciprocal = 1.0 / gamma;
    let mut map = Vec::with_capacity(65536);
    for index in 0..=65535u32 {
        let value = 65535.0 * (index as f64 / 65535.0).powf(reciprocal);
        let quantum = if value <= 0.0 {
            0.0
        } else if value >= 65535.0 {
            65535.0
        } else {
            value as f32
        };
        map.push(quantum);
    }
    map
}

/// `GammaImage` (Magick.NET `GammaCorrect`) on every channel with the
/// update trait: all but the index (alpha and meta channels too). A
/// PseudoClass image's colormap goes through the gamma map first (alpha
/// too when the image has it). Writing the pixels leaves the type
/// undefined.
fn gamma_correct(cache: &mut PixelCache, gamma: f64) {
    if cache.is_pseudo_class() && !cache.colormap.is_empty() {
        let map = gamma_map(gamma);
        let corrected_count = if cache.has_channel(PixelChannel::Alpha) { 4 } else { 3 };
        for entry in &mut cache.colormap {
            for value in entry.iter_mut().take(corrected_count) {
                *value = map[scale_quantum_to_map(*value as f32)] as f64;
            }
        }
    }
    let updated_positions: Vec<usize> = (0..cache.channels.len()).filter(|&index| cache.channels[index] != PixelChannel::Index).collect();
    let stride = cache.channels.len();
    let mut samples = Vec::with_capacity(cache.width * cache.height * updated_positions.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &position in &updated_positions {
            samples.push(pixel[position]);
        }
    }
    // Every sample on its own (an image of four channels would take the
    // last for alpha).
    let sample_count = samples.len();
    let mut image = Image::from_data(sample_count, 1, 1, samples);
    quanta::gamma(&mut image, gamma);
    for (pixel, corrected) in cache.data.chunks_exact_mut(stride).zip(image.data.chunks_exact(updated_positions.len())) {
        for (&position, &value) in updated_positions.iter().zip(corrected.iter()) {
            pixel[position] = value;
        }
    }
    if cache.gamma != 0.0 {
        cache.gamma *= gamma;
    }
    cache.image_type = ImageType::Undefined;
}

/// The cache with its alpha channel gone (other channels kept), as
/// `Alpha(Off)` leaves it: resyncing the cache undefines the type. When IM
/// no longer counts the alpha channel it does nothing, and the channel
/// stays.
fn without_alpha(cache: PixelCache) -> PixelCache {
    if !cache.alpha_trait {
        return cache;
    }
    let Some(position) = cache.channels.iter().position(|channel| *channel == PixelChannel::Alpha) else {
        return cache;
    };
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * (stride - 1));
    for pixel in cache.data.chunks_exact(stride) {
        for (index, &value) in pixel.iter().enumerate() {
            if index != position {
                data.push(value);
            }
        }
    }
    let mut channels = cache.channels;
    channels.remove(position);
    PixelCache { channels, data, image_type: ImageType::Undefined, ..cache }
}

/// The cache as a map. A cache of fewer than three channels is made GRAY
/// and read as one channel — its first width × height values, whatever
/// the cache interleaves; otherwise alpha is turned off and every
/// remaining channel kept. Quanta are scaled to 0..1.
fn cache_to_map(mut cache: PixelCache) -> Map {
    let resolution = Resolution { x: cache.width, y: cache.height };
    let area = cache.width * cache.height;
    let mut map = if cache.channels.len() < 3 {
        // A LinearGRAY cache is transformed to GRAY, which types it
        // Grayscale. `ColorType = Grayscale` leaves a Grayscale image alone,
        // so `Alpha(Off)` then drops its alpha; on any other image it turns
        // alpha off without resyncing the cache, `Alpha(Off)` finds nothing
        // to do, and the cache is read as it was, alpha interleaved. Meta
        // channels stay either way.
        if cache.colorspace == CacheColorspace::LinearGray {
            cache = linear_to_gray(&cache);
            cache = without_alpha(cache);
        } else if cache.image_type == ImageType::Grayscale {
            cache = without_alpha(cache);
        }
        let mut values = cache.data;
        values.resize(area.max(values.len()), 0.0);
        values.truncate(area);
        Map::from_data(resolution, values)
    } else {
        cache = without_alpha(cache);
        Map::from_interleaved(resolution, &cache.data, cache.channels.len())
    };
    map.apply(|quantum| quantum * QUANTUM_SCALE);
    map
}

/// A square map from raw samples (the side is the square root
/// of the count) or, for colour, from interleaved RGB.
fn raw_map(values: Vec<f32>, is_rgb: bool) -> Map {
    if is_rgb {
        let side = (values.len() as f64 / 3.0).sqrt() as usize;
        let resolution = Resolution::square(side);
        let mut padded = values;
        padded.resize(side * side * 3, 0.0);
        return Map::from_interleaved(resolution, &padded, 3);
    }
    let side = (values.len() as f32 as f64).sqrt() as usize;
    let mut padded = values;
    padded.resize(side * side, 0.0);
    Map::from_data(Resolution::square(side), padded)
}

/// `MagickImage.ChannelCount`: how many channels ImageMagick reads the
/// image with (an alpha or a palette's index channel counts too), or
/// nothing when it cannot be read.
pub fn image_channel_count(path: &Path) -> Option<usize> {
    let cache = read_pixel_cache(path).ok()?;
    Some(cache.channels.len())
}

/// A map from a file — `.r32` floats, `.raw` 16-bit unsigned samples,
/// `.graw` as it was stored (whatever IsRGB says), anything else an image.
pub fn load_map_file(path: &Path, is_rgb: bool, enforce_linear_gamma: bool) -> Result<Map, String> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    if extension == "r32" {
        let values = crate::graw::read_floats(path)?;
        return Ok(raw_map(values, is_rgb));
    }
    if extension == "raw" {
        let bytes = crate::graw::raw_values(path, 2)?;
        let values = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]]) as f32 / 65535.0).collect();
        return Ok(raw_map(values, is_rgb));
    }
    if extension == "graw" {
        let (map, _) = crate::graw::read(path)?;
        return Ok(map);
    }

    let mut cache = read_pixel_cache(path).map_err(|error| error.to_string())?;
    if enforce_linear_gamma {
        let gamma = cache.gamma;
        if gamma < 0.5 {
            gamma_correct(&mut cache, 2.2);
        } else if gamma > 2.0 {
            gamma_correct(&mut cache, 0.454545454545455);
        }
        if cache.colorspace != CacheColorspace::LinearGray {
            cache = to_linear_gray(&cache);
        }
    }
    let map = cache_to_map(cache);
    if !is_rgb && map.channel_count() > 1 {
        let first = map.channel(0).to_vec();
        return Ok(Map::from_data(map.resolution(), first));
    }
    if is_rgb && map.channel_count() == 1 {
        let gray = map.channel(0).to_vec();
        return Ok(Map::from_channels(map.resolution(), vec![gray.clone(), gray.clone(), gray]));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads every image named in `$NERTHUS_MAP_FILES` (a list split by
    /// newlines) with each combination of IsRGB and EnforceLinearGamma and
    /// writes the map as the oracle's `load-map` does, into
    /// `<file>.<rgb><linear>.ours/` (`map.raw`, or `map.cN.raw` per
    /// channel, and `map.json`), for comparing with the oracle's maps.
    #[test]
    #[ignore]
    fn dump_loaded_maps() {
        let list = std::env::var("NERTHUS_MAP_FILES").expect("set NERTHUS_MAP_FILES");
        for file in list.lines().filter(|line| !line.trim().is_empty()) {
            let path = Path::new(file.trim());
            for (is_rgb, linear) in [(false, false), (true, false), (false, true), (true, true)] {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let folder = path.with_file_name(format!("{name}.{}{}.ours", u8::from(is_rgb), u8::from(linear)));
                std::fs::create_dir_all(&folder).unwrap();
                let report = match load_map_file(path, is_rgb, linear) {
                    Ok(map) => {
                        let channels = map.channel_count();
                        for channel in 0..channels {
                            let file_name = if channels == 1 { "map.raw".to_string() } else { format!("map.c{channel}.raw") };
                            let bytes: Vec<u8> = map.channel(channel).iter().flat_map(|value| value.to_le_bytes()).collect();
                            std::fs::write(folder.join(file_name), bytes).unwrap();
                        }
                        format!("{{\"width\":{},\"height\":{},\"channels\":{channels}}}", map.width(), map.height())
                    }
                    Err(error) => format!("{{\"error\":{:?}}}", error),
                };
                std::fs::write(folder.join("map.json"), report).unwrap();
            }
        }
    }
}
