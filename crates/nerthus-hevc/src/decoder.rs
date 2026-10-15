//! The decoder context (`decctx.cc`, `nal-parser.cc`): NAL units in,
//! decoded pictures out.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::bitreader::BitReader;
use crate::cabac::{CabacDecoder, ContextTable};
use crate::deblock::apply_deblocking_filter;
use crate::error::Error;
use crate::params::{read_vps, Pps, Sps, Vps, MAX_PPS_SETS, MAX_SPS_SETS, MAX_VPS_SETS};
use crate::picture::Picture;
use crate::sao::apply_sample_adaptive_offset;
use crate::slice::{is_supported_slice, DecodeResult, ThreadContext};
use crate::slice_header::{is_rap_pic, is_rasl, ReadOutcome, SliceHeader, NAL_UNIT_BLA_W_LP, NAL_UNIT_IDR_N_LP, NAL_UNIT_IDR_W_RADL, NAL_UNIT_RASL_R};

const NAL_UNIT_RADL_N: i32 = 6;
const NAL_UNIT_BLA_N_LP: i32 = 18;
const NAL_UNIT_VPS: i32 = 32;
const NAL_UNIT_SPS: i32 = 33;
const NAL_UNIT_PPS: i32 = 34;
const NAL_UNIT_EOS: i32 = 36;
const NAL_UNIT_PREFIX_SEI: i32 = 39;
const NAL_UNIT_SUFFIX_SEI: i32 = 40;
const SEI_DECODED_PICTURE_HASH: i32 = 132;
const WARNING_SPS_MISSING_CANNOT_DECODE_SEI: Error = Error(1025);

/// A NAL unit with its emulation prevention bytes removed.
struct NalUnit {
    data: Vec<u8>,
    skipped_bytes: Vec<i32>,
}

impl NalUnit {
    /// `NAL_unit::remove_stuffing_bytes`.
    fn new(bytes: &[u8]) -> NalUnit {
        let mut data = bytes.to_vec();
        let mut skipped_bytes = Vec::new();
        let mut p = 0usize;
        let mut i = 0i32;
        while i < data.len() as i32 - 2 {
            if data[p + 2] != 3 && data[p + 2] != 0 {
                p += 2;
                i += 2;
            } else if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 3 {
                skipped_bytes.push(i + 2 + skipped_bytes.len() as i32);
                data.remove(p + 2);
                p += 1;
                i += 1;
            }
            p += 1;
            i += 1;
        }
        NalUnit { data, skipped_bytes }
    }

    /// `num_skipped_bytes_before`.
    fn num_skipped_bytes_before(&self, byte_position: i32, header_length: i32) -> i32 {
        for k in (0..self.skipped_bytes.len()).rev() {
            let skipped = self.skipped_bytes[k];
            if skipped >= header_length && skipped - header_length <= byte_position {
                return k as i32 + 1;
            }
        }
        0
    }
}

/// A decoded picture cropped to its conformance window.
#[derive(Clone, Debug)]
pub struct DecodedPicture {
    /// Luma size.
    pub width: usize,
    pub height: usize,
    pub chroma_width: usize,
    pub chroma_height: usize,
    /// 0 monochrome, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: i32,
    pub bit_depth_luma: i32,
    pub bit_depth_chroma: i32,
    /// Y, Cb, Cr (the chroma planes empty for monochrome).
    pub planes: [Vec<u16>; 3],
    pub full_range: bool,
    pub colour_primaries: i32,
    pub transfer_characteristics: i32,
    pub matrix_coefficients: i32,
    /// The user data of the NAL unit that started the picture.
    pub user_data: u32,
}

