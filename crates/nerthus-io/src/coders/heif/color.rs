//! libheif's colour conversion (`color-conversion/*.cc`): a cheapest-path
//! search over its operations from the decoded image's state to the one
//! asked for, then the operations themselves. The search keeps libheif's
//! operation order and tie-breaking, since equal-cost pipelines differ in
//! their arithmetic.

use anyhow::{bail, Result};

use super::image::{Channel, Chroma, Colorspace, PixelImage};
use super::nclx::{rgb_to_ycbcr_coefficients, ycbcr_to_rgb_coefficients, Nclx, YcbcrToRgb};

const COST_TRIVIAL: i32 = 1;
const COST_OPTIMIZED_SOFTWARE: i32 = 6;
const COST_UNOPTIMIZED: i32 = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromaUpsampling {
    NearestNeighbor,
    Bilinear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromaDownsampling {
    NearestNeighbor,
    Average,
    SharpYuv,
}

/// `heif_color_conversion_options` (the alpha composition of the
/// extended options is always "none" here).
#[derive(Clone, Copy, Debug)]
pub struct ConversionOptions {
    pub downsampling: ChromaDownsampling,
    pub upsampling: ChromaUpsampling,
    pub only_use_preferred: bool,
}

impl ConversionOptions {
    /// The decoding defaults (`fill_default_decoding_options`).
    pub fn decoding_defaults() -> ConversionOptions {
        ConversionOptions { downsampling: ChromaDownsampling::Average, upsampling: ChromaUpsampling::Bilinear, only_use_preferred: false }
    }

    /// `heif_color_conversion_options_set_defaults`, which the transforms
    /// use when they need 4:4:4 (sharp YUV where libheif has it).
    pub fn defaults_for_transforms() -> ConversionOptions {
        ConversionOptions { downsampling: ChromaDownsampling::SharpYuv, upsampling: ChromaUpsampling::Bilinear, only_use_preferred: true }
    }
}

/// `ColorState`.
#[derive(Clone, Copy, Debug)]
struct ColorState {
    colorspace: Colorspace,
    chroma: Chroma,
    has_alpha: bool,
    bits_per_pixel: i32,
    nclx: Nclx,
}

impl ColorState {
    fn new(colorspace: Colorspace, chroma: Chroma, has_alpha: bool, bits_per_pixel: i32) -> ColorState {
        ColorState { colorspace, chroma, has_alpha, bits_per_pixel, nclx: Nclx::undefined() }
    }
}

impl PartialEq for ColorState {
    fn eq(&self, other: &ColorState) -> bool {
        let main = self.colorspace == other.colorspace && self.chroma == other.chroma && self.has_alpha == other.has_alpha && self.bits_per_pixel == other.bits_per_pixel;
        if !main {
            return false;
        }
        if self.colorspace == Colorspace::YCbCr {
            return self.nclx.equal_except_transfer(&other.nclx);
        }
        true
    }
}

/// The operations, in `init_ops`'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    RgbToRgb2432,
    Rgb2432ToRgb,
    YcbcrToRgb16,
    YcbcrToRgb8,
    Ycbcr420ToRgb24,
    Ycbcr420ToRgb32,
    Ycbcr420ToRrggbbaa,
    RgbHdrToRrggbbaaBe,
    RgbToRrggbbaaBe,
    MonoToYcbcr420,
    MonoToRgb2432,
    RrggbbaaSwapEndianness,
    RrggbbaaBeToRgbHdr,
    Rgb2432ToYcbcr,
    RgbToYcbcr8,
    RgbToYcbcr16,
    RrggbbxxHdrToYcbcr420,
    Rgb2432ToYcbcr444Gbr,
    DropAlphaPlane,
    FlattenAlphaPlane8,
    FlattenAlphaPlane16,
    ToHdrPlanes,
    ToSdrPlanes,
    Ycbcr420BilinearToYcbcr444_8,
    Ycbcr420BilinearToYcbcr444_16,
    Ycbcr422BilinearToYcbcr444_8,
    Ycbcr422BilinearToYcbcr444_16,
    Ycbcr444ToYcbcr420Average8,
    Ycbcr444ToYcbcr420Average16,
    Ycbcr444ToYcbcr422Average8,
    Ycbcr444ToYcbcr422Average16,
    AnyRgbToYcbcr420Sharp,
}

const OPS: [Op; 32] = [
    Op::RgbToRgb2432,
    Op::Rgb2432ToRgb,
    Op::YcbcrToRgb16,
    Op::YcbcrToRgb8,
    Op::Ycbcr420ToRgb24,
    Op::Ycbcr420ToRgb32,
    Op::Ycbcr420ToRrggbbaa,
    Op::RgbHdrToRrggbbaaBe,
    Op::RgbToRrggbbaaBe,
    Op::MonoToYcbcr420,
    Op::MonoToRgb2432,
    Op::RrggbbaaSwapEndianness,
    Op::RrggbbaaBeToRgbHdr,
    Op::Rgb2432ToYcbcr,
    Op::RgbToYcbcr8,
    Op::RgbToYcbcr16,
    Op::RrggbbxxHdrToYcbcr420,
    Op::Rgb2432ToYcbcr444Gbr,
    Op::DropAlphaPlane,
    Op::FlattenAlphaPlane8,
    Op::FlattenAlphaPlane16,
    Op::ToHdrPlanes,
    Op::ToSdrPlanes,
    Op::Ycbcr420BilinearToYcbcr444_8,
    Op::Ycbcr420BilinearToYcbcr444_16,
    Op::Ycbcr422BilinearToYcbcr444_8,
    Op::Ycbcr422BilinearToYcbcr444_16,
    Op::Ycbcr444ToYcbcr420Average8,
    Op::Ycbcr444ToYcbcr420Average16,
    Op::Ycbcr444ToYcbcr422Average8,
    Op::Ycbcr444ToYcbcr422Average16,
    Op::AnyRgbToYcbcr420Sharp,
];

fn is_rrggbb(chroma: Chroma) -> bool {
    matches!(chroma, Chroma::InterleavedRrggbbBe | Chroma::InterleavedRrggbbLe | Chroma::InterleavedRrggbbaaBe | Chroma::InterleavedRrggbbaaLe)
}

/// The upsampling check every YCbCr-to-RGB operation starts with.
fn nearest_chroma_allowed(input: &ColorState, options: &ConversionOptions) -> bool {
    !(input.chroma != Chroma::C444 && options.upsampling != ChromaUpsampling::NearestNeighbor && options.only_use_preferred)
}

