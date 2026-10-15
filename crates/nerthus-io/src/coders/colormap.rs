//! Colormaps as ImageMagick's readers build them: `AcquireImageColormap`'s
//! grey ramp and `SyncImage`, which gives PseudoClass pixels their entries'
//! colours.

/// `AcquireImageColormap`: `max(colors, 1)` opaque grey levels from black
/// to white, a step worked out in single precision (`QuantumRange` is a
/// float); `None` past `MaxColormapSize` (65536).
pub(crate) fn acquire(colors: usize) -> Option<Vec<[f64; 4]>> {
    if colors > 65536 {
        return None;
    }
    let count = colors.max(1);
    let step = 65535.0f32 / colors.saturating_sub(1).max(1) as f32;
    let mut colormap = Vec::with_capacity(count);
    for index in 0..count {
        let level = index as f64 * step as f64;
        colormap.push([level, level, level, 65535.0]);
    }
    Some(colormap)
}

/// `SyncImage` over interleaved pixels whose red, green and blue are the
/// first three of `stride` channels and whose index is at `index_offset`:
/// each pixel takes its entry's colour (entry 0 for an index past the
/// colormap, which IM only warns about), and its alpha too when
/// `alpha_offset` is given.
pub(crate) fn sync(data: &mut [f32], stride: usize, index_offset: usize, alpha_offset: Option<usize>, colormap: &[[f64; 4]]) {
    for pixel in data.chunks_exact_mut(stride) {
        let index = pixel[index_offset] as usize;
        let entry = colormap.get(index).unwrap_or(&colormap[0]);
        pixel[0] = entry[0] as f32;
        pixel[1] = entry[1] as f32;
        pixel[2] = entry[2] as f32;
        if let Some(alpha_offset) = alpha_offset {
            pixel[alpha_offset] = entry[3] as f32;
        }
    }
}