impl DecodedPicture {
    fn from_picture(picture: &Picture, user_data: u32) -> DecodedPicture {
        let crop = |c_idx: usize, left: i32, top: i32, width: i32, height: i32| -> Vec<u16> {
            let stride = picture.stride(c_idx);
            let mut plane = Vec::with_capacity((width.max(0) * height.max(0)) as usize);
            for y in 0..height {
                let start = ((y + top) * stride + left) as usize;
                plane.extend_from_slice(&picture.planes[c_idx][start..start + width as usize]);
            }
            plane
        };
        let luma = crop(0, picture.crop_left, picture.crop_top, picture.width_confwin, picture.height_confwin);
        let (cb, cr) = if picture.chroma_format == 0 {
            (Vec::new(), Vec::new())
        } else {
            let cb = crop(1, picture.chroma_crop_left, picture.chroma_crop_top, picture.chroma_width_confwin, picture.chroma_height_confwin);
            let cr = crop(2, picture.chroma_crop_left, picture.chroma_crop_top, picture.chroma_width_confwin, picture.chroma_height_confwin);
            (cb, cr)
        };
        let vui = &picture.sps.vui;
        DecodedPicture {
            width: picture.width_confwin as usize,
            height: picture.height_confwin as usize,
            chroma_width: if picture.chroma_format == 0 { 0 } else { picture.chroma_width_confwin as usize },
            chroma_height: if picture.chroma_format == 0 { 0 } else { picture.chroma_height_confwin as usize },
            chroma_format: picture.chroma_format,
            bit_depth_luma: picture.sps.bit_depth_y,
            bit_depth_chroma: picture.sps.bit_depth_c,
            planes: [luma, cb, cr],
            full_range: vui.video_full_range_flag,
            colour_primaries: vui.colour_primaries,
            transfer_characteristics: vui.transfer_characteristics,
            matrix_coefficients: vui.matrix_coeffs,
            user_data,
        }
    }
}

/// A slice segment as libde265's decode loop processes it. Its samples are
/// decoded when it arrives, but libde265 processes a picture's slices one
/// call behind once an earlier picture is queued, so the reorder buffer
/// flush the slice carries and its decoding error take effect later.
struct SliceStep {
    flush_reorder_buffer: bool,
    error: Option<Error>,
    processed: bool,
}

/// A picture in decoding (`image_unit`).
struct PictureUnit {
    picture: Picture,
    slices: Vec<SliceHeader>,
    wpp_models: Vec<Option<Box<ContextTable>>>,
    steps: Vec<SliceStep>,
    poc: i32,
    user_data: u32,
    /// The reorder buffer size of the picture's video parameter set.
    max_num_reorder_pics: i32,
}

impl PictureUnit {
    fn all_slices_processed(&self) -> bool {
        self.steps.iter().all(|step| step.processed)
    }
}