/// `state_after_conversion` of each operation.
fn states_after(op: Op, input: &ColorState, target: &ColorState, options: &ConversionOptions) -> Vec<(ColorState, i32)> {
    let mut states = Vec::new();
    let matrix = input.nclx.matrix_coefficients;
    match op {
        Op::RgbToRgb2432 => {
            if input.colorspace == Colorspace::Rgb && input.chroma == Chroma::C444 && input.bits_per_pixel == 8 {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgba, true, 8), COST_UNOPTIMIZED));
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgb, false, 8), COST_UNOPTIMIZED));
            }
        }
        Op::Rgb2432ToRgb => {
            let interleaved = input.chroma == Chroma::InterleavedRgb || input.chroma == Chroma::InterleavedRgba;
            if input.colorspace == Colorspace::Rgb && interleaved && input.bits_per_pixel == 8 {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::C444, target.has_alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::YcbcrToRgb16 | Op::YcbcrToRgb8 => {
            let hdr = op == Op::YcbcrToRgb16;
            let subsampled_ok = matches!(input.chroma, Chroma::C444 | Chroma::C422 | Chroma::C420);
            if nearest_chroma_allowed(input, options)
                && input.colorspace == Colorspace::YCbCr
                && subsampled_ok
                && matrix != 11
                && matrix != 14
                && (input.bits_per_pixel > 8) == hdr
                && input.bits_per_pixel >= 8
                && input.bits_per_pixel <= 14
            {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::C444, input.has_alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::Ycbcr420ToRgb24 => {
            if nearest_chroma_allowed(input, options)
                && input.colorspace == Colorspace::YCbCr
                && input.chroma == Chroma::C420
                && input.bits_per_pixel == 8
                && !input.has_alpha
                && !matches!(matrix, 0 | 8 | 11 | 14)
                && input.nclx.full_range
            {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgb, false, 8), COST_UNOPTIMIZED));
            }
        }
        Op::Ycbcr420ToRgb32 => {
            if nearest_chroma_allowed(input, options)
                && input.colorspace == Colorspace::YCbCr
                && input.chroma == Chroma::C420
                && input.bits_per_pixel == 8
                && !matches!(matrix, 0 | 8 | 11 | 14)
                && input.nclx.full_range
            {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgba, true, 8), COST_UNOPTIMIZED));
            }
        }
        Op::Ycbcr420ToRrggbbaa => {
            if nearest_chroma_allowed(input, options) && input.colorspace == Colorspace::YCbCr && input.chroma == Chroma::C420 && input.bits_per_pixel > 8 && !matches!(matrix, 0 | 8 | 11 | 14) {
                let le = if input.has_alpha { Chroma::InterleavedRrggbbaaLe } else { Chroma::InterleavedRrggbbLe };
                let be = if input.has_alpha { Chroma::InterleavedRrggbbaaBe } else { Chroma::InterleavedRrggbbBe };
                states.push((ColorState::new(Colorspace::Rgb, le, input.has_alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
                states.push((ColorState::new(Colorspace::Rgb, be, input.has_alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::RgbHdrToRrggbbaaBe => {
            if input.colorspace == Colorspace::Rgb && input.chroma == Chroma::C444 && input.bits_per_pixel > 8 {
                if !input.has_alpha {
                    states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRrggbbBe, false, input.bits_per_pixel), COST_UNOPTIMIZED));
                }
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRrggbbaaBe, true, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::RgbToRrggbbaaBe => {
            if input.colorspace == Colorspace::Rgb && input.chroma == Chroma::C444 && input.bits_per_pixel == 8 {
                if !input.has_alpha {
                    states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRrggbbBe, false, input.bits_per_pixel), COST_UNOPTIMIZED));
                }
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRrggbbaaBe, true, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::MonoToYcbcr420 => {
            if input.colorspace == Colorspace::Monochrome && input.chroma == Chroma::Monochrome {
                states.push((ColorState::new(Colorspace::YCbCr, Chroma::C420, input.has_alpha, input.bits_per_pixel), COST_OPTIMIZED_SOFTWARE));
            }
        }
        Op::MonoToRgb2432 => {
            if input.colorspace == Colorspace::Monochrome && input.chroma == Chroma::Monochrome && input.bits_per_pixel == 8 {
                if !input.has_alpha {
                    states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgb, false, 8), COST_UNOPTIMIZED));
                }
                states.push((ColorState::new(Colorspace::Rgb, Chroma::InterleavedRgba, true, 8), COST_UNOPTIMIZED));
            }
        }
        Op::RrggbbaaSwapEndianness => {
            if input.colorspace == Colorspace::Rgb && is_rrggbb(input.chroma) {
                let swapped = match input.chroma {
                    Chroma::InterleavedRrggbbLe => Chroma::InterleavedRrggbbBe,
                    Chroma::InterleavedRrggbbBe => Chroma::InterleavedRrggbbLe,
                    Chroma::InterleavedRrggbbaaLe => Chroma::InterleavedRrggbbaaBe,
                    _ => Chroma::InterleavedRrggbbaaLe,
                };
                let alpha = swapped.interleaved_with_alpha();
                states.push((ColorState::new(Colorspace::Rgb, swapped, alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::RrggbbaaBeToRgbHdr => {
            let be = input.chroma == Chroma::InterleavedRrggbbBe || input.chroma == Chroma::InterleavedRrggbbaaBe;
            if input.colorspace == Colorspace::Rgb && be && input.bits_per_pixel > 8 {
                states.push((ColorState::new(Colorspace::Rgb, Chroma::C444, target.has_alpha, input.bits_per_pixel), COST_UNOPTIMIZED));
            }
        }
        Op::Rgb2432ToYcbcr => {
            let downsampling_ok = !(target.chroma != Chroma::C444 && options.downsampling != ChromaDownsampling::NearestNeighbor && options.only_use_preferred);
            let interleaved = input.chroma == Chroma::InterleavedRgb || input.chroma == Chroma::InterleavedRgba;
            let target_matrix = target.nclx.matrix_coefficients;
            if downsampling_ok
                && input.colorspace == Colorspace::Rgb
                && interleaved
                && matches!(target.chroma, Chroma::C420 | Chroma::C422 | Chroma::C444)
                && !matches!(target_matrix, 0 | 8 | 11 | 14)
            {
                let mut state = ColorState::new(Colorspace::YCbCr, target.chroma, target.has_alpha, 8);
                state.nclx = target.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::RgbToYcbcr8 | Op::RgbToYcbcr16 => {
            let hdr = op == Op::RgbToYcbcr16;
            let target_matrix = target.nclx.matrix_coefficients;
            if (input.bits_per_pixel > 8) == hdr && input.bits_per_pixel >= 8 && input.colorspace == Colorspace::Rgb && input.chroma == Chroma::C444 && target_matrix != 11 && target_matrix != 14 {
                let use_target_chroma = target.chroma != Chroma::C444 && (options.downsampling == ChromaDownsampling::NearestNeighbor || !options.only_use_preferred);
                let chroma = if use_target_chroma { target.chroma } else { Chroma::C444 };
                let mut state = ColorState::new(Colorspace::YCbCr, chroma, input.has_alpha, input.bits_per_pixel);
                state.nclx = target.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::RrggbbxxHdrToYcbcr420 => {
            let downsampling_ok = !(target.chroma != Chroma::C444 && options.downsampling != ChromaDownsampling::NearestNeighbor && options.only_use_preferred);
            let target_matrix = target.nclx.matrix_coefficients;
            if downsampling_ok
                && input.colorspace == Colorspace::Rgb
                && is_rrggbb(input.chroma)
                && input.bits_per_pixel > 8
                && !matches!(target_matrix, 0 | 8 | 11 | 14)
                && target.nclx.full_range
                && target.chroma == Chroma::C420
            {
                let mut state = ColorState::new(Colorspace::YCbCr, Chroma::C420, input.has_alpha, input.bits_per_pixel);
                state.nclx = target.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::Rgb2432ToYcbcr444Gbr => {
            let interleaved = input.chroma == Chroma::InterleavedRgb || input.chroma == Chroma::InterleavedRgba;
            if input.colorspace == Colorspace::Rgb && interleaved && target.nclx.matrix_coefficients == 0 && target.nclx.full_range {
                let mut state = ColorState::new(Colorspace::YCbCr, Chroma::C444, target.has_alpha, 8);
                state.nclx = target.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::DropAlphaPlane => {
            if input.chroma.is_planar() && input.has_alpha && !target.has_alpha {
                let mut state = *input;
                state.has_alpha = false;
                states.push((state, COST_TRIVIAL));
            }
        }
        // The alpha composition mode is always "none" for these decodes.
        Op::FlattenAlphaPlane8 | Op::FlattenAlphaPlane16 => {}
        Op::ToHdrPlanes => {
            if input.chroma.is_planar() && input.bits_per_pixel == 8 {
                let mut state = *input;
                state.bits_per_pixel = target.bits_per_pixel;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::ToSdrPlanes => {
            if input.chroma.is_planar() && input.bits_per_pixel != 8 && target.bits_per_pixel == 8 {
                let mut state = *input;
                state.bits_per_pixel = 8;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::Ycbcr420BilinearToYcbcr444_8 | Op::Ycbcr420BilinearToYcbcr444_16 | Op::Ycbcr422BilinearToYcbcr444_8 | Op::Ycbcr422BilinearToYcbcr444_16 => {
            let source = if matches!(op, Op::Ycbcr420BilinearToYcbcr444_8 | Op::Ycbcr420BilinearToYcbcr444_16) { Chroma::C420 } else { Chroma::C422 };
            let hdr = matches!(op, Op::Ycbcr420BilinearToYcbcr444_16 | Op::Ycbcr422BilinearToYcbcr444_16);
            if input.colorspace == Colorspace::YCbCr && input.chroma == source && options.upsampling == ChromaUpsampling::Bilinear && (input.bits_per_pixel > 8) == hdr && matrix != 0 {
                let mut state = ColorState::new(Colorspace::YCbCr, Chroma::C444, input.has_alpha, input.bits_per_pixel);
                state.nclx = input.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        Op::Ycbcr444ToYcbcr420Average8 | Op::Ycbcr444ToYcbcr420Average16 | Op::Ycbcr444ToYcbcr422Average8 | Op::Ycbcr444ToYcbcr422Average16 => {
            let goal = if matches!(op, Op::Ycbcr444ToYcbcr420Average8 | Op::Ycbcr444ToYcbcr420Average16) { Chroma::C420 } else { Chroma::C422 };
            let hdr = matches!(op, Op::Ycbcr444ToYcbcr420Average16 | Op::Ycbcr444ToYcbcr422Average16);
            if input.colorspace == Colorspace::YCbCr && input.chroma == Chroma::C444 && options.downsampling == ChromaDownsampling::Average && (input.bits_per_pixel > 8) == hdr && matrix != 0 && target.chroma == goal {
                let mut state = ColorState::new(Colorspace::YCbCr, goal, input.has_alpha, input.bits_per_pixel);
                state.nclx = input.nclx;
                states.push((state, COST_UNOPTIMIZED));
            }
        }
        // Sharp YUV only produces 4:2:0 for 4:2:0 targets, which these
        // decodes never ask for.
        Op::AnyRgbToYcbcr420Sharp => {}
    }
    states
}

struct Node {
    previous: Option<usize>,
    op: Option<Op>,
    state: ColorState,
    cost: i32,
}

/// `construct_pipeline`: the operations from `input` to `target`, or
/// `None` when no path exists.
fn construct_pipeline(input: ColorState, target: ColorState, options: &ConversionOptions) -> Option<Vec<(Op, ColorState, ColorState)>> {
    if input == target {
        return Some(Vec::new());
    }
    let mut processed: Vec<Node> = Vec::new();
    let mut border: Vec<Node> = vec![Node { previous: None, op: None, state: input, cost: 0 }];
    while !border.is_empty() {
        let mut minimum_index = 0;
        let mut minimum_cost = i32::MAX;
        for (index, node) in border.iter().enumerate() {
            if node.cost < minimum_cost {
                minimum_index = index;
                minimum_cost = node.cost;
            }
        }
        let last = border.len() - 1;
        border.swap(minimum_index, last);
        let node = border.pop().expect("border node");
        processed.push(node);
        let current_index = processed.len() - 1;
        let current_state = processed[current_index].state;
        let current_cost = processed[current_index].cost;

        if current_state == target {
            let mut steps = Vec::new();
            let mut index = current_index;
            while let Some(previous) = processed[index].previous {
                let op = processed[index].op.expect("pipeline op");
                steps.push((op, processed[previous].state, processed[index].state));
                index = previous;
            }
            steps.reverse();
            // The first step's input is the image's own state.
            if let Some(first) = steps.first_mut() {
                first.1 = input;
            }
            return Some(steps);
        }

        for op in OPS {
            for (state, cost) in states_after(op, &current_state, &target, options) {
                let new_cost = cost + current_cost;
                if processed.iter().any(|node| node.state == state) {
                    continue;
                }
                if let Some(existing) = border.iter_mut().find(|node| node.state == state) {
                    if existing.cost > new_cost {
                        *existing = Node { previous: Some(current_index), op: Some(op), state, cost: new_cost };
                    }
                    continue;
                }
                border.push(Node { previous: Some(current_index), op: Some(op), state, cost: new_cost });
            }
        }
    }
    None
}

/// `convert_colorspace`.
pub fn convert_colorspace(input: &PixelImage, target_colorspace: Colorspace, target_chroma: Chroma, target_nclx: Nclx, output_bpp: i32, options: &ConversionOptions) -> Result<PixelImage> {
    let width = input.width;
    let height = input.height;
    if input.has_channel(Channel::Alpha) && (input.channel_width(Channel::Alpha) != width || input.channel_height(Channel::Alpha) != height) {
        bail!("Internal error");
    }
    if target_colorspace == Colorspace::YCbCr && !matches!(target_chroma, Chroma::C420 | Chroma::C422 | Chroma::C444) {
        bail!("Internal error");
    }

    let input_nclx = if input.has_nclx() { input.nclx } else { Nclx::undefined() };
    let first_channel = *input.planes.keys().next().expect("image planes");
    let input_state = ColorState {
        colorspace: input.colorspace,
        chroma: input.chroma,
        has_alpha: input.has_channel(Channel::Alpha) || input.chroma.interleaved_with_alpha(),
        bits_per_pixel: input.bits_per_pixel(first_channel) as i32,
        nclx: input_nclx.with_srgb_defaults_for_unspecified(),
    };

    let mut output_state = input_state;
    output_state.colorspace = target_colorspace;
    output_state.chroma = target_chroma;
    output_state.nclx = target_nclx;
    if output_state.nclx.matrix_coefficients == 2 {
        output_state.nclx.matrix_coefficients = input_state.nclx.matrix_coefficients;
    }
    if output_state.nclx.colour_primaries == 2 {
        output_state.nclx.colour_primaries = input_state.nclx.colour_primaries;
    }
    if output_state.nclx.transfer_characteristics == 2 {
        output_state.nclx.transfer_characteristics = input_state.nclx.transfer_characteristics;
    }
    if target_chroma.interleaved_components() > 1 {
        output_state.has_alpha = target_chroma.interleaved_with_alpha();
    } else {
        output_state.has_alpha = input_state.has_alpha;
    }
    if output_bpp != 0 {
        output_state.bits_per_pixel = output_bpp;
    }
    if target_chroma == Chroma::InterleavedRgb || target_chroma == Chroma::InterleavedRgba {
        output_state.bits_per_pixel = 8;
    }
    if is_rrggbb(target_chroma) && output_state.bits_per_pixel <= 8 {
        output_state.bits_per_pixel = 10;
    }

    let Some(steps) = construct_pipeline(input_state, output_state, options) else {
        bail!("Unsupported color conversion");
    };
    let mut image = input.clone();
    for (op, step_input, step_output) in steps {
        let mut converted = run_op(op, &image, &step_input, &step_output)?;
        converted.nclx = step_output.nclx;
        converted.premultiplied_alpha = image.premultiplied_alpha;
        image = converted;
    }
    Ok(image)
}

/// `clip_f_u16`.
fn clip_f_u16(value: f32, maximum: i32) -> u16 {
    let x = (value + 0.5) as i32;
    if x < 0 {
        0
    } else if x > maximum {
        maximum as u16
    } else {
        x as u16
    }
}

fn clip_int_u8(value: i32) -> u16 {
    value.clamp(0, 255) as u16
}

fn clip_int_u16(value: i32, maximum: u16) -> u16 {
    if value < 0 {
        0
    } else if value > maximum as i32 {
        maximum
    } else {
        value as u16
    }
}

/// The YCbCr matrix and range an operation reads from the image's own
/// profile (`has_nclx_color_profile` or the defaults).
fn image_matrix(image: &PixelImage) -> (u16, bool, YcbcrToRgb) {
    if image.has_nclx() {
        let nclx = image.nclx;
        (nclx.matrix_coefficients, nclx.full_range, ycbcr_to_rgb_coefficients(nclx.matrix_coefficients, nclx.colour_primaries))
    } else {
        (2, true, YcbcrToRgb::defaults())
    }
}

fn copy_alpha(input: &PixelImage, output: &mut PixelImage) {
    if let Some(alpha) = input.planes.get(&Channel::Alpha) {
        output.planes.insert(Channel::Alpha, alpha.clone());
    }
}

fn run_op(op: Op, input: &PixelImage, input_state: &ColorState, target: &ColorState) -> Result<PixelImage> {
    let _ = input_state;
    match op {
        Op::YcbcrToRgb8 | Op::YcbcrToRgb16 => ycbcr_to_rgb(input, op == Op::YcbcrToRgb16),
        Op::Ycbcr420ToRgb24 | Op::Ycbcr420ToRgb32 => ycbcr420_to_rgb_interleaved(input, op == Op::Ycbcr420ToRgb32),
        Op::Ycbcr420ToRrggbbaa => ycbcr420_to_rrggbbaa(input, target),
        Op::RgbToRgb2432 => rgb_to_rgb24_32(input, target),
        Op::Rgb2432ToRgb => rgb24_32_to_rgb(input, target),
        Op::RgbHdrToRrggbbaaBe | Op::RgbToRrggbbaaBe => rgb_to_rrggbbaa(input, target, op == Op::RgbHdrToRrggbbaaBe),
        Op::RrggbbaaBeToRgbHdr => rrggbbaa_to_rgb(input, target),
        Op::RrggbbaaSwapEndianness => {
            let mut output = input.clone();
            output.chroma = target.chroma;
            Ok(output)
        }
        Op::MonoToYcbcr420 => mono_to_ycbcr420(input),
        Op::MonoToRgb2432 => mono_to_rgb24_32(input, target),
        Op::DropAlphaPlane => {
            let mut output = input.clone();
            output.planes.remove(&Channel::Alpha);
            Ok(output)
        }
        Op::ToHdrPlanes => to_hdr_planes(input, target),
        Op::ToSdrPlanes => to_sdr_planes(input),
        Op::Ycbcr420BilinearToYcbcr444_8 | Op::Ycbcr420BilinearToYcbcr444_16 => ycbcr420_bilinear_to_444(input, op == Op::Ycbcr420BilinearToYcbcr444_16),
        Op::Ycbcr422BilinearToYcbcr444_8 | Op::Ycbcr422BilinearToYcbcr444_16 => ycbcr422_bilinear_to_444(input, op == Op::Ycbcr422BilinearToYcbcr444_16),
        Op::RgbToYcbcr8 | Op::RgbToYcbcr16 => rgb_to_ycbcr(input, target, op == Op::RgbToYcbcr16),
        _ => bail!("color conversion {op:?} is not supported"),
    }
}

/// `Op_YCbCr_to_RGB`.
fn ycbcr_to_rgb(input: &PixelImage, hdr: bool) -> Result<PixelImage> {
    let bpp_y = input.bits_per_pixel(Channel::Y);
    let bpp_cb = input.bits_per_pixel(Channel::Cb);
    let bpp_cr = input.bits_per_pixel(Channel::Cr);
    if !hdr {
        if bpp_y != 8 || bpp_cb != 8 || bpp_cr != 8 {
            bail!("Internal error");
        }
    } else if bpp_y == 8 || bpp_cb == 8 || bpp_cr == 8 {
        bail!("Internal error");
    }
    if bpp_y != bpp_cb || bpp_y != bpp_cr {
        bail!("Internal error");
    }
    let width = input.width;
    let height = input.height;
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, Chroma::C444);
    for channel in [Channel::R, Channel::G, Channel::B] {
        output.add_plane(channel, width, height, bpp_y);
    }
    copy_alpha(input, &mut output);

    let half_range = 1i32 << (bpp_y - 1);
    let half_range_chroma = 1i32 << (bpp_cb - 1);
    let full_range_value = (1i32 << bpp_y) - 1;
    let limited_offset = (16i32 << (bpp_y - 8)) as f32;
    let shift_h = input.chroma.h_subsampling() - 1;
    let shift_v = input.chroma.v_subsampling() - 1;
    let (matrix, full_range, coefficients) = image_matrix(input);

    let y_plane = input.plane(Channel::Y);
    let cb_plane = input.plane(Channel::Cb);
    let cr_plane = input.plane(Channel::Cr);
    let mut red = vec![0u16; (width * height) as usize];
    let mut green = vec![0u16; (width * height) as usize];
    let mut blue = vec![0u16; (width * height) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let cx = x >> shift_h;
            let cy = y >> shift_v;
            let luma = y_plane.get(x, y) as i32;
            let cb_value = cb_plane.get(cx, cy) as i32;
            let cr_value = cr_plane.get(cx, cy) as i32;
            let index = y * width as usize + x;
            if matrix == 0 {
                if full_range {
                    red[index] = cr_value as u16;
                    green[index] = luma as u16;
                    blue[index] = cb_value as u16;
                } else {
                    red[index] = clip_f_u16((cr_value as f32 - limited_offset) * 1.1429, full_range_value);
                    green[index] = clip_f_u16((luma as f32 - limited_offset) * 1.1689, full_range_value);
                    blue[index] = clip_f_u16((cb_value as f32 - limited_offset) * 1.1429, full_range_value);
                }
            } else if matrix == 8 {
                let cb = cb_value - half_range;
                let cr = cr_value - half_range;
                red[index] = clip_int_u8(luma - cb + cr);
                green[index] = clip_int_u8(luma + cb);
                blue[index] = clip_int_u8(luma - cb - cr);
            } else if matrix == 16 {
                let yy = luma as i16;
                let cb = (cb_value as i16).wrapping_sub(half_range_chroma as i16);
                let cr = (cr_value as i16).wrapping_sub(half_range_chroma as i16);
                let t = yy.wrapping_sub(cb >> 1);
                let g = t.wrapping_add(cb);
                let b = t.wrapping_sub(cr >> 1);
                let r = b.wrapping_add(cr);
                let maximum = ((1i32 << bpp_y) - 1) as u16;
                red[index] = clip_int_u16(r as i32 * 4, maximum);
                green[index] = clip_int_u16(g as i32 * 4, maximum);
                blue[index] = clip_int_u16(b as i32 * 4, maximum);
            } else {
                let mut yv = luma as f32;
                let mut cb = (cb_value - half_range) as f32;
                let mut cr = (cr_value - half_range) as f32;
                if !full_range {
                    yv = (yv - limited_offset) * 1.1689;
                    cb *= 1.1429;
                    cr *= 1.1429;
                }
                red[index] = clip_f_u16(yv + coefficients.r_cr * cr, full_range_value);
                green[index] = clip_f_u16(yv + coefficients.g_cb * cb + coefficients.g_cr * cr, full_range_value);
                blue[index] = clip_f_u16(yv + coefficients.b_cb * cb, full_range_value);
            }
        }
    }
    output.plane_mut(Channel::R).samples = red;
    output.plane_mut(Channel::G).samples = green;
    output.plane_mut(Channel::B).samples = blue;
    Ok(output)
}

/// `std::lround`.
fn lround(value: f32) -> i32 {
    let value = value as f64;
    let rounded = if value >= 0.0 { (value + 0.5).floor() } else { (value - 0.5).ceil() };
    rounded as i32
}

/// `Op_YCbCr420_to_RGB24` and `Op_YCbCr420_to_RGB32`.
fn ycbcr420_to_rgb_interleaved(input: &PixelImage, with_alpha_output: bool) -> Result<PixelImage> {
    if input.bits_per_pixel(Channel::Y) != 8 || input.bits_per_pixel(Channel::Cb) != 8 || input.bits_per_pixel(Channel::Cr) != 8 {
        bail!("Internal error");
    }
    let width = input.width;
    let height = input.height;
    let chroma = if with_alpha_output { Chroma::InterleavedRgba } else { Chroma::InterleavedRgb };
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, chroma);
    output.add_plane(Channel::Interleaved, width, height, 8);
    let coefficients = if input.has_nclx() { ycbcr_to_rgb_coefficients(input.nclx.matrix_coefficients, input.nclx.colour_primaries) } else { YcbcrToRgb::defaults() };
    let r_cr = lround(256.0 * coefficients.r_cr);
    let g_cr = lround(256.0 * coefficients.g_cr);
    let g_cb = lround(256.0 * coefficients.g_cb);
    let b_cb = lround(256.0 * coefficients.b_cb);
    let y_plane = input.plane(Channel::Y);
    let cb_plane = input.plane(Channel::Cb);
    let cr_plane = input.plane(Channel::Cr);
    let alpha = input.planes.get(&Channel::Alpha);
    let components = chroma.interleaved_components();
    let out = output.plane_mut(Channel::Interleaved);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let yv = y_plane.get(x, y) as i32;
            let cb = cb_plane.get(x / 2, y / 2) as i32 - 128;
            let cr = cr_plane.get(x / 2, y / 2) as i32 - 128;
            let base = y * out.stride() + components * x;
            out.samples[base] = clip_int_u8(yv + ((r_cr * cr + 128) >> 8));
            out.samples[base + 1] = clip_int_u8(yv + ((g_cb * cb + g_cr * cr + 128) >> 8));
            out.samples[base + 2] = clip_int_u8(yv + ((b_cb * cb + 128) >> 8));
            if with_alpha_output {
                out.samples[base + 3] = alpha.map_or(0xff, |plane| plane.get(x, y) & 0xff);
            }
        }
    }
    Ok(output)
}

/// `Op_YCbCr420_to_RRGGBBaa`.
fn ycbcr420_to_rrggbbaa(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    let width = input.width;
    let height = input.height;
    let bpp = input.bits_per_pixel(Channel::Y);
    let has_alpha = input.has_channel(Channel::Alpha);
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, target.chroma);
    output.add_plane(Channel::Interleaved, width, height, bpp);
    if has_alpha {
        if input.channel_width(Channel::Alpha) != width || input.channel_height(Channel::Alpha) != height {
            bail!("Color conversion cannot handle alpha images with sizes differing from the main image size.");
        }
        if input.bits_per_pixel(Channel::Alpha) != bpp {
            bail!("Color conversion cannot handle alpha images with bits-per-pixel differing from the main image.");
        }
        output.add_plane(Channel::Alpha, width, height, bpp);
    }
    let maximum = (1i32 << bpp) - 1;
    let (full_range, coefficients) = if input.has_nclx() { (input.nclx.full_range, ycbcr_to_rgb_coefficients(input.nclx.matrix_coefficients, input.nclx.colour_primaries)) } else { (true, YcbcrToRgb::defaults()) };
    let limited_offset = (16i32 << (bpp - 8)) as f32;
    let half = 1i32 << (bpp - 1);
    let y_plane = input.plane(Channel::Y).clone();
    let cb_plane = input.plane(Channel::Cb).clone();
    let cr_plane = input.plane(Channel::Cr).clone();
    let alpha = input.planes.get(&Channel::Alpha).cloned();
    let components = target.chroma.interleaved_components();
    let out = output.plane_mut(Channel::Interleaved);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut luma = y_plane.get(x, y) as f32;
            let mut cb = (cb_plane.get(x / 2, y / 2) as i32 - half) as f32;
            let mut cr = (cr_plane.get(x / 2, y / 2) as i32 - half) as f32;
            if !full_range {
                luma = (luma - limited_offset) * 1.1689;
                cb *= 1.1429;
                cr *= 1.1429;
            }
            let base = y * out.stride() + components * x;
            out.samples[base] = clip_f_u16(luma + coefficients.r_cr * cr, maximum);
            out.samples[base + 1] = clip_f_u16(luma + coefficients.g_cb * cb + coefficients.g_cr * cr, maximum);
            out.samples[base + 2] = clip_f_u16(luma + coefficients.b_cb * cb, maximum);
            if let Some(alpha) = &alpha
                && components == 4
            {
                out.samples[base + 3] = alpha.get(x, y);
            }
        }
    }
    if let Some(alpha) = alpha {
        output.planes.insert(Channel::Alpha, alpha);
    }
    Ok(output)
}

/// `Op_RGB_to_RGB24_32`.
fn rgb_to_rgb24_32(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    let has_alpha = input.has_channel(Channel::Alpha);
    let want_alpha = target.has_alpha;
    if input.bits_per_pixel(Channel::R) != 8 || input.bits_per_pixel(Channel::G) != 8 || input.bits_per_pixel(Channel::B) != 8 {
        bail!("Internal error");
    }
    if has_alpha && input.bits_per_pixel(Channel::Alpha) != 8 {
        bail!("Internal error");
    }
    let width = input.width;
    let height = input.height;
    let chroma = if want_alpha { Chroma::InterleavedRgba } else { Chroma::InterleavedRgb };
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, chroma);
    output.add_plane(Channel::Interleaved, width, height, 8);
    let components = chroma.interleaved_components();
    let red = input.plane(Channel::R);
    let green = input.plane(Channel::G);
    let blue = input.plane(Channel::B);
    let alpha = input.planes.get(&Channel::Alpha);
    let out = output.plane_mut(Channel::Interleaved);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = y * out.stride() + components * x;
            out.samples[base] = red.get(x, y);
            out.samples[base + 1] = green.get(x, y);
            out.samples[base + 2] = blue.get(x, y);
            if want_alpha {
                out.samples[base + 3] = if has_alpha { alpha.map_or(0xff, |plane| plane.get(x, y)) } else { 0xff };
            }
        }
    }
    Ok(output)
}

/// `Op_RGB24_32_to_RGB`.
fn rgb24_32_to_rgb(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    let has_alpha = input.chroma == Chroma::InterleavedRgba;
    let want_alpha = target.has_alpha;
    let width = input.width;
    let height = input.height;
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, Chroma::C444);
    for channel in [Channel::R, Channel::G, Channel::B] {
        output.add_plane(channel, width, height, 8);
    }
    if want_alpha {
        output.add_plane(Channel::Alpha, width, height, 8);
    }
    let source = input.plane(Channel::Interleaved).clone();
    let components = if has_alpha { 4 } else { 3 };
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = y * source.stride() + components * x;
            output.plane_mut(Channel::R).set(x, y, source.samples[base]);
            output.plane_mut(Channel::G).set(x, y, source.samples[base + 1]);
            output.plane_mut(Channel::B).set(x, y, source.samples[base + 2]);
            if want_alpha {
                let value = if has_alpha { source.samples[base + 3] } else { 0xff };
                output.plane_mut(Channel::Alpha).set(x, y, value);
            }
        }
    }
    Ok(output)
}

/// `Op_RGB_HDR_to_RRGGBBaa_BE` and `Op_RGB_to_RRGGBBaa_BE`.
fn rgb_to_rrggbbaa(input: &PixelImage, target: &ColorState, hdr: bool) -> Result<PixelImage> {
    let input_has_alpha = input.has_channel(Channel::Alpha);
    let output_has_alpha = input_has_alpha || target.has_alpha;
    let depths = [input.bits_per_pixel(Channel::R), input.bits_per_pixel(Channel::G), input.bits_per_pixel(Channel::B)];
    if hdr {
        if depths.iter().any(|&depth| depth <= 8) {
            bail!("Internal error");
        }
        if input_has_alpha {
            if input.bits_per_pixel(Channel::Alpha) <= 8 {
                bail!("Internal error");
            }
            if input.channel_width(Channel::Alpha) != input.channel_width(Channel::G) || input.channel_height(Channel::Alpha) != input.channel_height(Channel::G) {
                bail!("Internal error");
            }
        }
    } else {
        if depths.iter().any(|&depth| depth != 8) {
            bail!("Internal error");
        }
        if input_has_alpha && input.bits_per_pixel(Channel::Alpha) != 8 {
            bail!("Internal error");
        }
    }
    let bpp = input.bits_per_pixel(Channel::R);
    let width = input.width;
    let height = input.height;
    let chroma = if output_has_alpha { Chroma::InterleavedRrggbbaaBe } else { Chroma::InterleavedRrggbbBe };
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, chroma);
    output.add_plane(Channel::Interleaved, width, height, bpp);
    let alpha_maximum = if hdr { ((1u32 << bpp) - 1) as u16 } else { 0xff };
    let components = chroma.interleaved_components();
    let red = input.plane(Channel::R);
    let green = input.plane(Channel::G);
    let blue = input.plane(Channel::B);
    let alpha = input.planes.get(&Channel::Alpha);
    let out = output.plane_mut(Channel::Interleaved);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = y * out.stride() + components * x;
            out.samples[base] = red.get(x, y);
            out.samples[base + 1] = green.get(x, y);
            out.samples[base + 2] = blue.get(x, y);
            if output_has_alpha {
                out.samples[base + 3] = alpha.map_or(alpha_maximum, |plane| plane.get(x, y));
            }
        }
    }
    Ok(output)
}

/// `Op_RRGGBBaa_BE_to_RGB_HDR`.
fn rrggbbaa_to_rgb(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    let has_alpha = input.chroma.interleaved_with_alpha();
    let want_alpha = target.has_alpha;
    let width = input.width;
    let height = input.height;
    let bpp = input.bits_per_pixel(Channel::Interleaved);
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, Chroma::C444);
    for channel in [Channel::R, Channel::G, Channel::B] {
        output.add_plane(channel, width, height, bpp);
    }
    if want_alpha {
        output.add_plane(Channel::Alpha, width, height, bpp);
    }
    let alpha_maximum = ((1u32 << bpp) - 1) as u16;
    let source = input.plane(Channel::Interleaved).clone();
    let components = if has_alpha { 4 } else { 3 };
    for y in 0..height as usize {
        for x in 0..width as usize {
            let base = y * source.stride() + components * x;
            output.plane_mut(Channel::R).set(x, y, source.samples[base]);
            output.plane_mut(Channel::G).set(x, y, source.samples[base + 1]);
            output.plane_mut(Channel::B).set(x, y, source.samples[base + 2]);
            if want_alpha {
                let value = if has_alpha { source.samples[base + 3] } else { alpha_maximum };
                output.plane_mut(Channel::Alpha).set(x, y, value);
            }
        }
    }
    Ok(output)
}

/// `Op_mono_to_YCbCr420` (the output profile left undefined).
fn mono_to_ycbcr420(input: &PixelImage) -> Result<PixelImage> {
    let width = input.width;
    let height = input.height;
    let bpp = input.bits_per_pixel(Channel::Y);
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let mut output = PixelImage::new(width, height, Colorspace::YCbCr, Chroma::C420);
    output.planes.insert(Channel::Y, input.plane(Channel::Y).clone());
    output.add_plane(Channel::Cb, chroma_width, chroma_height, bpp);
    output.add_plane(Channel::Cr, chroma_width, chroma_height, bpp);
    let neutral = if bpp <= 8 { 1u16 << (bpp - 1) } else { 128u16 << (bpp - 8) };
    output.fill_plane(Channel::Cb, neutral);
    output.fill_plane(Channel::Cr, neutral);
    copy_alpha(input, &mut output);
    Ok(output)
}

/// `Op_mono_to_RGB24_32`.
fn mono_to_rgb24_32(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    if input.bits_per_pixel(Channel::Y) != 8 {
        bail!("Internal error");
    }
    let width = input.width;
    let height = input.height;
    let chroma = if target.has_alpha { Chroma::InterleavedRgba } else { Chroma::InterleavedRgb };
    let mut output = PixelImage::new(width, height, Colorspace::Rgb, chroma);
    output.add_plane(Channel::Interleaved, width, height, 8);
    let components = chroma.interleaved_components();
    let grey = input.plane(Channel::Y);
    let alpha = input.planes.get(&Channel::Alpha);
    let out = output.plane_mut(Channel::Interleaved);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let value = grey.get(x, y);
            let base = y * out.stride() + components * x;
            out.samples[base] = value;
            out.samples[base + 1] = value;
            out.samples[base + 2] = value;
            if target.has_alpha {
                out.samples[base + 3] = alpha.map_or(0xff, |plane| plane.get(x, y));
            }
        }
    }
    Ok(output)
}

