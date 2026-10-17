//! The format-specific pixel loaders (`load_raw`) for camera raw files
//! other than DNG: one loader per maker family, filling the flat-field
//! raw buffer the common processing path consumes, followed by the image
//! bounds' masked-area black levels and the black fold the unpacking
//! tail runs over the decoded buffer.

use anyhow::{bail, Result};

use super::camera_raw_meta::{CameraMeta, SensorKind};
use super::identify::{NOT_TILED, Raw};
use super::ljpeg::{BitReader, ljpeg_diff as ljpeg_diff_bits, ljpeg_row, ljpeg_start};
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
    // The loaders read with the order the metadata parse ended with; the
    // Samsung body forces its own little-endian inside.
    stream.order = meta.order;
    stream.seek(meta.data_offset.max(0) as u64);

    let mut data_error = false;
    if !(state.filters != 0 || meta.colors == 1) {
        // The legacy path: the sRAW-style bodies decode into the four-channel
        // image buffer instead of the flat raw store. The row stride is the
        // active width's 8 bytes unless the loader keeps the raw shape's, the
        // buffer spans whichever of the two shapes is larger — calloc'd zero,
        // since the decode may leave pixels untouched — and the loader may
        // still hold a flat view over the whole store.
        if meta.raw_pitch == 0 {
            meta.raw_pitch = if meta.loader == SensorKind::SonyArq {
                meta.raw_width * 8
            } else {
                meta.width * 8
            };
        }
        let image_width = meta.raw_width.max(meta.width) as usize;
        let image_height = meta.raw_height.max(meta.height) as usize;
        let mut image = vec![[0u16; 4]; image_width * (image_height + 8)];
        match meta.loader {
            SensorKind::CanonSraw => canon_sraw_load_raw(meta, &mut image, &mut stream)?,
            SensorKind::NikonSraw => nikon_sraw_load_raw(meta, &mut image, &mut stream)?,
            SensorKind::SonyArq => sony_arq_load_raw(meta, &mut image, &mut stream, &mut data_error)?,
            SensorKind::Nikon14Bit => nikon_14bit_load_raw(meta, image.as_flattened_mut(), &mut stream)?,
            _ => bail!("the pixel loader for this camera raw is not ported yet"),
        }
        state.maximum = meta.maximum;
        // The legacy tail: crop_masked_pixels only runs for the flat raw
        // store, so the black levels stay as the maker parse built them, and
        // without the open crop's 256 flag the decode shape replaces the raw
        // shape outright — except the pixel-shift loader, whose margins the
        // engine keeps.
        if meta.load_flags & 256 == 0 && meta.loader != SensorKind::SonyArq {
            meta.raw_width = meta.width;
            meta.left_margin = 0;
            meta.raw_height = meta.height;
            meta.top_margin = 0;
        }
        state.raw_width = meta.raw_width;
        state.raw_height = meta.raw_height;
        state.width = meta.width;
        state.height = meta.height;
        state.left_margin = meta.left_margin;
        state.top_margin = meta.top_margin;
        fold_black_to_common(state);
        image.truncate(meta.raw_width as usize * meta.raw_height as usize);
        return Ok(RawData::Color4(image));
    }
    // The Bayer allocation, `calloc`: the zero-filled slack rows are part of
    // the loaders' contract, since they may write past their last row.
    if meta.raw_pitch == 0 {
        meta.raw_pitch = meta.raw_width * 2;
    }
    let mut raw_image = vec![0u16; rwidth as usize * (rheight as usize + 8)];
    // A `unpacked_load_raw` body's temporary 65535 maximum is restored around
    // the loader when that loader lands.
    match meta.loader {
        SensorKind::Samsung => samsung_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Pentax => pentax_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Olympus => olympus_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Panasonic => panasonic_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Unpacked => {
            // Nikon and Hasselblad bodies scan their unpacked rows against a
            // temporary full maximum; the engine's own value comes straight
            // back from the unpack, where this copy leaves it untouched.
            let overriding_maximum = if meta.make.eq_ignore_ascii_case("Nikon")
                || meta.make.eq_ignore_ascii_case("Hasselblad")
            {
                0xffff
            } else {
                meta.maximum
            };
            unpacked_load_raw(meta, &mut raw_image, &mut stream, &mut data_error, overriding_maximum)?
        }
        SensorKind::SonyArw2 => sony_arw2_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::SonyArw => sony_arw_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::NikonPlain => nikon_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::CanonLosslessJpeg => lossless_jpeg_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::CanonCiff => {
            // The CRW body's low-bit probe picks between the row layouts;
            // without low bits the engine drops the maximum to 1023.
            let heads = canon_has_lowbits(data) != 0;
            if !heads {
                meta.maximum = 0x3ff;
            }
            canon_load_raw(meta, &mut raw_image, &mut stream, &mut data_error, heads)?
        }
        SensorKind::CanonCrx => canon_crx_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::FujiCompressed => {
            fuji_compressed_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?
        }
        SensorKind::EightBit => eight_bit_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Packed => packed_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Nikon14Bit => nikon_14bit_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::NikonStripedPacked => nikon_striped_packed_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::NikonPaddedPacked => nikon_padded_packed_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::Fuji14Bit => fuji_14bit_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::FujiDbp => fuji_dbp_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::FujiF700S20 => {
            fuji_f700s20_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?
        }
        SensorKind::Samsung2 => samsung2_load_raw(meta, &mut raw_image, &mut stream, &mut data_error)?,
        SensorKind::Samsung3 => samsung3_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::PanasonicC6 => panasonic_c6_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::PanasonicC7 => panasonic_c7_load_raw(meta, &mut raw_image, &mut stream)?,
        SensorKind::SonyLjpeg => sony_ljpeg_load_raw(meta, &mut raw_image, &mut stream)?,
        // The JPEG-XS form has no decoder in the reference engine: the unpack
        // refuses the file before any pixels move.
        SensorKind::NikonHe => bail!("this camera raw's compressed stream has no decoder"),
        // The uncompressed YUV stream is rewritten to the sRAW loader before
        // the decode; the marker never reaches the dispatcher.
        SensorKind::NikonYuv => bail!("this camera raw's YUV loader was not rewritten by the open stage"),
        _ => bail!("the pixel loader for this camera raw is not ported yet"),
    }
    // The CRW body's maximum falls with its low-bit probe; the state the
    // processing runs on takes the value the loader left.
    state.maximum = meta.maximum;
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
fn samsung_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
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
                    derror(stream, data_error)?;
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

/// The `getbithuff` byte reader: bytes one at a time out of the stream, the
/// maker Huffman tables decoding through it. Only `canon_load_raw` turns the
/// stuffed-zero flag on; while it is on, the byte after a 0xff is dropped and
/// a stuffed zero freezes the fill until the next reset.
#[derive(Default)]
struct ByteBits {
    bitbuf: u32,
    vbits: i32,
    zero_after_ff: bool,
    reset: bool,
}

impl ByteBits {
    /// `getbits(-1)`: reset for the next decode.
    fn reset(&mut self) {
        self.bitbuf = 0;
        self.vbits = 0;
        self.reset = false;
    }

    /// `getbits(nbits)`, or with a table `gethuff`: the table's entry reads
    /// its high byte as the code's length and its low byte as the symbol,
    /// and the first call reads `table[0]` bits; without a table the call
    /// just yields `nbits` raw bits.
    fn bits(
        &mut self,
        stream: &mut Stream,
        data_error: &mut bool,
        nbits: i32,
        table: Option<&[u16]>,
    ) -> Result<u32> {
        if nbits > 25 {
            return Ok(0);
        }
        if nbits < 0 {
            self.reset();
            return Ok(0);
        }
        if nbits == 0 || self.vbits < 0 {
            return Ok(0);
        }
        while !self.reset && self.vbits < nbits {
            let Some(byte) = stream.getc() else { break };
            if self.zero_after_ff && byte == 0xff {
                // The follower decides: a stuffed zero is dropped and the
                // 0xff goes on as data, while any other follower (or the
                // stream's end) stops the fill until the next reset.
                if !matches!(stream.getc(), Some(0)) {
                    self.reset = true;
                    break;
                }
            }
            self.bitbuf = self.bitbuf << 8 | u32::from(byte);
            self.vbits += 8;
        }
        let mut value = if self.vbits == 0 {
            0
        } else {
            self.bitbuf
                .wrapping_shl((32 - self.vbits) as u32)
                .wrapping_shr((32 - nbits) as u32)
        };
        match table {
            Some(table) => {
                let entry = table[value as usize];
                self.vbits -= i32::from(entry >> 8);
                value = u32::from(entry as u8);
            }
            None => {
                self.vbits -= nbits;
            }
        }
        if self.vbits < 0 {
            derror(stream, data_error)?;
        }
        Ok(value)
    }
}

/// `ljpeg_diff`: one difference from a maker Huffman table, sign-extended the
/// lossless-JPEG predictor way. The camera raw files never carry a DNG
/// version, so a 16-bit code always reads as the −32768 clamped value.
fn ljpeg_diff(
    bits: &mut ByteBits,
    stream: &mut Stream,
    data_error: &mut bool,
    table: &[u16],
) -> Result<i32> {
    let len = bits.bits(stream, data_error, i32::from(table[0]), Some(&table[1..]))? as i32;
    if len == 16 {
        return Ok(-32768);
    }
    let mut diff = bits.bits(stream, data_error, len, None)? as i32;
    // The x86 shift truncates its count to five bits, and a garbled table
    // may carry any length; without the sign bit the value wraps below
    // zero.
    if diff & (1u32 << ((len - 1) & 31)) as i32 == 0 {
        diff -= (1i32 << (len & 31)) - 1;
    }
    Ok(diff)
}

/// `pentax_load_raw`: the PEF rows decoded through the Huffman table the
/// body's 0x0220 makernote record carries: one difference per pixel against
/// the two running left predictors and the row pair's own, with a byte's
/// shortfall at the stream's end counted silently like any other row error.
fn pentax_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let mut bit = [[0u16; 15], [0u16; 15]];
    let mut huff = [0u16; 4097];

    stream.seek(meta.meta_offset.max(0) as u64);
    let dep = ((i32::from(stream.get2()) + 12) & 15) as usize;
    stream.skip(12);
    for value in bit[0].iter_mut().take(dep) {
        *value = stream.get2();
    }
    // A single byte each, and an end-of-file read keeps its −1 the way
    // `fgetc` hands it to a `ushort`.
    for value in bit[1].iter_mut().take(dep) {
        *value = stream.getc().map_or(-1i16, i16::from) as u16;
    }
    for c in 0..dep {
        // The x86 shift truncates its count to five bits.
        let window = 4096i32 >> (i32::from(bit[1][c]) & 31);
        let entry = ((u32::from(bit[1][c]) << 8 | c as u32) & 0xffff) as u16;
        let end = (i32::from(bit[0][c]) + window - 1) & 4095;
        let mut i = bit[0][c] as usize;
        while i <= end as usize {
            i += 1;
            huff[i] = entry;
        }
    }
    huff[0] = 12;

    stream.seek(meta.data_offset.max(0) as u64);
    let mut bits = ByteBits::default();
    let mut vpred = [[0u16; 2]; 2];
    let mut hpred = [0u16; 2];
    let raw_width = meta.raw_width as usize;
    for row in 0..meta.raw_height as usize {
        for column in 0..raw_width {
            let diff = ljpeg_diff(&mut bits, stream, data_error, &huff)?;
            if column < 2 {
                vpred[row & 1][column] = vpred[row & 1][column].wrapping_add(diff as u16);
                hpred[column] = vpred[row & 1][column];
            } else {
                hpred[column & 1] = hpred[column & 1].wrapping_add(diff as u16);
            }
            raw_image[row * raw_width + column] = hpred[column & 1];
            if u32::from(hpred[column & 1]) >> meta.tiff_bps != 0 {
                derror(stream, data_error)?;
            }
        }
    }
    Ok(())
}

/// Panasonic bodies' read buffer (`pana_data`'s `buf`): 64 KiB plus two,
/// refilled in two passes that weave the file over itself — the low half of
/// the read lands in the buffer's high half, the next chunk over the
/// buffer's start. A short read at the file's end leaves the stale bytes
/// readable, the engine's reads do the same.
struct PanasonicData {
    buffer: Box<[u8]>,
    vpos: i32,
}

impl PanasonicData {
    /// The loader's own `pana_data(0, 0)`: buffer zeroed, cursor reset.
    fn new() -> Self {
        PanasonicData { buffer: vec![0u8; 0x4002].into_boxed_slice(), vpos: 0 }
    }

    /// The refill (`if (!vpos)`) at the cursor's wrap or start.
    fn refill(&mut self, stream: &mut Stream, load_flags: u32) {
        let low = load_flags as usize;
        if load_flags < 0x4000 {
            stream.read(&mut self.buffer[low..0x4000]);
        }
        stream.read(&mut self.buffer[..low]);
    }

    /// `pana_data(nb)` for the bit-packed rows: the cursor counts down
    /// through the weave and pulls `nb` bits off its byte pair.
    fn bits(&mut self, stream: &mut Stream, nb: u32, load_flags: u32) -> u32 {
        if self.vpos == 0 {
            self.refill(stream, load_flags);
        }
        self.vpos = (self.vpos.wrapping_sub(nb as i32)) & 0x1ffff;
        let byte = ((self.vpos as usize) >> 3) ^ 0x3ff0;
        let word = u32::from(self.buffer[byte]) | u32::from(self.buffer[byte + 1]) << 8;
        word >> (self.vpos as u32 & 7) & !(u32::MAX << nb)
    }

    /// `pana_data(0, bytes)` for the block rows: sixteen buffer bytes at
    /// the cursor, the cursor wrapping in a 14-bit space.
    fn block(&mut self, stream: &mut Stream, load_flags: u32) -> [u16; 16] {
        if self.vpos == 0 {
            self.refill(stream, load_flags);
        }
        let mut block = [0u16; 16];
        for slot in &mut block {
            *slot = u16::from(self.buffer[self.vpos as usize]);
            self.vpos += 1;
            self.vpos &= 0x3fff;
        }
        block
    }
}

/// `panasonic_load_raw`: the RW2 rows. `RawFormat` 5 files lay down whole
/// sixteen-byte blocks of sample records (ten or nine samples a block by
/// the body's BitsPerSample); the rest unpacks bit-packed differences
/// through the weave cursor, each fourteen-pixel stretch carrying its own
/// predictor state and the group scale it last drew.
fn panasonic_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let bits = meta.panasonic.bits;
    let encoding = meta.panasonic.encoding;
    let raw_width = meta.raw_width as usize;
    let mut reader = PanasonicData::new();

    if encoding == 5 {
        let block_size = if bits == 12 { 10 } else { 9 };
        for row in 0..meta.raw_height {
            let base = row as usize * raw_width;
            let mut col = 0usize;
            while col < raw_width {
                let block = reader.block(stream, meta.load_flags);
                if bits == 12 {
                    // Ten samples a block, two per group of three bytes.
                    raw_image[base + col] = ((block[1] & 0xf) << 8) | block[0];
                    raw_image[base + col + 1] = 16 * block[2] + (block[1] >> 4);
                    raw_image[base + col + 2] = ((block[4] & 0xf) << 8) | block[3];
                    raw_image[base + col + 3] = 16 * block[5] + (block[4] >> 4);
                    raw_image[base + col + 4] = ((block[7] & 0xf) << 8) | block[6];
                    raw_image[base + col + 5] = 16 * block[8] + (block[7] >> 4);
                    raw_image[base + col + 6] = ((block[10] & 0xf) << 8) | block[9];
                    raw_image[base + col + 7] = 16 * block[11] + (block[10] >> 4);
                    raw_image[base + col + 8] = ((block[13] & 0xf) << 8) | block[12];
                    raw_image[base + col + 9] = 16 * block[14] + (block[13] >> 4);
                } else if bits == 14 {
                    // Nine samples a block, the last one on the block's
                    // fifteenth byte.
                    raw_image[base + col] = block[0] + ((block[1] & 0x3f) << 8);
                    raw_image[base + col + 1] = (block[1] >> 6) + 4 * block[2] + ((block[3] & 0xf) << 10);
                    raw_image[base + col + 2] = (block[3] >> 4) + 16 * block[4] + ((block[5] & 3) << 12);
                    raw_image[base + col + 3] = ((block[5] & 0xfc) >> 2) + (block[6] << 6);
                    raw_image[base + col + 4] = block[7] + ((block[8] & 0x3f) << 8);
                    raw_image[base + col + 5] = (block[8] >> 6) + 4 * block[9] + ((block[10] & 0xf) << 10);
                    raw_image[base + col + 6] = (block[10] >> 4) + 16 * block[11] + ((block[12] & 3) << 12);
                    raw_image[base + col + 7] = ((block[12] & 0xfc) >> 2) + (block[13] << 6);
                    raw_image[base + col + 8] = block[14] + ((block[15] & 0x3f) << 8);
                }
                col += block_size;
            }
        }
        return Ok(());
    }

    if meta.load_flags >= 0x4000 {
        bail!("the Panasonic raw stream uses an unsupported packing");
    }
    let mut pred = [0i32; 2];
    let mut nonz = [0i32; 2];
    // The group scale persists until the next one is drawn.
    let mut sh = 0i32;
    for row in 0..meta.raw_height as i32 {
        for col in 0..raw_width as i32 {
            let i = col % 14;
            if i == 0 {
                pred = [0, 0];
                nonz = [0, 0];
            }
            if i % 3 == 2 {
                let group = reader.bits(stream, 2, meta.load_flags) as i32;
                sh = 4 >> (3 - group);
            }
            let slot = (i & 1) as usize;
            if nonz[slot] != 0 {
                let j = reader.bits(stream, 8, meta.load_flags);
                if j != 0 {
                    pred[slot] = pred[slot].wrapping_sub(0x80 << sh);
                    if pred[slot] < 0 || sh == 4 {
                        pred[slot] = ((pred[slot] as u32) & !(u32::MAX << sh)) as i32;
                    }
                    pred[slot] = pred[slot].wrapping_add((j << sh) as i32);
                }
            } else {
                nonz[slot] = reader.bits(stream, 8, meta.load_flags) as i32;
                if nonz[slot] != 0 || i > 11 {
                    let low = reader.bits(stream, 4, meta.load_flags) as i32;
                    pred[slot] = (nonz[slot] << 4) | low;
                }
            }
            let stored = pred[(col & 1) as usize] as u16;
            raw_image[row as usize * raw_width + col as usize] = stored;
            if stored > 4098 && col < meta.width as i32 && row < meta.height as i32 {
                derror(stream, data_error)?;
            }
        }
    }
    Ok(())
}

/// Olympus bodies' read-ahead bit pump (`buffered_bitpump_t`): 64 KiB of the
/// file at a time, the bits read off a big-endian word that refills keep
/// topping up. A refill shorter than the ask leaves the buffer's stale tail
/// readable — the engine's own slow paths read those bytes too.
struct OlympusPump {
    bitcount: u32,
    bitstorage: u32,
    buffer: Box<[u8]>,
    pos: i32,
    datasz: i32,
}

impl OlympusPump {
    fn new() -> Self {
        OlympusPump { bitcount: 0, bitstorage: 0, buffer: vec![0u8; 65536].into_boxed_slice(), pos: 0, datasz: 0 }
    }

    /// `refill(b)`: one bulk read over the buffer; the pump always presents
    /// at least `b` bytes, stale ones when the file has run out.
    fn refill(&mut self, stream: &mut Stream, minimum: i32) {
        let read = stream.read(&mut self.buffer);
        self.pos = 0;
        self.datasz = if read as i32 > minimum { read as i32 } else { minimum };
    }

    /// `replace_data`: a fresh four-byte word, the bits read top first.
    fn replace_data(&mut self, stream: &mut Stream) {
        self.bitstorage = self.get4(stream);
        self.bitcount = 32;
    }

    /// `add_data`: two more file bytes over the word's low end.
    fn add_data(&mut self, stream: &mut Stream) -> u32 {
        let value = u32::from(self.get2(stream));
        self.bitstorage = self.bitstorage << 16 | value;
        self.bitcount = self.bitcount.wrapping_add(16);
        self.bitcount
    }

    /// The pump's `getbits(wbits)`.
    fn getbits(&mut self, stream: &mut Stream, widths: u32) -> u32 {
        while self.bitcount < widths {
            self.add_data(stream);
        }
        let value = self
            .bitstorage
            .wrapping_shl(32u32.wrapping_sub(self.bitcount & 0x1f))
            .wrapping_shr(32u32.wrapping_sub(widths));
        self.bitcount = self.bitcount.wrapping_sub(widths);
        value
    }

    /// The buffer's next byte pair, big-endian assembled. Short buffers
    /// drain their last bytes first, then refill and read the rest through
    /// the fresh start.
    fn get2(&mut self, stream: &mut Stream) -> u16 {
        if self.pos < self.datasz - 1 {
            let value =
                u16::from(self.buffer[self.pos as usize]) << 8 | u16::from(self.buffer[self.pos as usize + 1]);
            self.pos += 2;
            return value;
        }
        let mut bytes = [0u8; 2];
        let mut count = 0;
        while self.pos < self.datasz {
            bytes[count] = self.buffer[self.pos as usize];
            count += 1;
            self.pos += 1;
        }
        self.refill(stream, 2);
        while count < 2 {
            bytes[count] = self.buffer[self.pos as usize];
            count += 1;
            self.pos += 1;
        }
        u16::from(bytes[0]) << 8 | u16::from(bytes[1])
    }

    /// The buffer's next four bytes, big-endian assembled.
    fn get4(&mut self, stream: &mut Stream) -> u32 {
        if self.pos < self.datasz - 3 {
            let position = self.pos as usize;
            let value = u32::from(self.buffer[position]) << 24
                | u32::from(self.buffer[position + 1]) << 16
                | u32::from(self.buffer[position + 2]) << 8
                | u32::from(self.buffer[position + 3]);
            self.pos += 4;
            return value;
        }
        let mut bytes = [0u8; 4];
        let mut count = 0;
        while self.pos < self.datasz {
            bytes[count] = self.buffer[self.pos as usize];
            count += 1;
            self.pos += 1;
        }
        self.refill(stream, 4);
        while count < 4 {
            bytes[count] = self.buffer[self.pos as usize];
            count += 1;
            self.pos += 1;
        }
        u32::from(bytes[0]) << 24 | u32::from(bytes[1]) << 16 | u32::from(bytes[2]) << 8 | u32::from(bytes[3])
    }

    /// `skip_bit`: one bit's position spent, a spent word replaced with a
    /// fresh one when the position is already at the word's start.
    fn skip_bit(&mut self, stream: &mut Stream, vbits: &mut u32) {
        if *vbits == 0 {
            self.replace_data(stream);
            self.bitcount -= 1;
            *vbits = self.bitcount;
        } else {
            *vbits -= 1;
            self.bitcount = *vbits;
        }
    }
}

/// The Olympus raw stream's header check: `bytes` status shorts' worth —
/// the body's 0x653 tag count — of leading ones, then two zero bytes. An
/// end-of-file read keeps its all-ones the way `fgetc` hands it over.
fn olympus_checkhdr(stream: &mut Stream, bytes: i32) -> bool {
    if bytes <= 0 {
        return false;
    }
    let mut code: u64 = 0;
    for _ in 0..bytes {
        let byte = stream.getc().unwrap_or(0xff);
        code = code << 8 | u64::from(byte);
    }
    if code != 1 {
        return false;
    }
    let first = stream.getc().unwrap_or(0xff);
    let second = stream.getc().unwrap_or(0xff);
    if first | second != 0 {
        return false;
    }
    true
}

/// The branchless absolute value the loader uses on its prediction window.
fn olympus_iabs(x: i32) -> i32 {
    (x ^ (x >> 31)) - (x >> 31)
}

/// One pixel's difference: the header bit and the small side bits, then the
/// terminator run that carries the number — a run of zero bits up to the
/// body's 0x652 bound, or a fresh refill past the escape's own length. The
/// read result's complement is written back in `glc` un-flipped, so the
/// next pixel's context sees the value before the flip.
fn oly_code(
    pump: &mut OlympusPump,
    stream: &mut Stream,
    widths: u32,
    tag_0x640: u32,
    tag_0x643: u32,
    tag_0x652: u32,
    glc: &mut i32,
    t640_bits: &mut i32,
    t643_bits: &mut i32,
) -> i32 {
    if pump.bitcount == 0 {
        pump.replace_data(stream);
    }
    pump.bitcount -= 1;
    let v = pump.bitcount as i32;
    let high_data_bit = (pump.bitstorage.wrapping_shr(v as u32) & 1) as i32;
    *t643_bits = if tag_0x643 != 0 { pump.getbits(stream, tag_0x643) as i32 } else { 0 };
    *t640_bits = if tag_0x640 != 0 { pump.getbits(stream, tag_0x640) as i32 } else { 0 };

    let mut vbits = pump.bitcount;
    let mut high: u32 = 0;
    let mut low: u32 = 0;

    loop {
        pump.skip_bit(stream, &mut vbits);
        if pump.bitstorage.wrapping_shr(vbits) & 1 != 0 {
            break;
        }
        high += 1;
        if u32::from(tag_0x652) == high {
            high = 0;
            if widths != 15 {
                let escape = 15u32.wrapping_sub(widths);
                while vbits < escape {
                    vbits = pump.add_data(stream);
                }
                high = pump
                    .bitstorage
                    .wrapping_shl(32u32.wrapping_sub(vbits & 0x1f))
                    .wrapping_shr(widths.wrapping_add(17));
                vbits = vbits.wrapping_sub(escape);
                pump.bitcount = vbits;
            }
            pump.skip_bit(stream, &mut vbits);
            break;
        }
    }

    if widths < 1 {
        low = 0;
    } else {
        while vbits < widths {
            vbits = pump.add_data(stream);
        }
        low = pump
            .bitstorage
            .wrapping_shl(32u32.wrapping_sub(vbits & 0x1f))
            .wrapping_shr(32u32.wrapping_sub(widths & 0x1f));
        pump.bitcount = vbits.wrapping_sub(widths);
    }
    *glc = (low | high.wrapping_shl(widths)) as i32;
    // The x86 shift truncates its count to five bits throughout.
    let mask = (high_data_bit as i32).wrapping_neg() as u32;
    (*glc as u32 ^ mask) as i32
}

/// `olympus_load_raw`: the maker's own compressed rows, one difference per
/// pixel off the body's 0x640-0x653 plan — a per-pixel code length from the
/// running sample history, a terminator run, then a median prediction
/// through the left, up and up-left neighbours of the body's own pixel.
fn olympus_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let olympus = &meta.olympus;
    if !olympus_checkhdr(stream, olympus.tag_x653 as i32) {
        bail!("the Olympus raw stream is missing its header");
    }
    let tag_0x640 = olympus.tag_x640 as i32;
    let tag_0x643 = olympus.tag_x643;
    let mut pump = OlympusPump::new();
    let one_shl_tag_0x641 = 1i32.wrapping_shl(olympus.tag_x641);
    let tag_0x642 = olympus.tag_x642 as i32;
    let tag_0x644 = olympus.tag_x644 as i32;
    let one_shl_tag_645 = 1i32 << (olympus.tag_x645 & 0x1f);
    let tag_0x646 = olympus.tag_x646 as i32;
    let tag_0x647 = olympus.tag_x647 as i32;
    let tag_0x648 = olympus.tag_x648 as i32;
    let tag_0x649 = olympus.tag_x649 as i32;
    let tag_0x650 = olympus.tag_x650 as i32;
    let tag_0x651 = olympus.tag_x651 as i32;
    let tag_0x652 = 1.max(olympus.tag_x652 as i32);
    // `datamax = 1 << ValidBits`, truncated into a `ushort`.
    let data_maximum = (1i32 << (olympus.valid_bits & 31)) as u16;

    let raw_width = meta.raw_width as i32;
    let mut bitcounts = [0u32; 65536];
    let mut count = 1;
    for i in 0..16u32 {
        for _ in 0..(1u32 << i) {
            bitcounts[count] = i + 1;
            count += 1;
        }
    }

    let mut lpred: i32 = 0;
    let mut pred1: i32 = 0;
    let mut glc: i32 = 0;
    // The C loader shadows its own `gcode` inside the column loop: the
    // outer one only ever holds the write-back value two pixels back.
    let mut carried_gcode: i32 = 0;
    let mut context = [0u32; 4];

    for row in 0..meta.raw_height as i32 {
        for col in 0..raw_width {
            let p = carried_gcode;
            carried_gcode = glc;
            let psel: i32 = if col > 1 && one_shl_tag_645 >= p { context[3] as i32 + 1 } else { 0 };
            context[3] = context[2];
            context[2] = psel as u32;
            let widths = if col > 1 {
                let high_bit_count = bitcounts[(p & 0xffff) as usize] as i32;
                let t1 = if psel >= tag_0x646 { tag_0x648 } else { tag_0x650 };
                let t2 = if psel >= tag_0x646 { tag_0x647 } else { tag_0x649 };
                let shift = (high_bit_count - t1).max(-1);
                t2.wrapping_add(shift).wrapping_add(1)
            } else {
                tag_0x649
            };
            let widths = if widths > tag_0x651 { tag_0x651 } else { widths };

            let mut t640_bits = 0;
            let mut t643_bits = 0;
            let mut gcode = oly_code(
                &mut pump,
                stream,
                widths as u32,
                olympus.tag_x640,
                tag_0x643,
                tag_0x652 as u32,
                &mut glc,
                &mut t640_bits,
                &mut t643_bits,
            );
            let vbits: i32;
            if tag_0x643 != 0 {
                let v = if col < 2 { 0 } else { pred1 };
                if tag_0x643 == 1 {
                    gcode = t643_bits.wrapping_add(v.wrapping_add(gcode).wrapping_mul(2));
                    vbits = v.wrapping_add(gcode >> 1);
                } else {
                    gcode = t643_bits.wrapping_add(v.wrapping_add(gcode).wrapping_mul(4));
                    // The engine's 0xFFFFFFFE mask, its low bit cleared.
                    vbits = ((gcode >> 1) & -2i32).wrapping_add(v).wrapping_add(gcode >> 2);
                }
            } else {
                vbits = 0;
            }

            pred1 = context[0] as i32;
            context[0] = if tag_0x644 == 15 { 0 } else { (vbits >> (tag_0x644 & 31)) as u32 };
            let w = if col < 2 { tag_0x642 } else { context[1] as i32 };
            let n = if row < 2 {
                tag_0x642
            } else {
                i32::from(raw_image[((row - 2) * raw_width + col) as usize]) >> (tag_0x640 & 31)
            };
            let nw = if row < 2 || col < 2 {
                tag_0x642
            } else {
                i32::from(raw_image[((row - 2) * raw_width + col - 2) as usize]) >> (tag_0x640 & 31)
            };

            context[1] = lpred as u32;
            let n = {
                let mut n = n;
                if w < n || nw < w {
                    if nw <= n && w >= n {
                        n = w;
                    } else if nw < n || w >= n {
                        if nw > w || w >= n {
                            if olympus_iabs(n - nw) > one_shl_tag_0x641 {
                                n = n.wrapping_add(w.wrapping_sub(nw));
                            } else if olympus_iabs(w - nw) <= one_shl_tag_0x641 {
                                n = n.wrapping_add(w) >> 1;
                            } else {
                                n = n.wrapping_add(w).wrapping_sub(nw);
                            }
                        }
                    } else {
                        n = w;
                    }
                }
                n
            };

            lpred = gcode.wrapping_add(n);
            let pixel = (t640_bits as u32)
                .wrapping_add((gcode.wrapping_add(n) as u32).wrapping_shl(olympus.tag_x640));
            let column = (row * raw_width + col) as usize;
            raw_image[column] = pixel as u16;
            if pixel as u16 > data_maximum && col >= meta.width as i32 {
                derror(stream, data_error)?;
            }
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

/// `unpacked_load_raw`: the rows as whole 16-bit samples in the file's own
/// byte order, then the over-maximum scan the body's bounds — only inside
/// the active area — turns into a shortfall count. Nikon and Hasselblad
/// bodies scan against the temporary full maximum the unpack passes in.
fn unpacked_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
    maximum: u32,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    // The smallest shift a sample needs before it reads past the maximum.
    let mut bits = 0;
    loop {
        bits += 1;
        if !(1i32.wrapping_shl(bits as u32) < maximum as i32) {
            break;
        }
    }
    // LibRaw's read_shorts counts its shortfall; the stream holds nothing
    // else between the seek and this read.
    let count = raw_width * raw_height;
    let short_read = stream.size().saturating_sub(stream.tell()) < (count * 2) as u64;
    stream.read_shorts(&mut raw_image[..count]);
    if short_read {
        derror(stream, data_error)?;
    }
    // The two bytes read back keep a later short-fall check from seeing the
    // file's end.
    stream.skip(-2);
    if maximum < 0xffff || meta.load_flags != 0 {
        for row in 0..raw_height {
            for col in 0..raw_width {
                let index = row * raw_width + col;
                // The engine's ushort shifts are int shifts behind the mask.
                let shifted = (i32::from(raw_image[index]) >> (meta.load_flags & 31)) as u16;
                raw_image[index] = shifted;
                if i32::wrapping_shr(i32::from(shifted), bits as u32) != 0
                    && ((row as i32 - meta.top_margin as i32) as u32) < meta.height
                    && ((col as i32 - meta.left_margin as i32) as u32) < meta.width
                {
                    derror(stream, data_error)?;
                }
            }
        }
    }
    Ok(())
}

/// `sony_arw2_load_raw`: each row's 16-sample blocks carry the block's own
/// minimum and maximum plus 14 seven-bit deltas against it, and every
/// sample reads through the tone curve. The engine's walk alternates a
/// block's output between the row's even and odd columns.
fn sony_arw2_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let height = meta.height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let curve = &meta.curve;
    // A reused row buffer; a short read leaves the previous row's tail in
    // place, as the engine's fread does, and this loader raises no data
    // error.
    let mut buffer = vec![0u8; raw_width + 1];
    for row in 0..height {
        stream.read(&mut buffer[..raw_width]);
        let mut column = 0usize;
        let mut block = 0usize;
        while (column as i32) < raw_width as i32 - 30 {
            let header = stream
                .sget4([buffer[block], buffer[block + 1], buffer[block + 2], buffer[block + 3]])
                as i32;
            let max = header & 0x7ff;
            let min = header.wrapping_shr(11) & 0x7ff;
            let max_slot = header.wrapping_shr(22) & 0x0f;
            let min_slot = header.wrapping_shr(26) & 0x0f;
            let mut shift = 0i32;
            while shift < 4 && (0x80i32 << shift) <= max - min {
                shift += 1;
            }
            let mut samples = [0u16; 16];
            let mut bit = 30i32;
            for (slot, sample) in samples.iter_mut().enumerate() {
                if slot == max_slot as usize {
                    *sample = max as u16;
                } else if slot == min_slot as usize {
                    *sample = min as u16;
                } else {
                    let offset = block + (bit >> 3) as usize;
                    let word = stream.sget2([buffer[offset], buffer[offset + 1]]) as i32;
                    let value = ((word >> (bit & 7)) & 0x7f) << shift;
                    *sample = (value + min) as u16;
                    if *sample > 0x7ff {
                        *sample = 0x7ff;
                    }
                    bit += 7;
                }
            }
            for &sample in samples.iter() {
                raw_image[row * pitch + column] = curve[(sample as usize) << 1];
                column += 2;
            }
            column -= if column & 1 != 0 { 1 } else { 31 };
            block += 16;
        }
    }
    Ok(())
}