/// `decoder_context`: NAL units are queued, and each `decode` call
/// processes one of them.
pub struct Decoder {
    vps: Vec<Option<Vps>>,
    sps: Vec<Option<Arc<Sps>>>,
    pps: Vec<Option<Arc<Pps>>>,
    current_sps: bool,
    nal_queue: VecDeque<(Vec<u8>, u32)>,
    end_of_stream: bool,
    units: VecDeque<PictureUnit>,
    previous_slice_header: Option<Box<SliceHeader>>,
    first_decoded_picture: bool,
    first_after_end_of_sequence: bool,
    no_rasl_output_flag: bool,
    previous_poc_lsb: i32,
    previous_poc_msb: i32,
    reorder_buffer: Vec<(i32, DecodedPicture)>,
    output: VecDeque<DecodedPicture>,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder::new()
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            vps: vec![None; MAX_VPS_SETS as usize],
            sps: vec![None; MAX_SPS_SETS as usize],
            pps: vec![None; MAX_PPS_SETS as usize],
            current_sps: false,
            nal_queue: VecDeque::new(),
            end_of_stream: false,
            units: VecDeque::new(),
            previous_slice_header: None,
            first_decoded_picture: true,
            first_after_end_of_sequence: false,
            no_rasl_output_flag: false,
            previous_poc_lsb: 0,
            previous_poc_msb: 0,
            reorder_buffer: Vec::new(),
            output: VecDeque::new(),
        }
    }

    /// `de265_push_NAL`, with the user data the pictures it starts carry.
    pub fn push_nal(&mut self, bytes: &[u8], user_data: u32) {
        self.nal_queue.push_back((bytes.to_vec(), user_data));
    }

    /// `de265_flush_data`: the end of the stream.
    pub fn flush_data(&mut self) {
        self.end_of_stream = true;
    }

    /// `de265_get_next_picture`.
    pub fn next_picture(&mut self) -> Option<DecodedPicture> {
        self.output.pop_front()
    }

    /// libheif's `decode_next_image` for libde265: decodes until a picture
    /// is output, the decoder needs more input or a step fails.
    pub fn decode_next_picture(&mut self) -> Option<DecodedPicture> {
        loop {
            let Ok(more) = self.decode() else {
                return None;
            };
            if let Some(picture) = self.next_picture() {
                return Some(picture);
            }
            if !more {
                return None;
            }
        }
    }

    /// `decoder_context::decode`: one step, returning whether more work
    /// may follow.
    pub fn decode(&mut self) -> Result<bool, Error> {
        if self.nal_queue.is_empty() && self.end_of_stream && self.units.is_empty() {
            self.flush_reorder_buffer();
            return Ok(!self.output.is_empty());
        }
        if !self.end_of_stream && self.nal_queue.is_empty() {
            return Err(Error::WAITING_FOR_INPUT_DATA);
        }
        if let Some((bytes, user_data)) = self.nal_queue.pop_front() {
            self.decode_nal(&bytes, user_data)?;
            return Ok(true);
        }
        let did_work = self.decode_some()?;
        Ok(did_work)
    }

    /// `decode_some`: processes the oldest picture's next slice, then
    /// outputs that picture once it is complete.
    fn decode_some(&mut self) -> Result<bool, Error> {
        let Some(unit) = self.units.front_mut() else {
            return Ok(false);
        };
        let mut did_work = false;
        if let Some(step) = unit.steps.iter_mut().find(|step| !step.processed) {
            step.processed = true;
            did_work = true;
            let flush = step.flush_reorder_buffer;
            let error = step.error;
            if flush {
                self.flush_reorder_buffer();
            }
            if let Some(error) = error {
                return Err(error);
            }
        }
        let first_complete = self.units.front().is_some_and(PictureUnit::all_slices_processed);
        let stream_done = self.nal_queue.is_empty() && self.end_of_stream;
        if first_complete && (self.units.len() >= 2 || stream_done) {
            did_work = true;
            if let Some(unit) = self.units.pop_front() {
                self.push_picture_to_output_queue(unit);
            }
        }
        Ok(did_work)
    }

    /// `push_picture_to_output_queue`, after the loop filters.
    fn push_picture_to_output_queue(&mut self, mut unit: PictureUnit) {
        apply_deblocking_filter(&mut unit.picture, &unit.slices);
        apply_sample_adaptive_offset(&mut unit.picture, &unit.slices);
        if unit.picture.pic_output_flag {
            let picture = DecodedPicture::from_picture(&unit.picture, unit.user_data);
            self.reorder_buffer.push((unit.poc, picture));
        }
        if self.reorder_buffer.len() as i32 > unit.max_num_reorder_pics {
            self.output_next_picture_in_reorder_buffer();
        }
    }

    /// The picture with the lowest picture order count moves to the
    /// output queue (its place taken by the last one).
    fn output_next_picture_in_reorder_buffer(&mut self) {
        let mut lowest = 0;
        for index in 1..self.reorder_buffer.len() {
            if self.reorder_buffer[index].0 < self.reorder_buffer[lowest].0 {
                lowest = index;
            }
        }
        let (_, picture) = self.reorder_buffer.swap_remove(lowest);
        self.output.push_back(picture);
    }

    fn flush_reorder_buffer(&mut self) {
        while !self.reorder_buffer.is_empty() {
            self.output_next_picture_in_reorder_buffer();
        }
    }

    /// `decode_NAL`: an error drops the NAL unit.
    fn decode_nal(&mut self, bytes: &[u8], user_data: u32) -> Result<(), Error> {
        let nal = NalUnit::new(bytes);
        let mut reader = BitReader::new(&nal.data);
        reader.skip_bits(1);
        let nal_unit_type = reader.get_bits(6);
        let layer_id = reader.get_bits(6);
        let temporal_id = reader.get_bits(3) - 1;
        if layer_id > 0 {
            return Ok(());
        }
        if nal_unit_type < 32 {
            return self.read_slice_nal(&nal, reader, nal_unit_type, temporal_id, user_data);
        }
        match nal_unit_type {
            NAL_UNIT_VPS => {
                let vps = read_vps(&mut reader)?;
                self.vps[vps.id as usize] = Some(vps);
            }
            NAL_UNIT_SPS => {
                let sps = Sps::read(&mut reader)?;
                let id = sps.id;
                for pps in self.pps.iter_mut() {
                    if pps.as_ref().is_some_and(|pps| pps.seq_parameter_set_id == id) {
                        *pps = None;
                    }
                }
                self.sps[id as usize] = Some(Arc::new(sps));
            }
            NAL_UNIT_PPS => {
                let sps_table = &self.sps;
                let get_sps = |id: i32| sps_table.get(id as usize).cloned().flatten();
                let Some(pps) = Pps::read(&mut reader, get_sps) else {
                    return Err(Error::WARNING_PPS_HEADER_INVALID);
                };
                let id = pps.id as usize;
                self.pps[id] = Some(Arc::new(pps));
            }
            NAL_UNIT_PREFIX_SEI | NAL_UNIT_SUFFIX_SEI => {
                let mut payload_type = 0;
                loop {
                    let byte = reader.get_bits(8);
                    payload_type += byte;
                    if byte != 0xff {
                        break;
                    }
                }
                if payload_type == SEI_DECODED_PICTURE_HASH && !self.current_sps {
                    return Err(WARNING_SPS_MISSING_CANNOT_DECODE_SEI);
                }
            }
            NAL_UNIT_EOS => self.first_after_end_of_sequence = true,
            _ => {}
        }
        Ok(())
    }

    /// `read_slice_NAL` with `process_slice_segment_header`. Only intra
    /// slices are decoded; others are taken as decoded without error.
    fn read_slice_nal(&mut self, nal: &NalUnit, mut reader: BitReader, nal_unit_type: i32, temporal_id: i32, user_data: u32) -> Result<(), Error> {
        let pps_table = &self.pps;
        let get_pps = |id: i32| pps_table.get(id as usize).cloned().flatten();
        let outcome = SliceHeader::read(&mut reader, nal_unit_type, get_pps, self.previous_slice_header.as_deref())?;
        let mut header = match outcome {
            ReadOutcome::Decode(header) => header,
            ReadOutcome::Skip => return Ok(()),
        };
        self.current_sps = true;

        let mut flush_reorder_buffer = false;
        if header.first_slice_segment_in_pic_flag {
            let pps = header.pps.clone();
            let sps = pps.sps.clone();
            let max_num_reorder_pics = self.vps[sps.video_parameter_set_id as usize].map_or(0, |vps| vps.max_num_reorder_pics);
            let Some(mut picture) = Picture::new(sps.clone(), pps) else {
                return Err(Error::CODED_PARAMETER_OUT_OF_RANGE);
            };
            picture.clear_metadata();
            let irap = is_rap_pic(nal_unit_type);
            if irap {
                let idr = nal_unit_type == NAL_UNIT_IDR_W_RADL || nal_unit_type == NAL_UNIT_IDR_N_LP;
                let bla = (NAL_UNIT_BLA_W_LP..=NAL_UNIT_BLA_N_LP).contains(&nal_unit_type);
                if idr || bla || self.first_decoded_picture || self.first_after_end_of_sequence {
                    self.no_rasl_output_flag = true;
                    self.first_after_end_of_sequence = false;
                } else {
                    self.no_rasl_output_flag = false;
                }
            }
            picture.pic_output_flag = if is_rasl(nal_unit_type) && self.no_rasl_output_flag { false } else { header.pic_output_flag };

            // Picture order count (8.3.1).
            let lsb = header.slice_pic_order_cnt_lsb;
            let msb = if irap && self.no_rasl_output_flag {
                flush_reorder_buffer = true;
                0
            } else {
                let max_lsb = 1 << sps.log2_max_pic_order_cnt_lsb;
                if lsb < self.previous_poc_lsb && self.previous_poc_lsb - lsb >= max_lsb / 2 {
                    self.previous_poc_msb + max_lsb
                } else if lsb > self.previous_poc_lsb && lsb - self.previous_poc_lsb > max_lsb / 2 {
                    self.previous_poc_msb - max_lsb
                } else {
                    self.previous_poc_msb
                }
            };
            let sublayer_non_reference = nal_unit_type <= 14 && nal_unit_type % 2 == 0;
            let leading = (NAL_UNIT_RADL_N..=NAL_UNIT_RASL_R).contains(&nal_unit_type);
            if temporal_id == 0 && !sublayer_non_reference && !leading {
                self.previous_poc_lsb = lsb;
                self.previous_poc_msb = msb;
            }

            self.first_decoded_picture = false;
            let unit = PictureUnit { picture, slices: Vec::new(), wpp_models: Vec::new(), steps: Vec::new(), poc: msb + lsb, user_data, max_num_reorder_pics };
            self.units.push_back(unit);
        }

        header.slice_addr_rs = if header.dependent_slice_segment_flag {
            self.previous_slice_header.as_ref().map_or(0, |previous| previous.slice_addr_rs)
        } else {
            header.slice_segment_address
        };
        self.previous_slice_header = Some(header.clone());

        if let Some(unit) = self.units.back_mut() {
            header.slice_index = unit.slices.len();
            reader.skip_bits(1);
            reader.prepare_for_cabac();
            let header_length = reader.position() as i32;
            for index in 0..header.num_entry_point_offsets as usize {
                let offset = header.entry_point_offset[index];
                header.entry_point_offset[index] = offset - nal.num_skipped_bytes_before(offset, header_length);
            }
            let supported = is_supported_slice(&header);
            unit.slices.push(*header);
            let error = if supported {
                let data: Arc<[u8]> = Arc::from(nal.data.as_slice());
                decode_slice_unit(unit, data, header_length).err()
            } else {
                None
            };
            unit.steps.push(SliceStep { flush_reorder_buffer, error, processed: false });
        }

        // A slice processed here does not report its error.
        let _ = self.decode_some();
        Ok(())
    }
}