const PLANAR_CHANNELS: [Channel; 7] = [Channel::Y, Channel::Cb, Channel::Cr, Channel::R, Channel::G, Channel::B, Channel::Alpha];

/// `Op_to_hdr_planes`.
fn to_hdr_planes(input: &PixelImage, target: &ColorState) -> Result<PixelImage> {
    let mut output = PixelImage::new(input.width, input.height, input.colorspace, input.chroma);
    for channel in PLANAR_CHANNELS {
        let Some(plane) = input.planes.get(&channel) else {
            continue;
        };
        let output_bits = target.bits_per_pixel;
        let input_bits = plane.bit_depth as i32;
        let shift1 = output_bits - input_bits;
        let shift2 = 2 * input_bits - output_bits;
        let mut converted = plane.clone();
        converted.bit_depth = output_bits as u8;
        for sample in converted.samples.iter_mut() {
            let value = (*sample & 0xff) as i32;
            *sample = ((value << shift1) | (value >> shift2)) as u16;
        }
        output.planes.insert(channel, converted);
    }
    Ok(output)
}

/// `Op_to_sdr_planes`.
fn to_sdr_planes(input: &PixelImage) -> Result<PixelImage> {
    let mut output = PixelImage::new(input.width, input.height, input.colorspace, input.chroma);
    for channel in PLANAR_CHANNELS {
        let Some(plane) = input.planes.get(&channel) else {
            continue;
        };
        let input_bits = plane.bit_depth as i32;
        let mut converted = plane.clone();
        converted.bit_depth = 8;
        if input_bits > 8 {
            let shift = input_bits - 8;
            for sample in converted.samples.iter_mut() {
                *sample = ((*sample as i32 >> shift) & 0xff) as u16;
            }
        } else if input_bits < 8 {
            let mut bit = 1u16 << (16 - input_bits);
            let mut factor = bit;
            loop {
                bit >>= input_bits;
                if bit == 0 {
                    break;
                }
                factor |= bit;
            }
            for sample in converted.samples.iter_mut() {
                *sample = (((*sample as u32) * factor as u32) >> 8) as u16 & 0xff;
            }
        }
        output.planes.insert(channel, converted);
    }
    Ok(output)
}