/// `sony_arw_load_raw`: the oldest Sony bitstream, column first, where one
/// per-pixel difference runs through the 15-bit lookup the loader builds
/// from its own fixed code list and accumulates without clearing between
/// columns. A row read past the sensor's height falls onto row 1, the way
/// the engine's two-row-per-column walk does.
fn sony_arw_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let height = meta.height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;

    const TABLE: [u16; 18] = [
        0xf11, 0xf10, 0xe0f, 0xd0e, 0xc0d, 0xb0c, 0xa0b, 0x90a, 0x809, 0x708, 0x607, 0x506,
        0x405, 0x304, 0x303, 0x300, 0x202, 0x201,
    ];
    let mut huff = vec![0u16; 32770];
    huff[0] = 15;
    let mut n = 0usize;
    for &entry in TABLE.iter() {
        for _ in 0..(32768usize >> i32::from(entry >> 8)) {
            n += 1;
            huff[n] = entry;
        }
    }

    let mut bits = ByteBits::default();
    bits.reset();
    let mut sum = 0i32;
    for column in (0..raw_width).rev() {
        let mut row = 0usize;
        while row < raw_height + 1 {
            if row == raw_height {
                row = 1;
            }
            let diff = ljpeg_diff(&mut bits, stream, data_error, &huff)?;
            sum = sum.wrapping_add(diff);
            if sum >> 12 != 0 {
                derror(stream, data_error)?;
            }
            if row < height {
                raw_image[row * pitch + column] = sum as u16;
            }
            row += 2;
        }
    }
    Ok(())
}

/// The six Huffman trees the compressed-Nikon bodies pick between: for the
/// 12- and the 14-bit depth, the lossy group, that group's variant after the
/// split row, and the lossless group. Each row leads with sixteen code
/// lengths' counts (the first one's count first) and then the symbols.
const NIKON_TREES: [[u8; 32]; 6] = [
    // 12-bit, lossy
    [
        0, 1, 5, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0, 5, 4, 3, 6, 2, 7, 1, 0, 8, 9, 11, 10,
        12, 0, 0, 0,
    ],
    // 12-bit, lossy after the split row; the codes pack the bit shift in
    // their high half
    [
        0, 1, 5, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0, 0x39, 0x5a, 0x38, 0x27, 0x16, 5, 4, 3,
        2, 1, 0, 11, 12, 12, 0, 0,
    ],
    // 12-bit, lossless
    [
        0, 1, 4, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 4, 6, 3, 7, 2, 8, 1, 9, 0, 10, 11,
        12, 0, 0, 0,
    ],
    // 14-bit, lossy
    [
        0, 1, 4, 3, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0, 5, 6, 4, 7, 8, 3, 9, 2, 1, 0, 10, 11,
        12, 13, 14, 0,
    ],
    // 14-bit, lossy after the split row
    [
        0, 1, 5, 1, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 8, 0x5c, 0x4b, 0x3a, 0x29, 7, 6, 5, 4,
        3, 2, 1, 0, 13, 14, 0,
    ],
    // 14-bit, lossless
    [
        0, 1, 4, 2, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 7, 6, 8, 5, 9, 4, 10, 3, 11, 12, 2, 0,
        1, 13, 14, 0,
    ],
];

/// `make_decoder`: one tree's counts and symbols expanded into the flat
/// table the bit pump reads — entry 0 leads with the deepest code's length
/// and every later slot repeats a symbol once per code of its length, so a
/// read of that many bits lands straight on its symbol.
fn make_decoder(tree: &[u8]) -> Vec<u16> {
    let mut max_len = 16usize;
    while max_len > 0 && tree[max_len - 1] == 0 {
        max_len -= 1;
    }
    let table_size = 1usize << max_len;
    let mut huff = vec![0u16; table_size + 1];
    huff[0] = max_len as u16;
    let mut slot = 1usize;
    let mut symbol_index = 16usize;
    for code_len in 1..=max_len {
        for _ in 0..tree[code_len - 1] {
            let symbol = u16::from(tree[symbol_index]);
            symbol_index += 1;
            for _ in 0..(1usize << (max_len - code_len)) {
                if slot <= table_size {
                    huff[slot] = (code_len as u16) << 8 | symbol;
                    slot += 1;
                }
            }
        }
    }
    huff
}

/// The two JPEG trees the CRW body picks between by its decoder-table
/// record: three DC trees lead the rows and three AC trees follow, one
/// pair per compression mark.
const CANON_FIRST_TREE: [[u8; 29]; 3] = [
    [0, 1, 4, 2, 3, 1, 2, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 4, 3, 5, 6, 2, 7, 1, 8,
        9, 0, 10, 11, 255,],
    [0, 2, 2, 3, 1, 1, 1, 1, 2, 0, 0, 0,
        0, 0, 0, 0, 3, 2, 4, 1, 5, 0, 6, 7,
        9, 8, 10, 11, 255,],
    [0, 0, 6, 3, 1, 1, 2, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 6, 5, 7, 4, 8, 3, 9, 2,
        0, 10, 1, 11, 255,],
];

const CANON_SECOND_TREE: [[u8; 180]; 3] = [
    [0, 2, 2, 2, 1, 4, 2, 1, 2, 5, 1, 1,
        0, 0, 0, 139, 3, 4, 2, 5, 1, 6, 7, 8,
        18, 19, 17, 20, 9, 21, 34, 0, 33, 22, 10, 240,
        35, 23, 36, 49, 50, 24, 25, 51, 37, 65, 52, 66,
        53, 81, 54, 55, 56, 41, 121, 38, 26, 57, 86, 87,
        40, 39, 82, 85, 88, 67, 118, 89, 119, 84, 97, 249,
        113, 120, 117, 150, 151, 73, 183, 83, 215, 116, 182, 152,
        71, 72, 149, 105, 153, 145, 250, 184, 104, 181, 185, 214,
        247, 216, 103, 70, 69, 148, 137, 248, 129, 213, 246, 180,
        136, 177, 42, 68, 114, 217, 135, 102, 212, 245, 58, 167,
        115, 169, 168, 134, 98, 199, 101, 200, 201, 161, 244, 209,
        233, 90, 146, 133, 166, 231, 147, 232, 193, 198, 122, 100,
        225, 74, 106, 230, 179, 241, 211, 165, 138, 178, 154, 186,
        132, 164, 99, 229, 197, 243, 210, 196, 130, 170, 218, 228,
        242, 202, 131, 163, 162, 195, 234, 194, 226, 227, 255, 255,],
    [0, 2, 2, 1, 4, 1, 4, 1, 3, 3, 1, 0,
        0, 0, 0, 140, 2, 3, 1, 4, 5, 18, 17, 6,
        19, 7, 8, 20, 34, 9, 33, 0, 35, 21, 49, 50,
        10, 22, 240, 36, 51, 65, 66, 25, 23, 37, 24, 81,
        52, 67, 82, 41, 53, 97, 57, 113, 98, 54, 83, 38,
        56, 26, 55, 129, 39, 145, 121, 85, 69, 40, 114, 89,
        161, 177, 68, 105, 84, 88, 209, 250, 87, 225, 241, 185,
        73, 71, 99, 106, 249, 86, 70, 168, 42, 74, 120, 153,
        58, 117, 116, 134, 101, 193, 118, 182, 150, 214, 137, 133,
        201, 245, 149, 180, 199, 247, 138, 151, 184, 115, 183, 216,
        217, 135, 167, 122, 72, 130, 132, 234, 244, 166, 197, 90,
        148, 164, 198, 146, 195, 104, 181, 200, 228, 229, 230, 233,
        162, 163, 227, 194, 102, 103, 147, 170, 212, 213, 231, 248,
        136, 154, 215, 119, 196, 100, 226, 152, 165, 202, 218, 232,
        243, 246, 169, 178, 179, 242, 210, 131, 186, 211, 255, 255,],
    [0, 0, 6, 2, 1, 3, 3, 2, 5, 1, 2, 2,
        8, 10, 0, 117, 4, 5, 3, 6, 2, 7, 1, 8,
        9, 18, 19, 20, 17, 21, 10, 22, 23, 240, 0, 34,
        33, 24, 35, 25, 36, 50, 49, 37, 51, 56, 55, 52,
        53, 54, 57, 121, 87, 88, 89, 40, 86, 120, 39, 65,
        41, 119, 38, 66, 118, 153, 26, 85, 152, 151, 249, 72,
        84, 150, 137, 71, 183, 73, 250, 117, 104, 182, 103, 105,
        185, 184, 216, 82, 215, 136, 181, 116, 81, 70, 217, 248,
        58, 214, 135, 69, 122, 149, 213, 246, 134, 180, 169, 148,
        83, 42, 168, 67, 245, 247, 212, 102, 167, 90, 68, 138,
        201, 232, 200, 231, 154, 106, 115, 74, 97, 199, 244, 198,
        101, 233, 114, 230, 113, 145, 147, 166, 218, 146, 133, 98,
        243, 197, 178, 164, 132, 186, 100, 165, 179, 210, 129, 229,
        211, 170, 196, 202, 242, 177, 228, 209, 131, 99, 234, 195,
        226, 130, 241, 163, 194, 161, 193, 227, 162, 225, 255, 255,],
];
/// `canon_has_lowbits`: a probe of the file head for the compressed rows'
/// low-order bits. A 0xff in these rows is always stuffed with a zero, so
/// any 0xff found with a follower marks the with-lows layout, and a probe
/// through the whole window finding only pairs (or no 0xff at all) keeps
/// the engine's head answer.
fn canon_has_lowbits(data: &[u8]) -> i32 {
    let read_length = data.len().min(0x4000);
    let mut heads = 1i32;
    for index in 540..read_length.saturating_sub(1) {
        if data[index] == 0xff {
            if data[index + 1] != 0 {
                return 1;
            }
            heads = 0;
        }
    }
    heads
}

/// `canon_load_raw`: the CRW rows in blocks of eight. Each block reads one
/// direct-current leaf and then run-length leaves through the body's pair
/// of JPEG trees, the direct-current carry passes between blocks, the left
/// pixel and the pixel above seed the odd and even cells, and when the
/// file's rows keep their low-order bits a second pass pastes them back two
/// bits to a pixel from the low-order section.
fn canon_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
    lowbits: bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;

    let table = (meta.tiff_compress as usize).min(2);
    let first_huff = make_decoder(&CANON_FIRST_TREE[table]);
    let second_huff = make_decoder(&CANON_SECOND_TREE[table]);

    let low_bits_offset = i64::from(lowbits)
        * i64::from(raw_height as i32)
        * i64::from(raw_width as i32)
        / 4;
    stream.seek((540 + low_bits_offset) as u64);
    let mut bits = ByteBits::default();
    bits.zero_after_ff = true;
    bits.reset();
    let mut base = [512i32, 512];
    let mut carry = 0i32;
    let mut pixel_count = 0usize;
    let mut row = 0usize;
    while row < raw_height {
        let pixel = row * raw_width;
        let rows_here = (raw_height - row).min(8);
        let nblocks = rows_here * raw_width >> 6;
        for block in 0..nblocks {
            let mut diffbuf = [0i32; 64];
            let mut index = 0usize;
            while index < 64 {
                let huff = if index == 0 { &first_huff } else { &second_huff };
                let leaf = bits.bits(stream, data_error, i32::from(huff[0]), Some(&huff[1..]))? as i32;
                if leaf == 0 && index != 0 {
                    break;
                }
                if leaf == 0xff {
                    index += 1;
                    continue;
                }
                index += i32::from(leaf >> 4) as usize;
                let len = leaf & 15;
                if len == 0 {
                    index += 1;
                    continue;
                }
                let mut diff = bits.bits(stream, data_error, len, None)? as i32;
                if diff & (1i32 << (len - 1)) == 0 {
                    diff -= (1i32 << len) - 1;
                }
                if index < 64 {
                    diffbuf[index] = diff;
                }
                index += 1;
            }
            diffbuf[0] += carry;
            carry = diffbuf[0];
            for cell in 0..64usize {
                if pixel_count % raw_width == 0 {
                    base[0] = 512;
                    base[1] = 512;
                }
                pixel_count += 1;
                base[cell & 1] = base[cell & 1].wrapping_add(diffbuf[cell]);
                let stored = base[cell & 1] as u16;
                raw_image[pixel + (block << 6) + cell] = stored;
                if i32::from(stored) >> 10 != 0 {
                    derror(stream, data_error)?;
                }
            }
        }
        if lowbits {
            let save = stream.tell();
            stream.seek((26 + (row * raw_width / 4) as i64) as u64);
            let mut prow = pixel;
            for _ in 0..raw_width * 2 {
                let byte = stream.getc().map_or(-1i32, i32::from);
                for r in [0i32, 2, 4, 6] {
                    let low_bits = (byte >> r) & 3;
                    let mut val = ((raw_image[prow] as i32) << 2) + low_bits;
                    if raw_width == 2672 && val < 512 {
                        val += 2;
                    }
                    raw_image[prow] = val as u16;
                    prow += 1;
                }
            }
            stream.seek(save);
        }
        row += 8;
    }
    Ok(())
}

/// `lossless_jpeg_load_raw`: the CR2 rows, one lossless-JPEG scan with the
/// file's own Huffman tables, written pixel by pixel; when the body used
/// cr2 slices the scan's column grid restarts per slice, and a body of the
/// 3984-pixel wide class shifts its rows two columns left.
fn lossless_jpeg_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let height = meta.height as i32;
    let curve = &meta.curve;

    let mut bits = BitReader::default();
    let Some(mut head) = ljpeg_start(stream, &mut bits, 0, false) else {
        return Ok(());
    };
    if head.wide < 1 || head.high < 1 || head.clrs < 1 || head.bits < 1 {
        bail!("the camera raw's lossless JPEG is corrupt");
    }
    if meta.cr2_slice[0] != 0 && meta.cr2_slice[1] == 0 {
        bail!("the camera raw's lossless JPEG is corrupt");
    }
    let jwide = head.wide * head.clrs;
    let mut row = 0i32;
    let mut col = 0i32;
    for jrow in 0..head.high {
        let row_start = ljpeg_row(stream, &mut bits, &mut head, jrow, 0);
        if meta.load_flags & 1 != 0 {
            row = if jrow & 1 != 0 { height - 1 - jrow / 2 } else { jrow / 2 };
        }
        for jcol in 0..jwide {
            let value = curve[head.rows[row_start + jcol as usize] as usize];
            if meta.cr2_slice[0] != 0 {
                let slice_width = meta.cr2_slice[1] as i32;
                let slice_height = meta.cr2_slice[1] as i32 * meta.raw_height as i32;
                let mut jidx = jrow.wrapping_mul(jwide).wrapping_add(jcol);
                let mut slice = jidx / slice_height;
                let stacked = slice >= meta.cr2_slice[0] as i32;
                if stacked {
                    slice = meta.cr2_slice[0] as i32;
                }
                if meta.cr2_slice[1 + usize::from(stacked)] == 0 {
                    bail!("the camera raw's lossless JPEG is corrupt");
                }
                jidx -= slice * slice_height;
                row = jidx / meta.cr2_slice[1 + usize::from(stacked)] as i32;
                col = jidx % meta.cr2_slice[1 + usize::from(stacked)] as i32 + slice * slice_width;
            }
            if raw_width == 3984 && {
                col -= 2;
                col < 0
            } {
                row -= 1;
                col += raw_width as i32;
            }
            if row > meta.raw_height as i32 {
                bail!("the camera raw's lossless JPEG is corrupt");
            }
            if (row as u32) < meta.raw_height {
                raw_image[row as usize * pitch + col as usize] = value;
            }
            col += 1;
            if col >= raw_width as i32 {
                row += 1;
                col = 0;
            }
        }
    }
    Ok(())
}

/// The engine's sRAW body ids whose chroma records come pre-scaled by
/// their superpixel's factor (EOS 5D Mark II, 7D, 50D, 1D Mark IV, 60D);
/// the loader's RGB pass and the older bodies' 512 trim branch on them.
const CANON_SRAW_SHIFTED_CHROMA_IDS: [u64; 5] = [
    0x8000_0218, // EOS 5D Mark II
    0x8000_0250, // EOS 7D
    0x8000_0261, // EOS 50D
    0x8000_0281, // EOS 1D Mark IV
    0x8000_0287, // EOS 60D
];

/// `canon_sraw_load_raw`: the sRAW bodies' lossless JPEG of superpixels.
/// Each decoded row holds one wide line's luma pairs and chroma; the slice
/// plan splits the scan into column bands, the horizontal pass spreads
/// every row's chroma over its pixel pair, and the RGB pass folds luma and
/// chroma into colour through the body's multipliers. The engine's wide
/// mutable `short` reads re-sign-extend every stored sample, so each
/// access reinterprets the cell as `i16`, and the engine's `int` stores
/// keep only the low 16 bits.
fn canon_sraw_load_raw(meta: &mut CameraMeta, image: &mut [[u16; 4]], stream: &mut Stream) -> Result<()> {
    let saved_width = meta.width;
    let saved_height = meta.height;
    let mut bits = BitReader::default();
    let Some(mut head) = ljpeg_start(stream, &mut bits, 0, false) else {
        return Ok(());
    };
    if head.clrs < 4 {
        return Ok(());
    }
    head.wide >>= 1;
    let jwide = head.wide * head.clrs;
    if jwide < 32 || jwide > 65535 {
        bail!("the camera raw's sRAW scan is corrupt");
    }
    if meta.load_flags & 256 != 0 {
        meta.width = meta.raw_width;
        meta.height = meta.raw_height;
    }
    let width = meta.width as i32;
    let height = meta.height as i32;
    let raw_width = meta.raw_width as i32;
    let stride = width as usize;

    // One scan position lives past every slice and row: `jcol` wraps at the
    // decoded row's width and starts a new `ljpeg_row` there, so a slice's
    // column band keeps walking the scan stream the engine ordered it in.
    let mut jrow = 0i32;
    let mut jcol = 0i32;
    let mut slice_end = 0i32;
    let mut row_start = 0usize;
    for _ in 0..=meta.cr2_slice[0] {
        let slice_start = slice_end;
        slice_end += meta.cr2_slice[1] as i32 * 2 / head.clrs;
        // A zero slice count means the whole frame is one band, and a band
        // that reaches past the frame ends at the frame's even edge.
        if meta.cr2_slice[0] == 0 || slice_end > raw_width - 1 {
            slice_end = raw_width & -2;
        }
        let mut row = 0i32;
        while row < height {
            let pixel_row = row as usize * stride;
            let mut col = slice_start;
            while col < slice_end {
                let wrapped = jcol % jwide;
                if wrapped == 0 {
                    row_start = ljpeg_row(stream, &mut bits, &mut head, jrow, 0);
                    jrow += 1;
                }
                jcol = wrapped + head.clrs;
                if col >= width {
                    col += 2;
                    continue;
                }
                let pixel = pixel_row + col as usize;
                for c in 0..(head.clrs - 2) as usize {
                    // The luma samples lay out as a two-wide, one- or
                    // two-row tall run inside the superpixel.
                    image[pixel + (c >> 1) * stride + (c & 1)][0] = head.rows[row_start + (wrapped as usize + c)];
                }
                // The chroma offsets sit under a fixed mid-scale pedestal.
                image[pixel][1] = (i32::from(head.rows[row_start + (wrapped + head.clrs - 2) as usize] as i16) - 16384) as u16;
                image[pixel][2] = (i32::from(head.rows[row_start + (wrapped + head.clrs - 1) as usize] as i16) - 16384) as u16;
                col += 2;
            }
            row += (head.clrs >> 1) - 1;
        }
    }

    // The interpolation spreads each row's chroma over its pixel pair and,
    // when the superpixel carries two chroma rows, odd rows over their
    // neighbours; every neighbour read re-signs the stored sample.
    for row in 0..height {
        let pixel_row = row as usize * stride;
        if row & (head.sraw >> 1) != 0 {
            let mut col = 0i32;
            while col < width {
                let pixel = pixel_row + col as usize;
                for channel in 1..3usize {
                    if row == height - 1 {
                        image[pixel][channel] = image[pixel - stride][channel];
                    } else {
                        let sum = i32::from(image[pixel - stride][channel] as i16)
                            + i32::from(image[pixel + stride][channel] as i16)
                            + 1;
                        image[pixel][channel] = (sum >> 1) as u16;
                    }
                }
                col += 2;
            }
        }
        let mut col = 1i32;
        while col < width {
            let pixel = pixel_row + col as usize;
            for channel in 1..3usize {
                if col == width - 1 {
                    image[pixel][channel] = image[pixel - 1][channel];
                } else {
                    let sum = i32::from(image[pixel - 1][channel] as i16) + i32::from(image[pixel + 1][channel] as i16) + 1;
                    image[pixel][channel] = (sum >> 1) as u16;
                }
            }
            col += 2;
        }
    }

    // The hue shift depends on the body and, for the 5D Mark II, on its
    // firmware version; the RGB pass stores the shifted chroma back into
    // the pixel before folding colour out of it.
    let shifted_chroma = CANON_SRAW_SHIFTED_CHROMA_IDS.contains(&meta.canon_unique_id);
    let mut hue = (head.sraw + 1) << 2;
    if meta.canon_unique_id >= 0x8000_0281
        || (meta.canon_unique_id == 0x8000_0218 && sraw_firmware_version(&meta.model2) > 1_000_006)
    {
        hue = head.sraw << 1;
    }
    let pixel_count = height as usize * stride;
    for pixel_index in 0..pixel_count {
        let pixel = &mut image[pixel_index];
        let mut luma = i32::from(pixel[0] as i16);
        let mut chroma_a = i32::from(pixel[1] as i16);
        let mut chroma_b = i32::from(pixel[2] as i16);

        let (red, green, blue) = if shifted_chroma {
            let shifted_a = ((chroma_a << 2) + hue) as u16;
            let shifted_b = ((chroma_b << 2) + hue) as u16;
            pixel[1] = shifted_a;
            pixel[2] = shifted_b;
            chroma_a = i32::from(shifted_a as i16);
            chroma_b = i32::from(shifted_b as i16);
            (
                luma + ((50 * chroma_a + 22929 * chroma_b) >> 14),
                luma + ((-5640 * chroma_a - 11751 * chroma_b) >> 14),
                luma + ((29040 * chroma_a - 101 * chroma_b) >> 14),
            )
        } else {
            if meta.canon_unique_id < 0x8000_0218 {
                // The pre-5D bodies' luma sits one half step too high.
                luma = i32::from(((luma - 512) as u16) as i16);
                pixel[0] = luma as u16;
            }
            (
                luma + chroma_b,
                luma + ((-778 * chroma_a - (chroma_b << 11)) >> 12),
                luma + chroma_a,
            )
        };

        let values = [red, green, blue];
        for (channel, value) in values.iter().enumerate() {
            let scaled = value.wrapping_mul(i32::from(meta.sraw_mul[channel])) >> 10;
            pixel[channel] = scaled.clamp(0, 32767) as u16;
        }
    }
    meta.width = saved_width;
    meta.height = saved_height;
    meta.maximum = 0x3fff;
    Ok(())
}

/// The sRAW body's firmware sniff: the engine skips to the first digit of
/// the makernote string and `sscanf("%d.%d.%d")`s the version out of it —
/// a field whose digits are missing, or whose dot separator fails to
/// match, leaves the rest of the fields at zero.
fn sraw_firmware_version(model2: &str) -> i32 {
    let Some((digits_start, _)) = model2.char_indices().find(|&(_, character)| character.is_ascii_digit()) else {
        return 0;
    };
    let mut fields = [0i32; 3];
    let mut cursor = &model2[digits_start..];
    for field in &mut fields {
        cursor = cursor.trim_start();
        let mut negative = false;
        if let Some(rest) = cursor.strip_prefix('-') {
            cursor = rest;
            negative = true;
        } else if let Some(rest) = cursor.strip_prefix('+') {
            cursor = rest;
        }
        let digits_end = cursor
            .bytes()
            .position(|byte| !byte.is_ascii_digit())
            .unwrap_or(cursor.len());
        if digits_end == 0 {
            break;
        }
        let value = cursor[..digits_end]
            .bytes()
            .fold(0i32, |value, byte| value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0')));
        *field = if negative { -value } else { value };
        match cursor[digits_end..].strip_prefix('.') {
            Some(rest) => cursor = rest,
            None => break,
        }
    }
    (fields[0].wrapping_mul(1000).wrapping_add(fields[1])).wrapping_mul(1000).wrapping_add(fields[2])
}

/// `nikon_load_raw`: the 12- and 14-bit compressed NEF bodies whose
/// compression record is none of the packed variants. Each pixel reads one
/// code's difference against the pixel to its left and the row pair's own,
/// folded through the tone curve the compression record supplies at open,
/// and a body that split its frame switches to that group's second tree at
/// the split row and raises the pixel ceiling past it.
fn nikon_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let height = meta.height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let curve = &meta.curve;

    stream.seek(meta.meta_offset.max(0) as u64);
    let ver0 = stream.getc().map_or(-1i32, |byte| i32::from(byte));
    let ver1 = stream.getc().map_or(-1i32, |byte| i32::from(byte));
    if ver0 == 0x49 || ver1 == 0x58 {
        stream.skip(2110);
    }
    let mut tree = 0usize;
    if ver0 == 0x46 {
        tree = 2;
    }
    if meta.tiff_bps == 14 {
        tree += 3;
    }
    let mut vpred = [0u16; 4];
    let before = stream.tell();
    stream.read_shorts(&mut vpred);
    if stream.tell() - before < 8 {
        derror(stream, data_error)?;
    }
    let mut vpred = [[vpred[0], vpred[1]], [vpred[2], vpred[3]]];
    let mut max = 1i32.wrapping_shl(meta.tiff_bps as u32) & 0x7fff;
    let mut split = 0i32;
    if ver0 == 0x44 && (ver1 == 0x20 || ver1 == 0x40) {
        if ver1 == 0x40 {
            max /= 4;
        }
        stream.seek((meta.meta_offset + 562).max(0) as u64);
        split = i32::from(stream.get2());
    }
    while max > 2 && curve[(max - 2) as usize] == curve[(max - 1) as usize] {
        max -= 1;
    }

    let mut huff = make_decoder(&NIKON_TREES[tree]);
    stream.seek(meta.data_offset.max(0) as u64);
    let mut bits = ByteBits::default();
    bits.reset();
    let mut hpred = [0u16; 2];
    let mut min = 0i32;
    for row in 0..height {
        if split != 0 && row as i32 == split {
            huff = make_decoder(&NIKON_TREES[tree + 1]);
            min = 16;
            max += min << 1;
        }
        for column in 0..raw_width {
            let code = bits.bits(stream, data_error, i32::from(huff[0]), Some(&huff[1..]))? as i32;
            let len = code & 15;
            let shl = code >> 4;
            let low_bits = bits.bits(stream, data_error, len - shl, None)?;
            let mut diff = (((low_bits << 1) + 1) << (shl as u32) >> 1) as i32;
            if len > 0 && diff & (1i32 << (len - 1)) == 0 {
                diff -= (1i32 << len) - i32::from(shl == 0);
            }
            if column < 2 {
                vpred[row & 1][column] = vpred[row & 1][column].wrapping_add(diff as u16);
                hpred[column] = vpred[row & 1][column];
            } else {
                hpred[column & 1] = hpred[column & 1].wrapping_add(diff as u16);
            }
            let checked = hpred[column & 1].wrapping_add(min as u16);
            if i32::from(checked) >= max {
                derror(stream, data_error)?;
            }
            let value = (hpred[column & 1] as i16).clamp(0, 0x3fff) as usize;
            raw_image[row * pitch + column] = curve[value];
        }
    }
    Ok(())
}

// ===== CRX (CR3) ============================================================

/// Canon's CR3 wavelet codec: the raw track's data is a run of Golomb-coded
/// wavelet subbands under one image header record, decoded line by line
/// through a 2-D inverse 5/3 transform into the Bayer store. The loader
/// mirrors `crxLoadRaw` and its helpers; the tile and subband plan comes
/// from the header the track selection copied into `meta.crx`.
const CRX_BUF_SIZE: usize = 0x10000;

/// The wavelet tiling's neighbour flags, as the engine's `TileFlags`.
const CRX_TILES_ON_THE_RIGHT: u8 = 1;
const CRX_TILES_ON_THE_LEFT: u8 = 2;
const CRX_TILES_ON_THE_BOTTOM: u8 = 4;
const CRX_TILES_ON_THE_TOP: u8 = 8;

/// `exCoefNumTbl`: the extra subband coefficients per row/column when a
/// tile neighbours another tile, indexed by level block, edge pick and the
/// tile width or height's remainder of eight.
const EX_COEF_NUM_TBL: [i32; 144] = [
    1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1,
    0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1,
    0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 1,
    1, 1, 1, 2, 2, 1, 1, 1, 1, 2, 2, 1, 1, 0, 1, 1, 1, 1, 1, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

const Q_STEP_TBL: [u32; 8] = [0x28, 0x2D, 0x33, 0x39, 0x40, 0x48, 0, 0];

/// The run-length growth ramp and its trailing bit counts.
const CRX_JS: [u32; 32] = [
    1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 8, 8, 8, 8, 0x10, 0x10, 0x20, 0x20, 0x40, 0x40, 0x80, 0x80, 0x100, 0x200,
    0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000,
];

const CRX_J: [u32; 32] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];

/// The extra sample precision bit per encoding type, indexed 4*enc + 2.
const INCR_BIT_TABLE: [i32; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0];

/// The subband header marks: the wide form carries per-band quantization.
const CRX_SUBBAND_HEADER_OLD: u32 = 0xFF03;

const CRX_SUBBAND_HEADER_NEW: u32 = 0xFF13;

/// `CrxBitstream`: the byte-window reader a subband's compressed band data
/// streams through — at most 64k of the file held at once, consumed
/// MSB-first. The band's readable span is `mdat_size` bytes; the window
/// refills from the file once the cursor reaches it.
struct CrxBitstream<'a> {
    data: &'a [u8],
    /// `mdatSize`: the band data bytes this stream may still read.
    mdat_size: i64,
    /// `curBufOffset`: the absolute file position of the window's first byte.
    cur_buf_offset: i64,
    /// `curPos`: the read cursor inside the window.
    cur_pos: usize,
    /// `curBufSize`: the bytes the last refill brought in.
    cur_buf_size: usize,
    /// `bitData`: the not-yet-consumed bits, packed against the top of the
    /// word; `bitsLeft` counts the valid low bits.
    bit_data: u32,
    bits_left: i32,
}