/// `initialize_CABAC_at_slice_segment_start`.
fn initialize_at_slice_segment_start(context: &mut ThreadContext, slices: &mut [SliceHeader]) -> bool {
    let header = context.header;
    if !header.dependent_slice_segment_flag {
        context.initialize_models();
        return true;
    }
    let pps = context.picture.pps.clone();
    let sps = context.picture.sps.clone();
    let address = header.slice_segment_address;
    let previous_ctb = pps.ctb_addr_ts_to_rs[(pps.ctb_addr_rs_to_ts[address as usize] - 1) as usize];
    let slice_index = context.picture.ctb_info.data[previous_ctb as usize].slice_header_index as usize;
    if slice_index >= slices.len() {
        return false;
    }
    if pps.is_tile_start_ctb(address % sps.pic_width_in_ctbs_y, address / sps.pic_width_in_ctbs_y) {
        context.initialize_models();
        return true;
    }
    if header.slice_index == 0 {
        return false;
    }
    let previous = &mut slices[slice_index];
    if !previous.ctx_model_storage_defined {
        return false;
    }
    context.models = previous.ctx_model_storage.take();
    true
}

/// `decode_slice_unit_parallel` as libheif runs it (one worker thread).
fn decode_slice_unit(unit: &mut PictureUnit, data: Arc<[u8]>, header_length: i32) -> Result<(), Error> {
    let slice_index = unit.slices.len() - 1;
    let pps = unit.picture.pps.clone();
    let use_wpp = pps.entropy_coding_sync_enabled_flag;
    let use_tiles = pps.tiles_enabled_flag;
    if use_wpp && use_tiles {
        return Err(Error::WARNING_PPS_HEADER_INVALID);
    }
    let header = unit.slices[slice_index].clone();
    let bytes_remaining = data.len() as i32 - header_length;
    let data_start = header_length.max(0) as usize;
    let mut stored_models = None;

    if !use_wpp && !use_tiles {
        if header.slice_segment_address as usize >= pps.ctb_addr_rs_to_ts.len() {
            return Err(Error::CTB_OUTSIDE_IMAGE_AREA);
        }
        if bytes_remaining <= 0 {
            return Err(Error::PREMATURE_END_OF_SLICE);
        }
        let cabac = CabacDecoder::new(data.clone(), data_start, data.len());
        let ctb_addr_in_ts = pps.ctb_addr_rs_to_ts[header.slice_segment_address as usize];
        let mut context = ThreadContext::new(&mut unit.picture, &header, cabac, ctb_addr_in_ts);
        let result = read_slice_segment_data(&mut context, &mut unit.slices, &mut unit.wpp_models);
        stored_models = context.stored_models.take();
        store_models(&mut unit.slices[slice_index], stored_models);
        return result;
    }

    let sps = unit.picture.sps.clone();
    let entries = header.num_entry_point_offsets + 1;
    let ctb_width = sps.pic_width_in_ctbs_y;
    if use_wpp && header.first_slice_segment_in_pic_flag {
        unit.wpp_models = vec![None; (sps.pic_height_in_ctbs_y - 1).max(0) as usize];
    }

    let mut ctb_addr_rs = header.slice_segment_address;
    let mut ctb_row = ctb_addr_rs / ctb_width;
    let mut tile_id = pps.tile_id_rs[ctb_addr_rs as usize];
    let mut error = Ok(());
    for entry in 0..entries {
        if use_wpp {
            if entry > 0 {
                ctb_row += 1;
                ctb_addr_rs = ctb_row * ctb_width;
            } else if entries > 1 && ctb_addr_rs % ctb_width != 0 {
                error = Err(Error::WARNING_SLICEHEADER_INVALID);
                break;
            }
        } else if entry > 0 {
            tile_id += 1;
            if tile_id >= pps.num_tile_columns * pps.num_tile_rows {
                error = Err(Error::WARNING_SLICEHEADER_INVALID);
                break;
            }
            let ctb_x = pps.col_bd[(tile_id % pps.num_tile_columns) as usize];
            let ctb_y = pps.row_bd[(tile_id / pps.num_tile_columns) as usize];
            ctb_addr_rs = ctb_y * ctb_width + ctb_x;
        }

        let start = if entry == 0 { 0 } else { header.entry_point_offset[(entry - 1) as usize] };
        let end = if entry == entries - 1 { bytes_remaining } else { header.entry_point_offset[entry as usize] };
        if start < 0 || end > bytes_remaining || end <= start {
            error = Err(Error::PREMATURE_END_OF_SLICE);
            break;
        }
        let cabac = CabacDecoder::new(data.clone(), data_start + start as usize, data_start + end as usize);
        let ctb_addr_in_ts = pps.ctb_addr_rs_to_ts[ctb_addr_rs as usize];
        let mut context = ThreadContext::new(&mut unit.picture, &header, cabac, ctb_addr_in_ts);
        context.set_ctb_addr_from_ts();
        let first = entry == 0;
        if first && !initialize_at_slice_segment_start(&mut context, &mut unit.slices) {
            continue;
        }
        if use_tiles && !first {
            context.initialize_models();
        }
        context.cabac.start();
        if use_wpp {
            let first_independent = first && !header.dependent_slice_segment_flag;
            context.decode_substream(&mut unit.wpp_models, first_independent);
        } else {
            context.decode_substream(&mut unit.wpp_models, first);
        }
        if context.stored_models.is_some() {
            stored_models = context.stored_models.take();
        }
    }
    store_models(&mut unit.slices[slice_index], stored_models);
    if use_wpp {
        return Ok(());
    }
    error
}