/// The bilinear upsamplers' input checks.
fn check_bilinear_depths(input: &PixelImage, hdr: bool) -> Result<u8> {
    let bpp_y = input.bits_per_pixel(Channel::Y);
    let bpp_cb = input.bits_per_pixel(Channel::Cb);
    let bpp_cr = input.bits_per_pixel(Channel::Cr);
    if !hdr {
        if bpp_y > 8 || bpp_cb > 8 || bpp_cr > 8 {
            bail!("Internal error");
        }
    } else if bpp_y <= 8 || bpp_cb <= 8 || bpp_cr <= 8 {
        bail!("Internal error");
    }
    if bpp_y != bpp_cb || bpp_y != bpp_cr {
        bail!("Internal error");
    }
    Ok(bpp_y)
}

/// `Op_YCbCr420_bilinear_to_YCbCr444` (the borders sample the chroma at
/// half their index, as libheif does).
fn ycbcr420_bilinear_to_444(input: &PixelImage, hdr: bool) -> Result<PixelImage> {
    let bpp = check_bilinear_depths(input, hdr)?;
    let width = input.width as usize;
    let height = input.height as usize;
    let mut output = PixelImage::new(input.width, input.height, Colorspace::YCbCr, Chroma::C444);
    output.planes.insert(Channel::Y, input.plane(Channel::Y).clone());
    output.add_plane(Channel::Cb, input.width, input.height, bpp);
    output.add_plane(Channel::Cr, input.width, input.height, bpp);
    copy_alpha(input, &mut output);
    for channel in [Channel::Cb, Channel::Cr] {
        let source = input.plane(channel);
        let stride = source.stride();
        let get = |index: usize| -> i32 { source.samples.get(index).copied().unwrap_or(0) as i32 };
        let mut out = vec![0u16; width * height];
        out[0] = get(0) as u16;
        for cx in 0..width.saturating_sub(1) / 2 {
            out[2 * cx + 1] = ((3 * get(cx / 2) + get(cx / 2 + 1) + 2) / 4) as u16;
            out[2 * cx + 2] = ((get(cx / 2) + 3 * get(cx / 2 + 1) + 2) / 4) as u16;
        }
        if width.is_multiple_of(2) {
            out[width - 1] = get(width / 2 - 1) as u16;
        }
        for cy in 0..height.saturating_sub(1) / 2 {
            out[(2 * cy + 1) * width] = ((3 * get(cy / 2 * stride) + get((cy / 2 + 1) * stride) + 2) / 4) as u16;
            out[(2 * cy + 2) * width] = ((get(cy / 2 * stride) + 3 * get((cy / 2 + 1) * stride) + 2) / 4) as u16;
        }
        if height.is_multiple_of(2) {
            out[(height - 1) * width] = get((height / 2 - 1) * stride) as u16;
        }
        if width.is_multiple_of(2) {
            for cy in 0..height.saturating_sub(1) / 2 {
                let column = width / 2 - 1;
                out[(2 * cy + 1) * width + width - 1] = ((3 * get(cy / 2 * stride + column) + get((cy / 2 + 1) * stride + column) + 2) / 4) as u16;
                out[(2 * cy + 2) * width + width - 1] = ((get(cy / 2 * stride + column) + 3 * get((cy / 2 + 1) * stride + column) + 2) / 4) as u16;
            }
        }
        if height.is_multiple_of(2) {
            let row = (height / 2 - 1) * stride;
            for cx in 0..width.saturating_sub(1) / 2 {
                out[(height - 1) * width + 2 * cx + 1] = ((3 * get(row + cx / 2) + get(row + cx / 2 + 1) + 2) / 4) as u16;
                out[(height - 1) * width + 2 * cx + 2] = ((get(row + cx / 2) + 3 * get(row + cx / 2 + 1) + 2) / 4) as u16;
            }
        }
        if width.is_multiple_of(2) && height.is_multiple_of(2) {
            out[(height - 1) * width + width - 1] = get((height / 2 - 1) * stride + width / 2 - 1) as u16;
        }
        let mut y = 1;
        while y + 1 < height {
            let mut x = 1;
            while x + 1 < width {
                let cx = x / 2;
                let cy = y / 2;
                let c00 = get(cy * stride + cx);
                let c01 = get(cy * stride + cx + 1);
                let c10 = get((cy + 1) * stride + cx);
                let c11 = get((cy + 1) * stride + cx + 1);
                out[y * width + x] = ((c00 * 9 + c01 * 3 + c10 * 3 + c11 + 8) / 16) as u16;
                out[y * width + x + 1] = ((c00 * 3 + c01 * 9 + c10 + c11 * 3 + 8) / 16) as u16;
                out[(y + 1) * width + x] = ((c00 * 3 + c01 + c10 * 9 + c11 * 3 + 8) / 16) as u16;
                out[(y + 1) * width + x + 1] = ((c00 + c01 * 3 + c10 * 3 + c11 * 9 + 8) / 16) as u16;
                x += 2;
            }
            y += 2;
        }
        output.plane_mut(channel).samples = out;
    }
    Ok(output)
}