impl<'a> CrxBitstream<'a> {
    fn new(data: &'a [u8], mdat_size: i64, cur_buf_offset: i64) -> CrxBitstream<'a> {
        CrxBitstream {
            data,
            mdat_size,
            cur_buf_offset,
            cur_pos: 0,
            cur_buf_size: 0,
            bit_data: 0,
            bits_left: 0,
        }
    }

    /// One byte out of the window.
    fn window_byte(&self, at: usize) -> u8 {
        let absolute = (self.cur_buf_offset.max(0) as usize).checked_add(at);
        match absolute.and_then(|offset| self.data.get(offset..offset + 1)) {
            Some(window) => window[0],
            None => 0,
        }
    }

    /// One big-endian word out of the window.
    fn window_word(&self, at: usize) -> u32 {
        let absolute = (self.cur_buf_offset.max(0) as usize).checked_add(at);
        let word = match absolute.and_then(|offset| self.data.get(offset..offset + 4)) {
            Some(window) => u32::from(window[0]) << 24 | u32::from(window[1]) << 16 | u32::from(window[2]) << 8 | u32::from(window[3]),
            None => 0,
        };
        word
    }

    /// `crxFillBuffer`: refill the window from the file when the cursor ran
    /// out and band data remains. The read is capped by what is left of the
    /// band and by the window size.
    fn fill_buffer(&mut self) -> Result<()> {
        if self.cur_pos >= self.cur_buf_size && self.mdat_size != 0 {
            self.cur_pos = 0;
            self.cur_buf_offset += self.cur_buf_size as i64;
            let start = self.cur_buf_offset.max(0) as usize;
            let wanted = self.mdat_size.min(CRX_BUF_SIZE as i64) as usize;
            let remaining = self.data.len().saturating_sub(start);
            self.cur_buf_size = wanted.min(remaining);
            if self.cur_buf_size < 1 {
                bail!("the CRX band data ends before the image is decoded");
            }
            self.mdat_size -= self.cur_buf_size as i64;
        }
        Ok(())
    }

    /// `crxBitstreamGetZeros`: the count of zero bits before the next set
    /// bit. Once a word's bits are partly consumed the count comes from the
    /// held pipeline; otherwise whole words of nothing are skipped. The
    /// scan's tail keeps the found bit's low bits as the new pipeline.
    fn get_zeros(&mut self) -> Result<u32> {
        if self.bit_data != 0 {
            let index = 31 - self.bit_data.leading_zeros() as i32;
            self.bit_data = self.bit_data.wrapping_shl((32 - index) as u32);
            self.bits_left -= 32 - index;
            return Ok((31 - index) as u32);
        }
        let mut bits_left: i32 = self.bits_left;
        let mut next_data: u32 = 0;
        loop {
            while self.cur_pos + 4 <= self.cur_buf_size {
                next_data = self.window_word(self.cur_pos);
                self.cur_pos += 4;
                self.fill_buffer()?;
                if next_data != 0 {
                    let index = 31 - next_data.leading_zeros() as i32;
                    let result = bits_left + 31 - index;
                    // The original keeps the scanned word in a 64-bit local
                    // and masks after the shift, so a set bit at the word's
                    // low end (index 0) leaves an empty pipeline rather than
                    // wrapping the shift to an identity.
                    self.bit_data = ((u64::from(next_data) << (32 - index)) & 0xffff_ffff) as u32;
                    self.bits_left = index;
                    return Ok(result as u32);
                }
                bits_left += 32;
            }
            if self.cur_buf_size < self.cur_pos + 1 {
                break;
            }
            next_data = u32::from(self.window_byte(self.cur_pos));
            self.cur_pos += 1;
            self.fill_buffer()?;
            if next_data != 0 {
                break;
            }
            bits_left += 8;
        }
        // A zero `nextData` means the scan hit the end of the window with no
        // set bit; the word-scan intrinsic is undefined there, and the
        // stand-in index keeps the result deterministic.
        let index = if next_data != 0 { 31 - next_data.leading_zeros() as i32 } else { 0 };
        let result = bits_left + 7 - index;
        self.bit_data = ((u64::from(next_data) << (32 - index)) & 0xffff_ffff) as u32;
        self.bits_left = index;
        Ok(result as u32)
    }

    /// `crxBitstreamGetBits`: the next `bits` bits. Words are preferred and
    /// the byte-at-a-time form only fills a short tail; the leftover bits
    /// stay in the pipeline.
    fn get_bits(&mut self, bits: i32) -> Result<u32> {
        let mut bits_left: i32 = self.bits_left;
        let mut bit_data: u32 = self.bit_data;
        if bits_left < bits {
            if self.cur_pos + 4 <= self.cur_buf_size {
                let next_word = self.window_word(self.cur_pos);
                self.cur_pos += 4;
                self.fill_buffer()?;
                self.bits_left = 32 - (bits - bits_left);
                let result = (next_word.wrapping_shr(bits_left as u32) | bit_data).wrapping_shr((32 - bits) as u32);
                self.bit_data = next_word.wrapping_shl((bits - bits_left) as u32);
                return Ok(result);
            }
            loop {
                if self.cur_pos >= self.cur_buf_size {
                    break;
                }
                bits_left += 8;
                let next_byte = self.window_byte(self.cur_pos);
                self.cur_pos += 1;
                self.fill_buffer()?;
                bit_data |= u32::from(next_byte).wrapping_shl((32 - bits_left) as u32);
                if bits_left >= bits {
                    break;
                }
            }
        }
        let result = bit_data.wrapping_shr((32 - bits) as u32);
        self.bit_data = bit_data.wrapping_shl(bits as u32);
        self.bits_left = bits_left - bits;
        Ok(result)
    }
}

/// `_constrain`: clamp to the range, as the engine's helper does.
fn crx_constrain(value: i32, low: i32, high: i32) -> i32 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

/// The coded symbol's signed value: odd codes come out negative, so the
/// zero bit and the shifted value XOR to the signed magnitude.
fn crx_zigzag(bit_code: u32) -> i32 {
    ((bit_code >> 1) as i32) ^ -((bit_code & 1) as i32)
}

/// `crxPrediction`: the median predictor of the pixel above, left and the
/// diagonal, picking the corner the gradients suggest.
fn crx_prediction(left: i32, top: i32, delta_h: i32, delta_v: i32) -> i32 {
    let median_left = left.wrapping_add(delta_h);
    let symbols = [median_left, median_left, left, top];
    let vertical_sign = i32::from(delta_v < 0) ^ i32::from(delta_h < 0);
    let horizontal_sign = i32::from(left < top) ^ i32::from(delta_h < 0);
    symbols[((vertical_sign << 1) + horizontal_sign) as usize]
}

/// `crxPredictKParameter`: the adaptive Golomb exponent follows the error
/// magnitude, capped when asked.
fn crx_predict_k_parameter(prev_k: i32, bit_code: u32, max_val: i32) -> i32 {
    let one_shifted = 1i32.wrapping_shl(prev_k as u32);
    let bit_shifted = bit_code.wrapping_shr(prev_k as u32);
    let new_k = prev_k
        - i32::from(bit_code < (one_shifted >> 1) as u32)
        + i32::from(bit_shifted > 2)
        + i32::from(bit_shifted > 5);
    if max_val == 0 || new_k < max_val {
        new_k
    } else {
        max_val
    }
}

/// One subband's line decoding state. The band's three rolling line cursors
/// sit in the small per-band area `paramData` (two subband-width lines plus
/// the sentinels); the bands that decode without the previous line also
/// keep the per-column K history `nonProgrData`.
struct CrxBandParam<'a> {
    bit_stream: CrxBitstream<'a>,
    subband_width: i16,
    subband_height: i16,
    rounded_bits_mask: i32,
    rounded_bits: i32,
    cur_line: i16,
    /// The `paramData` area and the `lineBuf0`/`lineBuf1` cursors into it.
    lines: Vec<i32>,
    line0: usize,
    line1: usize,
    /// The `nonProgrData` area and the `lineBuf2` cursor into it.
    lines2: Vec<i32>,
    line2: usize,
    s_param: i32,
    k_param: i32,
    supports_partial: bool,
}

impl<'a> CrxBandParam<'a> {
    fn new(
        data: &'a [u8],
        subband_mdat_offset: i64,
        subband_data_size: i64,
        subband_width: i32,
        subband_height: i32,
        supports_partial: bool,
        rounded_bits_mask: i32,
    ) -> CrxBandParam<'a> {
        // `paramLength = 2 * subbandWidth + 4`: the two line cursors' area;
        // `progrDataSize = subbandWidth` ints once the band drops the
        // previous-line references.
        let param_length = (2 * subband_width + 4) as usize;
        let progr_data_size = if supports_partial { 0 } else { subband_width as usize };
        CrxBandParam {
            bit_stream: CrxBitstream::new(data, subband_data_size, subband_mdat_offset),
            subband_width: subband_width as i16,
            subband_height: subband_height as i16,
            rounded_bits_mask,
            rounded_bits: 0,
            cur_line: 0,
            lines: vec![0; param_length],
            line0: 0,
            line1: 0,
            lines2: vec![0; progr_data_size],
            line2: 0,
            s_param: 0,
            k_param: 0,
            supports_partial,
        }
    }

    /// `crxDecodeSymbolL1`: one predictive symbol; the median prediction
    /// runs when asked, and the follow-up error re-anchors the K estimate
    /// away from the line end.
    fn decode_symbol_l1(&mut self, do_median_prediction: bool, not_eol: bool) -> Result<()> {
        if do_median_prediction {
            let delta = self.lines[self.line0 + 1].wrapping_sub(self.lines[self.line0]);
            let mid = self.lines[self.line1];
            let symb01 = delta.wrapping_add(mid);
            let symb23 = self.lines[self.line0 + 1];
            let index = ((i32::from(self.lines[self.line0] < mid) ^ i32::from(delta < 0)) << 1)
                + (i32::from(mid < self.lines[self.line0 + 1]) ^ i32::from(delta < 0));
            let predicted = if index <= 1 { symb01 } else if index == 2 { mid } else { symb23 };
            self.lines[self.line1 + 1] = predicted;
        } else {
            self.lines[self.line1 + 1] = self.lines[self.line0 + 1];
        }
        // get next error symbol
        let mut bit_code = self.bit_stream.get_zeros()?;
        if bit_code >= 41 {
            bit_code = self.bit_stream.get_bits(21)?;
        } else if self.k_param != 0 {
            bit_code = self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32);
        }
        // add converted (+/-) error code to predicted value
        self.lines[self.line1 + 1] = self.lines[self.line1 + 1].wrapping_add(crx_zigzag(bit_code));
        // for not end of the line - use one symbol ahead to estimate next K
        if not_eol {
            let next_delta = self.lines[self.line0 + 2].wrapping_sub(self.lines[self.line0 + 1]).wrapping_shl(1);
            bit_code = bit_code.wrapping_add(next_delta.wrapping_abs() as u32) >> 1;
            self.line0 += 1;
        }
        // update K parameter
        self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
        self.line1 += 1;
        Ok(())
    }

    /// `crxDecodeSymbolL1Rounded`: the rounded variant folds the error into
    /// quantization steps; the follow-up gradient also re-anchors the K
    /// estimate, from the rounded code this time.
    fn decode_symbol_l1_rounded(&mut self, do_sym: bool, do_code: bool) -> Result<()> {
        let mut sym = self.lines[self.line0 + 1];
        if do_sym {
            // calculate the next symbol gradient
            let delta_h = self.lines[self.line0 + 1].wrapping_sub(self.lines[self.line0]);
            let mid = self.lines[self.line1];
            let symb01 = delta_h.wrapping_add(mid);
            let symb23 = self.lines[self.line0 + 1];
            let index = ((i32::from(self.lines[self.line0] < mid) ^ i32::from(delta_h < 0)) << 1)
                + (i32::from(mid < self.lines[self.line0 + 1]) ^ i32::from(delta_h < 0));
            sym = if index <= 1 { symb01 } else if index == 2 { mid } else { symb23 };
        }
        let mut bit_code = self.bit_stream.get_zeros()?;
        if bit_code >= 41 {
            bit_code = self.bit_stream.get_bits(21)?;
        } else if self.k_param != 0 {
            bit_code = self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32);
        }
        let code = crx_zigzag(bit_code);
        self.lines[self.line1 + 1] = self
            .rounded_bits_mask
            .wrapping_mul(2)
            .wrapping_mul(code)
            .wrapping_add(code >> 31)
            .wrapping_add(sym);
        if do_code {
            let code = if self.lines[self.line0 + 2] > self.lines[self.line0 + 1] {
                (self.lines[self.line0 + 2].wrapping_sub(self.lines[self.line0 + 1]).wrapping_add(self.rounded_bits_mask)
                    - 1)
                    >> self.rounded_bits as u32
            } else {
                -((self.lines[self.line0 + 1].wrapping_sub(self.lines[self.line0 + 2]).wrapping_add(self.rounded_bits_mask))
                    >> self.rounded_bits as u32)
            };
            self.k_param = crx_predict_k_parameter(
                self.k_param,
                bit_code.wrapping_add(2i32.wrapping_mul(code.wrapping_abs()) as u32) >> 1,
                15,
            );
        } else {
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
        }
        self.line1 += 1;
        Ok(())
    }

    /// The common run scan of the plain, top-line and rounded line
    /// decoders: one run start bit, then the JS-ramp growth with the J-bit
    /// tail. The caller applies the count to its own line cursors.
    fn run_scan(&mut self, length: i32) -> Result<i32> {
        let mut n_syms: i32 = 0;
        if self.bit_stream.get_bits(1)? != 0 {
            n_syms = 1;
            while self.bit_stream.get_bits(1)? != 0 {
                n_syms = n_syms.wrapping_add(CRX_JS[self.s_param as usize] as i32);
                if (n_syms as u32) > length as u32 {
                    n_syms = length;
                    break;
                }
                if self.s_param < 31 {
                    self.s_param += 1;
                }
                if (n_syms as u32) == length as u32 {
                    break;
                }
            }
            if (n_syms as u32) < length as u32 {
                if CRX_J[self.s_param as usize] != 0 {
                    n_syms = n_syms.wrapping_add(self.bit_stream.get_bits(CRX_J[self.s_param as usize] as i32)? as i32);
                }
                if self.s_param > 0 {
                    self.s_param -= 1;
                }
                if (n_syms as u32) > length as u32 {
                    bail!("the CRX band's symbols overrun the line");
                }
            }
        }
        Ok(n_syms)
    }

    /// `crxDecodeLine(param)`: the plain line decoder. Runs of the previous
    /// symbol keep the line flat; the median-predicted symbol uses the one
    /// ahead except at the line end.
    fn decode_line_plain(&mut self) -> Result<()> {
        let mut length = self.subband_width as i32;
        self.lines[self.line1] = self.lines[self.line0 + 1];
        while length > 1 {
            if self.lines[self.line1] != self.lines[self.line0 + 1] || self.lines[self.line1] != self.lines[self.line0 + 2]
            {
                self.decode_symbol_l1(true, true)?;
            } else {
                let mut n_syms = self.run_scan(length)?;
                length = length.wrapping_sub(n_syms);
                self.line0 += n_syms as usize;
                while n_syms > 0 {
                    n_syms -= 1;
                    self.lines[self.line1 + 1] = self.lines[self.line1];
                    self.line1 += 1;
                }
                if length > 0 {
                    self.decode_symbol_l1(false, length > 1)?;
                }
            }
            length -= 1;
        }
        if length == 1 {
            self.decode_symbol_l1(true, false)?;
        }
        self.lines[self.line1 + 1] = self.lines[self.line1].wrapping_add(1);
        Ok(())
    }

    /// `crxDecodeLineRounded`: the run path also re-anchors valueReached
    /// from the just-decoded symbol; the decoded value stays within the
    /// rounded quantization step.
    fn decode_line_rounded(&mut self) -> Result<()> {
        let mut value_reached = false;
        self.lines[self.line0] = self.lines[self.line0 + 1];
        self.lines[self.line1] = self.lines[self.line0 + 1];
        let mut length = self.subband_width as i32;
        while length > 1 {
            if self.lines[self.line0 + 2].wrapping_sub(self.lines[self.line0 + 1]).wrapping_abs() > self.rounded_bits_mask
            {
                self.decode_symbol_l1_rounded(true, true)?;
                self.line0 += 1;
                value_reached = true;
            } else if value_reached
                || self.lines[self.line0].wrapping_sub(self.lines[self.line1]).wrapping_abs() > self.rounded_bits_mask
            {
                self.decode_symbol_l1_rounded(true, true)?;
                self.line0 += 1;
                value_reached = false;
            } else {
                let mut n_syms = self.run_scan(length)?;
                length = length.wrapping_sub(n_syms);
                self.line0 += n_syms as usize;
                while n_syms > 0 {
                    n_syms -= 1;
                    self.lines[self.line1 + 1] = self.lines[self.line1];
                    self.line1 += 1;
                }
                if length > 1 {
                    self.decode_symbol_l1_rounded(false, true)?;
                    self.line0 += 1;
                    value_reached = self.lines[self.line0 + 1].wrapping_sub(self.lines[self.line0]).wrapping_abs()
                        > self.rounded_bits_mask;
                } else if length == 1 {
                    self.decode_symbol_l1_rounded(false, false)?;
                }
            }
            length -= 1;
        }
        if length == 1 {
            self.decode_symbol_l1_rounded(true, false)?;
        }
        self.lines[self.line1 + 1] = self.lines[self.line1].wrapping_add(1);
        Ok(())
    }

    /// `crxDecodeLineNoRefPrevLine`: flat lines with no reference above;
    /// the per-column K history chases the encoded K and every symbol's
    /// sign works off the next value.
    fn decode_line_no_ref_prev_line(&mut self) -> Result<()> {
        let width = self.subband_width as i32;
        let mut i: i32 = 0;
        while i < width - 1 {
            if self.lines[self.line0 + (i + 2) as usize] | self.lines[self.line0 + (i + 1) as usize] | self.lines[self.line1 + i as usize] != 0
            {
                let mut bit_code = self.bit_stream.get_zeros()?;
                if bit_code >= 41 {
                    bit_code = self.bit_stream.get_bits(21)?;
                } else if self.k_param != 0 {
                    bit_code = self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32);
                }
                self.lines[self.line1 + (i + 1) as usize] = crx_zigzag(bit_code);
                self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 0);
                self.chase_k(i)?;
                // Stored after both branches in the original (the end-of-line
                // case bypasses it because it writes the slot itself).
                self.lines2[self.line2 + i as usize] = self.k_param;
            } else {
                let mut n_syms: i32 = 0;
                if self.bit_stream.get_bits(1)? != 0 {
                    n_syms = 1;
                    if i != width - 1 {
                        while self.bit_stream.get_bits(1)? != 0 {
                            n_syms = n_syms.wrapping_add(CRX_JS[self.s_param as usize] as i32);
                            if (i.wrapping_add(n_syms) as u32) > width as u32 {
                                n_syms = width.wrapping_sub(i);
                                break;
                            }
                            if self.s_param < 31 {
                                self.s_param += 1;
                            }
                            if (i.wrapping_add(n_syms) as u32) == width as u32 {
                                break;
                            }
                        }
                        if (i.wrapping_add(n_syms) as u32) < width as u32 {
                            if CRX_J[self.s_param as usize] != 0 {
                                n_syms = n_syms
                                    .wrapping_add(self.bit_stream.get_bits(CRX_J[self.s_param as usize] as i32)? as i32);
                            }
                            if self.s_param > 0 {
                                self.s_param -= 1;
                            }
                        }
                        if (i.wrapping_add(n_syms) as u32) > width as u32 {
                            bail!("the CRX band data is out of order");
                        }
                    }
                } else if i > width {
                    bail!("the CRX band data is out of order");
                }

                if n_syms > 0 {
                    let one_start = self.line1 + (i + 1) as usize;
                    self.lines[one_start..one_start + n_syms as usize].fill(0);
                    let two_start = self.line2 + i as usize;
                    self.lines2[two_start..two_start + n_syms as usize].fill(0);
                    i += n_syms;
                }

                if i >= width - 1 {
                    if i == width - 1 {
                        let mut bit_code = self.bit_stream.get_zeros()?;
                        if bit_code >= 41 {
                            bit_code = self.bit_stream.get_bits(21)?;
                        } else if self.k_param != 0 {
                            bit_code =
                                self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32);
                        }
                        self.lines[self.line1 + (i + 1) as usize] = crx_zigzag(bit_code.wrapping_add(1));
                        self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
                        self.lines2[self.line2 + i as usize] = self.k_param;
                    }
                    // The loop's increment runs after the continue.
                    i += 1;
                    continue;
                } else {
                    let mut bit_code = self.bit_stream.get_zeros()?;
                    if bit_code >= 41 {
                        bit_code = self.bit_stream.get_bits(21)?;
                    } else if self.k_param != 0 {
                        bit_code = self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32);
                    }
                    self.lines[self.line1 + (i + 1) as usize] = crx_zigzag(bit_code.wrapping_add(1));
                    self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 0);
                    self.chase_k(i)?;
                }
                self.lines2[self.line2 + i as usize] = self.k_param;
            }
            i += 1;
        }
        if i == width - 1 {
            let bit_code = self.bit_stream.get_zeros()?;
            let bit_code = self.follow_up_bits(bit_code)?;
            self.lines[self.line1 + (i + 1) as usize] = crx_zigzag(bit_code);
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.lines2[self.line2 + i as usize] = self.k_param;
        }
        Ok(())
    }

    /// The K history chase after an uncapped K decode: a stalled history
    /// holds the cap, an outpacing one counts up.
    fn chase_k(&mut self, i: i32) -> Result<()> {
        if self.lines2[self.line2 + (i + 1) as usize].wrapping_sub(self.k_param) <= 1 {
            if self.k_param >= 15 {
                self.k_param = 15;
            }
        } else {
            self.k_param += 1;
        }
        Ok(())
    }

    /// The shared bit-code refinement after a zero-run count.
    fn follow_up_bits(&mut self, bit_code: u32) -> Result<u32> {
        let bit_code = if bit_code >= 41 {
            self.bit_stream.get_bits(21)?
        } else if self.k_param != 0 {
            self.bit_stream.get_bits(self.k_param)? | bit_code.wrapping_shl(self.k_param as u32)
        } else {
            bit_code
        };
        Ok(bit_code)
    }

    /// `crxDecodeTopLine`: the first line has history only from itself
    /// leftward.
    fn decode_top_line(&mut self) -> Result<()> {
        self.lines[self.line1] = 0;
        let mut length = self.subband_width as i32;
        while length > 1 {
            if self.lines[self.line1] != 0 {
                self.lines[self.line1 + 1] = self.lines[self.line1];
            } else {
                let mut n_syms = self.run_scan(length)?;
                length = length.wrapping_sub(n_syms);
                while n_syms > 0 {
                    n_syms -= 1;
                    self.lines[self.line1 + 1] = self.lines[self.line1];
                    self.line1 += 1;
                }
                if length <= 0 {
                    break;
                }
                self.lines[self.line1 + 1] = 0;
            }
            // read the line from bitstream
            let bit_code = self.follow_up_bits_from_zeros()?;
            self.lines[self.line1 + 1] = self.lines[self.line1 + 1].wrapping_add(crx_zigzag(bit_code));
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.line1 += 1;
            length -= 1;
        }
        if length == 1 {
            self.lines[self.line1 + 1] = self.lines[self.line1];
            let bit_code = self.follow_up_bits_from_zeros()?;
            self.lines[self.line1 + 1] = self.lines[self.line1 + 1].wrapping_add(crx_zigzag(bit_code));
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.line1 += 1;
        }
        self.lines[self.line1 + 1] = self.lines[self.line1].wrapping_add(1);
        Ok(())
    }

    /// The top line's rounded form; the symbol appends the quantization
    /// step's even-side rounding instead of the error.
    fn decode_top_line_rounded(&mut self) -> Result<()> {
        self.lines[self.line1] = 0;
        let mut length = self.subband_width as i32;
        while length > 1 {
            if self.lines[self.line1].wrapping_abs() > self.rounded_bits_mask {
                self.lines[self.line1 + 1] = self.lines[self.line1];
            } else {
                let mut n_syms = self.run_scan(length)?;
                length = length.wrapping_sub(n_syms);
                while n_syms > 0 {
                    n_syms -= 1;
                    self.lines[self.line1 + 1] = self.lines[self.line1];
                    self.line1 += 1;
                }
                if length <= 0 {
                    break;
                }
                self.lines[self.line1 + 1] = 0;
            }
            let bit_code = self.follow_up_bits_from_zeros()?;
            let s_value = crx_zigzag(bit_code);
            self.lines[self.line1 + 1] = self.lines[self.line1 + 1].wrapping_add(
                self.rounded_bits_mask
                    .wrapping_mul(2)
                    .wrapping_mul(s_value)
                    .wrapping_add(s_value >> 31),
            );
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.line1 += 1;
            length -= 1;
        }
        if length == 1 {
            let bit_code = self.follow_up_bits_from_zeros()?;
            let s_value = crx_zigzag(bit_code);
            self.lines[self.line1 + 1] = self.lines[self.line1 + 1].wrapping_add(
                self.rounded_bits_mask
                    .wrapping_mul(2)
                    .wrapping_mul(s_value)
                    .wrapping_add(s_value >> 31),
            );
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.line1 += 1;
        }
        self.lines[self.line1 + 1] = self.lines[self.line1].wrapping_add(1);
        Ok(())
    }

    /// `crxDecodeTopLineNoRefPrevLine`: the first line's flat run and the
    /// K history both start from zero.
    fn decode_top_line_no_ref_prev_line(&mut self) -> Result<()> {
        self.lines[self.line0] = 0;
        self.lines[self.line1] = 0;
        let mut length = self.subband_width as i32;
        while length > 1 {
            if self.lines[self.line1] != 0 {
                let bit_code = self.follow_up_bits_from_zeros()?;
                self.lines[self.line1 + 1] = crx_zigzag(bit_code);
                self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            } else {
                let mut n_syms = self.run_scan(length)?;
                length = length.wrapping_sub(n_syms);
                while n_syms > 0 {
                    n_syms -= 1;
                    self.lines2[self.line2] = 0;
                    self.lines[self.line1 + 1] = 0;
                    self.line1 += 1;
                    self.line2 += 1;
                }
                if length <= 0 {
                    break;
                }
                let bit_code = self.follow_up_bits_from_zeros()?;
                self.lines[self.line1 + 1] = crx_zigzag(bit_code.wrapping_add(1));
                self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            }
            self.lines2[self.line2] = self.k_param;
            self.line2 += 1;
            self.line1 += 1;
            length -= 1;
        }
        if length == 1 {
            let bit_code = self.follow_up_bits_from_zeros()?;
            self.lines[self.line1 + 1] = crx_zigzag(bit_code);
            self.k_param = crx_predict_k_parameter(self.k_param, bit_code, 15);
            self.lines2[self.line2] = self.k_param;
            self.line1 += 1;
        }
        self.lines[self.line1 + 1] = 0;
        Ok(())
    }

    /// The top lines' bit-code refinement: the zero-run then the follow-up
    /// bits, uncapped K included.
    fn follow_up_bits_from_zeros(&mut self) -> Result<u32> {
        let bit_code = self.bit_stream.get_zeros()?;
        self.follow_up_bits(bit_code)
    }

    /// `crxDecodeLine(param, bandBuf)`: the dispatcher — set the line
    /// cursors the row's parity requires, decode the line, then copy the
    /// middle of the second cursor's region out as the band row. The copy
    /// source is captured before the decode advances that cursor.
    fn decode_band_line(&mut self, band_buf: &mut [i32]) -> Result<()> {
        if self.cur_line >= self.subband_height {
            bail!("the CRX band decodes more rows than it holds");
        }
        let line_length = self.subband_width as usize + 2;
        if self.cur_line == 0 {
            self.s_param = 0;
            self.k_param = 0;
            if self.supports_partial {
                if self.rounded_bits_mask <= 0 {
                    self.line0 = 0;
                    self.line1 = line_length;
                    let source = self.line1 + 1;
                    self.decode_top_line()?;
                    band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
                    self.cur_line += 1;
                } else {
                    self.rounded_bits = 1;
                    if self.rounded_bits_mask & !1 != 0 {
                        while self.rounded_bits_mask >> self.rounded_bits != 0 {
                            self.rounded_bits += 1;
                        }
                    }
                    self.line0 = 0;
                    self.line1 = line_length;
                    let source = self.line1 + 1;
                    self.decode_top_line_rounded()?;
                    band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
                    self.cur_line += 1;
                }
            } else {
                self.line2 = 0;
                self.line0 = 0;
                self.line1 = line_length;
                let source = self.line1 + 1;
                self.decode_top_line_no_ref_prev_line()?;
                band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
                self.cur_line += 1;
            }
        } else if !self.supports_partial {
            self.line2 = 0;
            if self.cur_line & 1 == 1 {
                self.line1 = 0;
                self.line0 = line_length;
            } else {
                self.line0 = 0;
                self.line1 = line_length;
            }
            let source = self.line1 + 1;
            self.decode_line_no_ref_prev_line()?;
            band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
            self.cur_line += 1;
        } else if self.rounded_bits_mask <= 0 {
            if self.cur_line & 1 == 1 {
                self.line1 = 0;
                self.line0 = line_length;
            } else {
                self.line0 = 0;
                self.line1 = line_length;
            }
            let source = self.line1 + 1;
            self.decode_line_plain()?;
            band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
            self.cur_line += 1;
        } else {
            if self.cur_line & 1 == 1 {
                self.line1 = 0;
                self.line0 = line_length;
            } else {
                self.line0 = 0;
                self.line1 = line_length;
            }
            let source = self.line1 + 1;
            self.decode_line_rounded()?;
            band_buf.copy_from_slice(&self.lines[source..source + self.subband_width as usize]);
            self.cur_line += 1;
        }
        Ok(())
    }
}

/// One wavelet subband: its row region in the component's arena, the
/// streaming decode state once the band data begins, and the quantization
/// facts the per-row application multiplies through.
struct CrxSubband<'a> {
    /// The band's row region as an arena offset in ints.
    band_start: usize,
    mdat_offset: i64,
    width: i32,
    height: i32,
    q_param: i32,
    k_param: i32,
    q_step_base: i32,
    q_step_mult: u32,
    supports_partial: bool,
    /// The row region's length as the engine's `bandSize` carries it.
    band_size: usize,
    data_size: i64,
    data_offset: i64,
    row_start_add_on: i32,
    row_end_add_on: i32,
    col_start_add_on: i32,
    col_end_add_on: i32,
    level_shift: i32,
    /// `bandParam`, created by the subband data setup for bands with data.
    band_param: Option<Box<CrxBandParam<'a>>>,
}

impl CrxSubband<'_> {
    fn new() -> CrxSubband<'static> {
        CrxSubband {
            band_start: 0,
            mdat_offset: 0,
            width: 0,
            height: 0,
            q_param: 0,
            k_param: 0,
            q_step_base: 0,
            q_step_mult: 0,
            supports_partial: false,
            band_size: 0,
            data_size: 0,
            data_offset: 0,
            row_start_add_on: 0,
            row_end_add_on: 0,
            col_start_add_on: 0,
            col_end_add_on: 0,
            level_shift: 0,
            band_param: None,
        }
    }
}

/// One 5/3 inverse wavelet level: the four subband rows it combines, the
/// eight rolling line buffers, and the tap positions.
struct CrxWavelet {
    /// `subband0Buf`: the coarse row feed. The only band pointer that
    /// moves once the transform runs.
    subband0: usize,
    subband1: usize,
    subband2: usize,
    subband3: usize,
    /// The eight line buffers' arena offsets.
    line_buf: [usize; 8],
    cur_line: i16,
    cur_h: i16,
    flt_tap_h: i8,
    height: i16,
    width: i16,
}

impl CrxWavelet {
    fn new() -> CrxWavelet {
        CrxWavelet {
            subband0: 0,
            subband1: 0,
            subband2: 0,
            subband3: 0,
            line_buf: [0; 8],
            cur_line: 0,
            cur_h: 0,
            flt_tap_h: 0,
            height: 0,
            width: 0,
        }
    }
}

/// The level's quantization facts: this level's table slice inside the
/// tile's table plus the table's column count.
struct CrxQStepLevel<'a> {
    table: &'a [u32],
    width: usize,
}

/// One plane component of a tile: the subbands, the wavelet levels and the
/// int32 arena that holds all their rows and line buffers.
struct CrxPlaneComp<'a> {
    arena: Vec<i32>,
    subs: Vec<CrxSubband<'a>>,
    wvlts: Vec<CrxWavelet>,
    comp_number: i32,
    data_offset: i64,
    comp_size: i32,
    supports_partial: bool,
    rounded_bits_mask: i32,
    tile_flag: u8,
}