fn store_models(header: &mut SliceHeader, models: Option<Box<ContextTable>>) {
    if models.is_some() {
        header.ctx_model_storage = models;
        header.ctx_model_storage_defined = true;
    }
}

/// `read_slice_segment_data` (sequential substreams).
fn read_slice_segment_data(context: &mut ThreadContext, slices: &mut [SliceHeader], wpp_models: &mut Vec<Option<Box<ContextTable>>>) -> Result<(), Error> {
    context.set_ctb_addr_from_ts();
    if !initialize_at_slice_segment_start(context, slices) {
        return Err(Error::UNSPECIFIED_DECODING_ERROR);
    }
    context.cabac.start();
    let mut first_slice_substream = !context.header.dependent_slice_segment_flag;
    let tiles = context.picture.pps.tiles_enabled_flag;
    loop {
        let result = context.decode_substream(wpp_models, first_slice_substream);
        match result {
            DecodeResult::EndOfSliceSegment | DecodeResult::Error => break,
            DecodeResult::EndOfSubstream => {}
        }
        first_slice_substream = false;
        if tiles {
            context.initialize_models();
        }
    }
    Ok(())
}

/// Decodes an HEVC stream given as NAL units (without start codes or
/// length prefixes) the way libheif decodes a still image with libde265:
/// all units pushed, the stream flushed, and at most 50 requests for a
/// picture.
pub fn decode_nal_units(units: &[&[u8]]) -> Result<DecodedPicture, Error> {
    let mut decoder = Decoder::new();
    for unit in units {
        decoder.push_nal(unit, 0);
    }
    decoder.flush_data();
    for _ in 0..50 {
        if let Some(picture) = decoder.decode_next_picture() {
            return Ok(picture);
        }
    }
    Err(Error::UNSPECIFIED_DECODING_ERROR)
}
