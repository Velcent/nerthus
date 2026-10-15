//! AV1 items through rav1d (a port of dav1d), converted the way libheif's
//! aom plugin converts libaom's output (`decoder_aom.cc`): the first frame
//! of the configuration OBUs and the item's data, its planes at the
//! stream's bit depth and its colour description as the nclx profile.
//! AV1 decoding, film grain included, is normative, so dav1d's output is
//! libaom's.

use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use anyhow::{anyhow, bail, Result};
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture, dav1d_open, dav1d_picture_unref, dav1d_send_data};

use super::image::{Channel, Chroma, Colorspace, PixelImage};
use super::nclx::{known_colour_primaries, known_matrix_coefficients, known_transfer_characteristics, Nclx};

/// dav1d's "try again" (`DAV1D_ERR(EAGAIN)`).
const EAGAIN: i32 = -libc::EAGAIN;

/// A decoder context closed when dropped.
struct Decoder {
    context: Option<Dav1dContext>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the context came from `dav1d_open` and is closed once.
        unsafe { dav1d_close(NonNull::new(&mut self.context)) };
    }
}

impl Decoder {
    /// `aom_new_decoder2`: one thread, the highest operating point's
    /// layer, film grain applied.
    fn open() -> Result<Decoder> {
        let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
        let settings_pointer = NonNull::new(settings.as_mut_ptr()).expect("settings");
        // SAFETY: `dav1d_default_settings` writes the whole struct.
        unsafe { dav1d_default_settings(settings_pointer) };
        // SAFETY: initialised just above.
        let mut settings = unsafe { settings.assume_init() };
        settings.n_threads = 1;
        settings.max_frame_delay = 1;
        settings.all_layers = 0;
        settings.operating_point = 0;
        settings.apply_grain = 1;

        let mut decoder = Decoder { context: None };
        // SAFETY: both pointers are valid for the call.
        let opened = unsafe { dav1d_open(NonNull::new(&mut decoder.context), NonNull::new(&mut settings)) };
        if opened.0 != 0 || decoder.context.is_none() {
            bail!("AV1 decoder could not be opened");
        }
        Ok(decoder)
    }

    /// Feeds `data` to the decoder, passing each picture it gives back to
    /// `on_picture` (which returns whether to keep going).
    fn send(&mut self, data: &[u8], on_picture: &mut dyn FnMut(&Dav1dPicture) -> Result<bool>) -> Result<()> {
        let mut input = MaybeUninit::<Dav1dData>::zeroed();
        // SAFETY: `dav1d_data_create` writes the whole struct and returns a
        // buffer of `data.len()` bytes.
        let buffer = unsafe { dav1d_data_create(NonNull::new(input.as_mut_ptr()), data.len()) };
        if buffer.is_null() {
            bail!("Memory allocation error");
        }
        // SAFETY: `buffer` holds `data.len()` writable bytes.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buffer, data.len()) };
        // SAFETY: written by `dav1d_data_create`.
        let mut input = unsafe { input.assume_init() };

        let result = self.send_input(&mut input, on_picture);
        // SAFETY: `input` came from `dav1d_data_create`.
        unsafe { dav1d_data_unref(NonNull::new(&mut input)) };
        result
    }

    fn send_input(&mut self, input: &mut Dav1dData, on_picture: &mut dyn FnMut(&Dav1dPicture) -> Result<bool>) -> Result<()> {
        let context = self.context;
        loop {
            // SAFETY: the context is open and `input` is a valid data buffer.
            let sent = unsafe { dav1d_send_data(context, NonNull::new(input)) };
            if sent.0 < 0 && sent.0 != EAGAIN {
                bail!("AV1 decoding error {}", sent.0);
            }
            loop {
                let mut picture = MaybeUninit::<Dav1dPicture>::zeroed();
                // SAFETY: `picture` is valid to write to.
                let received = unsafe { dav1d_get_picture(context, NonNull::new(picture.as_mut_ptr())) };
                if received.0 == EAGAIN {
                    break;
                }
                if received.0 != 0 {
                    bail!("AV1 decoding error {}", received.0);
                }
                // SAFETY: `dav1d_get_picture` succeeded and wrote the picture.
                let mut picture = unsafe { picture.assume_init() };
                let keep_going = on_picture(&picture);
                // SAFETY: the picture was returned by `dav1d_get_picture`.
                unsafe { dav1d_picture_unref(NonNull::new(&mut picture)) };
                if !keep_going? {
                    return Ok(());
                }
            }
            if input.sz == 0 {
                return Ok(());
            }
        }
    }
}