impl CrxPlaneComp<'_> {
    fn new() -> CrxPlaneComp<'static> {
        CrxPlaneComp {
            arena: Vec::new(),
            subs: Vec::new(),
            wvlts: Vec::new(),
            comp_number: 0,
            data_offset: 0,
            comp_size: 0,
            supports_partial: false,
            rounded_bits_mask: 0,
            tile_flag: 0,
        }
    }
}

/// `crxUpdateQparam`: the band's quantization parameter is Golomb coded on
/// the band's own bitstream ahead of the line data.
fn crx_update_q_param(band: &mut CrxSubband<'_>) -> Result<()> {
    let Some(param) = band.band_param.as_mut() else {
        bail!("the CRX band misses its decode state");
    };
    let zeros = param.bit_stream.get_zeros()?;
    let bit_code = param.follow_up_bits(zeros)?;
    band.q_param = band.q_param.wrapping_add(crx_zigzag(bit_code));
    band.k_param = crx_predict_k_parameter(band.k_param, bit_code, 0);
    if band.k_param > 7 {
        bail!("the CRX band's quantization K parameter runs away");
    }
    Ok(())
}

/// `getSubbandRow`: the quantization row behind the subband row, accounting
/// the addon rows the tile edges contribute.
fn crx_get_subband_row(band: &CrxSubband<'_>, row: i32) -> i32 {
    if row < band.row_start_add_on {
        0
    } else if row < band.height - band.row_end_add_on {
        row - band.row_end_add_on
    } else {
        band.height - band.row_end_add_on - band.row_start_add_on - 1
    }
}

/// `crxDecodeLineWithIQuantization`: one band row, then the per-pixel scale
/// it came encoded with. The new header form carries a per-row table; the
/// old form carries one exponent in the band's quantization parameter.
fn crx_decode_line_with_iquantization(arena: &mut [i32], band: &mut CrxSubband<'_>, q_step: Option<&CrxQStepLevel<'_>>) -> Result<()> {
    if band.data_size == 0 {
        arena[band.band_start..band.band_start + band.band_size].fill(0);
        return Ok(());
    }

    if band.supports_partial && q_step.is_none() {
        crx_update_q_param(band)?;
    }
    {
        let Some(param) = band.band_param.as_mut() else {
            bail!("the CRX band misses its decode state");
        };
        let band_region = &mut arena[band.band_start..];
        param.decode_band_line(&mut band_region[..band.width as usize])?;
    }
    if band.width <= 0 {
        return Ok(());
    }

    // update band buffers
    let band_end = band.band_start + band.width as usize;
    let band_buf = &mut arena[band.band_start..band_end];
    if let Some(q_step) = q_step {
        // new version
        let current_row = band.band_param.as_ref().map_or(0, |param| i32::from(param.cur_line) - 1);
        let row = crx_get_subband_row(band, current_row);
        let row_table_start = q_step.width * row as usize;
        for i in 0..band.col_start_add_on {
            let quant_value = band
                .q_step_base
                .wrapping_add((q_step.table[i as usize].wrapping_mul(band.q_step_mult) >> 3) as i32);
            let index = band.band_start + i as usize;
            arena[index] = arena[index].wrapping_mul(crx_constrain(quant_value, 1, 0x168000));
        }
        for i in band.col_start_add_on..band.width - band.col_end_add_on {
            let cell = (i - band.col_start_add_on).wrapping_shr(band.level_shift as u32) as usize;
            let quant_value = band
                .q_step_base
                .wrapping_add((q_step.table[row_table_start + cell].wrapping_mul(band.q_step_mult) >> 3) as i32);
            let index = band.band_start + i as usize;
            arena[index] = arena[index].wrapping_mul(crx_constrain(quant_value, 1, 0x168000));
        }
        let last_index = (band.width - band.col_end_add_on - band.col_start_add_on - 1).wrapping_shr(band.level_shift as u32)
            as usize;
        for i in band.width - band.col_end_add_on..band.width {
            let quant_value = band
                .q_step_base
                .wrapping_add((q_step.table[row_table_start + last_index].wrapping_mul(band.q_step_mult) >> 3) as i32);
            let index = band.band_start + i as usize;
            arena[index] = arena[index].wrapping_mul(crx_constrain(quant_value, 1, 0x168000));
        }
    } else {
        // prev. version
        let quant_cell = band.q_param.rem_euclid(6) as usize;
        let mut q_scale = (Q_STEP_TBL[quant_cell] as u32).wrapping_shr((6 - band.q_param / 6) as u32) as i32;
        if band.q_param / 6 >= 6 {
            q_scale = (Q_STEP_TBL[quant_cell] as u32 as i32).wrapping_mul(1i32.wrapping_shl((band.q_param / 6 + 26) as u32));
        }

        if q_scale != 1 {
            for index in 0..band.width as usize {
                band_buf[index] = band_buf[index].wrapping_mul(q_scale);
            }
        }
    }
    Ok(())
}

/// The subband setup's per-band row region and the streaming band state:
/// the band data streams straight from the file, one 64k window at a time.
fn crx_setup_band_param<'a>(
    data: &'a [u8],
    subband_mdat_offset: i64,
    subband_data_size: i64,
    subband_width: i32,
    subband_height: i32,
    supports_partial: bool,
    rounded_bits_mask: i32,
) -> Result<Box<CrxBandParam<'a>>> {
    let mut param = Box::new(CrxBandParam::new(
        data,
        subband_mdat_offset,
        subband_data_size,
        subband_width,
        subband_height,
        supports_partial,
        rounded_bits_mask,
    ));
    // The engine's param init fills the first window and ignores the
    // result; the refill throws when the band data is absent.
    param.bit_stream.fill_buffer()?;
    Ok(param)
}

/// One quantization lookup for a wavelet level: the level's slice of the
/// tile's shared table and the table's column count.
struct CrxQStepRef {
    /// The level's first cell inside the tile's table.
    table_start: usize,
    width: usize,
}

/// One tile: its per-plane components and, for the version 0x200 header
/// form, the per-level quantization tables the band rows scale with.
struct CrxTile<'a> {
    tile_flag: u8,
    comps: Vec<CrxPlaneComp<'a>>,
    /// Only the header walk's own accounting (`tileSize`).
    tile_size: u32,
    /// `dataOffset`: where the tile's records begin inside the mdat.
    data_offset: i64,
    width: u16,
    height: u16,
    /// `mdatQPDataSize`: the quantization stream's length the extended
    /// tile header counts.
    mdat_q_p_data_size: u32,
    /// `mdatExtraSize`: the records between the quant stream and the bands.
    mdat_extra_size: u16,
    /// `hasQPData`: the extended tile header carries a quantization stream.
    has_q_p_data: bool,
    /// One reference per wavelet level into `q_step_table`.
    q_step: Vec<CrxQStepRef>,
    /// The level tables share one contiguous array, coarse first.
    q_step_table: Vec<u32>,
}

impl CrxTile<'_> {
    fn new() -> CrxTile<'static> {
        CrxTile {
            tile_flag: 0,
            comps: Vec::new(),
            tile_size: 0,
            data_offset: 0,
            width: 0,
            height: 0,
            mdat_q_p_data_size: 0,
            mdat_extra_size: 0,
            has_q_p_data: false,
            q_step: Vec::new(),
            q_step_table: Vec::new(),
        }
    }
}

/// The decoding image: `crx_image_t` without the stream bookkeeping, its
/// output planes addressed as offsets into the raw store.
struct CrxImage<'a> {
    tiles: Vec<CrxTile<'a>>,
    n_planes: u8,
    plane_width: u16,
    plane_height: u16,
    sample_precision: u8,
    median_bits: u8,
    /// `subbandCount`: one LL band plus three per wavelet level.
    subband_count: u8,
    levels: u8,
    n_bits: u8,
    enc_type: u8,
    tile_cols: u8,
    tile_rows: u8,
    /// `mdatOffset`: where the tile records begin inside the file.
    mdat_offset: u64,
    /// `outBufs`: each plane's first cell inside the raw store.
    out_bufs: [usize; 4],
    /// `planeBuf`: the intermediate store the encoding type 3 round trip
    /// folds into four Bayer planes.
    plane_buf: Option<Vec<i16>>,
}

/// `crxHorizontal53`: one line pair's horizontal 5/3 synthesis from the
/// wavelet's four band rows into the line pair the caller names.
fn crx_horizontal_53(arena: &mut [i32], wavelet: &CrxWavelet, line_la: usize, line_lb: usize, tile_flag: u8) {
    let mut band0 = wavelet.subband0;
    let mut band1 = wavelet.subband1;
    let mut band2 = wavelet.subband2;
    let mut band3 = wavelet.subband3;
    let mut la = line_la;
    let mut lb = line_lb;

    if wavelet.width <= 1 {
        arena[la] = arena[band0];
        arena[lb] = arena[band2];
        return;
    }

    if tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
        arena[la] = arena[band0].wrapping_sub((arena[band1].wrapping_add(arena[band1 + 1]).wrapping_add(2)) >> 2);
        arena[lb] = arena[band2].wrapping_sub((arena[band3].wrapping_add(arena[band3 + 1]).wrapping_add(2)) >> 2);
        band1 += 1;
        band3 += 1;
    } else {
        arena[la] = arena[band0].wrapping_sub((arena[band1].wrapping_add(1)) >> 1);
        arena[lb] = arena[band2].wrapping_sub((arena[band3].wrapping_add(1)) >> 1);
    }
    band0 += 1;
    band2 += 1;

    let width = i32::from(wavelet.width);
    let mut i = 0;
    while i < width - 3 {
        let delta = arena[band0].wrapping_sub((arena[band1].wrapping_add(arena[band1 + 1]).wrapping_add(2)) >> 2);
        arena[la + 1] = arena[band1].wrapping_add((delta.wrapping_add(arena[la])) >> 1);
        arena[la + 2] = delta;

        let delta = arena[band2].wrapping_sub((arena[band3].wrapping_add(arena[band3 + 1]).wrapping_add(2)) >> 2);
        arena[lb + 1] = arena[band3].wrapping_add((delta.wrapping_add(arena[lb])) >> 1);
        arena[lb + 2] = delta;

        band0 += 1;
        band1 += 1;
        band2 += 1;
        band3 += 1;
        la += 2;
        lb += 2;
        i += 2;
    }

    if tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
        let delta_a = arena[band0].wrapping_sub((arena[band1].wrapping_add(arena[band1 + 1]).wrapping_add(2)) >> 2);
        arena[la + 1] = arena[band1].wrapping_add((delta_a.wrapping_add(arena[la])) >> 1);

        let delta_b = arena[band2].wrapping_sub((arena[band3].wrapping_add(arena[band3 + 1]).wrapping_add(2)) >> 2);
        arena[lb + 1] = arena[band3].wrapping_add((delta_b.wrapping_add(arena[lb])) >> 1);

        if width & 1 != 0 {
            arena[la + 2] = delta_a;
            arena[lb + 2] = delta_b;
        }
    } else if width & 1 != 0 {
        arena[la + 1] = arena[band1]
            .wrapping_add((arena[la].wrapping_add(arena[band0].wrapping_sub((arena[band1].wrapping_add(1)) >> 1))) >> 1);
        arena[la + 2] = arena[band0].wrapping_sub((arena[band1].wrapping_add(1)) >> 1);

        arena[lb + 1] = arena[band3]
            .wrapping_add((arena[lb].wrapping_add(arena[band2].wrapping_sub((arena[band3].wrapping_add(1)) >> 1))) >> 1);
        arena[lb + 2] = arena[band2].wrapping_sub((arena[band3].wrapping_add(1)) >> 1);
    } else {
        arena[la + 1] = arena[la].wrapping_add(arena[band1]);
        arena[lb + 1] = arena[lb].wrapping_add(arena[band3]);
    }
}

/// `crxIdwt53FilterGetLine`: the next transformed line from the level's
/// rolling buffer chain.
fn crx_idwt53_get_line(comp: &mut CrxPlaneComp<'_>, level: usize) -> usize {
    let tap = i32::from(comp.wvlts[level].flt_tap_h);
    let cur_h = i32::from(comp.wvlts[level].cur_h);
    let line = comp.wvlts[level].line_buf[((tap - cur_h + 5) % 5 + 3) as usize];
    comp.wvlts[level].cur_h = (cur_h - 1) as i16;
    line
}

/// The level quantization lookup the decode functions build per level.
fn crx_q_step_level<'q>(q_steps: Option<(&'q [u32], &'q [CrxQStepRef])>, level: usize) -> Option<CrxQStepLevel<'q>> {
    let Some((table, levels)) = q_steps else { return None; };
    let Some(level_ref) = levels.get(level) else { return None; };
    Some(CrxQStepLevel {
        table: &table[level_ref.table_start..],
        width: level_ref.width,
    })
}

/// `crxIdwt53FilterDecode`: the band rows the level's next transform
/// consumes. The bottom rows of an odd-height level only need two bands.
fn crx_idwt53_decode(comp: &mut CrxPlaneComp<'_>, level: i32, q_steps: Option<(&[u32], &[CrxQStepRef])>) -> Result<()> {
    if comp.wvlts[level as usize].cur_h != 0 {
        return Ok(());
    }
    let q_step_level = crx_q_step_level(q_steps, level as usize);
    let wavelet_height = comp.wvlts[level as usize].height;
    let wavelet_cur_line = comp.wvlts[level as usize].cur_line;
    let band = 3 * level;
    let bottom_rows = i32::from(wavelet_height) - 3 <= i32::from(wavelet_cur_line)
        && comp.tile_flag & CRX_TILES_ON_THE_BOTTOM == 0;

    if bottom_rows {
        // the last row of an odd-height level decodes only its first
        // two bands, so the transform's short tail can run
        if i32::from(wavelet_height) & 1 != 0 {
            if level != 0 {
                crx_idwt53_decode(comp, level - 1, q_steps)?;
            } else {
                crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize], q_step_level.as_ref())?;
            }
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 1], q_step_level.as_ref())?;
        }
    } else {
        if level != 0 {
            crx_idwt53_decode(comp, level - 1, q_steps)?;
        } else {
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize], q_step_level.as_ref())?;
        }
        crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 1], q_step_level.as_ref())?;
        crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 2], q_step_level.as_ref())?;
        crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 3], q_step_level.as_ref())?;
    }
    Ok(())
}

/// `crxIdwt53FilterTransform`: the level's vertical 5/3 synthesis, one
/// output line set at a time, from the four band rows and the three line
/// buffers the previous calls left.
fn crx_idwt53_transform(comp: &mut CrxPlaneComp<'_>, level: i32) -> Result<()> {
    if comp.wvlts[level as usize].cur_h != 0 {
        return Ok(());
    }
    let wavelet_height = i32::from(comp.wvlts[level as usize].height);
    let tail_rows = i32::from(comp.wvlts[level as usize].cur_line) >= wavelet_height - 3;

    if tail_rows {
        if comp.tile_flag & CRX_TILES_ON_THE_BOTTOM == 0 {
            if wavelet_height & 1 != 0 {
                // the odd-height tail: one more coarse line feeds the
                // final transform
                if level != 0 {
                    if comp.wvlts[level as usize - 1].cur_h == 0 {
                        crx_idwt53_transform(comp, level - 1)?;
                    }
                    let line = crx_idwt53_get_line(comp, level as usize - 1);
                    comp.wvlts[level as usize].subband0 = line;
                }
                let mut band0 = comp.wvlts[level as usize].subband0;
                let mut band1 = comp.wvlts[level as usize].subband1;
                let tap = i32::from(comp.wvlts[level as usize].flt_tap_h);
                let line_h0 = comp.wvlts[level as usize].line_buf[(tap + 3) as usize];
                let line_h1 = comp.wvlts[level as usize].line_buf[((tap + 1) % 5 + 3) as usize];
                let line_h2 = comp.wvlts[level as usize].line_buf[((tap + 2) % 5 + 3) as usize];
                let mut line_l0 = comp.wvlts[level as usize].line_buf[0];
                let line_l1 = comp.wvlts[level as usize].line_buf[1];
                // the line pair rolls one place: the coarse pair's buffers swap
                comp.wvlts[level as usize].line_buf[1] = comp.wvlts[level as usize].line_buf[2];
                comp.wvlts[level as usize].line_buf[2] = line_l1;

                // process L bands
                let width = i32::from(comp.wvlts[level as usize].width);
                if width <= 1 {
                    comp.arena[line_l0] = comp.arena[band0];
                } else {
                    if comp.tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
                        comp.arena[line_l0] =
                            comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                        band1 += 1;
                    } else {
                        comp.arena[line_l0] = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                    }
                    band0 += 1;
                    let mut i = 0;
                    while i < width - 3 {
                        let delta =
                            comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                        comp.arena[line_l0 + 1] =
                            comp.arena[band1].wrapping_add((comp.arena[line_l0].wrapping_add(delta)) >> 1);
                        comp.arena[line_l0 + 2] = delta;
                        band0 += 1;
                        band1 += 1;
                        line_l0 += 2;
                        i += 2;
                    }
                    if comp.tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
                        let delta =
                            comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                        comp.arena[line_l0 + 1] =
                            comp.arena[band1].wrapping_add((comp.arena[line_l0].wrapping_add(delta)) >> 1);
                        if width & 1 != 0 {
                            comp.arena[line_l0 + 2] = delta;
                        }
                    } else if width & 1 != 0 {
                        let delta = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                        comp.arena[line_l0 + 1] =
                            comp.arena[band1].wrapping_add((comp.arena[line_l0].wrapping_add(delta)) >> 1);
                        comp.arena[line_l0 + 2] = delta;
                    } else {
                        comp.arena[line_l0 + 1] = comp.arena[band1].wrapping_add(comp.arena[line_l0]);
                    }
                }

                // process H bands
                let line_l0 = comp.wvlts[level as usize].line_buf[0];
                let line_l1 = comp.wvlts[level as usize].line_buf[1];
                for i in 0..width {
                    let delta = comp.arena[line_l0 + i as usize]
                        .wrapping_sub((comp.arena[line_l1 + i as usize].wrapping_add(1)) >> 1);
                    comp.arena[line_h1 + i as usize] = comp.arena[line_l1 + i as usize]
                        .wrapping_add((delta.wrapping_add(comp.arena[line_h0 + i as usize])) >> 1);
                    comp.arena[line_h2 + i as usize] = delta;
                }
                let cur_line = comp.wvlts[level as usize].cur_line;
                let cur_h = comp.wvlts[level as usize].cur_h;
                comp.wvlts[level as usize].cur_line = cur_line.wrapping_add(3);
                comp.wvlts[level as usize].cur_h = cur_h.wrapping_add(3);
                comp.wvlts[level as usize].flt_tap_h = ((tap + 3) % 5) as i8;
            } else {
                // the even-height tail: one more coarse line pairs with the
                // previous one
                let tap = i32::from(comp.wvlts[level as usize].flt_tap_h);
                let line_l2 = comp.wvlts[level as usize].line_buf[2];
                let line_h0 = comp.wvlts[level as usize].line_buf[(tap + 3) as usize];
                let line_h1 = comp.wvlts[level as usize].line_buf[((tap + 1) % 5 + 3) as usize];
                comp.wvlts[level as usize].line_buf[1] = line_l2;
                comp.wvlts[level as usize].line_buf[2] = comp.wvlts[level as usize].line_buf[1];

                let width = i32::from(comp.wvlts[level as usize].width);
                for i in 0..width {
                    comp.arena[line_h1 + i as usize] = comp.arena[line_h0 + i as usize]
                        .wrapping_add(comp.arena[line_l2 + i as usize]);
                }
                let cur_line = comp.wvlts[level as usize].cur_line;
                let cur_h = comp.wvlts[level as usize].cur_h;
                comp.wvlts[level as usize].cur_line = cur_line.wrapping_add(2);
                comp.wvlts[level as usize].cur_h = cur_h.wrapping_add(2);
                comp.wvlts[level as usize].flt_tap_h = ((tap + 2) % 5) as i8;
            }
        }
    } else {
        if level != 0 {
            if comp.wvlts[level as usize - 1].cur_h == 0 {
                crx_idwt53_transform(comp, level - 1)?;
            }
            let line = crx_idwt53_get_line(comp, level as usize - 1);
            comp.wvlts[level as usize].subband0 = line;
        }
        let mut band0 = comp.wvlts[level as usize].subband0;
        let mut band1 = comp.wvlts[level as usize].subband1;
        let mut band2 = comp.wvlts[level as usize].subband2;
        let mut band3 = comp.wvlts[level as usize].subband3;
        let tap = i32::from(comp.wvlts[level as usize].flt_tap_h);
        let line_h0 = comp.wvlts[level as usize].line_buf[(tap + 3) as usize];
        let line_h1 = comp.wvlts[level as usize].line_buf[((tap + 1) % 5 + 3) as usize];
        let line_h2 = comp.wvlts[level as usize].line_buf[((tap + 2) % 5 + 3) as usize];
        let mut line_l0 = comp.wvlts[level as usize].line_buf[0];
        let mut line_l1 = comp.wvlts[level as usize].line_buf[1];
        // the line pair rolls one place: the coarse pair's buffers swap
        comp.wvlts[level as usize].line_buf[1] = comp.wvlts[level as usize].line_buf[2];
        comp.wvlts[level as usize].line_buf[2] = line_l1;

        // process L bands
        let width = i32::from(comp.wvlts[level as usize].width);
        if width <= 1 {
            comp.arena[line_l0] = comp.arena[band0];
            comp.arena[line_l1] = comp.arena[band2];
        } else {
            if comp.tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
                comp.arena[line_l0] =
                    comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                comp.arena[line_l1] =
                    comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                band1 += 1;
                band3 += 1;
            } else {
                comp.arena[line_l0] = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                comp.arena[line_l1] = comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(1)) >> 1);
            }
            band0 += 1;
            band2 += 1;
            let mut i = 0;
            while i < width - 3 {
                let delta = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                comp.arena[line_l0 + 1] = comp.arena[band1].wrapping_add((delta.wrapping_add(comp.arena[line_l0])) >> 1);
                comp.arena[line_l0 + 2] = delta;

                let delta = comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                comp.arena[line_l1 + 1] = comp.arena[band3].wrapping_add((delta.wrapping_add(comp.arena[line_l1])) >> 1);
                comp.arena[line_l1 + 2] = delta;

                band0 += 1;
                band1 += 1;
                band2 += 1;
                band3 += 1;
                line_l0 += 2;
                line_l1 += 2;
                i += 2;
            }
            if comp.tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
                let delta_a =
                    comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                comp.arena[line_l0 + 1] = comp.arena[band1].wrapping_add((delta_a.wrapping_add(comp.arena[line_l0])) >> 1);

                let delta_b =
                    comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                comp.arena[line_l1 + 1] = comp.arena[band3].wrapping_add((delta_b.wrapping_add(comp.arena[line_l1])) >> 1);

                if width & 1 != 0 {
                    comp.arena[line_l0 + 2] = delta_a;
                    comp.arena[line_l1 + 2] = delta_b;
                }
            } else if width & 1 != 0 {
                let delta = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                comp.arena[line_l0 + 1] = comp.arena[band1].wrapping_add((delta.wrapping_add(comp.arena[line_l0])) >> 1);
                comp.arena[line_l0 + 2] = delta;

                let delta = comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(1)) >> 1);
                comp.arena[line_l1 + 1] = comp.arena[band3].wrapping_add((delta.wrapping_add(comp.arena[line_l1])) >> 1);
                comp.arena[line_l1 + 2] = delta;
            } else {
                comp.arena[line_l0 + 1] = comp.arena[line_l0].wrapping_add(comp.arena[band1]);
                comp.arena[line_l1 + 1] = comp.arena[line_l1].wrapping_add(comp.arena[band3]);
            }
        }

        // process H bands
        let line_l0 = comp.wvlts[level as usize].line_buf[0];
        let line_l1 = comp.wvlts[level as usize].line_buf[1];
        let line_l2 = comp.wvlts[level as usize].line_buf[2];
        for i in 0..width {
            let delta = comp.arena[line_l0 + i as usize]
                .wrapping_sub((comp.arena[line_l2 + i as usize].wrapping_add(comp.arena[line_l1 + i as usize]).wrapping_add(2)) >> 2);
            comp.arena[line_h1 + i as usize] = comp.arena[line_l1 + i as usize]
                .wrapping_add((delta.wrapping_add(comp.arena[line_h0 + i as usize])) >> 1);
            comp.arena[line_h2 + i as usize] = delta;
        }
        let cur_line = comp.wvlts[level as usize].cur_line;
        let cur_h = comp.wvlts[level as usize].cur_h;
        if i32::from(cur_line) >= wavelet_height - 3 && wavelet_height & 1 != 0 {
            comp.wvlts[level as usize].cur_line = cur_line.wrapping_add(3);
            comp.wvlts[level as usize].cur_h = cur_h.wrapping_add(3);
            comp.wvlts[level as usize].flt_tap_h = ((tap + 3) % 5) as i8;
        } else {
            comp.wvlts[level as usize].cur_line = cur_line.wrapping_add(2);
            comp.wvlts[level as usize].cur_h = cur_h.wrapping_add(2);
            comp.wvlts[level as usize].flt_tap_h = ((tap + 2) % 5) as i8;
        }
    }
    Ok(())
}

/// `crxIdwt53FilterInitialize`: the first three coarse lines and the first
/// detail line of every level, decoded straight from the band rows; the
/// per-call cycle takes over from there.
fn crx_idwt53_initialize(comp: &mut CrxPlaneComp<'_>, level: i32, q_steps: Option<(&[u32], &[CrxQStepRef])>) -> Result<()> {
    if level == 0 {
        return Ok(());
    }
    for cur_level in 0..level {
        let q_step_level = crx_q_step_level(q_steps, cur_level as usize);
        let band = 3 * cur_level;
        let wavelet_height = i32::from(comp.wvlts[cur_level as usize].height);
        let wavelet_width = i32::from(comp.wvlts[cur_level as usize].width);
        let tap = i32::from(comp.wvlts[cur_level as usize].flt_tap_h);
        let mut line_h0 = comp.wvlts[cur_level as usize].line_buf[(tap + 3) as usize];

        if cur_level != 0 {
            let line = crx_idwt53_get_line(comp, cur_level as usize - 1);
            comp.wvlts[cur_level as usize].subband0 = line;
        } else {
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize], q_step_level.as_ref())?;
        }

        if wavelet_height > 1 {
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 1], q_step_level.as_ref())?;
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 2], q_step_level.as_ref())?;
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 3], q_step_level.as_ref())?;
            let line_l0 = comp.wvlts[cur_level as usize].line_buf[0];
            let line_l1 = comp.wvlts[cur_level as usize].line_buf[1];
            let mut line_l2 = comp.wvlts[cur_level as usize].line_buf[2];

            if comp.tile_flag & CRX_TILES_ON_THE_TOP != 0 {
                crx_horizontal_53(&mut comp.arena, &comp.wvlts[cur_level as usize], line_l0, line_l1, comp.tile_flag);
                crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 3], q_step_level.as_ref())?;
                crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 2], q_step_level.as_ref())?;
                let mut band2 = comp.wvlts[cur_level as usize].subband2;
                let mut band3 = comp.wvlts[cur_level as usize].subband3;

                // process L band
                if wavelet_width <= 1 {
                    comp.arena[line_l2] = comp.arena[band2];
                } else {
                    if comp.tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
                        comp.arena[line_l2] =
                            comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                        band3 += 1;
                    } else {
                        comp.arena[line_l2] = comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(1)) >> 1);
                    }
                    band2 += 1;
                    let mut i = 0;
                    while i < wavelet_width - 3 {
                        let delta =
                            comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                        comp.arena[line_l2 + 1] =
                            comp.arena[band3].wrapping_add((comp.arena[line_l2].wrapping_add(delta)) >> 1);
                        comp.arena[line_l2 + 2] = delta;
                        band2 += 1;
                        band3 += 1;
                        line_l2 += 2;
                        i += 2;
                    }
                    if comp.tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
                        let delta =
                            comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(comp.arena[band3 + 1]).wrapping_add(2)) >> 2);
                        comp.arena[line_l2 + 1] =
                            comp.arena[band3].wrapping_add((comp.arena[line_l2].wrapping_add(delta)) >> 1);
                        if wavelet_width & 1 != 0 {
                            comp.arena[line_l2 + 2] = delta;
                        }
                    } else if wavelet_width & 1 != 0 {
                        let delta = comp.arena[band2].wrapping_sub((comp.arena[band3].wrapping_add(1)) >> 1);
                        comp.arena[line_l2 + 1] =
                            comp.arena[band3].wrapping_add((comp.arena[line_l2].wrapping_add(delta)) >> 1);
                        comp.arena[line_l2 + 2] = delta;
                    } else {
                        comp.arena[line_l2 + 1] = comp.arena[band3].wrapping_add(comp.arena[line_l2]);
                    }
                }

                // process H band
                for i in 0..wavelet_width {
                    comp.arena[line_h0 + i as usize] = comp.arena[line_l0 + i as usize]
                        .wrapping_sub((comp.arena[line_l1 + i as usize]
                            .wrapping_add(comp.arena[line_l2 + i as usize])
                            .wrapping_add(2)) >> 2);
                }
            } else {
                crx_horizontal_53(&mut comp.arena, &comp.wvlts[cur_level as usize], line_l0, line_l2, comp.tile_flag);
                for i in 0..wavelet_width {
                    comp.arena[line_h0 + i as usize] = comp.arena[line_l0 + i as usize]
                        .wrapping_sub((comp.arena[line_l2 + i as usize].wrapping_add(1)) >> 1);
                }
            }

            crx_idwt53_decode(comp, cur_level, q_steps)?;
            crx_idwt53_transform(comp, cur_level)?;
        } else {
            crx_decode_line_with_iquantization(&mut comp.arena, &mut comp.subs[band as usize + 1], q_step_level.as_ref())?;
            let mut band0 = comp.wvlts[cur_level as usize].subband0;
            let mut band1 = comp.wvlts[cur_level as usize].subband1;

            // process H band
            if wavelet_width <= 1 {
                comp.arena[line_h0] = comp.arena[band0];
            } else {
                if comp.tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
                    comp.arena[line_h0] =
                        comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                    band1 += 1;
                } else {
                    comp.arena[line_h0] = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                }
                band0 += 1;
                let mut i = 0;
                while i < wavelet_width - 3 {
                    let delta =
                        comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                    comp.arena[line_h0 + 1] = comp.arena[band1].wrapping_add((comp.arena[line_h0].wrapping_add(delta)) >> 1);
                    comp.arena[line_h0 + 2] = delta;
                    band0 += 1;
                    band1 += 1;
                    line_h0 += 2;
                    i += 2;
                }
                if comp.tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
                    let delta =
                        comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(comp.arena[band1 + 1]).wrapping_add(2)) >> 2);
                    comp.arena[line_h0 + 1] = comp.arena[band1].wrapping_add((comp.arena[line_h0].wrapping_add(delta)) >> 1);
                    comp.arena[line_h0 + 2] = delta;
                } else if wavelet_width & 1 != 0 {
                    let delta = comp.arena[band0].wrapping_sub((comp.arena[band1].wrapping_add(1)) >> 1);
                    comp.arena[line_h0 + 1] = comp.arena[band1].wrapping_add((comp.arena[line_h0].wrapping_add(delta)) >> 1);
                    comp.arena[line_h0 + 2] = delta;
                } else {
                    comp.arena[line_h0 + 1] = comp.arena[band1].wrapping_add(comp.arena[line_h0]);
                }
            }
            let cur_line = comp.wvlts[cur_level as usize].cur_line;
            let cur_h = comp.wvlts[cur_level as usize].cur_h;
            comp.wvlts[cur_level as usize].cur_line = cur_line.wrapping_add(1);
            comp.wvlts[cur_level as usize].cur_h = cur_h.wrapping_add(1);
            comp.wvlts[cur_level as usize].flt_tap_h = ((tap + 1) % 5) as i8;
        }
    }
    Ok(())
}

