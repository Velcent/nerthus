//! The format-specific pixel loaders (`load_raw`) for camera raw files
//! other than DNG: one loader per maker family, filling the flat-field
//! raw buffer the common processing path consumes, followed by the image
//! bounds' masked-area black levels and the black fold the unpacking
//! tail runs over the decoded buffer.

use anyhow::{bail, Result};

use super::camera_raw_meta::{CameraMeta, SensorKind};
use super::identify::Raw;
use super::stream::{Stream, LITTLE_ENDIAN};
use super::unpack::{fold_black_to_common, RawData};

/// The shared camera unpack: the sizes' allocation (`rwidth`/`rheight`
/// widening, the 64k cap, the zero-filled slack rows the loaders may write
/// into), the maker's loader, then the masked-area black levels, the black
/// fold and the buffer trim to the sensor's own shape.
pub(super) fn unpack_camera(meta: &mut CameraMeta, state: &mut Raw, data: &[u8]) -> Result<RawData> {
    let mut rwidth = meta.raw_width;
    let mut rheight = meta.raw_height;
    if meta.fuji_width == 0 {
        // Non-Fuji allocation widens to the active area's far corner.
        rwidth = rwidth.max(meta.width + meta.left_margin);
        rheight = rheight.max(meta.height + meta.top_margin);
    }
    if rwidth > 65535 || rheight > 65535 {
        bail!("the camera raw image is larger than 64k pixels");
    }

    let mut stream = Stream::new(data);
    stream.seek(meta.data_offset.max(0) as u64);
    // The Bayer allocation, `calloc`: the zero-filled slack rows are part of
    // the loaders' contract, since they may write past their last row. The
    // legacy four-channel buffer shape is not modelled yet.
    if !(state.filters != 0 || meta.colors == 1) {
        bail!("the legacy camera raw loader is not ported yet");
    }
    if meta.raw_pitch == 0 {
        meta.raw_pitch = meta.raw_width * 2;
    }
    let mut raw_image = vec![0u16; rwidth as usize * (rheight as usize + 8)];
    // A `unpacked_load_raw` body's temporary 65535 maximum is restored around
    // the loader when that loader lands.
    match meta.loader {
        SensorKind::Samsung => samsung_load_raw(meta, &mut raw_image, &mut stream)?,
        _ => bail!("the pixel loader for this camera raw is not ported yet"),
    }
    crop_masked_pixels(meta, state, &raw_image);
    fold_black_to_common(state);
    raw_image.truncate(meta.raw_width as usize * meta.raw_height as usize);
    Ok(RawData::Bayer(raw_image))
}

/// The masked-area black levels: the file's MaskedAreas records or, for the
/// Canon loaders, the margins around the active area; the mean of each
/// colour's area replaces the pattern's black levels.
fn crop_masked_pixels(meta: &CameraMeta, state: &mut Raw, raw_image: &[u16]) {
    let mut mask = meta.mask;
    if mask[0][3] > 0 {
        // The file named its own masked areas.
    } else if matches!(
        meta.loader,
        SensorKind::CanonCiff | SensorKind::CanonLosslessJpeg | SensorKind::CanonCrx
    ) {
        // The two left and right strips inset by the Canon loader's noise
        // border columns.
        mask[0][1] += 2;
        mask[1][1] += 2;
        mask[0][3] -= 2;
        mask[0][0] = meta.top_margin as i32;
        mask[1][0] = meta.top_margin as i32;
        mask[0][2] = meta.top_margin as i32 + meta.height as i32;
        mask[1][2] = meta.top_margin as i32 + meta.height as i32;
        mask[0][3] += meta.left_margin as i32;
        mask[1][1] += meta.left_margin as i32 + meta.width as i32;
        mask[1][3] += meta.raw_width as i32;
    } else if (matches!(meta.loader, SensorKind::EightBit) && !meta.model.starts_with("DC2"))
        || (matches!(meta.loader, SensorKind::Packed) && meta.load_flags & 32 != 0)
    {
        // The strips alongside the active area; the canon_600, sony_load and
        // kodak_262 loaders fold into this step too once they are modelled.
        mask[0][0] = meta.top_margin as i32;
        mask[1][0] = meta.top_margin as i32;
        mask[0][2] = meta.top_margin as i32 + meta.height as i32;
        mask[1][2] = meta.top_margin as i32 + meta.height as i32;
        mask[0][3] += meta.left_margin as i32;
        mask[1][1] += meta.left_margin as i32 + meta.width as i32;
        mask[1][3] += meta.raw_width as i32;
    }

    let raw_width = meta.raw_width as i32;
    let raw_height = meta.raw_height as i32;
    let raw_pitch = meta.raw_pitch as usize;
    let mut mblack = [0u32; 8];
    let mut zero = 0u32;
    for area in mask {
        for row in area[0].max(0)..area[2].min(raw_height) {
            for column in area[1].max(0)..area[3].min(raw_width) {
                // No need to subtract margins because full area and active
                // area filters are the same.
                let colour = super::identify::fc(state.filters, row as u32, column as u32) as usize;
                let value = raw_image[row as usize * raw_pitch / 2 + column as usize] as u32;
                mblack[colour] += value;
                mblack[4 + colour] += 1;
                zero += u32::from(value == 0);
            }
        }
    }
    // canon_600_load_raw's body-wide mean is skipped; that loader is not
    // modelled yet.
    if zero < mblack[4] && mblack[5] != 0 && mblack[6] != 0 && mblack[7] != 0 {
        for colour in 0..4 {
            state.cblack[colour] = mblack[colour] / mblack[4 + colour].max(1);
        }
        state.black = 0;
        state.cblack[4] = 0;
        state.cblack[5] = 0;
        state.cblack[6] = 0;
    }
}