/// Decodes `data` (configuration OBUs and the item's OBUs) to its first
/// picture.
pub fn decode_av1(data: &[u8]) -> Result<PixelImage> {
    if data.is_empty() {
        bail!("Input with empty data extent.");
    }
    let mut decoder = Decoder::open()?;
    let mut first = None;
    let mut on_picture = |picture: &Dav1dPicture| -> Result<bool> {
        first = Some(convert_picture(picture)?);
        Ok(false)
    };
    decoder.send(data, &mut on_picture)?;
    first.ok_or_else(|| anyhow!("Decoding the input data did not give a decompressed image."))
}

/// A decoder kept across the samples of a sequence track, as libheif's aom
/// plugin keeps one: every picture a sample gives is queued.
pub struct SequenceDecoder {
    decoder: Decoder,
    /// Pictures with the user data of the sample that gave them.
    queue: VecDeque<(PixelImage, u32)>,
}

impl SequenceDecoder {
    pub fn new() -> Result<SequenceDecoder> {
        let decoder = Decoder::open()?;
        Ok(SequenceDecoder { decoder, queue: VecDeque::new() })
    }

    /// `aom_push_data2`.
    pub fn push(&mut self, data: &[u8], user_data: u32) -> Result<()> {
        let queue = &mut self.queue;
        let mut on_picture = |picture: &Dav1dPicture| -> Result<bool> {
            queue.push_back((convert_picture(picture)?, user_data));
            Ok(true)
        };
        self.decoder.send(data, &mut on_picture)
    }

    /// `aom_decode_next_image2`.
    pub fn next_image(&mut self) -> Option<(PixelImage, u32)> {
        self.queue.pop_front()
    }
}

/// `get_next_image_from_decoder`.
fn convert_picture(picture: &Dav1dPicture) -> Result<PixelImage> {
    let width = picture.p.w as u32;
    let height = picture.p.h as u32;
    let bit_depth = picture.p.bpc as u8;
    let layout = picture.p.layout;
    let (colorspace, chroma) = if layout == DAV1D_PIXEL_LAYOUT_I400 {
        (Colorspace::Monochrome, Chroma::Monochrome)
    } else if layout == DAV1D_PIXEL_LAYOUT_I420 {
        (Colorspace::YCbCr, Chroma::C420)
    } else if layout == DAV1D_PIXEL_LAYOUT_I422 {
        (Colorspace::YCbCr, Chroma::C422)
    } else {
        (Colorspace::YCbCr, Chroma::C444)
    };
    let mut image = PixelImage::new(width, height, colorspace, chroma);
    let Some(sequence) = picture.seq_hdr else {
        bail!("AV1 picture without sequence header");
    };
    // SAFETY: the picture holds a reference to its sequence header.
    let sequence = unsafe { sequence.as_ref() };
    image.nclx = Nclx {
        colour_primaries: known_colour_primaries(sequence.pri as u16),
        transfer_characteristics: known_transfer_characteristics(sequence.trc as u16),
        matrix_coefficients: known_matrix_coefficients(sequence.mtrx as u16),
        full_range: sequence.color_range != 0,
    };

    let plane_count = if chroma == Chroma::Monochrome { 1 } else { 3 };
    let channels = [Channel::Y, Channel::Cb, Channel::Cr];
    for (index, &channel) in channels.iter().enumerate().take(plane_count) {
        let (mut plane_width, mut plane_height) = (width, height);
        if index > 0 && chroma == Chroma::C420 {
            plane_width = width.div_ceil(2);
            plane_height = height.div_ceil(2);
        } else if index > 0 && chroma == Chroma::C422 {
            plane_width = width.div_ceil(2);
        }
        image.add_plane(channel, plane_width, plane_height, bit_depth);
        let Some(base) = picture.data[index] else {
            bail!("AV1 picture without plane data");
        };
        let stride = picture.stride[if index == 0 { 0 } else { 1 }];
        let mut samples = Vec::with_capacity((plane_width * plane_height) as usize);
        for y in 0..plane_height as isize {
            // SAFETY: rows of the plane lie `stride` bytes apart and hold
            // at least `plane_width` samples.
            let row = unsafe { (base.as_ptr() as *const u8).offset(y * stride) };
            for x in 0..plane_width as usize {
                let value = if bit_depth > 8 {
                    // SAFETY: as above, two bytes a sample.
                    unsafe { (row as *const u16).add(x).read_unaligned() }
                } else {
                    // SAFETY: as above, one byte a sample.
                    unsafe { *row.add(x) as u16 }
                };
                samples.push(value);
            }
        }
        image.plane_mut(channel).samples = samples;
    }
    Ok(image)
}