/// `Op_YCbCr422_bilinear_to_YCbCr444`.
fn ycbcr422_bilinear_to_444(input: &PixelImage, hdr: bool) -> Result<PixelImage> {
    let bpp = check_bilinear_depths(input, hdr)?;
    let width = input.width as usize;
    let height = input.height as usize;
    let mut output = PixelImage::new(input.width, input.height, Colorspace::YCbCr, Chroma::C444);
    output.planes.insert(Channel::Y, input.plane(Channel::Y).clone());
    output.add_plane(Channel::Cb, input.width, input.height, bpp);
    output.add_plane(Channel::Cr, input.width, input.height, bpp);
    copy_alpha(input, &mut output);
    for channel in [Channel::Cb, Channel::Cr] {
        let source = input.plane(channel);
        let stride = source.stride();
        let mut out = vec![0u16; width * height];
        for y in 0..height {
            out[y * width] = source.samples[y * stride];
        }
        if width.is_multiple_of(2) {
            for y in 0..height {
                out[y * width + width - 1] = source.samples[y * stride + width / 2 - 1];
            }
        }
        for y in 0..height {
            let mut x = 1;
            while x + 1 < width {
                let cx = x / 2;
                let c00 = source.samples[y * stride + cx] as i32;
                let c01 = source.samples[y * stride + cx + 1] as i32;
                out[y * width + x] = ((c00 * 3 + c01 + 2) / 4) as u16;
                out[y * width + x + 1] = ((c00 + c01 * 3 + 2) / 4) as u16;
                x += 2;
            }
        }
        output.plane_mut(channel).samples = out;
    }
    Ok(output)
}