/// `crxConvertPlaneLine`: one decoded line into the raw store. The
/// encoding types carry the samples straight over or through the
/// intermediate plane store; the other forms add the bit depth's midpoint
/// and clamp. With no line, the encoding type 3 store folds into the four
/// Bayer planes.
fn crx_convert_plane_line(img: &mut CrxImage<'_>, image: &mut [u16], image_row: i32, image_col: i32, plane: usize, line: Option<(&[i32], i32)>) -> Result<()> {
    if let Some((line_data, line_length)) = line {
        // the C bounds every store loop by the caller's line length
        let sample_count = line_length.max(0) as usize;
        let plane_width = i32::from(img.plane_width);
        let raw_offset = 4i32
            .wrapping_mul(plane_width)
            .wrapping_mul(image_row)
            .wrapping_add(2i32.wrapping_mul(image_col)) as u64 as usize;
        if img.enc_type == 1 {
            let max_value = 1i32.wrapping_shl((i32::from(img.n_bits) - 1) as u32);
            let min_value = -max_value;
            let max_value = max_value - 1;
            for i in 0..sample_count {
                image[img.out_bufs[plane] + raw_offset + 2 * i] = crx_constrain(line_data[i], min_value, max_value) as u16;
            }
        } else if img.enc_type == 3 {
            // copy to the intermediate plane store
            let raw_offset = (plane as i32)
                .wrapping_mul(plane_width)
                .wrapping_mul(i32::from(img.plane_height))
                .wrapping_add(plane_width.wrapping_mul(image_row))
                .wrapping_add(image_col) as u64 as usize;
            let Some(plane_buf) = img.plane_buf.as_mut() else {
                bail!("the encoding type 3 plane store is missing");
            };
            for i in 0..sample_count {
                plane_buf[raw_offset + i] = line_data[i] as i16;
            }
        } else if img.n_planes == 4 {
            let median = 1i32.wrapping_shl((i32::from(img.n_bits) - 1) as u32);
            let max_value = 1i32.wrapping_shl(i32::from(img.n_bits) as u32).wrapping_sub(1);
            for i in 0..sample_count {
                image[img.out_bufs[plane] + raw_offset + 2 * i] =
                    crx_constrain(median.wrapping_add(line_data[i]), 0, max_value) as u16;
            }
        } else if img.n_planes == 1 {
            let max_value = 1i32.wrapping_shl(i32::from(img.n_bits) as u32).wrapping_sub(1);
            let median = 1i32.wrapping_shl((i32::from(img.n_bits) - 1) as u32);
            let raw_offset = plane_width.wrapping_mul(image_row).wrapping_add(image_col) as u64 as usize;
            for i in 0..sample_count {
                image[img.out_bufs[0] + raw_offset + i] =
                    crx_constrain(median.wrapping_add(line_data[i]), 0, max_value) as u16;
            }
        }
    } else if img.enc_type == 3 {
        if let Some(plane_buf) = img.plane_buf.as_mut() {
            let plane_width = i32::from(img.plane_width);
            let plane_height = i32::from(img.plane_height);
            let plane_size = plane_width.wrapping_mul(plane_height) as usize;
            let plane0 = image_row.wrapping_mul(plane_width) as usize;
            let plane1 = plane0 + plane_size;
            let plane2 = plane1 + plane_size;
            let plane3 = plane2 + plane_size;
            let median = 1i32.wrapping_shl((i32::from(img.median_bits) - 1) as u32).wrapping_shl(10);
            let max_value = 1i32.wrapping_shl(i32::from(img.median_bits) as u32).wrapping_sub(1);
            // the engine's stage writes through a wrapping 32-bit row offset
            let raw_line_offset = 4i32.wrapping_mul(plane_width).wrapping_mul(image_row) as u32 as usize;

            for i in 0..plane_width as usize {
                let gr = median
                    .wrapping_add(i32::from(plane_buf[plane0 + i]).wrapping_shl(10))
                    .wrapping_sub(168 * i32::from(plane_buf[plane1 + i]))
                    .wrapping_sub(585 * i32::from(plane_buf[plane3 + i]));
                let gr = if gr < 0 {
                    -(gr.wrapping_abs().wrapping_add(512) >> 9) & !1
                } else {
                    (gr.wrapping_abs().wrapping_add(512) >> 9) & !1
                };

                // R = round(median + P0 + 1.474*P3)
                let val = median
                    .wrapping_add(i32::from(plane_buf[plane0 + i]).wrapping_shl(10))
                    .wrapping_add(1510 * i32::from(plane_buf[plane3 + i]))
                    .wrapping_add(512) >> 10;
                image[img.out_bufs[0] + raw_line_offset + 2 * i] = crx_constrain(val, 0, max_value) as u16;
                // G1 = round(median + P0 + P2 - 0.164*P1 - 0.571*P3)
                let val = i32::from(plane_buf[plane2 + i]).wrapping_add(gr).wrapping_add(1) >> 1;
                image[img.out_bufs[1] + raw_line_offset + 2 * i] = crx_constrain(val, 0, max_value) as u16;
                // G2 = round(median + P0 - P2 - 0.164*P1 - 0.571*P3)
                let val = gr.wrapping_sub(i32::from(plane_buf[plane2 + i])).wrapping_add(1) >> 1;
                image[img.out_bufs[2] + raw_line_offset + 2 * i] = crx_constrain(val, 0, max_value) as u16;
                // B = round(median + P0 + 1.881*P1)
                let val = median
                    .wrapping_add(i32::from(plane_buf[plane0 + i]).wrapping_shl(10))
                    .wrapping_add(1927 * i32::from(plane_buf[plane1 + i]))
                    .wrapping_add(512) >> 10;
                image[img.out_bufs[3] + raw_line_offset + 2 * i] = crx_constrain(val, 0, max_value) as u16;
            }
        }
    }
    Ok(())
}

/// `crxSetupSubbandData`: the component's arena — every band's row region
/// and, behind them, the wavelet levels' eight line buffers — the band
/// mdat offsets and the streaming band decoders.
fn crx_setup_subband_data<'i>(
    img: &mut CrxImage<'i>,
    tile_index: usize,
    plane_number: usize,
    levels: i32,
    mdat_offset: u64,
    data: &'i [u8],
) -> Result<()> {
    let to_subbands = (3 * levels + 1) as usize;
    {
        let tile = &img.tiles[tile_index];
        let comp = &tile.comps[plane_number];
        if comp.subs.len() < to_subbands {
            bail!("the CR3 component misses its subbands");
        }
    }

    // band sizes and the arena's total
    let mut comp_data_size = 0usize;
    for band in &img.tiles[tile_index].comps[plane_number].subs[..to_subbands] {
        comp_data_size += band.width as usize;
    }
    let mut line_region = 0usize;
    if levels != 0 {
        for level in 0..levels {
            let width = if level < levels - 1 {
                img.tiles[tile_index].comps[plane_number].subs[(3 * (level + 1) + 2) as usize].width
            } else {
                i32::from(img.tiles[tile_index].width)
            };
            line_region += 8 * width as usize;
        }
    }
    let arena = &mut img.tiles[tile_index].comps[plane_number].arena;
    *arena = vec![0; comp_data_size + line_region];

    // band regions and mdat offsets
    let subband_mdat_offset = img.mdat_offset.wrapping_add(mdat_offset);
    let mut cursor = 0usize;
    for band in &mut img.tiles[tile_index].comps[plane_number].subs[..to_subbands] {
        band.band_start = cursor;
        band.band_size = band.width as usize * 4;
        cursor += band.width as usize;
        band.mdat_offset = subband_mdat_offset as i64 + band.data_offset;
    }

    // wavelet data
    if levels != 0 {
        img.tiles[tile_index].comps[plane_number].wvlts.resize_with(levels as usize, CrxWavelet::new);
        img.tiles[tile_index].comps[plane_number].wvlts[0].subband0 = img.tiles[tile_index].comps[plane_number].subs[0].band_start;
        let mut next = cursor;
        for level in 0..levels {
            let band = 3 * level + 1;
            let (height, transform_width) = if level >= levels - 1 {
                (i32::from(img.tiles[tile_index].height), i32::from(img.tiles[tile_index].width))
            } else {
                let comp = &img.tiles[tile_index].comps[plane_number];
                (comp.subs[(band + 3) as usize].height, comp.subs[(band + 4) as usize].width)
            };
            let (sub1_start, sub2_start, sub3_start) = {
                let comp = &img.tiles[tile_index].comps[plane_number];
                (
                    comp.subs[band as usize].band_start,
                    comp.subs[band as usize + 1].band_start,
                    comp.subs[band as usize + 2].band_start,
                )
            };
            let wavelet = &mut img.tiles[tile_index].comps[plane_number].wvlts[level as usize];
            wavelet.height = height as i16;
            wavelet.width = transform_width as i16;
            wavelet.line_buf[0] = next;
            for line in 1..8 {
                wavelet.line_buf[line] = wavelet.line_buf[line - 1] + transform_width as usize;
            }
            wavelet.cur_line = 0;
            wavelet.cur_h = 0;
            wavelet.flt_tap_h = 0;
            wavelet.subband1 = sub1_start;
            wavelet.subband2 = sub2_start;
            wavelet.subband3 = sub3_start;
            next = wavelet.line_buf[7] + transform_width as usize;
        }
    }

    // decoding params and bitstreams
    {
        let comp = &mut img.tiles[tile_index].comps[plane_number];
        let comp_supports_partial = comp.supports_partial;
        let comp_rounded_bits_mask = comp.rounded_bits_mask;
        for index in 0..to_subbands {
            if comp.subs[index].data_size != 0 {
                let mut supports_partial = false;
                let mut rounded_bits_mask = 0;
                if comp_supports_partial && index == 0 {
                    rounded_bits_mask = comp_rounded_bits_mask;
                    supports_partial = true;
                }
                let band = &mut comp.subs[index];
                let width = band.width;
                let height = band.height;
                let data_size = band.data_size;
                let mdat = band.mdat_offset;
                band.band_param = Some(crx_setup_band_param(
                    data,
                    mdat,
                    data_size,
                    width,
                    height,
                    supports_partial,
                    rounded_bits_mask,
                )?);
            }
        }
    }
    Ok(())
}

/// `crxReadQP`: the Golomb-coded quantization value, split between a zero
/// run and literal bits.
fn crx_read_qp(bit_stream: &mut CrxBitstream<'_>, k_param: i32) -> Result<u32> {
    let mut qp = bit_stream.get_zeros()?;
    if qp >= 23 {
        qp = bit_stream.get_bits(8)?;
    } else if k_param != 0 {
        qp = bit_stream
            .get_bits(k_param)?
            .wrapping_add(qp.wrapping_shl(k_param as u32));
    }
    Ok(qp)
}

/// `crxDecodeGolombTop`: the first quantization row, predicted from zero.
fn crx_decode_golomb_top(bit_stream: &mut CrxBitstream<'_>, width: i32, table: &mut [i32], line1: usize, k_param: &mut i32) -> Result<()> {
    table[line1] = 0;
    let mut cur = line1;
    let mut width = width;
    while width > 0 {
        width -= 1;
        table[cur + 1] = table[cur];
        let qp = crx_read_qp(bit_stream, *k_param)?;
        table[cur + 1] = table[cur + 1].wrapping_add(crx_zigzag(qp));
        *k_param = crx_predict_k_parameter(*k_param, qp, 7);
        cur += 1;
    }
    table[cur + 1] = table[cur].wrapping_add(1);
    Ok(())
}

/// `crxDecodeGolombNormal`: a following quantization row, predicted from
/// the line above and its own trailing pixel.
fn crx_decode_golomb_normal(bit_stream: &mut CrxBitstream<'_>, width: i32, table: &mut [i32], line0: usize, line1: usize, k_param: &mut i32) -> Result<()> {
    table[line1] = table[line0 + 1];
    let mut delta_h = table[line0 + 1].wrapping_sub(table[line0]);
    let mut cur0 = line0;
    let mut cur1 = line1;
    let mut width = width;
    while width > 0 {
        width -= 1;
        table[cur1 + 1] = crx_prediction(table[cur1], table[cur0 + 1], delta_h, table[cur0].wrapping_sub(table[cur1]));
        let qp = crx_read_qp(bit_stream, *k_param)?;
        table[cur1 + 1] = table[cur1 + 1].wrapping_add(crx_zigzag(qp));
        if width != 0 {
            delta_h = table[cur0 + 2].wrapping_sub(table[cur0 + 1]);
            let bit_code = qp
                .wrapping_add(2u32.wrapping_mul(delta_h.wrapping_abs() as u32))
                >> 1;
            *k_param = crx_predict_k_parameter(*k_param, bit_code, 7);
            cur0 += 1;
        } else {
            *k_param = crx_predict_k_parameter(*k_param, qp, 7);
        }
        cur1 += 1;
    }
    table[cur1 + 1] = table[cur1].wrapping_add(1);
    Ok(())
}

/// One step of the quantization scaling: the value's whole and fractional
/// parts pick the base step and a power of two around it.
fn crx_q_step_cell(quant_value: i32) -> u32 {
    let cell = quant_value.rem_euclid(6) as usize;
    if quant_value / 6 >= 6 {
        Q_STEP_TBL[cell].wrapping_shl(((quant_value / 6 - 6) & 0x1f) as u32)
    } else {
        Q_STEP_TBL[cell].wrapping_shr((6 - quant_value / 6) as u32)
    }
}

/// `crxMakeQStep`: the tile's per-level quantization tables from the
/// decoded quantization map — one shared array, coarsest collapse first.
fn crx_make_q_step(tile: &mut CrxTile<'_>, levels: u8, qp_table: &[i32]) -> Result<()> {
    if levels > 3 || levels < 1 {
        bail!("the CR3 wavelet depth is out of range");
    }
    let tile_width = i32::from(tile.width);
    let tile_height = i32::from(tile.height);
    let qp_width = (tile_width >> 3) + i32::from((tile_width & 7) != 0);
    let qp_height = (tile_height >> 1) + i32::from(tile_height & 1);
    let qp_height4 = (tile_height >> 2) + i32::from((tile_height & 3) != 0);
    let qp_height8 = (tile_height >> 3) + i32::from((tile_height & 7) != 0);

    // the engine's switch falls through: the coarsest collapse first
    let mut q_step_table = Vec::new();
    let mut q_step = Vec::new();
    if levels > 2 {
        for qp_row in 0..qp_height8 {
            let row0_base = qp_width * _min_i32(4 * qp_row, qp_height - 1);
            let row1_base = qp_width * _min_i32(4 * qp_row + 1, qp_height - 1);
            let row2_base = qp_width * _min_i32(4 * qp_row + 2, qp_height - 1);
            let row3_base = qp_width * _min_i32(4 * qp_row + 3, qp_height - 1);
            for qp_col in 0..qp_width {
                let mut quant_value = qp_table[row0_base as usize + qp_col as usize]
                    .wrapping_add(qp_table[row1_base as usize + qp_col as usize])
                    .wrapping_add(qp_table[row2_base as usize + qp_col as usize])
                    .wrapping_add(qp_table[row3_base as usize + qp_col as usize]);
                quant_value = ((i32::from(quant_value < 0) * 3) + quant_value) >> 2;
                q_step_table.push(crx_q_step_cell(quant_value));
            }
        }
        q_step.push(CrxQStepRef { table_start: q_step_table.len() - (qp_height8 * qp_width) as usize, width: qp_width as usize });
    }
    if levels > 1 {
        let table_start = q_step_table.len();
        for qp_row in 0..qp_height4 {
            let row0_base = qp_width * _min_i32(2 * qp_row, qp_height - 1);
            let row1_base = qp_width * _min_i32(2 * qp_row + 1, qp_height - 1);
            for qp_col in 0..qp_width {
                let sum = qp_table[row0_base as usize + qp_col as usize]
                    .wrapping_add(qp_table[row1_base as usize + qp_col as usize]);
                let quant_value = sum / 2;
                q_step_table.push(crx_q_step_cell(quant_value));
            }
        }
        q_step.push(CrxQStepRef {
            table_start,
            width: qp_width as usize,
        });
    }
    {
        let table_start = q_step_table.len();
        let mut cursor = 0usize;
        for _ in 0..qp_height {
            for _ in 0..qp_width {
                q_step_table.push(crx_q_step_cell(qp_table[cursor]));
                cursor += 1;
            }
        }
        q_step.push(CrxQStepRef {
            table_start,
            width: qp_width as usize,
        });
    }

    tile.q_step_table = q_step_table;
    tile.q_step = q_step;
    Ok(())
}

/// The engine's own local minimum the q-step builder indexes with.
fn _min_i32(first: i32, second: i32) -> i32 {
    if first < second {
        first
    } else {
        second
    }
}

/// `crxSetupSubbandIdx`: the band's addon rows and columns at the tile
/// edges; only the wide header form tracks them.
fn crx_setup_subband_idx(version: u32, band: &mut CrxSubband<'_>, level: i32, col_start_idx: i32, band_width_ex_coef: i32, row_start_idx: i32, band_height_ex_coef: i32) {
    if version == 0x200 {
        band.row_start_add_on = row_start_idx as i16 as i32;
        band.row_end_add_on = band_height_ex_coef as i16 as i32;
        band.col_start_add_on = col_start_idx as i16 as i32;
        band.col_end_add_on = band_width_ex_coef as i16 as i32;
        band.level_shift = 3 - level;
    } else {
        band.row_start_add_on = 0;
        band.row_end_add_on = 0;
        band.col_start_add_on = 0;
        band.col_end_add_on = 0;
        band.level_shift = 0;
    }
}

/// `crxProcessSubbands`: every band's width, height and edge addons, built
/// down from the last band to the coefficient level the tile carries.
fn crx_process_subbands(version: u32, img: &mut CrxImage<'_>, tile_index: usize, plane_number: usize) -> Result<()> {
    let subband_count = usize::from(img.subband_count);
    let levels = u32::from(img.levels);
    let (tile_width, tile_height, tile_flag) = {
        let tile = &img.tiles[tile_index];
        (u32::from(tile.width), u32::from(tile.height), tile.tile_flag)
    };

    let mut band_width = tile_width;
    let mut band_height = tile_height;
    let mut band_width_ex_coef = 0i32;
    let mut band_height_ex_coef = 0i32;

    if subband_count == 0 {
        bail!("the CR3 component misses its subbands");
    }
    let mut band_index = subband_count as i64 - 1;

    if levels != 0 {
        // Build up subband sequences to crxDecode to a level in a header
        let row_ex_base = 0x30 * (levels as usize - 1) + 6 * (tile_width as usize & 7);
        let col_ex_base = 0x30 * (levels as usize - 1) + 6 * (tile_height as usize & 7);
        for level in 0..levels {
            let width_odd_pixel = band_width & 1;
            let height_odd_pixel = band_height & 1;
            band_width = (width_odd_pixel.wrapping_add(band_width)) >> 1;
            band_height = (height_odd_pixel.wrapping_add(band_height)) >> 1;

            let mut band_width_ex_coef0 = 0i32;
            let mut band_width_ex_coef1 = 0i32;
            let mut band_height_ex_coef0 = 0i32;
            let mut band_height_ex_coef1 = 0i32;
            let mut col_start_idx = 0i32;
            let mut row_start_idx = 0i32;
            if tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
                band_width_ex_coef0 = EX_COEF_NUM_TBL[row_ex_base + 2 * level as usize];
                band_width_ex_coef1 = EX_COEF_NUM_TBL[row_ex_base + 2 * level as usize + 1];
            }
            if tile_flag & CRX_TILES_ON_THE_LEFT != 0 {
                band_width_ex_coef0 += 1;
                col_start_idx = 1;
            }

            if tile_flag & CRX_TILES_ON_THE_BOTTOM != 0 {
                band_height_ex_coef0 = EX_COEF_NUM_TBL[col_ex_base + 2 * level as usize];
                band_height_ex_coef1 = EX_COEF_NUM_TBL[col_ex_base + 2 * level as usize + 1];
            }
            if tile_flag & CRX_TILES_ON_THE_TOP != 0 {
                band_height_ex_coef0 += 1;
                row_start_idx = 1;
            }

            let band = band_index as usize;
            let plane_comp = &mut img.tiles[tile_index].comps[plane_number];
            plane_comp.subs[band].width = (band_width
                .wrapping_add(band_width_ex_coef0 as u32)
                .wrapping_sub(width_odd_pixel)) as u16 as i32;
            plane_comp.subs[band].height = (band_height
                .wrapping_add(band_height_ex_coef0 as u32)
                .wrapping_sub(height_odd_pixel)) as u16 as i32;
            crx_setup_subband_idx(
                version,
                &mut plane_comp.subs[band],
                level as i32 + 1,
                col_start_idx,
                band_width_ex_coef0 - col_start_idx,
                row_start_idx,
                band_height_ex_coef0 - row_start_idx,
            );

            plane_comp.subs[band - 1].width = (band_width.wrapping_add(band_width_ex_coef1 as u32)) as u16 as i32;
            plane_comp.subs[band - 1].height = (band_height
                .wrapping_add(band_height_ex_coef0 as u32)
                .wrapping_sub(height_odd_pixel)) as u16 as i32;
            crx_setup_subband_idx(
                version,
                &mut plane_comp.subs[band - 1],
                level as i32 + 1,
                0,
                band_width_ex_coef1,
                row_start_idx,
                band_height_ex_coef0 - row_start_idx,
            );

            plane_comp.subs[band - 2].width = (band_width
                .wrapping_add(band_width_ex_coef0 as u32)
                .wrapping_sub(width_odd_pixel)) as u16 as i32;
            plane_comp.subs[band - 2].height = (band_height.wrapping_add(band_height_ex_coef1 as u32)) as u16 as i32;
            crx_setup_subband_idx(
                version,
                &mut plane_comp.subs[band - 2],
                level as i32 + 1,
                col_start_idx,
                band_width_ex_coef0 - col_start_idx,
                0,
                band_height_ex_coef1,
            );

            band_index -= 3;
        }
        band_width_ex_coef = 0;
        band_height_ex_coef = 0;
        if tile_flag & CRX_TILES_ON_THE_RIGHT != 0 {
            band_width_ex_coef = EX_COEF_NUM_TBL[row_ex_base + 2 * levels as usize - 1];
        }
        if tile_flag & CRX_TILES_ON_THE_BOTTOM != 0 {
            band_height_ex_coef = EX_COEF_NUM_TBL[col_ex_base + 2 * levels as usize - 1];
        }
    }

    let plane_comp = &mut img.tiles[tile_index].comps[plane_number];
    let band = band_index as usize;
    plane_comp.subs[band].width = ((band_width_ex_coef as u32).wrapping_add(band_width)) as u16 as i32;
    plane_comp.subs[band].height = ((band_height_ex_coef as u32).wrapping_add(band_height)) as u16 as i32;
    if levels != 0 {
        crx_setup_subband_idx(version, &mut plane_comp.subs[band], levels as i32, 0, band_width_ex_coef, 0, band_height_ex_coef);
    }
    Ok(())
}

/// `crxReadSubbandHeaders`: each band's header inside the mdat — its
/// compressed size, mdat offset and quantization facts — walking the
/// header buffer by cursor.
fn crx_read_subband_headers(
    subband_count: usize,
    comp: &mut CrxPlaneComp<'_>,
    hdr_buf: &[u8],
    cursor: &mut usize,
    data_size: &mut i32,
) -> Result<()> {
    if subband_count == 0 {
        return Ok(());
    }
    let mut subband_offset = 0i32;
    for cur_subband in 0..subband_count {
        if *data_size < 4 {
            bail!("the CR3 subband header data is short");
        }

        let hdr_sign = crx_sget_n(hdr_buf, *cursor, 2);
        let hdr_size = crx_sget_n(hdr_buf, *cursor + 2, 2);
        if *data_size < hdr_size as i32 + 4 {
            bail!("the CR3 subband header data is short");
        }
        if !(hdr_sign == CRX_SUBBAND_HEADER_OLD && hdr_size == 8)
            && !(hdr_sign == CRX_SUBBAND_HEADER_NEW && hdr_size == 16)
        {
            bail!("the CR3 subband header mark is unexpected");
        }

        let subband_size = crx_sget_n(hdr_buf, *cursor + 4, 4) as i32;

        if cur_subband as u32 != (u32::from(hdr_buf[*cursor + 8]) & 0xF0) >> 4 {
            comp.subs[cur_subband].data_size = i64::from(subband_size);
            bail!("the CR3 subband header order is unexpected");
        }

        let band = &mut comp.subs[cur_subband];
        band.data_offset = i64::from(subband_offset);
        band.k_param = 0;
        band.band_param = None;
        band.band_start = 0;
        band.band_size = 0;

        if hdr_sign == CRX_SUBBAND_HEADER_OLD {
            // old header
            let bit_data = crx_sget_n(hdr_buf, *cursor + 8, 4);
            band.data_size = i64::from(subband_size.wrapping_sub((bit_data & 0x7FFFF) as i32));
            band.supports_partial = bit_data & 0x8000000 != 0;
            band.q_param = ((bit_data >> 19) & 0xFF) as i32;
            band.q_step_base = 0;
            band.q_step_mult = 0;
        } else {
            // new header
            if crx_sget_n(hdr_buf, *cursor + 8, 2) & 0xFFF != 0 {
                // partial and qParam are not supported
                bail!("the CR3 subband header is not supported");
            }
            if crx_sget_n(hdr_buf, *cursor + 18, 2) != 0 {
                // new header terninated by 2 zero bytes
                bail!("the CR3 subband header terminator is unexpected");
            }
            band.supports_partial = false;
            band.q_param = 0;
            band.data_size = i64::from(subband_size.wrapping_sub(crx_sget_n(hdr_buf, *cursor + 16, 2) as i32));
            band.q_step_base = crx_sget_n(hdr_buf, *cursor + 12, 4) as i32;
            band.q_step_mult = crx_sget_n(hdr_buf, *cursor + 10, 2);
        }

        subband_offset = subband_offset.wrapping_add(subband_size);

        *cursor += hdr_size as usize + 4;
        *data_size -= (hdr_size as i32) + 4;
    }

    Ok(())
}

/// One CR3 codec header, copied from the selected track's parsed facts.
struct CrxHeader {
    version: u32,
    f_width: u32,
    f_height: u32,
    tile_width: u32,
    tile_height: u32,
    n_planes: u32,
    cfa_layout: u32,
    enc_type: u32,
    image_levels: u32,
    n_bits: u32,
    median_bits: u32,
    mdat_header_size: u32,
}

/// `sgetn` for the header walk: a fixed big-endian read at a cursor.
fn crx_sget_n(data: &[u8], at: usize, count: usize) -> u32 {
    let mut result = 0u32;
    for byte in 0..count {
        result = (result << 8) | u32::from(data[at + byte]);
    }
    result
}

/// `crxDecodePlane`: one plane's tiles in order, each decoded through its
/// subband data, the wavelet cycle and the per-line conversion.
fn crx_decode_plane<'a>(
    img: &mut CrxImage<'a>,
    image: &mut [u16],
    plane_number: usize,
    file_data: &'a [u8],
) -> Result<()> {
    let mut image_row = 0i32;
    for t_row in 0..usize::from(img.tile_rows) {
        let mut image_col = 0i32;
        let t_cols = usize::from(img.tile_cols);
        for t_col in 0..t_cols {
            let tile_index = t_row * usize::from(img.tile_cols) + t_col;
            let top_level = i32::from(img.levels) - 1;
            let levels = i32::from(img.levels);
            let tile_width = i32::from(img.tiles[tile_index].width);
            let tile_height = i32::from(img.tiles[tile_index].height);
            // the tile's mdat start for its compressed records
            let tile_mdat_offset = {
                let tile = &img.tiles[tile_index];
                let plane_comp = &tile.comps[plane_number];
                tile.data_offset
                    .wrapping_add(i64::from(tile.mdat_q_p_data_size))
                    .wrapping_add(i64::from(tile.mdat_extra_size))
                    .wrapping_add(plane_comp.data_offset)
            };

            // decode single tile
            crx_setup_subband_data(img, tile_index, plane_number, levels, tile_mdat_offset as u64, file_data)?;

            if levels != 0 {
                {
                    let tile = &mut img.tiles[tile_index];
                    let CrxTile { q_step_table, q_step, comps, .. } = tile;
                    let comp = &mut comps[plane_number];
                    let q_steps = (q_step_table.as_slice(), q_step.as_slice());
                    crx_idwt53_initialize(comp, levels, Some(q_steps))?;
                }
                for i in 0..tile_height {
                    let line = {
                        let tile = &mut img.tiles[tile_index];
                        let CrxTile { q_step_table, q_step, comps, .. } = tile;
                        let comp = &mut comps[plane_number];
                        let q_steps = (q_step_table.as_slice(), q_step.as_slice());
                        crx_idwt53_decode(comp, top_level, Some(q_steps))?;
                        crx_idwt53_transform(comp, top_level)?;
                        let line_start = crx_idwt53_get_line(comp, top_level as usize);
                        let line = comp.arena[line_start..line_start + tile_width as usize].to_vec();
                        line
                    };
                    crx_convert_plane_line(img, image, image_row + i, image_col, plane_number, Some((&line, tile_width)))?;
                }
            } else {
                // we have the only subband in this case
                let band_has_data = img.tiles[tile_index].comps[plane_number].subs[0].data_size != 0;
                if !band_has_data {
                    let tile = &mut img.tiles[tile_index];
                    let comp = &mut tile.comps[plane_number];
                    let band = &mut comp.subs[0];
                    let band_cells = band.width as usize;
                    let band_start = band.band_start;
                    comp.arena[band_start..band_start + band_cells].fill(0);
                    return Ok(());
                }

                for i in 0..tile_height {
                    let line = {
                        let tile = &mut img.tiles[tile_index];
                        let comp = &mut tile.comps[plane_number];
                        let band = &mut comp.subs[0];
                        let band_cells = band.width as usize;
                        let band_start = band.band_start;
                        let Some(param) = band.band_param.as_mut() else {
                            bail!("the CR3 band misses its decode state");
                        };
                        param.decode_band_line(&mut comp.arena[band_start..band_start + band_cells])?;
                        comp.arena[band_start..band_start + band_cells].to_vec()
                    };
                    crx_convert_plane_line(img, image, image_row + i, image_col, plane_number, Some((&line, tile_width)))?;
                }
            }
            image_col += tile_width;
        }
        image_row += i32::from(img.tiles[t_row * usize::from(img.tile_cols)].height);
    }

    Ok(())
}