/// Samsung bodies' bit reader (`ph1_bits`): whole words out of the stream,
/// the reader keeping its position across the call.
#[derive(Default)]
struct Ph1Bits {
    bitbuf: u64,
    vbits: i32,
}

impl Ph1Bits {
    /// `ph1_bits(-1)`: reset for the next row.
    fn reset(&mut self) {
        self.bitbuf = 0;
        self.vbits = 0;
    }

    fn bits(&mut self, stream: &mut Stream, nbits: i32) -> u32 {
        if nbits == -1 {
            self.reset();
            return 0;
        }
        if nbits == 0 {
            return 0;
        }
        if self.vbits < nbits {
            let word = stream.get4();
            self.bitbuf = self.bitbuf << 32 | u64::from(word);
            self.vbits += 32;
        }
        let head = self.bitbuf.wrapping_shl((64 - self.vbits) as u32);
        let value = (head.wrapping_shr((64 - nbits) as u32) & 0xffffffff) as u32;
        self.vbits -= nbits;
        value
    }
}

/// `samsung_load_raw`: the first SRW generation's compressed rows, decoded in
/// sixteen-pixel blocks with prediction from the diagonal window above the
/// block or the row's left edge, then the 2×2 green swap turns the diagonal
/// block pattern square.
fn samsung_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    if meta.raw_width > 32768 || meta.raw_height > 32768 {
        bail!("the Samsung raw sensor is too large for its loader");
    }
    let raw_width = meta.raw_width as i32;
    let raw_height = meta.raw_height as i32;
    // `maxpixels`: the loader's bounds guard, in sensor indexes.
    let maxpixels = (meta.raw_width as u32) * (meta.raw_height + 7);
    // The strip table and block headers both read little-endian.
    stream.order = LITTLE_ENDIAN;
    let mut bits = Ph1Bits::default();
    let mut operation = [0i32; 4];
    let mut len = [0i32; 4];
    let mut data_error = false;

    for row in 0..raw_height {
        stream.seek((meta.strip_offset + row as i64 * 4) as u64);
        let strip = i64::from(stream.get4());
        stream.seek((meta.data_offset + strip).max(0) as u64);
        bits.reset();
        let head = if row < 2 { 7 } else { 4 };
        for value in len.iter_mut() {
            *value = head;
        }
        for column in (0..raw_width).step_by(16) {
            let dir = bits.bits(stream, 1);
            for slot in 0..4 {
                operation[slot] = bits.bits(stream, 2) as i32;
            }
            for slot in 0..4 {
                match operation[slot] {
                    3 => len[slot] = bits.bits(stream, 4) as i32,
                    2 => len[slot] -= 1,
                    1 => len[slot] += 1,
                    _ => {}
                }
            }
            // c walks the block's even columns first, 0 through 14, then its
            // odd columns 1 through 15; each pair shares one length.
            let mut c: i32 = 0;
            while c < 16 {
                let len_index = (((c & 1) << 1) | (c >> 3)) as usize;
                let i = len[len_index];
                // The bounds guard works on sensor indexes: a wrap below or
                // past the sensor reads as an unsigned overflow.
                let destination = (row * raw_width + column + c) as u32;
                let source = if dir != 0 {
                    ((row + (!c | -2)) * raw_width + column + c) as u32
                } else if column != 0 {
                    (row * raw_width + column + (c | -2)) as u32
                } else {
                    0
                };
                if destination < maxpixels && source < maxpixels {
                    let prediction = if dir != 0 {
                        raw_image[((row + (!c | -2)) * raw_width + column + c) as usize] as i32
                    } else if column != 0 {
                        raw_image[(row * raw_width + column + (c | -2)) as usize] as i32
                    } else {
                        128
                    };
                    let delta = if i > 0 {
                        let word = bits.bits(stream, i) as i32;
                        // The x86 shift truncates its count to five bits;
                        // `wrapping_shl`/`wrapping_shr` keep the same
                        // arithmetic for overlong lengths.
                        let shift = (32 - i) as u32;
                        word.wrapping_shl(shift).wrapping_shr(shift)
                    } else {
                        0
                    };
                    raw_image[destination as usize] = delta.wrapping_add(prediction) as u16;
                } else {
                    derror(stream, &mut data_error)?;
                }
                if c == 14 {
                    c = -1;
                }
                c += 2;
            }
        }
    }

    for row in (0..raw_height - 1).step_by(2) {
        for column in (0..raw_width - 1).step_by(2) {
            let left = raw_image[(row * raw_width + column + 1) as usize];
            let right = raw_image[((row + 1) * raw_width + column) as usize];
            raw_image[(row * raw_width + column + 1) as usize] = right;
            raw_image[((row + 1) * raw_width + column) as usize] = left;
        }
    }
    Ok(())
}

/// The loader's byte-shortfall counter: the first one on a stream already at
/// its end aborts the whole unpack, later ones are silent.
fn derror(stream: &Stream, data_error: &mut bool) -> Result<()> {
    if !*data_error && stream.eof() {
        bail!("the raw stream ended before the image did");
    }
    *data_error = true;
    Ok(())
}