/// `Op_RGB_to_YCbCr`.
fn rgb_to_ycbcr(input: &PixelImage, target: &ColorState, hdr: bool) -> Result<PixelImage> {
    let width = input.width as usize;
    let height = input.height as usize;
    let chroma = target.chroma;
    let sub_h = chroma.h_subsampling() as usize;
    let sub_v = chroma.v_subsampling() as usize;
    let bpp = input.bits_per_pixel(Channel::R);
    if bpp < 8 || (bpp > 8) != hdr {
        bail!("Internal error");
    }
    let has_alpha = input.has_channel(Channel::Alpha);
    if has_alpha && input.bits_per_pixel(Channel::Alpha) != bpp {
        bail!("Internal error");
    }
    let chroma_width = width.div_ceil(sub_h);
    let chroma_height = height.div_ceil(sub_v);
    let mut output = PixelImage::new(input.width, input.height, Colorspace::YCbCr, chroma);
    output.add_plane(Channel::Y, input.width, input.height, bpp);
    output.add_plane(Channel::Cb, chroma_width as u32, chroma_height as u32, bpp);
    output.add_plane(Channel::Cr, chroma_width as u32, chroma_height as u32, bpp);
    copy_alpha(input, &mut output);

    let half_range = (1i32 << (bpp - 1)) as f32;
    let half_range_int = 1i32 << (bpp - 1);
    let full_range_value = (1i32 << bpp) - 1;
    let limited_offset = (16i32 << (bpp - 8)) as f32;
    let full_range = target.nclx.full_range;
    let matrix = target.nclx.matrix_coefficients;
    let coefficients = rgb_to_ycbcr_coefficients(matrix, target.nclx.colour_primaries);
    let red = input.plane(Channel::R).clone();
    let green = input.plane(Channel::G).clone();
    let blue = input.plane(Channel::B).clone();

    for y in 0..height {
        for x in 0..width {
            let value = if matrix == 0 {
                if full_range {
                    green.get(x, y)
                } else {
                    let v = ((green.get(x, y) as f32 * 219.0) / 256.0) + limited_offset;
                    clip_f_u16(v, full_range_value)
                }
            } else if matrix == 8 {
                (green.get(x, y) as i32 / 2 + (red.get(x, y) as i32 + blue.get(x, y) as i32) / 4) as u16
            } else {
                let r = red.get(x, y) as f32;
                let g = green.get(x, y) as f32;
                let b = blue.get(x, y) as f32;
                let mut v = r * coefficients.c[0][0] + g * coefficients.c[0][1] + b * coefficients.c[0][2];
                if !full_range {
                    v = ((v * 219.0) / 256.0) + limited_offset;
                }
                clip_f_u16(v, full_range_value)
            };
            output.plane_mut(Channel::Y).set(x, y, value);
        }
    }

    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let (cb, cr) = if matrix == 0 {
                if full_range {
                    (blue.get(x, y), red.get(x, y))
                } else {
                    let cb = clip_f_u16(((blue.get(x, y) as f32 * 224.0) / 256.0) + limited_offset, full_range_value);
                    let cr = clip_f_u16(((red.get(x, y) as f32 * 224.0) / 256.0) + limited_offset, full_range_value);
                    (cb, cr)
                }
            } else if matrix == 8 {
                let g = green.get(x, y) as i32;
                let r = red.get(x, y) as i32;
                let b = blue.get(x, y) as i32;
                let cb = clip_int_u16(g / 2 - (r + b) / 4 + half_range_int, full_range_value as u16);
                let cr = clip_int_u16((r - b) / 2 + half_range_int, full_range_value as u16);
                (cb, cr)
            } else {
                let mut r = red.get(x, y) as f32;
                let mut g = green.get(x, y) as f32;
                let mut b = blue.get(x, y) as f32;
                if sub_h > 1 || sub_v > 1 {
                    let x2 = if x + 1 < width && sub_h == 2 && sub_v == 2 { x + 1 } else { x };
                    let y2 = if y + 1 < height && sub_v == 2 { y + 1 } else { y };
                    r += red.get(x2, y) as f32;
                    g += green.get(x2, y) as f32;
                    b += blue.get(x2, y) as f32;
                    r += red.get(x, y2) as f32;
                    g += green.get(x, y2) as f32;
                    b += blue.get(x, y2) as f32;
                    r += red.get(x2, y2) as f32;
                    g += green.get(x2, y2) as f32;
                    b += blue.get(x2, y2) as f32;
                    r *= 0.25;
                    g *= 0.25;
                    b *= 0.25;
                }
                let mut cb = r * coefficients.c[1][0] + g * coefficients.c[1][1] + b * coefficients.c[1][2];
                let mut cr = r * coefficients.c[2][0] + g * coefficients.c[2][1] + b * coefficients.c[2][2];
                if !full_range {
                    cb = (cb * 224.0) / 256.0;
                    cr = (cr * 224.0) / 256.0;
                }
                (clip_f_u16(cb + half_range, full_range_value), clip_f_u16(cr + half_range, full_range_value))
            };
            output.plane_mut(Channel::Cb).set(x / sub_h, y / sub_v, cb);
            output.plane_mut(Channel::Cr).set(x / sub_h, y / sub_v, cr);
            x += sub_h;
        }
        y += sub_v;
    }
    Ok(output)
}