/// `crxReadImageHeaders`: the tiles' sizes and compressed records walked
/// through the mdat header, each component's subband headers parsed, and
/// for the wide header form the tile's quantization stream decoded.
fn crx_read_image_headers(img: &mut CrxImage<'_>, hdr_buf: &[u8], file_data: &[u8], hdr: &CrxHeader) -> Result<()> {
    let n_tiles = i32::from(img.tile_rows) * i32::from(img.tile_cols);
    let version = hdr.version;
    let comp_count = usize::from(img.n_planes);
    let subband_count = usize::from(img.subband_count);
    if n_tiles == 0 {
        bail!("the CR3 image misses its tiles");
    }

    // the tiles' dims, neighbour flags and their components and bands
    for cur_tile in 0..n_tiles as usize {
        let mut tile_flag = 0u8;
        let tile_width = if (cur_tile as i32 + 1) % i32::from(img.tile_cols) != 0 {
            // not the last tile in a tile row
            if i32::from(img.tile_cols) > 1 {
                tile_flag = CRX_TILES_ON_THE_RIGHT;
                if cur_tile as i32 % i32::from(img.tile_cols) != 0 {
                    // not the first tile in tile row
                    tile_flag |= CRX_TILES_ON_THE_LEFT;
                }
            }
            hdr.tile_width as u16
        } else {
            // last tile in a tile row
            if i32::from(img.tile_cols) > 1 {
                tile_flag = CRX_TILES_ON_THE_LEFT;
            }
            let last_width = (img.plane_width as u32)
                .wrapping_sub(hdr.tile_width.wrapping_mul(i32::from(img.tile_cols).wrapping_sub(1) as u32));
            last_width as u16
        };
        if (cur_tile as i32) < n_tiles - i32::from(img.tile_cols) {
            // in first tile row
            if i32::from(img.tile_rows) > 1 {
                tile_flag |= CRX_TILES_ON_THE_BOTTOM;
                if cur_tile as i32 >= i32::from(img.tile_cols) {
                    tile_flag |= CRX_TILES_ON_THE_TOP;
                }
            }
        } else {
            // non first tile row
            if i32::from(img.tile_rows) > 1 {
                tile_flag |= CRX_TILES_ON_THE_TOP;
            }
        }
        let tile_height = if (cur_tile as i32) < n_tiles - i32::from(img.tile_cols) {
            hdr.tile_height as u16
        } else {
            let last_height = (img.plane_height as u32)
                .wrapping_sub(hdr.tile_height.wrapping_mul(i32::from(img.tile_rows).wrapping_sub(1) as u32));
            last_height as u16
        };

        let mut comps = Vec::with_capacity(comp_count);
        for cur_comp in 0..comp_count {
            let mut comp = CrxPlaneComp::new();
            comp.comp_number = cur_comp as i32;
            comp.supports_partial = true;
            comp.tile_flag = tile_flag;
            comp.subs = (0..subband_count)
                .map(|_| {
                    let mut band = CrxSubband::new();
                    band.supports_partial = false;
                    band.q_param = 4;
                    band
                })
                .collect();
            comps.push(comp);
        }
        let mut tile = CrxTile::new();
        tile.tile_flag = tile_flag;
        tile.width = tile_width;
        tile.height = tile_height;
        tile.comps = comps;
        img.tiles.push(tile);
    }

    // the header walk: each tile's records and their subbands
    let mut tile_offset = 0u32;
    let mut data_size = hdr.mdat_header_size as i32;
    let mut cursor = 0usize;

    for cur_tile in 0..n_tiles as usize {
        if data_size < 4 {
            bail!("the CR3 tile header data is short");
        }

        let hdr_sign = crx_sget_n(hdr_buf, cursor, 2);
        let hdr_size = crx_sget_n(hdr_buf, cursor + 2, 2);
        if (hdr_sign != 0xFF01 || hdr_size != 8) && (hdr_sign != 0xFF11 || (hdr_size != 8 && hdr_size != 16)) {
            bail!("the CR3 tile header mark is unexpected");
        }
        if data_size < hdr_size as i32 + 4 {
            bail!("the CR3 tile header data is short");
        }
        let tail_sign = crx_sget_n(hdr_buf, cursor + 10, 2);
        if (hdr_size == 8 && tail_sign != 0) || (hdr_size == 16 && tail_sign != 0x4000) {
            bail!("the CR3 tile header terminator is unexpected");
        }
        if crx_sget_n(hdr_buf, cursor + 8, 2) != cur_tile as u32 {
            bail!("the CR3 tile header order is unexpected");
        }

        data_size -= hdr_size as i32 + 4;

        let tile_size = crx_sget_n(hdr_buf, cursor + 4, 4);
        if hdr_size == 16 {
            // extended header data - terminated by 0 bytes
            if crx_sget_n(hdr_buf, cursor + 18, 2) != 0 {
                bail!("the CR3 tile header terminator is unexpected");
            }
            let q_p_data_size = crx_sget_n(hdr_buf, cursor + 12, 4);
            let extra_size = crx_sget_n(hdr_buf, cursor + 16, 2) as u16;
            let tile = &mut img.tiles[cur_tile];
            tile.tile_size = tile_size;
            tile.data_offset = i64::from(tile_offset);
            tile.has_q_p_data = true;
            tile.mdat_q_p_data_size = q_p_data_size;
            tile.mdat_extra_size = extra_size;
        } else {
            let tile = &mut img.tiles[cur_tile];
            tile.tile_size = tile_size;
            tile.data_offset = i64::from(tile_offset);
            tile.has_q_p_data = false;
            tile.mdat_q_p_data_size = 0;
            tile.mdat_extra_size = 0;
        }

        cursor += hdr_size as usize + 4;
        tile_offset = tile_offset.wrapping_add(tile_size);

        let mut comp_offset = 0u32;

        for comp_num in 0..comp_count {
            if data_size < 0xC {
                bail!("the CR3 component header data is short");
            }
            let hdr_sign = crx_sget_n(hdr_buf, cursor, 2);
            let hdr_size = crx_sget_n(hdr_buf, cursor + 2, 2);
            if (hdr_sign != 0xFF02 && hdr_sign != 0xFF12) || hdr_size != 8 {
                bail!("the CR3 component header mark is unexpected");
            }
            if comp_num as u32 != u32::from(hdr_buf[cursor + 8]) >> 4 {
                bail!("the CR3 component header order is unexpected");
            }
            if crx_sget_n(hdr_buf, cursor + 9, 3) != 0 {
                bail!("the CR3 component header tail is unexpected");
            }

            let comp_size = crx_sget_n(hdr_buf, cursor + 4, 4);

            let comp_hdr_rounded_bits = (u32::from(hdr_buf[cursor + 8]) >> 1) & 3;
            let supports_partial = u32::from(hdr_buf[cursor + 8]) & 8 != 0;

            data_size -= 0xC;
            cursor += 0xC;

            {
                let tile = &mut img.tiles[cur_tile];
                let comp = &mut tile.comps[comp_num];
                comp.comp_size = comp_size as i32;
                comp.supports_partial = supports_partial;
                comp.data_offset = i64::from(comp_offset);
                comp.tile_flag = tile.tile_flag;
                comp.rounded_bits_mask = if comp_hdr_rounded_bits != 0 {
                    if img.levels != 0 || !supports_partial {
                        bail!("the CR3 component rounding is unexpected");
                    }
                    1i32 << (comp_hdr_rounded_bits as i32 - 1)
                } else {
                    0
                };
            }

            comp_offset = comp_offset.wrapping_add(comp_size);

            crx_read_subband_headers(subband_count, &mut img.tiles[cur_tile].comps[comp_num], hdr_buf, &mut cursor, &mut data_size)?;
            crx_process_subbands(version, img, cur_tile, comp_num)?;
        }
    }

    if version != 0x200 {
        return Ok(());
    }

    for cur_tile in 0..n_tiles as usize {
        let has_q_p_data = img.tiles[cur_tile].has_q_p_data;
        if !has_q_p_data {
            continue;
        }
        let (q_p_data_size, tile_data_offset) = {
            let tile = &img.tiles[cur_tile];
            (i64::from(tile.mdat_q_p_data_size), tile.data_offset)
        };

        let mut bit_stream = CrxBitstream::new(
            file_data,
            q_p_data_size,
            img.mdat_offset.wrapping_add(tile_data_offset as u64) as i64,
        );
        // The engine's fill result is ignored here; the rows decode until
        // the stream runs out.
        let _ = bit_stream.fill_buffer();

        let (tile_width, tile_height) = {
            let tile = &img.tiles[cur_tile];
            (i32::from(tile.width), i32::from(tile.height))
        };
        let qp_width = (tile_width >> 3) + i32::from((tile_width & 7) != 0);
        let qp_height = (tile_height >> 1) + i32::from(tile_height & 1);
        let total_qp = qp_height as u64 * qp_width as u64;

        // 2 lines padded with extra pixels at the start and at the end
        let table_cells = total_qp + 2 * (qp_width + 2) as u64;
        let mut qp_table = vec![0i32; table_cells as usize];
        let qp_line_buf = total_qp as usize;
        let mut k_param = 0i32;
        let mut qp_cursor = 0usize;
        for qp_row in 0..qp_height {
            let (line0, line1) = if qp_row & 1 != 0 {
                (qp_line_buf + (qp_width + 2) as usize, qp_line_buf)
            } else {
                (qp_line_buf, qp_line_buf + (qp_width + 2) as usize)
            };

            if qp_row != 0 {
                crx_decode_golomb_normal(&mut bit_stream, qp_width, &mut qp_table, line0, line1, &mut k_param)?;
            } else {
                crx_decode_golomb_top(&mut bit_stream, qp_width, &mut qp_table, line1, &mut k_param)?;
            }

            for qp_col in 0..qp_width {
                qp_table[qp_cursor] = qp_table[line1 + (qp_col + 1) as usize] + 4;
                qp_cursor += 1;
            }
        }

        // now we read QP data - build tile QStep
        let levels = img.levels;
        crx_make_q_step(&mut img.tiles[cur_tile], levels, &qp_table)?;
    }

    Ok(())
}

/// `crxSetupImageData`: the image's plane facts, the encoding type 3's
/// intermediate plane store and the planes' Bayer positions, then the
/// tile headers.
fn crx_setup_image_data(hdr: &CrxHeader, img: &mut CrxImage<'_>, mdat_offset: u64, hdr_buf: &[u8], file_data: &[u8]) -> Result<()> {
    let incr_bit = INCR_BIT_TABLE[(4 * hdr.enc_type + 2) as usize];

    img.plane_width = hdr.f_width as u16;
    img.plane_height = hdr.f_height as u16;

    if hdr.tile_width < 0x16 || hdr.tile_height < 0x16 || i32::from(img.plane_width) > 0x7FFF || i32::from(img.plane_height) > 0x7FFF {
        bail!("the CR3 plane dims are out of range");
    }

    img.tile_cols = ((img.plane_width as u32).wrapping_add(hdr.tile_width).wrapping_sub(1) / hdr.tile_width) as u8;
    img.tile_rows = ((img.plane_height as u32).wrapping_add(hdr.tile_height).wrapping_sub(1) / hdr.tile_height) as u8;

    if i32::from(img.tile_cols) > 0xFF
        || i32::from(img.tile_rows) > 0xFF
        || (img.plane_width as u32)
            .wrapping_sub(hdr.tile_width.wrapping_mul(i32::from(img.tile_cols).wrapping_sub(1) as u32))
            < 0x16
        || (img.plane_height as u32)
            .wrapping_sub(hdr.tile_height.wrapping_mul(i32::from(img.tile_rows).wrapping_sub(1) as u32))
            < 0x16
    {
        bail!("the CR3 tile dims are out of range");
    }

    img.levels = hdr.image_levels as u8;
    // 3 bands per level + one last LL
    img.subband_count = (3 * u32::from(img.levels) + 1) as u8;
    img.n_planes = hdr.n_planes as u8;
    img.n_bits = hdr.n_bits as u8;
    img.enc_type = hdr.enc_type as u8;
    img.sample_precision = (hdr.n_bits as i32).wrapping_add(incr_bit).wrapping_add(1) as u8;
    img.mdat_offset = mdat_offset.wrapping_add(u64::from(hdr.mdat_header_size));
    img.median_bits = hdr.median_bits as u8;
    img.plane_buf = None;
    img.out_bufs = [0; 4];

    // The encoding type 3 needs all 4 planes to be decoded to generate row
    // of RGGB values.
    if img.enc_type == 3 && img.n_planes == 4 && img.n_bits > 8 {
        let store_bytes = i32::from(img.plane_height)
            .wrapping_mul(i32::from(img.plane_width))
            .wrapping_mul(i32::from(img.n_planes))
            .wrapping_mul((i32::from(img.sample_precision) + 7) >> 3);
        if store_bytes <= 0 {
            bail!("the CR3 plane store is too large");
        }
        img.plane_buf = Some(vec![0i16; (store_bytes >> 1) as usize]);
    }

    let row_size = 2usize * usize::from(img.plane_width);

    if img.n_planes == 1 {
        img.out_bufs[0] = 0;
    } else {
        match hdr.cfa_layout {
            0 => {
                // R G
                // G B
                img.out_bufs[0] = 0;
                img.out_bufs[1] = 1;
                img.out_bufs[2] = row_size;
                img.out_bufs[3] = img.out_bufs[2] + 1;
            }
            1 => {
                // G R
                // B G
                img.out_bufs[1] = 0;
                img.out_bufs[0] = 1;
                img.out_bufs[3] = row_size;
                img.out_bufs[2] = img.out_bufs[3] + 1;
            }
            2 => {
                // G B
                // R G
                img.out_bufs[2] = 0;
                img.out_bufs[3] = 1;
                img.out_bufs[0] = row_size;
                img.out_bufs[1] = img.out_bufs[0] + 1;
            }
            3 => {
                // B G
                // G R
                img.out_bufs[3] = 0;
                img.out_bufs[2] = 1;
                img.out_bufs[1] = row_size;
                img.out_bufs[0] = img.out_bufs[1] + 1;
            }
            _ => bail!("the CR3 CFA layout is out of range"),
        }
    }

    // read header
    crx_read_image_headers(img, hdr_buf, file_data, hdr)
}

/// The CR3 codec's decode entry: the selected track's header walks into the
/// image facts and the wavelet decoder runs plane by plane.
fn canon_crx_load_raw(meta: &mut CameraMeta, raw_image: &mut [u16], stream: &mut Stream, data_error: &mut bool) -> Result<()> {
    // the header, halved for the four-plane colour layout
    let mut hdr = CrxHeader {
        version: meta.crx.version,
        f_width: meta.crx.f_width,
        f_height: meta.crx.f_height,
        tile_width: meta.crx.tile_width,
        tile_height: meta.crx.tile_height,
        n_planes: meta.crx.n_planes,
        cfa_layout: meta.crx.cfa_layout,
        enc_type: meta.crx.enc_type,
        image_levels: meta.crx.image_levels,
        n_bits: meta.crx.n_bits,
        median_bits: meta.crx.median_bits,
        mdat_header_size: meta.crx.mdat_header_size,
    };
    if i64::from(hdr.mdat_header_size) > meta.data_size {
        // the soft mark only; the decode still runs to the next error
        derror(stream, data_error)?;
    }

    // update sizes for the planes
    if hdr.n_planes == 4 {
        hdr.f_width >>= 1;
        hdr.f_height >>= 1;
        hdr.tile_width >>= 1;
        hdr.tile_height >>= 1;
    }

    meta.maximum = 1u32.wrapping_shl(hdr.n_bits).wrapping_sub(1);

    let file_data = stream.data();
    let mut hdr_buf = vec![0u8; hdr.mdat_header_size as usize];
    let bytes = stream.read(&mut hdr_buf);
    if bytes != hdr.mdat_header_size as usize {
        bail!("the CR3 track header data is truncated");
    }

    let mut img = CrxImage {
        tiles: Vec::new(),
        n_planes: 0,
        plane_width: 0,
        plane_height: 0,
        sample_precision: 0,
        median_bits: 0,
        subband_count: 0,
        levels: 0,
        n_bits: 0,
        enc_type: 0,
        tile_cols: 0,
        tile_rows: 0,
        mdat_offset: 0,
        out_bufs: [0; 4],
        plane_buf: None,
    };

    // parse and setup the image data
    crx_setup_image_data(&hdr, &mut img, meta.data_offset as u64, &hdr_buf, file_data)?;

    for plane in 0..usize::from(img.n_planes) {
        let result = crx_decode_plane(&mut img, raw_image, plane, file_data);
        if result.is_err() {
            derror(stream, data_error)?;
        }
    }

    if img.enc_type == 3 {
        for image_row in 0..i32::from(img.plane_height) {
            crx_convert_plane_line(&mut img, raw_image, image_row, 0, 0, None)?;
        }
    }
    Ok(())
}

// ---- Fuji compressed RAF ----

/// The line buffer holds 18 u16 rows of `line_width + 2` cells: five read
/// rows, eight green rows (the X-Trans body decodes two green lines per
/// line group) and five blue rows.
const FUJI_LINES: usize = 18;
/// The line-buffer rows, indexed as the engine's `_R0`..`_R4`, `_G0`..`_G7`
/// and `_B0`..`_B4` pointers.
const FUJI_R0: usize = 0;
const FUJI_R2: usize = 2;
const FUJI_R3: usize = 3;
const FUJI_R4: usize = 4;
const FUJI_G0: usize = 5;
const FUJI_G2: usize = 7;
const FUJI_G3: usize = 8;
const FUJI_G4: usize = 9;
const FUJI_G5: usize = 10;
const FUJI_G6: usize = 11;
const FUJI_G7: usize = 12;
const FUJI_B0: usize = 13;
const FUJI_B2: usize = 15;
const FUJI_B3: usize = 16;
const FUJI_B4: usize = 17;
const FUJI_BUF_SIZE: usize = 0x10000;

/// One gradient's prediction history: `value1` accumulates absolute
/// prediction corrections and `value2` counts decodes since the pair was
/// halved.
#[derive(Clone, Copy)]
struct FujiGrad {
    value1: i32,
    value2: i32,
}

/// The gradient history a set of line decodes shares: the main table's 41
/// gradients plus the three finer lossy tables' 5-entry slices. Cells the
/// decode never selects keep their initial `{max_diff, 1}`.
#[derive(Clone, Copy)]
struct FujiGrads {
    grads: [FujiGrad; 41],
    lossy_grads: [[FujiGrad; 5]; 3],
}

impl FujiGrads {
    /// Every cell untouched by a decode keeps the engine's `{max_diff, 1}`
    /// seed.
    fn seeded(max_diff: i32) -> Self {
        FujiGrads {
            grads: [FujiGrad { value1: max_diff, value2: 1 }; 41],
            lossy_grads: [[FujiGrad { value1: max_diff, value2: 1 }; 5]; 3],
        }
    }
}

/// One quantization table: the prediction lookup, the base the decoded
/// values wrap around, its gradient thresholds and the code-shaping facts.
#[derive(Clone, Default)]
struct FujiQTable {
    q_table: Vec<i8>,
    q_base: i32,
    max_grad: i32,
    total_values: i32,
    raw_bits: i32,
    q_grad_mult: i32,
}

/// The Fuji compressed-stream facts one decode shares: the four tables and
/// the stream-shaping values derived from the file's bit depth and rows
/// kind.
#[derive(Clone, Default)]
struct FujiParams {
    qt: [FujiQTable; 4],
    max_bits: i32,
    min_value: i32,
    max_value: i32,
    line_width: usize,
}

/// One block's bit reader, its line buffer and gradient history. The
/// engine keeps the stream inside the block record and slides the refill
/// window forward as the decode consumes bytes.
struct FujiBlock<'data, 'stream> {
    cur_bit: i32,
    cur_pos: usize,
    cur_buf_offset: i64,
    max_read_size: u32,
    cur_buf_size: usize,
    cur_buf: Vec<u8>,
    fillbytes: i32,
    stream: &'stream mut Stream<'data>,
    even: [FujiGrads; 3],
    odd: [FujiGrads; 3],
    linebuf: Vec<u16>,
    line_span: usize,
}

impl<'data, 'stream> FujiBlock<'data, 'stream> {
    /// `fuji_fill_buffer`: slide the refill window forward and top the
    /// buffer up, or lay down zero fill when the file's raw data ran out
    /// (a short last block) before ending the stream.
    fn fill_buffer(&mut self) -> Result<()> {
        if self.cur_pos >= self.cur_buf_size {
            self.cur_pos = 0;
            self.cur_buf_offset += self.cur_buf_size as i64;
            self.stream.seek(self.cur_buf_offset.max(0) as u64);
            let want = self.max_read_size.min(FUJI_BUF_SIZE as u32) as usize;
            self.cur_buf_size = self.stream.read(&mut self.cur_buf[..want]);
            if self.cur_buf_size < 1 {
                if self.fillbytes > 0 {
                    let lay_size = self.fillbytes.clamp(1, FUJI_BUF_SIZE as i32) as usize;
                    self.cur_buf[..lay_size].fill(0);
                    self.fillbytes -= lay_size as i32;
                } else {
                    bail!("the Fuji compressed raw stream ends early");
                }
            }
            self.max_read_size = self.max_read_size.wrapping_sub(self.cur_buf_size as u32);
        }
        Ok(())
    }

    /// `fuji_zerobits`: read unary-coded zero bits until the set bit.
    fn zerobits(&mut self) -> Result<i32> {
        let mut count = 0;
        loop {
            let zero = (self.cur_buf[self.cur_pos] >> (7 - self.cur_bit)) & 1;
            self.cur_bit += 1;
            self.cur_bit &= 7;
            if self.cur_bit == 0 {
                self.cur_pos += 1;
                self.fill_buffer()?;
            }
            if zero != 0 {
                break;
            }
            count += 1;
        }
        Ok(count)
    }

    /// `fuji_read_code`: read `bits_to_read` big-endian-coded bits.
    fn read_code(&mut self, bits_to_read: i32) -> Result<i32> {
        if bits_to_read == 0 {
            return Ok(0);
        }
        let mut value = 0;
        let mut bits_left: i32 = bits_to_read;
        let mut bits_left_in_byte: i32 = 8 - (self.cur_bit & 7);
        if bits_left >= bits_left_in_byte {
            loop {
                let whole_mask: i32 = (1i32 << bits_left_in_byte) - 1;
                value = (value << bits_left_in_byte)
                    | (i32::from(self.cur_buf[self.cur_pos]) & whole_mask);
                bits_left -= bits_left_in_byte;
                self.cur_pos += 1;
                self.fill_buffer()?;
                bits_left_in_byte = 8;
                if bits_left < 8 {
                    break;
                }
            }
        }
        if bits_left == 0 {
            self.cur_bit = (8 - (bits_left_in_byte & 7)) & 7;
            return Ok(value);
        }
        value <<= bits_left;
        bits_left_in_byte -= bits_left;
        value |= ((1 << bits_left) - 1) & ((self.cur_buf[self.cur_pos] >> bits_left_in_byte) as i32);
        self.cur_bit = (8 - (bits_left_in_byte & 7)) & 7;
        Ok(value)
    }

    /// `fuji_decode_sample_even`: decode one sample whose context reads the
    /// rows above the line, fold the prediction correction into the
    /// gradient history and store it.
    fn decode_sample_even(
        &mut self,
        params: &FujiParams,
        line: usize,
        pos: i32,
        slot: usize,
    ) -> Result<i32> {
        let line_width = params.line_width;
        let cur = line * self.line_span + 1 + pos as usize;
        let rb = i32::from(self.linebuf[cur - 2 - line_width]);
        let rc = i32::from(self.linebuf[cur - 3 - line_width]);
        let rd = i32::from(self.linebuf[cur - 1 - line_width]);
        let rf = i32::from(self.linebuf[cur - 4 - 2 * line_width]);
        let diff_rc_rb = (rc - rb).abs();
        let diff_rf_rb = (rf - rb).abs();
        let diff_rd_rb = (rd - rb).abs();

        // A lossy stream picks one of the fine tables while its gradient
        // sum stays under that table's threshold; the lossless decode
        // always ends on the main table.
        let mut qt_idx = 0usize;
        let mut lossy_idx = None;
        let mut table_index = 1usize;
        while table_index < 4 && params.qt[0].q_base >= table_index as i32 {
            if diff_rf_rb + diff_rc_rb <= params.qt[table_index].max_grad {
                qt_idx = table_index;
                lossy_idx = Some(table_index - 1);
                break;
            }
            table_index += 1;
        }
        let table = &params.qt[qt_idx];
        let grad = table.q_grad_mult
            * i32::from(table.q_table[(params.max_value + (rb - rf)) as usize])
            + i32::from(table.q_table[(params.max_value + (rc - rb)) as usize]);
        let gradient = grad.unsigned_abs() as usize;

        let mut interp_val = if diff_rc_rb > diff_rf_rb && diff_rc_rb > diff_rd_rb {
            rf + rd + 2 * rb
        } else if diff_rd_rb > diff_rc_rb && diff_rd_rb > diff_rf_rb {
            rf + rc + 2 * rb
        } else {
            rd + rc + 2 * rb
        };

        let sample = self.zerobits()?;
        let mut code;
        if sample < params.max_bits - table.raw_bits - 1 {
            let (history_low, history_high) = self.grads_pair(false, slot, lossy_idx, gradient);
            let dec_bits = fuji_bit_diff(history_low, history_high);
            code = self.read_code(dec_bits)?;
            code += sample << dec_bits;
        } else {
            code = self.read_code(table.raw_bits)?;
            code += 1;
        }

        let mut errcnt = 0;
        if code < 0 || code >= table.total_values {
            errcnt += 1;
        }
        if (code & 1) != 0 {
            code = -1 - code / 2;
        } else {
            code /= 2;
        }
        self.grads_update(false, slot, lossy_idx, gradient, code.abs(), params.min_value);
        let correction = code * (2 * table.q_base + 1);
        if grad < 0 {
            interp_val = (interp_val >> 2) - correction;
        } else {
            interp_val = (interp_val >> 2) + correction;
        }
        let wrap_step = table.total_values * (2 * table.q_base + 1);
        if interp_val < -table.q_base {
            interp_val += wrap_step;
        } else if interp_val > table.q_base + params.max_value {
            interp_val -= wrap_step;
        }
        self.linebuf[cur] = if interp_val >= 0 {
            interp_val.min(params.max_value) as u16
        } else {
            0
        };
        Ok(errcnt)
    }

    /// `fuji_decode_sample_odd`: the odd-position variant, whose context
    /// also reads this row's own neighbours (`Ra`, `Rg`) and applies the
    /// prediction correction without the interpolation shift.
    fn decode_sample_odd(
        &mut self,
        params: &FujiParams,
        line: usize,
        pos: i32,
        slot: usize,
    ) -> Result<i32> {
        let line_width = params.line_width;
        let cur = line * self.line_span + 1 + pos as usize;
        let ra = i32::from(self.linebuf[cur - 1]);
        let rb = i32::from(self.linebuf[cur - 2 - line_width]);
        let rc = i32::from(self.linebuf[cur - 3 - line_width]);
        let rd = i32::from(self.linebuf[cur - 1 - line_width]);
        let rg = i32::from(self.linebuf[cur + 1]);
        let diff_rc_ra = (rc - ra).abs();
        let diff_rb_rc = (rb - rc).abs();

        let mut qt_idx = 0usize;
        let mut lossy_idx = None;
        let mut table_index = 1usize;
        while table_index < 4 && params.qt[0].q_base >= table_index as i32 {
            if diff_rb_rc + diff_rc_ra <= params.qt[table_index].max_grad {
                qt_idx = table_index;
                lossy_idx = Some(table_index - 1);
                break;
            }
            table_index += 1;
        }
        let table = &params.qt[qt_idx];
        let grad = table.q_grad_mult
            * i32::from(table.q_table[(params.max_value + (rb - rc)) as usize])
            + i32::from(table.q_table[(params.max_value + (rc - ra)) as usize]);
        let gradient = grad.unsigned_abs() as usize;

        let mut interp_val = if (rb > rc && rb > rd) || (rb < rc && rb < rd) {
            (rg + ra + 2 * rb) >> 2
        } else {
            (ra + rg) >> 1
        };

        let sample = self.zerobits()?;
        let mut code;
        if sample < params.max_bits - table.raw_bits - 1 {
            let (history_low, history_high) = self.grads_pair(true, slot, lossy_idx, gradient);
            let dec_bits = fuji_bit_diff(history_low, history_high);
            code = self.read_code(dec_bits)?;
            code += sample << dec_bits;
        } else {
            code = self.read_code(table.raw_bits)?;
            code += 1;
        }

        let mut errcnt = 0;
        if code < 0 || code >= table.total_values {
            errcnt += 1;
        }
        if (code & 1) != 0 {
            code = -1 - code / 2;
        } else {
            code /= 2;
        }
        self.grads_update(true, slot, lossy_idx, gradient, code.abs(), params.min_value);
        let correction = code * (2 * table.q_base + 1);
        if grad < 0 {
            interp_val -= correction;
        } else {
            interp_val += correction;
        }
        let wrap_step = table.total_values * (2 * table.q_base + 1);
        if interp_val < -table.q_base {
            interp_val += wrap_step;
        } else if interp_val > table.q_base + params.max_value {
            interp_val -= wrap_step;
        }
        self.linebuf[cur] = if interp_val >= 0 {
            interp_val.min(params.max_value) as u16
        } else {
            0
        };
        Ok(errcnt)
    }

    /// One gradient history cell's current pair, from the slot the line
    /// group decodes with.
    fn grads_pair(
        &self,
        odd: bool,
        slot: usize,
        lossy_idx: Option<usize>,
        gradient: usize,
    ) -> (i32, i32) {
        let grads = if odd { &self.odd[slot] } else { &self.even[slot] };
        let history = match lossy_idx {
            Some(table) => &grads.lossy_grads[table][..],
            None => &grads.grads[..],
        };
        (history[gradient].value1, history[gradient].value2)
    }

    /// `grads[gradient]` decode tail: accumulate the absolute correction,
    /// halve the pair once its counter reaches the minimum value, then let
    /// the counter run on.
    fn grads_update(
        &mut self,
        odd: bool,
        slot: usize,
        lossy_idx: Option<usize>,
        gradient: usize,
        code_abs: i32,
        min_value: i32,
    ) {
        let grads = if odd { &mut self.odd[slot] } else { &mut self.even[slot] };
        let entry = match lossy_idx {
            Some(table) => &mut grads.lossy_grads[table][gradient],
            None => &mut grads.grads[gradient],
        };
        entry.value1 += code_abs;
        if entry.value2 == min_value {
            entry.value1 >>= 1;
            entry.value2 >>= 1;
        }
        entry.value2 += 1;
    }

    /// `fuji_decode_interpolation_even`: fill one sample by averaging the
    /// rows above — the same prediction the decode's correction folds into.
    fn decode_interpolation_even(&mut self, line_width: usize, line: usize, pos: i32) {
        let cur = line * self.line_span + 1 + pos as usize;
        let rb = i32::from(self.linebuf[cur - 2 - line_width]);
        let rc = i32::from(self.linebuf[cur - 3 - line_width]);
        let rd = i32::from(self.linebuf[cur - 1 - line_width]);
        let rf = i32::from(self.linebuf[cur - 4 - 2 * line_width]);
        let diff_rc_rb = (rc - rb).abs();
        let diff_rf_rb = (rf - rb).abs();
        let diff_rd_rb = (rd - rb).abs();
        let average = if diff_rc_rb > diff_rf_rb && diff_rc_rb > diff_rd_rb {
            (rf + rd + 2 * rb) >> 2
        } else if diff_rd_rb > diff_rc_rb && diff_rd_rb > diff_rf_rb {
            (rf + rc + 2 * rb) >> 2
        } else {
            (rd + rc + 2 * rb) >> 2
        };
        self.linebuf[cur] = average as u16;
    }

    /// `fuji_extend_generic` over the red/green/blue line groups: carry the
    /// rows above's first and last data cells into the group's edge cells.
    fn extend_generic(&mut self, line_width: usize, start: usize, end: usize) {
        for line in start..=end {
            let line_cell = line * self.line_span;
            let prev_cell = (line - 1) * self.line_span;
            self.linebuf[line_cell] = self.linebuf[prev_cell + 1];
            self.linebuf[line_cell + line_width + 1] = self.linebuf[prev_cell + line_width];
        }
    }

    fn extend_red(&mut self, line_width: usize) {
        self.extend_generic(line_width, FUJI_R2, FUJI_R4);
    }

    fn extend_green(&mut self, line_width: usize) {
        self.extend_generic(line_width, FUJI_G2, FUJI_G7);
    }

    fn extend_blue(&mut self, line_width: usize) {
        self.extend_generic(line_width, FUJI_B2, FUJI_B4);
    }

    /// `xtrans_decode_block`: the X-Trans line-group decode — six phases,
    /// each decoding two rows of samples (several of them pure
    /// interpolations) and extending their line edges.
    fn xtrans_decode_block(&mut self, params: &FujiParams) -> Result<i32> {
        let line_width = params.line_width;
        let width = line_width as i32;
        let mut r_even_pos: i32 = 0;
        let mut r_odd_pos: i32 = 1;
        let mut g_even_pos: i32 = 0;
        let mut g_odd_pos: i32 = 1;
        let mut b_even_pos: i32 = 0;
        let mut b_odd_pos: i32 = 1;
        let mut errcnt: i32 = 0;

        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                self.decode_interpolation_even(line_width, FUJI_R2, r_even_pos);
                r_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_G2, g_even_pos, 0)?;
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R2, r_odd_pos, 0)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G2, g_odd_pos, 0)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_G3, g_even_pos, 1)?;
                g_even_pos += 2;
                self.decode_interpolation_even(line_width, FUJI_B2, b_even_pos);
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G3, g_odd_pos, 1)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B2, b_odd_pos, 1)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);

        r_even_pos = 0;
        r_odd_pos = 1;
        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                if (r_even_pos & 3) != 0 {
                    errcnt += self.decode_sample_even(params, FUJI_R3, r_even_pos, 2)?;
                } else {
                    self.decode_interpolation_even(line_width, FUJI_R3, r_even_pos);
                }
                r_even_pos += 2;
                self.decode_interpolation_even(line_width, FUJI_G4, g_even_pos);
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R3, r_odd_pos, 2)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G4, g_odd_pos, 2)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        b_even_pos = 0;
        b_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_G5, g_even_pos, 0)?;
                g_even_pos += 2;
                if (b_even_pos & 3) == 2 {
                    self.decode_interpolation_even(line_width, FUJI_B3, b_even_pos);
                } else {
                    errcnt += self.decode_sample_even(params, FUJI_B3, b_even_pos, 0)?;
                }
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G5, g_odd_pos, 0)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B3, b_odd_pos, 0)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);

        r_even_pos = 0;
        r_odd_pos = 1;
        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                if (r_even_pos & 3) == 2 {
                    self.decode_interpolation_even(line_width, FUJI_R4, r_even_pos);
                } else {
                    errcnt += self.decode_sample_even(params, FUJI_R4, r_even_pos, 1)?;
                }
                r_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_G6, g_even_pos, 1)?;
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R4, r_odd_pos, 1)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G6, g_odd_pos, 1)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        b_even_pos = 0;
        b_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                self.decode_interpolation_even(line_width, FUJI_G7, g_even_pos);
                g_even_pos += 2;
                if (b_even_pos & 3) != 0 {
                    errcnt += self.decode_sample_even(params, FUJI_B4, b_even_pos, 2)?;
                } else {
                    self.decode_interpolation_even(line_width, FUJI_B4, b_even_pos);
                }
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G7, g_odd_pos, 2)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B4, b_odd_pos, 2)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);
        Ok(errcnt)
    }

    /// `fuji_bayer_decode_block`: the Bayer line-group decode — the same
    /// six phases with every sample decoded (no interpolations).
    fn bayer_decode_block(&mut self, params: &FujiParams) -> Result<i32> {
        let line_width = params.line_width;
        let width = line_width as i32;
        let mut r_even_pos: i32 = 0;
        let mut r_odd_pos: i32 = 1;
        let mut g_even_pos: i32 = 0;
        let mut g_odd_pos: i32 = 1;
        let mut b_even_pos: i32 = 0;
        let mut b_odd_pos: i32 = 1;
        let mut errcnt: i32 = 0;

        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_R2, r_even_pos, 0)?;
                r_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_G2, g_even_pos, 0)?;
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R2, r_odd_pos, 0)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G2, g_odd_pos, 0)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_G3, g_even_pos, 1)?;
                g_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_B2, b_even_pos, 1)?;
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G3, g_odd_pos, 1)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B2, b_odd_pos, 1)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);

        r_even_pos = 0;
        r_odd_pos = 1;
        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_R3, r_even_pos, 2)?;
                r_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_G4, g_even_pos, 2)?;
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R3, r_odd_pos, 2)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G4, g_odd_pos, 2)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        b_even_pos = 0;
        b_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_G5, g_even_pos, 0)?;
                g_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_B3, b_even_pos, 0)?;
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G5, g_odd_pos, 0)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B3, b_odd_pos, 0)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);

        r_even_pos = 0;
        r_odd_pos = 1;
        g_even_pos = 0;
        g_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_R4, r_even_pos, 1)?;
                r_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_G6, g_even_pos, 1)?;
                g_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_R4, r_odd_pos, 1)?;
                r_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_G6, g_odd_pos, 1)?;
                g_odd_pos += 2;
            }
        }
        self.extend_red(line_width);
        self.extend_green(line_width);

        g_even_pos = 0;
        g_odd_pos = 1;
        b_even_pos = 0;
        b_odd_pos = 1;
        while g_even_pos < width || g_odd_pos < width {
            if g_even_pos < width {
                errcnt += self.decode_sample_even(params, FUJI_G7, g_even_pos, 2)?;
                g_even_pos += 2;
                errcnt += self.decode_sample_even(params, FUJI_B4, b_even_pos, 2)?;
                b_even_pos += 2;
            }
            if g_even_pos > 8 {
                errcnt += self.decode_sample_odd(params, FUJI_G7, g_odd_pos, 2)?;
                g_odd_pos += 2;
                errcnt += self.decode_sample_odd(params, FUJI_B4, b_odd_pos, 2)?;
                b_odd_pos += 2;
            }
        }
        self.extend_green(line_width);
        self.extend_blue(line_width);
        Ok(errcnt)
    }

    /// `copy_line_to_xtrans`: move the decoded line triplets into the raw
    /// image at the X-Trans pattern's shades.
    fn copy_line_to_xtrans(
        &self,
        meta: &CameraMeta,
        raw_image: &mut [u16],
        cur_block: usize,
        cur_line: usize,
        cur_block_width: usize,
    ) {
        let line_span = self.line_span;
        let block_width = meta.fuji.block_width as usize;
        let raw_width = meta.raw_width as usize;
        let base = block_width * cur_block + 6 * raw_width * cur_line;
        for row in 0..6usize {
            let row_base = base + row * raw_width;
            for pixel in 0..cur_block_width {
                let shade = meta.xtrans_abs[row][pixel % 6];
                let line = match shade {
                    0 => (FUJI_R2 + (row >> 1)) * line_span,
                    2 => (FUJI_B2 + (row >> 1)) * line_span,
                    _ => (FUJI_G2 + row) * line_span,
                };
                let shade_offset = pixel % 3;
                let index = (((pixel as u32 * 2 / 3) & 0x7FFF_FFFE)
                    | ((shade_offset as u32) & 1))
                    + (shade_offset as u32 >> 1);
                raw_image[row_base + pixel] = self.linebuf[line + 1 + index as usize];
            }
        }
    }

    /// `copy_line_to_bayer`: move the decoded line triplets into the raw
    /// image at the filter pattern's own shades (the second green read as
    /// the first).
    fn copy_line_to_bayer(
        &self,
        meta: &CameraMeta,
        raw_image: &mut [u16],
        cur_block: usize,
        cur_line: usize,
        cur_block_width: usize,
    ) {
        let line_span = self.line_span;
        let block_width = meta.fuji.block_width as usize;
        let raw_width = meta.raw_width as usize;
        let mut fuji_bayer = [[0u32; 2]; 2];
        for row in 0..2u32 {
            for col in 0..2u32 {
                fuji_bayer[row as usize][col as usize] = fuji_fc(meta.filters, row, col);
            }
        }
        let base = block_width * cur_block + 6 * raw_width * cur_line;
        for row in 0..6usize {
            let row_base = base + row * raw_width;
            for pixel in 0..cur_block_width {
                let shade = fuji_bayer[row & 1][pixel & 1];
                let line = match shade {
                    0 => (FUJI_R2 + (row >> 1)) * line_span,
                    2 => (FUJI_B2 + (row >> 1)) * line_span,
                    _ => (FUJI_G2 + row) * line_span,
                };
                raw_image[row_base + pixel] = self.linebuf[line + 1 + (pixel >> 1)];
            }
        }
    }
}

/// Missing line-buffer rows the slide table needs.
const FUJI_R1: usize = 1;
const FUJI_G1: usize = 6;
const FUJI_B1: usize = 14;

/// `mtable`: which decoded rows replace the two oldest ones of each colour
/// group after a line.
const FUJI_MTABLE: [(usize, usize); 6] = [
    (FUJI_R0, FUJI_R3),
    (FUJI_R1, FUJI_R4),
    (FUJI_G0, FUJI_G6),
    (FUJI_G1, FUJI_G7),
    (FUJI_B0, FUJI_B3),
    (FUJI_B1, FUJI_B4),
];
/// `ztable`: which line groups are zeroed (and re-edged) for the next line.
const FUJI_ZTABLE: [(usize, usize); 3] = [(FUJI_R2, 3), (FUJI_G2, 6), (FUJI_B2, 3)];

/// The Bayer engine's colour shade for a pixel position, reading the
/// filter pattern record directly.
fn fuji_fc(filters: u32, row: u32, col: u32) -> u32 {
    (filters >> ((((row << 1) & 14) + (col & 1)) << 1)) & 3
}

/// `bitDiff`: how many extra code bits the gradient history justifies.
fn fuji_bit_diff(value1: i32, value2: i32) -> i32 {
    let mut dec_bits: i32 = 0;
    if value2 < value1 {
        while dec_bits <= 14 {
            dec_bits += 1;
            if (value2 << dec_bits) >= value1 {
                break;
            }
        }
    }
    dec_bits
}

/// `log2ceil`
fn fuji_log2ceil(mut val: i32) -> i32 {
    let mut result = 0;
    if val != 0 {
        val -= 1;
        loop {
            result += 1;
            val >>= 1;
            if val == 0 {
                break;
            }
        }
    }
    result
}

/// `setup_qlut`: the prediction lookup — a signed input folds to the
/// -4..4 gradient bucket.
fn fuji_setup_qlut(q_table: &mut [i8], q_point: &[i32; 5]) {
    let mut cur_val = -q_point[4];
    let fill_len = (2 * q_point[4] + 1) as usize;
    for entry in q_table.iter_mut().take(fill_len) {
        *entry = if cur_val <= -q_point[3] {
            -4
        } else if cur_val <= -q_point[2] {
            -3
        } else if cur_val <= -q_point[1] {
            -2
        } else if cur_val < -q_point[0] {
            -1
        } else if cur_val <= q_point[0] {
            0
        } else if cur_val < q_point[1] {
            1
        } else if cur_val < q_point[2] {
            2
        } else if cur_val < q_point[3] {
            3
        } else {
            4
        };
        cur_val += 1;
    }
}

/// `init_main_qtable`: build the main quantization table around a base.
fn fuji_init_main_qtable(params: &mut FujiParams, q_base: i32) {
    let max_value = params.max_value;
    let max_val = max_value + 1;
    let mut qp = [0i32; 5];
    qp[0] = q_base;
    qp[1] = 3 * q_base + 0x12;
    qp[2] = 5 * q_base + 0x43;
    qp[3] = 7 * q_base + 0x114;
    qp[4] = max_value;
    if qp[1] >= max_val || qp[1] < q_base + 1 {
        qp[1] = q_base + 1;
    }
    if qp[2] < qp[1] || qp[2] >= max_val {
        qp[2] = qp[1];
    }
    if qp[3] < qp[2] || qp[3] >= max_val {
        qp[3] = qp[2];
    }
    fuji_setup_qlut(&mut params.qt[0].q_table, &qp);
    let main_table = &mut params.qt[0];
    main_table.q_base = q_base;
    main_table.max_grad = 0;
    main_table.total_values = (qp[4] + 2 * q_base) / (2 * q_base + 1) + 1;
    main_table.raw_bits = fuji_log2ceil(main_table.total_values);
    main_table.q_grad_mult = 9;
    params.max_bits = 4 * fuji_log2ceil(qp[4] + 1);
}

/// `init_fuji_compr`: the stream-shaping facts and the quantization
/// tables one decode shares.
fn fuji_make_params(lossless: bool, bits: u32, block_width: u32, raw_type: u32) -> FujiParams {
    let mut params = FujiParams::default();
    params.min_value = 0x40;
    params.max_value = (1i32 << bits) - 1;
    params.line_width = if raw_type == 16 {
        (block_width * 2 / 3) as usize
    } else {
        (block_width >> 1) as usize
    };

    let q_table_size = 2usize << bits;
    if lossless {
        // One table; the others stay zeroed and are never picked.
        params.qt[0].q_table = vec![0i8; q_table_size];
        params.qt[0].q_base = -1;
        fuji_init_main_qtable(&mut params, 0);
    } else {
        // Three fine tables; the main one is built per strip from the
        // strip's quantization base.
        let mut qp = [0i32; 5];
        qp[0] = 0;
        qp[4] = params.max_value;

        params.qt[1].q_table = vec![0i8; q_table_size];
        params.qt[1].q_base = 0;
        params.qt[1].max_grad = 5;
        params.qt[1].q_grad_mult = 3;
        params.qt[1].total_values = qp[4] + 1;
        params.qt[1].raw_bits = fuji_log2ceil(params.qt[1].total_values);
        qp[1] = if qp[4] >= 0x12 { 0x12 } else { qp[0] + 1 };
        qp[2] = if qp[4] >= 0x43 { 0x43 } else { qp[1] };
        qp[3] = if qp[4] >= 0x114 { 0x114 } else { qp[2] };
        fuji_setup_qlut(&mut params.qt[1].q_table, &qp);

        params.qt[2].q_table = vec![0i8; q_table_size];
        params.qt[2].q_base = 1;
        params.qt[2].max_grad = 6;
        params.qt[2].q_grad_mult = 3;
        params.qt[2].total_values = (qp[4] + 2) / 3 + 1;
        params.qt[2].raw_bits = fuji_log2ceil(params.qt[2].total_values);
        qp[0] = params.qt[2].q_base;
        qp[1] = if qp[4] >= 0x15 { 0x15 } else { qp[0] + 1 };
        qp[2] = if qp[4] >= 0x48 { 0x48 } else { qp[1] };
        qp[3] = if qp[4] >= 0x11B { 0x11B } else { qp[2] };
        fuji_setup_qlut(&mut params.qt[2].q_table, &qp);

        params.qt[3].q_table = vec![0i8; q_table_size];
        params.qt[3].q_base = 2;
        params.qt[3].max_grad = 7;
        params.qt[3].q_grad_mult = 3;
        params.qt[3].total_values = (qp[4] + 4) / 5 + 1;
        params.qt[3].raw_bits = fuji_log2ceil(params.qt[3].total_values);
        qp[0] = params.qt[3].q_base;
        qp[1] = if qp[4] >= 0x18 { 0x18 } else { qp[0] + 1 };
        qp[2] = if qp[4] >= 0x4D { 0x4D } else { qp[1] };
        qp[3] = if qp[4] >= 0x122 { 0x122 } else { qp[2] };
        fuji_setup_qlut(&mut params.qt[3].q_table, &qp);
    }
    params
}

/// `init_main_grads`: seed the main gradient history the line decodes
/// share.
fn fuji_init_main_grads(params: &FujiParams, block: &mut FujiBlock) {
    let max_diff = 2.max((params.qt[0].total_values + 0x20) >> 6);
    let seed = [FujiGrad { value1: max_diff, value2: 1 }; 41];
    for slot in 0..3 {
        block.even[slot].grads = seed;
        block.odd[slot].grads = seed;
    }
}

/// `init_fuji_block`: the line buffer, bit reader window and gradient
/// history for one strip's decode.
fn fuji_init_block<'data, 'stream>(
    params: &FujiParams,
    stream: &'stream mut Stream<'data>,
    raw_offset: i64,
    dsize: u32,
    lossless: bool,
) -> Result<FujiBlock<'data, 'stream>> {
    let line_span = params.line_width + 2;
    let linebuf = vec![0u16; FUJI_LINES * line_span];
    let fsize = stream.size() as i64;
    // The file span from the strip's start, clamped to the recorded data
    // size — the size record may be incorrect.
    let file_span = (fsize.wrapping_sub(raw_offset)) as u64 as u32;
    let max_read_size = file_span.min(dsize);

    let mut block = FujiBlock {
        cur_bit: 0,
        cur_pos: 0,
        cur_buf_offset: raw_offset,
        max_read_size,
        cur_buf_size: 0,
        cur_buf: vec![0u8; FUJI_BUF_SIZE],
        fillbytes: 1,
        stream,
        even: [FujiGrads::seeded(2); 3],
        odd: [FujiGrads::seeded(2); 3],
        linebuf,
        line_span,
    };
    block.fill_buffer()?;
    if lossless {
        fuji_init_main_grads(params, &mut block);
    } else {
        // Static history for the fine tables; the main one is seeded per
        // line from the strip's quantization base.
        for table in 0..3usize {
            let max_diff = 2.max((params.qt[table + 1].total_values + 0x20) >> 6);
            let seed = [FujiGrad { value1: max_diff, value2: 1 }; 5];
            for slot in 0..3usize {
                block.even[slot].lossy_grads[table] = seed;
                block.odd[slot].lossy_grads[table] = seed;
            }
        }
    }
    Ok(block)
}

/// `fuji_decode_strip`: decode one vertical block-strip of the compressed
/// body into the raw image's 6-row line groups.
#[allow(clippy::too_many_arguments)]
fn fuji_decode_strip(
    meta: &CameraMeta,
    params: &FujiParams,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
    cur_block: usize,
    raw_offset: i64,
    dsize: u32,
    q_bases: Option<&[u8]>,
) -> Result<()> {
    let lossless = meta.fuji.compr_lossless != 0;
    let raw_type = meta.fuji.compr_raw_type;
    // A lossy strip rebuilds the main table per line around its own
    // quantization base; the fine tables stay the decode's shared set.
    let mut strip_params = params.clone();
    if !lossless {
        strip_params.qt[0].q_table = vec![0i8; 2usize << meta.fuji.compr_bits];
        strip_params.qt[0].q_base = -1;
    }

    let mut block = fuji_init_block(&strip_params, stream, raw_offset, dsize, lossless)?;
    let line_cells = strip_params.line_width + 2;

    let mut cur_block_width = meta.fuji.block_width as usize;
    if cur_block + 1 == meta.fuji.total_blocks as usize {
        cur_block_width = meta.raw_width as usize - meta.fuji.block_width as usize * cur_block;
    }

    let bases: &[u8] = q_bases.unwrap_or(&[]);
    for cur_line in 0..meta.fuji.total_lines as usize {
        if !lossless {
            let q_base = bases[cur_line] as i32;
            if cur_line == 0 || q_base != strip_params.qt[0].q_base {
                fuji_init_main_qtable(&mut strip_params, q_base);
                fuji_init_main_grads(&strip_params, &mut block);
            }
        }

        let errcnt = if raw_type == 16 {
            block.xtrans_decode_block(&strip_params)?
        } else {
            block.bayer_decode_block(&strip_params)?
        };
        if errcnt != 0 {
            derror(&*block.stream, data_error)?;
        }

        // Slide the decoded rows up and clear the decode window: the two
        // oldest rows of each colour group are replaced by the newest two,
        // the raw image takes the middle group's cells, then the window is
        // zeroed and re-edged for the next line.
        for (dst_line, src_line) in FUJI_MTABLE {
            let dst = dst_line * line_cells;
            let src = src_line * line_cells;
            block.linebuf.copy_within(src..src + line_cells, dst);
        }
        if raw_type == 16 {
            block.copy_line_to_xtrans(meta, raw_image, cur_block, cur_line, cur_block_width);
        } else {
            block.copy_line_to_bayer(meta, raw_image, cur_block, cur_line, cur_block_width);
        }
        for (zero_line, lines) in FUJI_ZTABLE {
            let base = zero_line * line_cells;
            let zero_end = base + lines * line_cells;
            block.linebuf[base..zero_end].fill(0);
            let prev = (zero_line - 1) * line_cells;
            block.linebuf[base] = block.linebuf[prev + 1];
            let right_edge = strip_params.line_width + 1;
            block.linebuf[base + right_edge] = block.linebuf[prev + right_edge - 1];
        }
    }
    Ok(())
}

/// `fuji_compressed_load_raw`: the compressed RAF body — the block-size
/// table, the lossy quantization bases and one decode per block strip.
fn fuji_compressed_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let lossless = meta.fuji.compr_lossless != 0;
    let raw_type = meta.fuji.compr_raw_type;
    let block_width = meta.fuji.block_width;
    if (block_width % 3 != 0 && raw_type == 16) || (block_width & 1 != 0 && raw_type == 0) {
        derror(stream, data_error)?;
    }
    let common_params = fuji_make_params(lossless, meta.fuji.compr_bits, block_width, raw_type);

    // The block sizes sit right after the stream header; each entry is
    // big-endian.
    let total_blocks = meta.fuji.total_blocks as usize;
    stream.seek(meta.data_offset.max(0) as u64);
    let mut block_bytes = vec![0u8; 4 * total_blocks];
    if stream.read(&mut block_bytes) != block_bytes.len() {
        bail!("the Fuji compressed raw block table is truncated");
    }
    // The strip data follows the block table; a lossy body's quantization
    // bases pad it to a 16-byte boundary first.
    let mut raw_offset = (4i64 * total_blocks as i64 + 0xF) & !0xF;
    let line_step = ((meta.fuji.total_lines as u32 + 0xF) & !0xF) as usize;
    let q_bases: Option<Vec<u8>> = if lossless {
        None
    } else {
        let total_q_bases = total_blocks * line_step;
        let mut bases = vec![0u8; total_q_bases];
        stream.seek((raw_offset + meta.data_offset).max(0) as u64);
        // A short base read just leaves zeros behind.
        stream.read(&mut bases);
        raw_offset += total_q_bases as i64;
        Some(bases)
    };
    raw_offset += meta.data_offset;

    // Calculating raw block offsets: the strips chain one after the other.
    let mut block_sizes = vec![0u32; total_blocks];
    for cur_block in 0..total_blocks {
        block_sizes[cur_block] = crx_sget_n(&block_bytes, cur_block * 4, 4);
    }
    let mut offsets = vec![0i64; total_blocks];
    offsets[0] = raw_offset;
    for cur_block in 1..total_blocks {
        offsets[cur_block] = offsets[cur_block - 1] + i64::from(block_sizes[cur_block - 1]);
    }

    for cur_block in 0..total_blocks {
        let bases = q_bases.as_ref().map(|all| &all[cur_block * line_step..]);
        let strip_offset = offsets[cur_block];
        let strip_size = block_sizes[cur_block];
        fuji_decode_strip(
            meta,
            &common_params,
            raw_image,
            stream,
            data_error,
            cur_block,
            strip_offset,
            strip_size,
            bases,
        )?;
    }
    Ok(())
}

/// `unpack7bytesto4x16`: seven bytes into four 14-bit samples, the layout
/// the Fuji 14-bit bodies read after their per-word byte swap.
fn unpack7bytesto4x16(src: &[u8], dest: &mut [u16]) {
    dest[0] = ((u16::from(src[0]) << 6) | u16::from(src[1] >> 2)) as u16;
    dest[1] = (((u16::from(src[1]) & 0x3) << 12) | (u16::from(src[2]) << 4) | u16::from(src[3] >> 4)) as u16;
    dest[2] = (((u16::from(src[3]) & 0xf) << 10) | (u16::from(src[4]) << 2) | u16::from(src[5] >> 6)) as u16;
    dest[3] = (((u16::from(src[5]) & 0x3f) << 8) | u16::from(src[6])) as u16;
}

/// `unpack28bytesto16x16ns`: twenty-eight bytes into sixteen samples
/// written back to front, the layout the Fuji 14-bit bodies read alone.
fn unpack28bytesto16x16ns(src: &[u8], dest: &mut [u16]) {
    dest[0] = ((u16::from(src[3]) << 6) | u16::from(src[2] >> 2)) as u16;
    dest[1] = (((u16::from(src[2]) & 0x3) << 12) | (u16::from(src[1]) << 4) | u16::from(src[0] >> 4)) as u16;
    dest[2] = (((u16::from(src[0]) & 0xf) << 10) | (u16::from(src[7]) << 2) | u16::from(src[6] >> 6)) as u16;
    dest[3] = (((u16::from(src[6]) & 0x3f) << 8) | u16::from(src[5])) as u16;
    dest[4] = ((u16::from(src[4]) << 6) | u16::from(src[11] >> 2)) as u16;
    dest[5] = (((u16::from(src[11]) & 0x3) << 12) | (u16::from(src[10]) << 4) | u16::from(src[9] >> 4)) as u16;
    dest[6] = (((u16::from(src[9]) & 0xf) << 10) | (u16::from(src[8]) << 2) | u16::from(src[15] >> 6)) as u16;
    dest[7] = (((u16::from(src[15]) & 0x3f) << 8) | u16::from(src[14])) as u16;
    dest[8] = ((u16::from(src[13]) << 6) | u16::from(src[12] >> 2)) as u16;
    dest[9] = (((u16::from(src[12]) & 0x3) << 12) | (u16::from(src[19]) << 4) | u16::from(src[18] >> 4)) as u16;
    dest[10] = (((u16::from(src[18]) & 0xf) << 10) | (u16::from(src[17]) << 2) | u16::from(src[16] >> 6)) as u16;
    dest[11] = (((u16::from(src[16]) & 0x3f) << 8) | u16::from(src[23])) as u16;
    dest[12] = ((u16::from(src[22]) << 6) | u16::from(src[21] >> 2)) as u16;
    dest[13] = (((u16::from(src[21]) & 0x3) << 12) | (u16::from(src[20]) << 4) | u16::from(src[27] >> 4)) as u16;
    dest[14] = (((u16::from(src[27]) & 0xf) << 10) | (u16::from(src[26]) << 2) | u16::from(src[25] >> 6)) as u16;
    dest[15] = (((u16::from(src[25]) & 0x3f) << 8) | u16::from(src[24])) as u16;
}

/// `unpack7bytesto4x16_nikon`: the same seven-to-four layout with the
/// sample order reversed inside each group.
fn unpack7bytesto4x16_nikon(src: &[u8], dest: &mut [u16]) {
    dest[3] = ((u16::from(src[6]) << 6) | u16::from(src[5] >> 2)) as u16;
    dest[2] = (((u16::from(src[5]) & 0x3) << 12) | (u16::from(src[4]) << 4) | u16::from(src[3] >> 4)) as u16;
    dest[1] = (((u16::from(src[3]) & 0xf) << 10) | (u16::from(src[2]) << 2) | u16::from(src[1] >> 6)) as u16;
    dest[0] = (((u16::from(src[1]) & 0x3f) << 8) | u16::from(src[0])) as u16;
}

/// `unpack21bytesto12x16_nikon`: twenty-one bytes into four sRAW pixels,
/// each carrying three samples in the four-channel image buffer's own
/// stride with the fourth left at zero.
fn unpack21bytesto12x16_nikon(src: &[u8], dest: &mut [u16]) {
    dest[0] = (((u16::from(src[1]) & 0x3f) << 8) | u16::from(src[0])) as u16;
    dest[1] = (((u16::from(src[3]) & 0xf) << 10) | (u16::from(src[2]) << 2) | u16::from(src[1] >> 6)) as u16;
    dest[2] = (((u16::from(src[5]) & 0x3) << 12) | (u16::from(src[4]) << 4) | u16::from(src[3] >> 4)) as u16;
    dest[4] = ((u16::from(src[6]) << 6) | u16::from(src[5] >> 2)) as u16;
    dest[5] = (((u16::from(src[8]) & 0x3f) << 8) | u16::from(src[7])) as u16;
    dest[6] = (((u16::from(src[10]) & 0xf) << 10) | (u16::from(src[9]) << 2) | u16::from(src[8] >> 6)) as u16;
    dest[8] = (((u16::from(src[12]) & 0x3) << 12) | (u16::from(src[11]) << 4) | u16::from(src[10] >> 4)) as u16;
    dest[9] = ((u16::from(src[13]) << 6) | u16::from(src[12] >> 2)) as u16;
    dest[10] = (((u16::from(src[15]) & 0x3f) << 8) | u16::from(src[14])) as u16;
    dest[12] = (((u16::from(src[17]) & 0xf) << 10) | (u16::from(src[16]) << 2) | u16::from(src[15] >> 6)) as u16;
    dest[13] = (((u16::from(src[19]) & 0x3) << 12) | (u16::from(src[18]) << 4) | u16::from(src[17] >> 4)) as u16;
    dest[14] = ((u16::from(src[20]) << 6) | u16::from(src[19] >> 2)) as u16;
}

/// `eight_bit_load_raw`: byte-per-sample rows through the tone curve; the
/// loader's own colour maximum is the curve's top entry.
fn eight_bit_load_raw(
    meta: &mut CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let curve = &meta.curve;
    let mut pixel = vec![0u8; raw_width];
    for row in 0..raw_height {
        if stream.read(&mut pixel) < raw_width {
            derror(stream, data_error)?;
        }
        let row_base = row * pitch;
        for (col, &byte) in pixel.iter().enumerate() {
            raw_image[row_base + col] = curve[byte as usize];
        }
    }
    meta.maximum = u32::from(curve[0xff]);
    Ok(())
}

/// `packed_load_raw`: the bit-packed rows shared by several makers, where
/// the file's own row padding (`load_flags`'s upper bits) and the optional
/// interleave seeks (its low bits) steer the walk.
fn packed_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as i32;
    let raw_height = meta.raw_height as i32;
    let height = meta.height as i32;
    let width = meta.width as i32;
    let top_margin = meta.top_margin as i32;
    let left_margin = meta.left_margin as i32;
    let tiff_bps = meta.tiff_bps as i32;
    let load_flags = meta.load_flags;
    let pitch = (meta.raw_pitch / 2) as usize;
    let column_flip = (load_flags >> 6) & 1;

    let mut bitbuf: u64 = 0;
    let mut vbits: i32 = 0;
    let mut bwide = raw_width * tiff_bps / 8;
    bwide += bwide & (load_flags >> 7) as i32;
    let rbits = bwide * 8 - raw_width * tiff_bps;
    if load_flags & 1 != 0 {
        bwide = bwide * 16 / 15;
    }
    let bite = (8 + (load_flags & 24)) as i32;
    let half = (raw_height + 1) >> 1;
    for irow in 0..raw_height {
        let mut row = irow;
        if load_flags & 2 != 0 {
            row = irow % half * 2 + irow / half;
            if row == 1 && load_flags & 4 != 0 {
                vbits = 0;
                if meta.tiff_compress != 0 {
                    // The interleave boundary lands one padded row set past
                    // the data's start.
                    let row_bytes = (-(half * bwide) & -2048i32) as u32;
                    let target = (meta.data_offset as u32).wrapping_sub(row_bytes);
                    stream.seek(u64::from(target));
                } else {
                    stream.seek(stream.size());
                    stream.seek((stream.tell() >> 3) << 2);
                }
            }
        }
        if stream.eof() {
            bail!("the raw stream ended before the image did");
        }
        let row_base = row as usize * pitch;
        for col in 0..raw_width {
            vbits -= tiff_bps;
            while vbits < 0 {
                vbits += bite;
                bitbuf = bitbuf.wrapping_shl(bite as u32);
                let mut step = 0i32;
                while step < bite {
                    let word: u32 = match stream.getc() {
                        Some(byte) => u32::from(byte),
                        // The stream's past-the-end reads give the engine's
                        // all-ones fill.
                        None => u32::MAX,
                    };
                    bitbuf |= u64::from(word.wrapping_shl(step as u32));
                    step += 8;
                }
            }
            let head = bitbuf
                .wrapping_shl((64 - tiff_bps - vbits) as u32)
                .wrapping_shr((64 - tiff_bps) as u32);
            let value = (head & 0x7fff_ffff) as i32;
            raw_image[row_base + ((col as u32) ^ column_flip) as usize] = value as u16;
            // The packed bodies with a check bit per tenth column consume a
            // byte there; the engine raises an error when that byte is set
            // inside the active area.
            if load_flags & 1 != 0
                && col % 10 == 9
                && stream.getc() != Some(0)
                && row < height + top_margin
                && col < width + left_margin
            {
                derror(stream, data_error)?;
            }
        }
        vbits -= rbits;
    }
    Ok(())
}

/// `nikon_14bit_load_raw`: the plain 14-bit rows as seven-byte quarters
/// either flat (Bayer) or as three-sample sRAW pixels (the legacy shape),
/// both sharing the engine's own reversed unpackers.
fn nikon_14bit_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let cps: u32 = if meta.filters == 0 && meta.colors == 3 { 3 } else { 1 };
    let raw_width = meta.raw_width as i32;
    // The row's byte length rounds the seven-bytes-per-four-samples line up
    // to a multiple of 16.
    let linelen = (raw_width * cps as i32 * 7 / 4) as f32 / 16.0f32;
    let linelen = (linelen.ceil()) as u32 * 16;
    let pitch = if meta.raw_pitch != 0 {
        meta.raw_pitch / (if cps >= 3 { 8 } else { 2 })
    } else {
        meta.raw_width
    };
    let raw_height = meta.raw_height as i32;
    let mut buf = vec![0u8; linelen as usize];
    for row in 0..raw_height.max(0) {
        let bytesread = stream.read(&mut buf) as u32;
        let row_base = (pitch as usize) * row as usize;
        if cps == 1 {
            let mut sp = 0u32;
            let mut dp = 0u32;
            while dp < pitch.wrapping_sub(3)
                && sp < linelen.wrapping_sub(6)
                && sp < bytesread.wrapping_sub(6)
            {
                let dest = &mut raw_image[row_base + dp as usize..][..4];
                unpack7bytesto4x16_nikon(&buf[sp as usize..], dest);
                sp += 7;
                dp += 4;
            }
        } else {
            let row_pixels = row_base * 4;
            let mut sp = 0u32;
            let mut dp = 0u32;
            while dp < pitch.wrapping_sub(3)
                && sp < linelen.wrapping_sub(20)
                && sp < bytesread.wrapping_sub(20)
            {
                let dest = &mut raw_image[row_pixels + dp as usize * 4..][..16];
                unpack21bytesto12x16_nikon(&buf[sp as usize..], dest);
                sp += 21;
                dp += 4;
            }
        }
    }
    Ok(())
}

/// `nikon_striped_packed_load_raw`: the bit-packed rows read strip by strip
/// through the file's own strip table; the bit reader carries the tail of a
/// strip into the next one.
fn nikon_striped_packed_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let load_flags: u32 = 24;
    let tiff_bps = meta.tiff_bps as i32;
    let raw_width = meta.raw_width as i32;
    let Some(record) = meta
        .strip_ifds
        .iter()
        .find(|record| record.offset == meta.data_offset)
    else {
        bail!("the camera raw's strip table has no record for the raw data");
    };
    if record.rows_per_strip == 0 || record.strip_offsets.is_empty() {
        // The engine calls this "not unpacked" and leaves the pixels zero.
        return Ok(());
    }
    let mut bitbuf: u64 = 0;
    let mut vbits: i32 = 0;
    let mut bwide = raw_width * tiff_bps / 8;
    bwide += bwide & (load_flags >> 7) as i32;
    let rbits = bwide * 8 - raw_width * tiff_bps;
    let bite = (8 + (load_flags & 24)) as i32;
    let mut stripcnt = 0usize;
    for row in 0..meta.raw_height as i32 {
        if row % record.rows_per_strip == 0 {
            if stripcnt >= record.strip_offsets.len() {
                // Running out of strips just stops the decode.
                return Ok(());
            }
            stream.seek(record.strip_offsets[stripcnt].max(0) as u64);
            stripcnt += 1;
        }
        let row_base = row as usize * meta.raw_width as usize;
        for col in 0..raw_width {
            vbits -= tiff_bps;
            while vbits < 0 {
                vbits += bite;
                bitbuf = bitbuf.wrapping_shl(bite as u32);
                let mut step = 0i32;
                while step < bite {
                    let word: u32 = match stream.getc() {
                        Some(byte) => u32::from(byte),
                        None => u32::MAX,
                    };
                    bitbuf |= u64::from(word.wrapping_shl(step as u32));
                    step += 8;
                }
            }
            let head = bitbuf
                .wrapping_shl((64 - tiff_bps - vbits) as u32)
                .wrapping_shr((64 - tiff_bps) as u32);
            raw_image[row_base + col as usize] = (head & 0xffff) as u16;
        }
        vbits -= rbits;
    }
    Ok(())
}

/// `nikon_load_padded_packed_raw`: 12-bit samples padded into three bytes
/// per pixel pair, each row padded out to the file's own byte count.
fn nikon_padded_packed_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let load_flags = meta.load_flags;
    if load_flags < 2000 || load_flags > 64000 {
        return Ok(());
    }
    let raw_width = meta.raw_width as usize;
    let mut buf = vec![0u8; load_flags as usize];
    for row in 0..meta.raw_height as usize {
        // A short read keeps the previous row's tail, exactly like the
        // engine's read, and this loader raises no data error.
        stream.read(&mut buf);
        let row_base = row * raw_width;
        for icol in 0..raw_width / 2 {
            let byte_base = icol * 3;
            let high = u16::from(buf.get(byte_base + 1).copied().unwrap_or(0));
            let pixels = [
                (high & 0xf) << 8 | u16::from(buf.get(byte_base).copied().unwrap_or(0)),
                u16::from(buf.get(byte_base + 2).copied().unwrap_or(0)) << 4 | (high & 0xf0) >> 4,
            ];
            raw_image[row_base + icol * 2] = pixels[0];
            raw_image[row_base + icol * 2 + 1] = pixels[1];
        }
    }
    Ok(())
}

/// `fuji_14bit_load_raw`: the RAF 14-bit rows in seven-byte quarters; a row
/// whose byte count is not a multiple of 28 first swaps every whole word of
/// the buffer.
fn fuji_14bit_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as i32;
    let linelen = (raw_width.max(0) * 7 / 4) as u32;
    let pitch = if meta.raw_pitch != 0 { meta.raw_pitch / 2 } else { meta.raw_width };
    let mut buf = vec![0u8; linelen as usize];
    for row in 0..meta.raw_height as u32 {
        let bytesread = stream.read(&mut buf) as u32;
        let row_base = (pitch as usize) * row as usize;
        if bytesread % 28 != 0 {
            // The engine byte-swaps each whole u32 word in place; the
            // trailing partial word stays as it was.
            let word_bytes = (bytesread / 4) as usize * 4;
            for word in buf[..word_bytes].chunks_exact_mut(4) {
                word.swap(0, 3);
                word.swap(1, 2);
            }
            let mut sp = 0u32;
            let mut dp = 0u32;
            while dp < pitch.wrapping_sub(3)
                && sp < linelen.wrapping_sub(6)
                && sp < bytesread.wrapping_sub(6)
            {
                let dest = &mut raw_image[row_base + dp as usize..][..4];
                unpack7bytesto4x16(&buf[sp as usize..], dest);
                sp += 7;
                dp += 4;
            }
        } else {
            let mut sp = 0u32;
            let mut dp = 0u32;
            while dp < pitch.wrapping_sub(15)
                && sp < linelen.wrapping_sub(27)
                && sp < bytesread.wrapping_sub(27)
            {
                let dest = &mut raw_image[row_base + dp as usize..][..16];
                unpack28bytesto16x16ns(&buf[sp as usize..], dest);
                sp += 28;
                dp += 16;
            }
        }
    }
    Ok(())
}

/// `unpacked_load_raw_FujiDBP`: the GX680's eight tall tiles read one after
/// the other, each scattered into the row-interleaved store.
fn fuji_dbp_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let tile_width = raw_width / 8;
    let mut tile = vec![0u16; tile_width * raw_height];
    for tile_n in 0..8usize {
        let wanted = (tile_width * raw_height) * 2;
        let short_read = stream.size().saturating_sub(stream.tell()) < wanted as u64;
        stream.read_shorts(&mut tile);
        if short_read {
            derror(stream, data_error)?;
        }
        for scan_line in 0..raw_height {
            let source = scan_line * tile_width;
            let row_base = scan_line * raw_width + tile_n * tile_width;
            raw_image[row_base..row_base + tile_width]
                .copy_from_slice(&tile[source..source + tile_width]);
        }
    }
    // The read back past the last tile keeps a later check from seeing the
    // file's end.
    stream.skip(-2);
    Ok(())
}

/// `unpacked_load_raw_fuji_f700s20`: each row holds the sensor's two
/// half-row buffers back to back; only the first is the real image. The
/// engine's single-shot pass starts at the buffer head.
fn fuji_f700s20_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let mut buffer = vec![0u16; raw_width * 2];
    for row in 0..raw_height {
        let short_read = stream.size().saturating_sub(stream.tell()) < (raw_width * 4) as u64;
        stream.read_shorts(&mut buffer[..raw_width * 2]);
        if short_read {
            derror(stream, data_error)?;
        }
        let row_base = row * pitch;
        raw_image[row_base..row_base + raw_width].copy_from_slice(&buffer[..raw_width]);
    }
    Ok(())
}

/// `samsung2_load_raw`: the SRW's Huffman-coded differences under the
/// top-left and left predictors, with the accumulator carrying across the
/// row and the over-deep samples turning into data errors.
fn samsung2_load_raw(
    meta: &CameraMeta,
    raw_image: &mut [u16],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    const TAB: [u16; 14] = [
        0x304, 0x307, 0x206, 0x205, 0x403, 0x600, 0x709, 0x80a, 0x90b, 0xa0c, 0xa0d, 0x501, 0x408, 0x402,
    ];
    let mut huff = vec![0u16; 1026];
    huff[0] = 10;
    let mut n = 0usize;
    for &entry in TAB.iter() {
        let count = 1024usize >> (entry >> 8);
        for _ in 0..count {
            n += 1;
            huff[n] = entry;
        }
    }
    let mut bits = BitReader::default();
    bits.restart();
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let pitch = (meta.raw_pitch / 2) as usize;
    let tiff_bps = meta.tiff_bps;
    let mut vpred = [[0u16; 2]; 2];
    let mut hpred = [0u16; 2];
    for row in 0..raw_height {
        for col in 0..raw_width {
            let diff = ljpeg_diff_bits(stream, &mut bits, Some(&huff), 0);
            if col < 2 {
                vpred[row & 1][col] = vpred[row & 1][col].wrapping_add(diff as u16);
                hpred[col] = vpred[row & 1][col];
            } else {
                hpred[col & 1] = hpred[col & 1].wrapping_add(diff as u16);
            }
            raw_image[row * pitch + col] = hpred[col & 1];
            if i32::from(hpred[col & 1]).wrapping_shr(tiff_bps) != 0 {
                derror(stream, data_error)?;
            }
        }
    }
    Ok(())
}

/// `samsung3_load_raw`: the NX30's clustered rows — a motion model copies
/// sixteen-pixel blocks from the row above or two rows up, then per-colour
/// delta lengths widen and narrow across the row; the block reader runs on
/// little-endian words the loader forces on its own.
fn samsung3_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as i32;
    let raw_height = meta.raw_height as i32;
    if raw_height < 436 || raw_width < 646 {
        bail!("the camera raw image is too small for its compression");
    }
    // The engine reads this body's whole stream little-endian regardless of
    // the file's byte order.
    stream.order = LITTLE_ENDIAN;
    let maxpixidx = i64::from(meta.raw_width) * i64::from(raw_height);
    let pitch = (meta.raw_pitch / 2) as usize;

    let mut ph1 = Ph1Bits::default();
    ph1.reset();
    ph1.bits(stream, 16);
    ph1.bits(stream, 4);
    let bit_depth = ph1.bits(stream, 4) + 1;
    let datamax = (1i32.wrapping_shl(bit_depth)) - 1;
    ph1.bits(stream, 8);
    ph1.bits(stream, 16);
    ph1.bits(stream, 16);
    ph1.bits(stream, 16);
    ph1.bits(stream, 4);
    let optflags = ph1.bits(stream, 4);
    ph1.bits(stream, 16);
    ph1.bits(stream, 8);
    ph1.bits(stream, 2);
    let init_val = ph1.bits(stream, 14) as u16;

    for row in 0..raw_height {
        // Each row restarts at the next block boundary past the data's
        // start; the remainder is always zero to fifteen bytes forward.
        let gap = (meta.data_offset - stream.tell() as i64) & 15;
        stream.skip(gap);
        ph1.reset();

        let rowstart = i64::from(row) * i64::from(meta.raw_width);
        let mut motion = 7u32;
        let mut scale = 0i32;
        let mut diff_bits_mode = [[7u32; 2]; 3];
        if row >= 2 {
            for entry in diff_bits_mode.iter_mut() {
                entry[0] = 4;
                entry[1] = 4;
            }
        }

        let mut col = 0i32;
        while col < raw_width - 15 {
            if optflags & 4 == 0 && col & 63 == 0 {
                let scalevals = [0i32, -2, 2];
                let pick = ph1.bits(stream, 2);
                if pick < 3 {
                    scale += scalevals[pick as usize];
                } else {
                    scale = ph1.bits(stream, 12) as i32;
                }
            }

            if optflags & 2 != 0 {
                motion = if ph1.bits(stream, 1) != 0 { 3 } else { 7 };
            } else if ph1.bits(stream, 1) == 0 {
                motion = ph1.bits(stream, 3);
            }

            if motion == 7 {
                let row_base = row as usize * pitch;
                for i in 0..16i32 {
                    let value = if col == 0 { init_val } else { raw_image[row_base + (col + i - 2) as usize] };
                    raw_image[row_base + (col + i) as usize] = value;
                }
            } else {
                if row < 2 {
                    bail!("the camera raw's motion copy runs before the image starts");
                }
                let motion_offset = [-4i32, -2, -2, 0, 0, 2, 4];
                let motion_average = [0u32, 0, 1, 0, 1, 0, 0];
                let row_base = row as usize * pitch;
                for i in 0..16i32 {
                    let even = (row + i) & 1 != 0;
                    let refrow = if even { row - 2 } else { row - 1 };
                    let refcol = col + i + motion_offset[motion as usize] + (i32::from(!even)) * (1 - 2 * (i & 1));
                    if i64::from(refrow) * i64::from(meta.raw_width) + i64::from(refcol) < 0 {
                        bail!("the camera raw's motion copy references a row before the image starts");
                    }
                    // The reference cells sit one or two rows up, so the row
                    // bounds already cover them; a store whose pitch runs
                    // past its width, which no file sets, reads as zero.
                    let ref_flat = (refrow as i64 * pitch as i64) + i64::from(refcol);
                    let reference = raw_image.get(ref_flat as usize).copied().unwrap_or(0);
                    let value = if motion_average[motion as usize] != 0 {
                        let neighbour = raw_image.get((ref_flat + 2) as usize).copied().unwrap_or(0);
                        (i32::from(reference) + i32::from(neighbour) + 1) >> 1
                    } else {
                        i32::from(reference)
                    };
                    raw_image[row_base + (col + i) as usize] = value as u16;
                }
            }

            let mut diff_bits = [0u32; 4];
            if optflags & 1 != 0 || ph1.bits(stream, 1) == 0 {
                let mut flags = [0u32; 4];
                for flag in flags.iter_mut() {
                    *flag = ph1.bits(stream, 2);
                }
                for i in 0..4u32 {
                    let colornum = if row % 2 != 0 {
                        i >> 1
                    } else {
                        ((i >> 1) + 2) % 3
                    };
                    diff_bits[i as usize] = match flags[i as usize] {
                        0 => diff_bits_mode[colornum as usize][0],
                        1 => diff_bits_mode[colornum as usize][0].wrapping_add(1),
                        2 => diff_bits_mode[colornum as usize][0].wrapping_sub(1),
                        // The engine's mask makes a length of zero read as 31.
                        _ => ph1.bits(stream, 4),
                    };
                    diff_bits_mode[colornum as usize][0] = diff_bits_mode[colornum as usize][1];
                    diff_bits_mode[colornum as usize][1] = diff_bits[i as usize];
                    if diff_bits[i as usize] > bit_depth + 1 {
                        bail!("the camera raw's delta length exceeds its depth");
                    }
                }
            }

            for i in 0..16i32 {
                let len = diff_bits[(i >> 2) as usize];
                // The engine's masks turn a length of zero into the sign
                // bit's own value.
                let sign_mask = 1i32.wrapping_shl(len.wrapping_sub(1));
                let raw = ph1.bits(stream, len as i32);
                let signed = sign_mask as u32;
                let diff = (raw ^ signed)
                    .wrapping_sub(signed)
                    .wrapping_mul((scale * 2 + 1) as u32)
                    .wrapping_add(scale as u32) as i32;
                let rcol = if row % 2 != 0 {
                    col + ((i & 0x7) << 1) + 1 - (i >> 3)
                } else {
                    col + ((i & 0x7) << 1) + (i >> 3)
                };
                let bidx = rowstart + i64::from(rcol);
                if bidx < 0 || bidx >= maxpixidx {
                    bail!("the camera raw's delta writes outside its sensor");
                }
                let cell = row as usize * pitch + rcol as usize;
                let value = i32::from(raw_image[cell]) + diff;
                raw_image[cell] = if value < 0 {
                    0
                } else if value > datamax {
                    datamax as u16
                } else {
                    value as u16
                };
            }
            col += 16;
        }
    }
    Ok(())
}

/// `pana_cs6_page_decoder`: one 16-byte page's pixel fields, refilled per
/// block; the walker reads past the page's fields as explicit zeros.
struct PanaCs6Page<'a> {
    pixelbuffer: [u32; 18],
    lastoffset: u32,
    maxoffset: u32,
    current: usize,
    buffer: &'a [u8],
}

impl<'a> PanaCs6Page<'a> {
    fn new(buffer: &'a [u8], size: u32) -> PanaCs6Page<'a> {
        PanaCs6Page {
            pixelbuffer: [0; 18],
            lastoffset: 0,
            maxoffset: size,
            current: 0,
            buffer,
        }
    }

    fn nextpixel(&mut self) -> u32 {
        if self.current < 14 {
            self.current += 1;
            self.pixelbuffer[self.current - 1]
        } else {
            0
        }
    }

    fn nextpixel12(&mut self) -> u32 {
        if self.current < 18 {
            self.current += 1;
            self.pixelbuffer[self.current - 1]
        } else {
            0
        }
    }

    /// `read_page`: the 14-bit fields, read back to front out of the page.
    fn read_page(&mut self) -> Result<()> {
        if self.maxoffset.saturating_sub(self.lastoffset) < 16 {
            bail!("the camera raw's block header ran out of data");
        }
        let buffer = self.buffer;
        let base = self.lastoffset as usize;
        let pixel = &mut self.pixelbuffer;
        let wb = |i: usize| u32::from(buffer[base + 15 - i]);
        pixel[0] = (wb(0) << 6) | (wb(1) >> 2);
        pixel[1] = (((wb(1) & 0x3) << 12) | (wb(2) << 4) | (wb(3) >> 4)) & 0x3fff;
        pixel[2] = (wb(3) >> 2) & 0x3;
        pixel[3] = ((wb(3) & 0x3) << 8) | wb(4);
        pixel[4] = (wb(5) << 2) | (wb(6) >> 6);
        pixel[5] = ((wb(6) & 0x3f) << 4) | (wb(7) >> 4);
        pixel[6] = (wb(7) >> 2) & 0x3;
        pixel[7] = ((wb(7) & 0x3) << 8) | wb(8);
        pixel[8] = ((wb(9) << 2) & 0x3fc) | (wb(10) >> 6);
        pixel[9] = ((wb(10) << 4) | (wb(11) >> 4)) & 0x3ff;
        pixel[10] = (wb(11) >> 2) & 0x3;
        pixel[11] = ((wb(11) & 0x3) << 8) | wb(12);
        pixel[12] = (((wb(13) << 2) & 0x3fc) | (wb(14) >> 6)) & 0x3ff;
        pixel[13] = ((wb(14) << 4) | (wb(15) >> 4)) & 0x3ff;
        self.current = 0;
        self.lastoffset += 16;
        Ok(())
    }

    /// `read_page12`: the 12-bit fields.
    fn read_page12(&mut self) -> Result<()> {
        if self.maxoffset.saturating_sub(self.lastoffset) < 16 {
            bail!("the camera raw's block header ran out of data");
        }
        let buffer = self.buffer;
        let base = self.lastoffset as usize;
        let pixel = &mut self.pixelbuffer;
        let wb = |i: usize| u32::from(buffer[base + 15 - i]);
        pixel[0] = (wb(0) << 4) | (wb(1) >> 4);
        pixel[1] = (((wb(1) & 0xf) << 8) | wb(2)) & 0xfff;
        pixel[2] = (wb(3) >> 6) & 0x3;
        pixel[3] = ((wb(3) & 0x3f) << 2) | (wb(4) >> 6);
        pixel[4] = ((wb(4) & 0x3f) << 2) | (wb(5) >> 6);
        pixel[5] = ((wb(5) & 0x3f) << 2) | (wb(6) >> 6);
        pixel[6] = (wb(6) >> 4) & 0x3;
        pixel[7] = ((wb(6) & 0xf) << 4) | (wb(7) >> 4);
        pixel[8] = ((wb(7) & 0xf) << 4) | (wb(8) >> 4);
        pixel[9] = ((wb(8) & 0xf) << 4) | (wb(9) >> 4);
        pixel[10] = (wb(9) >> 2) & 0x3;
        pixel[11] = ((wb(9) & 0x3) << 6) | (wb(10) >> 2);
        pixel[12] = ((wb(10) & 0x3) << 6) | (wb(11) >> 2);
        pixel[13] = ((wb(11) & 0x3) << 6) | (wb(12) >> 2);
        pixel[14] = wb(12) & 0x3;
        pixel[15] = wb(13);
        pixel[16] = wb(14);
        pixel[17] = wb(15);
        self.current = 0;
        self.lastoffset += 16;
        Ok(())
    }
}

/// `panasonicC6_load_raw`: sixteen-row blocks of per-block split fields
/// under the Panasonic's own base/multiplier walk; every pixel writes
/// through the wrap-around pedestal the block's base fields choose.
fn panasonic_c6_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let rowstep = 16i32;
    let pana_bits_12 = meta.panasonic.bits == 12;
    let pixperblock = if pana_bits_12 { 14usize } else { 11usize };
    let blocksperrow = meta.raw_width as i32 / pixperblock as i32;
    let rowbytes = (blocksperrow * 16) as usize;
    let pixelbase0: u32 = if pana_bits_12 { 0x80 } else { 0x200 };
    let pixelbase_compare: u32 = if pana_bits_12 { 0x800 } else { 0x2000 };
    let spix_compare: u32 = if pana_bits_12 { 0x3fff } else { 0xffff };
    let pixel_mask: u32 = if pana_bits_12 { 0xfff } else { 0x3fff };

    let mut iobuf = vec![0u8; rowbytes * rowstep as usize];
    let mut row = 0i32;
    while row < meta.raw_height as i32 - rowstep + 1 {
        let rowstoread = rowstep.min(meta.raw_height as i32 - row);
        let wanted = rowbytes * rowstoread as usize;
        if stream.read(&mut iobuf[..wanted]) != wanted {
            bail!("the camera raw's block stream ended early");
        }
        let mut page = PanaCs6Page::new(&iobuf, wanted as u32);
        let mut crow = 0i32;
        while crow < rowstoread {
            let row_base = ((row + crow) as u64 * u64::from(meta.raw_pitch) / 2) as usize;
            let mut col = 0usize;
            for _ in 0..blocksperrow {
                if pana_bits_12 {
                    page.read_page12()?;
                } else {
                    page.read_page()?;
                }
                let mut oddeven = [0u32; 2];
                let mut nonzero = [0u32; 2];
                let mut pmul = 0u32;
                let mut pixel_base = 0u32;
                for pix in 0..pixperblock {
                    if pix % 3 == 2 {
                        let mut base = if pana_bits_12 { page.nextpixel12() } else { page.nextpixel() };
                        if base > 3 {
                            bail!("the camera raw's block header is corrupt");
                        }
                        if base == 3 {
                            base = 4;
                        }
                        pixel_base = pixelbase0 << base;
                        pmul = 1 << base;
                    }
                    let mut epixel = if pana_bits_12 { page.nextpixel12() } else { page.nextpixel() };
                    if oddeven[pix % 2] != 0 {
                        epixel = epixel.wrapping_mul(pmul);
                        if pixel_base < pixelbase_compare && nonzero[pix % 2] > pixel_base {
                            epixel = epixel.wrapping_add(nonzero[pix % 2] - pixel_base);
                        }
                        nonzero[pix % 2] = epixel;
                    } else {
                        oddeven[pix % 2] = epixel;
                        if epixel != 0 {
                            nonzero[pix % 2] = epixel;
                        } else {
                            epixel = nonzero[pix % 2];
                        }
                    }
                    let spix = epixel.wrapping_sub(0xf);
                    if spix <= spix_compare {
                        raw_image[row_base + col] = (spix & spix_compare) as u16;
                    } else {
                        let shifted = (epixel.wrapping_add(0x7fff_fff1)) as i32 >> 31;
                        raw_image[row_base + col] = ((shifted as u32) & pixel_mask) as u16;
                    }
                    col += 1;
                }
            }
            crow += 1;
        }
        row += rowstep;
    }
    Ok(())
}

/// `panasonicC7_load_raw`: the same sixteen-row block shape with whole
/// nine- or ten-pixel groups read straight out of each block's first bytes;
/// the byte cursor runs on across the block's rows.
fn panasonic_c7_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let rowstep = 16i32;
    let pana_bits = meta.panasonic.bits;
    let pixperblock = if pana_bits == 14 { 9usize } else { 10usize };
    let rowbytes = (meta.raw_width as i32 / pixperblock as i32 * 16) as usize;
    let raw_width = meta.raw_width as usize;

    let mut iobuf = vec![0u8; rowbytes * rowstep as usize];
    let mut row = 0i32;
    while row < meta.raw_height as i32 - rowstep + 1 {
        let rowstoread = rowstep.min(meta.raw_height as i32 - row);
        let wanted = rowbytes * rowstoread as usize;
        if stream.read(&mut iobuf[..wanted]) != wanted {
            bail!("the camera raw's block stream ended early");
        }
        let mut cursor = 0usize;
        let mut crow = 0i32;
        while crow < rowstoread {
            let row_base = ((row + crow) as u64 * u64::from(meta.raw_pitch) / 2) as usize;
            let mut col = 0usize;
            while col < raw_width.saturating_sub(pixperblock - 1) {
                let bytes = &iobuf[cursor..cursor + 16];
                if pana_bits == 14 {
                    let pixels = [
                        bytes[0] as u16 + ((bytes[1] as u16 & 0x3F) << 8),
                        (bytes[1] as u16 >> 6) + 4 * bytes[2] as u16 + ((bytes[3] as u16 & 0xF) << 10),
                        (bytes[3] as u16 >> 4) + 16 * bytes[4] as u16 + ((bytes[5] as u16 & 3) << 12),
                        ((bytes[5] as u16 & 0xFC) >> 2) + ((bytes[6] as u16) << 6),
                        bytes[7] as u16 + ((bytes[8] as u16 & 0x3F) << 8),
                        (bytes[8] as u16 >> 6) + 4 * bytes[9] as u16 + ((bytes[10] as u16 & 0xF) << 10),
                        (bytes[10] as u16 >> 4) + 16 * bytes[11] as u16 + ((bytes[12] as u16 & 3) << 12),
                        ((bytes[12] as u16 & 0xFC) >> 2) + ((bytes[13] as u16) << 6),
                        bytes[14] as u16 + ((bytes[15] as u16 & 0x3F) << 8),
                    ];
                    for (offset, value) in pixels.iter().enumerate() {
                        raw_image[row_base + col + offset] = *value;
                    }
                } else if pana_bits == 12 {
                    let pixels = [
                        ((bytes[1] as u16 & 0xF) << 8) + bytes[0] as u16,
                        16 * bytes[2] as u16 + (bytes[1] as u16 >> 4),
                        ((bytes[4] as u16 & 0xF) << 8) + bytes[3] as u16,
                        16 * bytes[5] as u16 + (bytes[4] as u16 >> 4),
                        ((bytes[7] as u16 & 0xF) << 8) + bytes[6] as u16,
                        16 * bytes[8] as u16 + (bytes[7] as u16 >> 4),
                        ((bytes[10] as u16 & 0xF) << 8) + bytes[9] as u16,
                        16 * bytes[11] as u16 + (bytes[10] as u16 >> 4),
                        ((bytes[13] as u16 & 0xF) << 8) + bytes[12] as u16,
                        16 * bytes[14] as u16 + (bytes[13] as u16 >> 4),
                    ];
                    for (offset, value) in pixels.iter().enumerate() {
                        raw_image[row_base + col + offset] = *value;
                    }
                }
                col += pixperblock;
                cursor += 16;
            }
            crow += 1;
        }
        row += rowstep;
    }
    Ok(())
}

/// `sony_ljpeg_load_raw`: the tiled lossless-JPEG decode, each tile's
/// header read from the file's own tile-offset list; the tiles walk left to
/// right and wrap after each full raw width.
fn sony_ljpeg_load_raw(meta: &CameraMeta, raw_image: &mut [u16], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as u32;
    let raw_height = meta.raw_height as u32;
    let pitch = (meta.raw_pitch / 2) as usize;
    let mut bits = BitReader::default();
    let mut trow = 0u32;
    let mut tcol = 0u32;
    while trow < raw_height {
        let save = stream.tell();
        if meta.tile_length < NOT_TILED {
            let tile_offset = stream.get4();
            stream.seek(u64::from(tile_offset));
        }
        let Some(mut head) = ljpeg_start(stream, &mut bits, 0, false) else {
            // A tile whose header fails stops the whole decode.
            break;
        };
        let mut jrow = 0u32;
        let mut row = 0u32;
        while jrow < head.high as u32 && trow.wrapping_add(row) < raw_height.wrapping_sub(1) {
            let row_start = ljpeg_row(stream, &mut bits, &mut head, jrow as i32, 0);
            let mut jcol = 0u32;
            let mut col = 0u32;
            while jcol < head.wide as u32 && tcol.wrapping_add(col) < raw_width.wrapping_sub(1) {
                // The engine's row pointer strides by the fixed four-channel
                // width, whatever the tile's own component count.
                let sample = |c: usize| head.rows.get(row_start + (jcol as usize) * 4 + c).copied().unwrap_or(0);
                let row_a = (trow + row) as usize * pitch;
                let row_b = (trow + row) as usize * pitch + pitch;
                raw_image[row_a + (tcol + col) as usize] = sample(0);
                raw_image[row_a + (tcol + col) as usize + 1] = sample(1);
                raw_image[row_b + (tcol + col) as usize] = sample(2);
                raw_image[row_b + (tcol + col) as usize + 1] = sample(3);
                jcol += 1;
                col += 2;
            }
            jrow += 1;
            row += 2;
        }
        stream.seek(save + 4);
        tcol = tcol.wrapping_add(meta.tile_width);
        if tcol >= raw_width {
            trow = trow.wrapping_add(meta.tile_length);
            tcol = 0;
        }
    }
    Ok(())
}

/// `sony_arq_load_raw`: the four-plane ARW stores, each pixel's third and
/// fourth plane swapped in place before the over-peak scan.
fn sony_arq_load_raw(
    meta: &CameraMeta,
    image: &mut [[u16; 4]],
    stream: &mut Stream,
    data_error: &mut bool,
) -> Result<()> {
    if meta.colors < 3 {
        bail!("the camera raw's four-plane store is corrupt");
    }
    let raw_width = meta.raw_width as usize;
    let raw_height = meta.raw_height as usize;
    let count = raw_width * raw_height * 4;
    let short_read = stream.size().saturating_sub(stream.tell()) < (count * 2) as u64;
    stream.read_shorts(&mut image.as_flattened_mut()[..count]);
    if short_read {
        derror(stream, data_error)?;
    }
    stream.skip(-2);
    for row in 0..raw_height {
        let row_start = row * raw_width;
        for pixel_index in row_start..row_start + raw_width {
            image[pixel_index].swap(2, 3);
            let in_active = ((row as i32 - meta.top_margin as i32) as u32) < meta.height
                && (((pixel_index - row_start) as i32 - meta.left_margin as i32) as u32) < meta.width;
            if in_active {
                let cell = image[pixel_index];
                let peak = cell[0].max(cell[1]).max(cell[2]).max(cell[3]);
                if peak > meta.maximum as u16 {
                    derror(stream, data_error)?;
                }
            }
        }
    }
    Ok(())
}

/// `nikon_load_sraw`: the sRGB NEF rows as pairs of 12-bit samples split
/// out of three bytes each, the chroma average spread over the pair, then
/// the YCbCr pass through the tone curve the open stage rebuilt.
fn nikon_sraw_load_raw(meta: &mut CameraMeta, image: &mut [[u16; 4]], stream: &mut Stream) -> Result<()> {
    let raw_width = meta.raw_width as i32;
    let raw_height = meta.raw_height as usize;
    // One shared row reader; a short row keeps its predecessor's tail, as
    // the engine's read does, and the buffer's slack bytes stay zero.
    let mut row_buffer = vec![0u8; (3 * (raw_width.max(0) + 2)) as usize];
    let read_len = (3 * raw_width.max(0)) as usize;
    for row in 0..raw_height {
        stream.read(&mut row_buffer[..read_len]);
        let row_base = row * raw_width.max(0) as usize;
        let mut col = 0i32;
        while col < raw_width - 1 {
            let bi = (col * 3) as usize;
            let bits1 = (u16::from(row_buffer[bi + 1] & 0xf) << 8) | u16::from(row_buffer[bi]);
            let bits2 = (u16::from(row_buffer[bi + 2]) << 4) | u16::from((row_buffer[bi + 1] >> 4) & 0xf);
            let bits3 = (u16::from(row_buffer[bi + 4] & 0xf) << 8) | u16::from(row_buffer[bi + 3]);
            let bits4 = (u16::from(row_buffer[bi + 5]) << 4) | u16::from((row_buffer[bi + 4] >> 4) & 0xf);
            let pixel = row_base + col as usize;
            image[pixel][0] = bits1;
            image[pixel][1] = bits3;
            image[pixel][2] = bits4;
            image[pixel + 1][0] = bits2;
            image[pixel + 1][1] = 2048;
            image[pixel + 1][2] = 2048;
            col += 2;
        }
    }

    // The chroma average spreads each pair's even pixel's samples over the
    // odd one; the engine casts the sum to a sample before halving.
    for row in 0..raw_height {
        let row_base = row * raw_width.max(0) as usize;
        let mut col = 0i32;
        while col < raw_width {
            let col2 = if col < raw_width - 2 { col + 2 } else { col };
            for channel in 1..3usize {
                let even = row_base + col as usize;
                let pair = row_base + col2 as usize;
                let summed = i32::from(image[even][channel]) + i32::from(image[pair][channel]);
                image[row_base + col as usize + 1][channel] = (summed / 2) as u16;
            }
            col += 2;
        }
    }

    // The YCbCr pass folds the chroma into luma with the camera's weights
    // and reads every channel back through the tone curve.
    let curve = &meta.curve;
    for row in 0..raw_height {
        let row_base = row * raw_width.max(0) as usize;
        for col in 0..raw_width.max(0) as usize {
            let pixel = &mut image[row_base + col];
            let y = f32::from(pixel[0]) / 2549.0f32;
            let chroma_a = ((i32::from(pixel[1]) - 1280) as f32) / 1536.0f32;
            let chroma_b = ((i32::from(pixel[2]) - 1280) as f32) / 1536.0f32;
            let mut luma = y;
            if luma > 1.0f32 {
                luma = 1.0f32;
            }
            let (chroma_a, chroma_b) = if luma > 0.803f32 { (0.5f32, 0.5f32) } else { (chroma_a, chroma_b) };
            let mut red = luma + 1.402f32 * (chroma_b - 0.5f32);
            if red < 0.0f32 {
                red = 0.0f32;
            }
            if red > 1.0f32 {
                red = 1.0f32;
            }
            let mut green = luma - 0.34414f32 * (chroma_a - 0.5f32) - 0.71414f32 * (chroma_b - 0.5f32);
            if green > 1.0f32 {
                green = 1.0f32;
            }
            if green < 0.0f32 {
                green = 0.0f32;
            }
            let mut blue = luma + 1.772f32 * (chroma_a - 0.5f32);
            if blue > 1.0f32 {
                blue = 1.0f32;
            }
            if blue < 0.0f32 {
                blue = 0.0f32;
            }
            pixel[0] = curve[(red * 3072.0f32) as i32 as usize];
            pixel[1] = curve[(green * 3072.0f32) as i32 as usize];
            pixel[2] = curve[(blue * 3072.0f32) as i32 as usize];
        }
    }
    meta.maximum = 16383;
    Ok(())
}
