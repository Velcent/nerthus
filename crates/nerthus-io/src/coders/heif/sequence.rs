//! Image sequences (`moov` tracks) as libheif 1.21.2 reads them
//! (`track.cc`, `track_visual.cc`, `chunk.cc`): the tracks are loaded and
//! checked when the file is opened, and a visual track's samples are fed
//! to a decoder kept for the track until it gives back a picture.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};

use super::boxes::{fourcc, Av1C, AvcC, Content, HeifBox, HvcC};
use super::image::{Channel, PixelImage};

const MAX_SEQUENCE_FRAMES: u32 = 18_000_000;

const ALPHA_URNS: [&str; 3] = ["urn:mpeg:mpegB:cicp:systems:auxiliary:alpha", "urn:mpeg:hevc:2015:auxid:1", "urn:mpeg:avc:2015:auxid:1"];

/// The coded format of a visual sample description
/// (`Decoder::alloc_for_sequence_sample_description_box`).
#[derive(Clone)]
enum SampleFormat {
    Hevc(Option<HvcC>),
    Av1(Option<Av1C>),
    Avc(Option<AvcC>),
    /// Formats the decoder plugins of the reading library do not cover
    /// here (VVC, JPEG 2000, JPEG).
    Unsupported,
}

/// A decoder with the state it keeps between samples.
enum DecoderState {
    Hevc(nerthus_hevc::Decoder),
    #[cfg(not(target_arch = "wasm32"))]
    Av1(super::av1::SequenceDecoder),
    Avc(super::avc::SequenceDecoder),
}

struct SampleDecoder {
    format: SampleFormat,
    state: Option<DecoderState>,
}

/// A run of samples stored back to back (`Chunk`).
struct Chunk {
    first_sample: u32,
    last_sample: u32,
    /// The file offset and size of each sample.
    ranges: Vec<(u64, u32)>,
    /// Index into the track's decoders; chunks with the same sample
    /// description in a row share one.
    decoder: Option<usize>,
}

/// `SampleTiming`, reduced to what decoding uses.
#[derive(Clone, Copy)]
struct SampleTiming {
    sample: u32,
    chunk: usize,
}

/// `SampleAuxInfoReader`: per-sample data kept outside the samples.
struct AuxInfoReader {
    default_size: u8,
    sizes: Vec<u8>,
    single_chunk_offset: Option<u64>,
    sample_offsets: Vec<u64>,
}

impl AuxInfoReader {
    fn new(default_size: u8, count: u32, sizes: &[u8], offsets: &[u64], chunks: &[Chunk]) -> Result<AuxInfoReader> {
        let one_chunk = offsets.len() == 1;
        let mut offset = offsets.first().copied().unwrap_or(0);
        let mut reader = AuxInfoReader { default_size, sizes: sizes.to_vec(), single_chunk_offset: None, sample_offsets: Vec::new() };
        if one_chunk && default_size != 0 {
            reader.single_chunk_offset = Some(offset);
            return Ok(reader);
        }
        let mut current_chunk = 0;
        for index in 0..count {
            if !one_chunk {
                let Some(chunk) = chunks.get(current_chunk) else {
                    bail!("'saio' box covers more chunks than the track has");
                };
                if index > chunk.last_sample {
                    current_chunk += 1;
                    if current_chunk >= chunks.len() {
                        bail!("'saio' box covers more chunks than the track has");
                    }
                    offset = offsets.get(current_chunk).copied().unwrap_or(0);
                }
            }
            reader.sample_offsets.push(offset);
            offset = offset.wrapping_add(reader.sample_size(index) as u64);
        }
        Ok(reader)
    }

    fn sample_size(&self, index: u32) -> u8 {
        if self.default_size != 0 {
            return self.default_size;
        }
        self.sizes.get(index as usize).copied().unwrap_or(0)
    }

    /// `get_sample_info`.
    fn sample_info(&self, stream: &mut FileStream, index: u32) -> Vec<u8> {
        let (offset, size) = if let Some(start) = self.single_chunk_offset {
            let size = self.sample_size(0);
            (start.wrapping_add(index as u64 * size as u64), size)
        } else {
            let size = self.sample_size(index);
            if size == 0 {
                return Vec::new();
            }
            let Some(&offset) = self.sample_offsets.get(index as usize) else {
                return Vec::new();
            };
            (offset, size)
        };
        stream.read_range(offset, size as u32)
    }
}

pub struct Track {
    pub id: u32,
    handler: u32,
    width: u16,
    height: u16,
    chunks: Vec<Chunk>,
    decoders: Vec<SampleDecoder>,
    timeline: Vec<SampleTiming>,
    num_output_samples: u64,
    auxiliary_info_type: String,
    tref: Option<Vec<(u32, Vec<u32>)>>,
    content_ids: Option<AuxInfoReader>,
    tai_timestamps: Option<AuxInfoReader>,
    /// The id of the auxiliary track holding this track's alpha.
    pub alpha_track: Option<u32>,
    next_sample_to_be_decoded: u64,
    next_sample_to_be_output: u64,
    decoder_is_flushed: bool,
}

/// The file as libheif's stream reader reads it, with its position.
pub struct FileStream<'a> {
    pub data: &'a [u8],
    pub position: u64,
}

impl FileStream<'_> {
    /// `HeifFile::append_data_from_file_range`, which ignores failures: a
    /// seek past the end leaves the position where it was (the read then
    /// starts there), and a read past the end gives zeros.
    fn read_range(&mut self, offset: u64, size: u32) -> Vec<u8> {
        let length = self.data.len() as u64;
        if offset <= length {
            self.position = offset;
        }
        let end = self.position + size as u64;
        if end > length {
            return vec![0; size as usize];
        }
        let bytes = self.data[self.position as usize..end as usize].to_vec();
        self.position = end;
        bytes
    }
}

/// `Box_stsc::get_chunk`: the entry covering 1-based chunk `index`.
fn chunk_entry(entries: &[(u32, u32, u32)], index: u32) -> Option<(u32, u32, u32)> {
    for (position, &entry) in entries.iter().enumerate() {
        let last = position == entries.len() - 1;
        if index >= entry.0 && (last || index < entries[position + 1].0) {
            return Some(entry);
        }
    }
    None
}

fn is_visual_sample_entry(entry: &HeifBox) -> bool {
    matches!(entry.content, Content::VisualSampleEntry { .. })
}

fn sample_format(entry: &HeifBox) -> SampleFormat {
    let kind = entry.kind();
    if kind == fourcc(b"hvc1") {
        let hvcc = entry.child(b"hvcC").and_then(|child| match &child.content {
            Content::HvcC(hvcc) => Some(hvcc.clone()),
            _ => None,
        });
        return SampleFormat::Hevc(hvcc);
    }
    if kind == fourcc(b"av01") {
        let av1c = entry.child(b"av1C").and_then(|child| match &child.content {
            Content::Av1C(av1c) => Some(av1c.clone()),
            _ => None,
        });
        return SampleFormat::Av1(av1c);
    }
    if kind == fourcc(b"avc1") {
        let configuration = entry.child(b"avcC").and_then(|child| match &child.content {
            Content::AvcC(configuration) => Some(configuration.clone()),
            _ => None,
        });
        return SampleFormat::Avc(configuration);
    }
    SampleFormat::Unsupported
}

impl Track {
    /// `Track::alloc_track`: `None` for a track type that is skipped.
    pub fn alloc(trak: &HeifBox, mvhd: Option<(u32, u64)>) -> Result<Option<Track>> {
        let Some(mdia) = trak.child(b"mdia") else {
            bail!("Track has no 'mdia' box.");
        };
        let Some(HeifBox { content: Content::Hdlr { handler }, .. }) = mdia.child(b"hdlr") else {
            bail!("Track has no 'hdlr' box.");
        };
        let handler = *handler;
        let visual = [b"pict", b"vide", b"auxv"].iter().any(|name| fourcc(name) == handler);
        if !visual && handler != fourcc(b"meta") {
            return Ok(None);
        }
        let track = Track::load(trak, handler, visual, mvhd)?;
        Ok(Some(track))
    }

    /// `Track::load` with `Track_Visual::load` or `Track_Metadata::load`.
    fn load(trak: &HeifBox, handler: u32, visual: bool, mvhd: Option<(u32, u64)>) -> Result<Track> {
        let Some(HeifBox { content: Content::Tkhd { track_id }, .. }) = trak.child(b"tkhd") else {
            bail!("Track has no 'tkhd' box.");
        };
        let elst = trak.child(b"edts").and_then(|edts| edts.child(b"elst"));
        let Some(mdia) = trak.child(b"mdia") else {
            bail!("Track has no 'mdia' box.");
        };
        let tref = match trak.child(b"tref") {
            Some(HeifBox { content: Content::Tref(references), .. }) => Some(references.clone()),
            _ => None,
        };
        if mdia.child(b"hdlr").is_none() {
            bail!("Track has no 'hdlr' box.");
        }
        let Some(minf) = mdia.child(b"minf") else {
            bail!("Track has no 'minf' box.");
        };
        let Some(HeifBox { content: Content::Mdhd { timescale, duration }, .. }) = mdia.child(b"mdhd") else {
            bail!("Track has no 'mdhd' box.");
        };
        let Some(stbl) = minf.child(b"stbl") else {
            bail!("Track has no 'stbl' box.");
        };
        let Some(HeifBox { content: Content::Stsd(entries), .. }) = stbl.child(b"stsd") else {
            bail!("Track has no 'stsd' box.");
        };
        let Some(HeifBox { content: Content::Stsc(stsc), .. }) = stbl.child(b"stsc") else {
            bail!("Track has no 'stsc' box.");
        };
        let Some(HeifBox { content: Content::Stco(offsets), .. }) = stbl.child(b"stco") else {
            bail!("Track has no 'stco' box.");
        };
        let Some(HeifBox { content: Content::Stsz { fixed, count: num_samples, sizes }, .. }) = stbl.child(b"stsz") else {
            bail!("Track has no 'stsz' box.");
        };
        let Some(HeifBox { content: Content::Stts(stts), .. }) = stbl.child(b"stts") else {
            bail!("Track has no 'stts' box.");
        };
        let stts_samples = stts.iter().fold(0u32, |total, entry| total.wrapping_add(entry.0));
        if stts_samples != *num_samples {
            bail!("Number of samples in 'stts' and 'stsz' is inconsistent.");
        }

        let mut track = Track {
            id: *track_id,
            handler,
            width: 0,
            height: 0,
            chunks: Vec::new(),
            decoders: Vec::new(),
            timeline: Vec::new(),
            num_output_samples: 0,
            auxiliary_info_type: String::new(),
            tref,
            content_ids: None,
            tai_timestamps: None,
            alpha_track: None,
            next_sample_to_be_decoded: 0,
            next_sample_to_be_output: 0,
            decoder_is_flushed: false,
        };

        let mut current_sample = 0u32;
        let mut previous_description = -1i64;
        for (chunk_index, &chunk_offset) in offsets.iter().enumerate() {
            let Some((_, samples_per_chunk, description_index)) = chunk_entry(stsc, chunk_index as u32 + 1) else {
                bail!("'stco' box references a non-existing chunk.");
            };
            let Some(description) = entries.get(description_index as usize - 1) else {
                bail!("Track references a non-existing sample description.");
            };
            if let Some(HeifBox { content: Content::Auxi(urn), .. }) = description.child(b"auxi") {
                track.auxiliary_info_type = urn.clone();
            }
            if current_sample as u64 + samples_per_chunk as u64 > *num_samples as u64 {
                bail!("Number of samples in 'stsc' box exceeds sample sizes in 'stsz' box.");
            }

            let mut ranges = Vec::with_capacity(samples_per_chunk as usize);
            let mut file_offset = chunk_offset as u64;
            for index in 0..samples_per_chunk {
                let size = if *fixed != 0 { *fixed } else { sizes[(current_sample + index) as usize] };
                ranges.push((file_offset, size));
                file_offset += size as u64;
            }
            let decoder = if is_visual_sample_entry(description) {
                if chunk_index > 0 && description_index as i32 as i64 == previous_description {
                    track.chunks[chunk_index - 1].decoder
                } else {
                    track.decoders.push(SampleDecoder { format: sample_format(description), state: None });
                    Some(track.decoders.len() - 1)
                }
            } else {
                None
            };
            let last_sample = current_sample.wrapping_add(samples_per_chunk).wrapping_sub(1);
            track.chunks.push(Chunk { first_sample: current_sample, last_sample, ranges, decoder });
            current_sample += samples_per_chunk;
            previous_description = description_index as i32 as i64;
        }

        track.read_aux_info_boxes(stbl, offsets.len())?;
        if *num_samples > MAX_SEQUENCE_FRAMES {
            bail!("Security limit for maximum number of sequence frames exceeded");
        }
        track.init_sample_timing_table(stts, *num_samples, elst, (*timescale, *duration), mvhd)?;

        // The resolution, from the first chunk's sample description.
        if !offsets.is_empty() {
            let kind = if visual { "Visual" } else { "Metadata" };
            let Some((_, _, description_index)) = chunk_entry(stsc, 1) else {
                bail!("{kind} track has no chunk 1");
            };
            let Some(description) = entries.get(description_index as usize - 1) else {
                bail!("{kind} track has no sample description");
            };
            if visual {
                let Content::VisualSampleEntry { width, height, .. } = description.content else {
                    bail!("Visual track sample description does not match visual track.");
                };
                track.width = width;
                track.height = height;
            }
        }
        Ok(track)
    }

    /// The `saiz`/`saio` pairs of the sample table.
    fn read_aux_info_boxes(&mut self, stbl: &HeifBox, chunk_count: usize) -> Result<()> {
        let saiz_boxes: Vec<&HeifBox> = stbl.children().iter().filter(|child| matches!(child.content, Content::Saiz { .. })).collect();
        let saio_boxes: Vec<&HeifBox> = stbl.children().iter().filter(|child| matches!(child.content, Content::Saio { .. })).collect();
        if saiz_boxes.len() != saio_boxes.len() {
            bail!("Boxes 'saiz' and `saio` must come in pairs.");
        }
        for saiz in saiz_boxes {
            let Content::Saiz { aux_type, parameter, default_size, count, ref sizes } = saiz.content else {
                continue;
            };
            let saio = saio_boxes.iter().find_map(|saio| match &saio.content {
                Content::Saio { aux_type: saio_type, parameter: saio_parameter, offsets } if *saio_type == aux_type && *saio_parameter == parameter => Some(offsets),
                _ => None,
            });
            let Some(saio_offsets) = saio else {
                bail!("'saiz' box without matching 'saio' box.");
            };
            if saio_offsets.len() != 1 && saio_offsets.len() != chunk_count {
                bail!("Invalid number of chunks in 'saio' box.");
            }
            if aux_type == fourcc(b"suid") {
                self.content_ids = Some(AuxInfoReader::new(default_size, count, sizes, saio_offsets, &self.chunks)?);
            }
            if aux_type == fourcc(b"stai") {
                self.tai_timestamps = Some(AuxInfoReader::new(default_size, count, sizes, saio_offsets, &self.chunks)?);
            }
        }
        Ok(())
    }

    /// `init_sample_timing_table`: the presentation timeline is the media
    /// timeline; an edit list repeating the whole track only changes how
    /// many samples are output.
    fn init_sample_timing_table(&mut self, stts: &[(u32, u32)], num_samples: u32, elst: Option<&HeifBox>, mdhd: (u32, u64), mvhd: Option<(u32, u64)>) -> Result<()> {
        // `Box_stts::get_sample_duration` never leaves the first entry, so
        // an empty first entry never returns.
        if num_samples > 0 && stts.first().is_some_and(|entry| entry.0 == 0) {
            bail!("'stts' box with an empty first entry");
        }
        let mut current_chunk = 0usize;
        for sample in 0..num_samples {
            while current_chunk < self.chunks.len() && sample > self.chunks[current_chunk].last_sample {
                current_chunk += 1;
            }
            self.timeline.push(SampleTiming { sample, chunk: current_chunk });
        }

        let (movie_timescale, movie_duration) = mvhd.unwrap_or((0, 0));
        let (timescale, duration) = mdhd;
        let repeat_whole_track = match elst {
            Some(HeifBox { content: Content::Elst { repeat, entries }, .. }) => *repeat && entries.len() == 1 && entries[0].1 == 0 && entries[0].0 == duration,
            _ => false,
        };
        if movie_timescale == timescale && repeat_whole_track {
            if duration == 0 {
                bail!("Track duration is zero.");
            }
            self.num_output_samples = movie_duration / duration * self.timeline.len() as u64;
        } else {
            self.num_output_samples = self.timeline.len() as u64;
        }
        Ok(())
    }

    pub fn is_visual(&self) -> bool {
        self.handler == fourcc(b"pict") || self.handler == fourcc(b"vide")
    }

    /// Visual tracks, auxiliary ones included, can be decoded.
    pub fn is_decodable(&self) -> bool {
        self.handler != fourcc(b"meta")
    }

    pub fn end_of_sequence_reached(&self) -> bool {
        self.next_sample_to_be_output >= self.num_output_samples
    }

    fn is_alpha_track(&self) -> bool {
        self.handler == fourcc(b"auxv") && ALPHA_URNS.contains(&self.auxiliary_info_type.as_str())
    }
}

/// `Track_Visual::initialize_after_parsing` for every track: image
/// sequence tracks get the auxiliary alpha track that refers to them.
pub fn assign_alpha_tracks(tracks: &mut BTreeMap<u32, Track>) -> Result<()> {
    let ids: Vec<u32> = tracks.keys().copied().collect();
    for &id in &ids {
        if tracks[&id].handler != fourcc(b"pict") {
            continue;
        }
        let mut alpha_track = None;
        for &other in &ids {
            if other == id || !tracks[&other].is_alpha_track() {
                continue;
            }
            let Some(references) = &tracks[&other].tref else {
                bail!("Auxiliary track without 'tref'");
            };
            let auxl = fourcc(b"auxl");
            let refers = references.iter().filter(|(kind, _)| *kind == auxl).any(|(_, targets)| targets.contains(&id));
            if refers {
                alpha_track = Some(other);
            }
        }
        if let Some(track) = tracks.get_mut(&id) {
            track.alpha_track = alpha_track;
        }
    }
    Ok(())
}

impl SampleDecoder {
    /// Creates the decoder on first use (`require_decoder_plugin` and
    /// `new_decoder2`).
    fn state(&mut self) -> Result<&mut DecoderState> {
        if self.state.is_none() {
            let state = match &self.format {
                SampleFormat::Hevc(_) => DecoderState::Hevc(nerthus_hevc::Decoder::new()),
                #[cfg(not(target_arch = "wasm32"))]
                SampleFormat::Av1(_) => DecoderState::Av1(super::av1::SequenceDecoder::new()?),
                #[cfg(target_arch = "wasm32")]
                SampleFormat::Av1(_) => bail!("AV1 images cannot be read in the web version"),
                SampleFormat::Avc(Some(configuration)) => DecoderState::Avc(super::avc::SequenceDecoder::new(configuration.clone())),
                SampleFormat::Avc(None) => bail!("Sample description without AVC decoder configuration"),
                SampleFormat::Unsupported => bail!("No decoding plugin installed for this compression format"),
            };
            self.state = Some(state);
        }
        self.state.as_mut().ok_or_else(|| anyhow!("no decoder"))
    }

    /// `read_bitstream_configuration_data`.
    fn configuration_data(&self) -> Result<Vec<u8>> {
        match &self.format {
            SampleFormat::Hevc(Some(hvcc)) => {
                let mut data = Vec::new();
                for unit in &hvcc.nal_units {
                    data.extend_from_slice(&(unit.len() as u32).to_be_bytes());
                    data.extend_from_slice(unit);
                }
                Ok(data)
            }
            SampleFormat::Av1(Some(av1c)) => Ok(av1c.config_obus.clone()),
            // The AVC decoder owns its parameter sets and inserts them
            // before its first sample, regardless of that sample's index.
            SampleFormat::Avc(Some(_)) => Ok(Vec::new()),
            _ => bail!("Sample description without decoder configuration"),
        }
    }

    /// `decode_sequence_frame_from_compressed_data`.
    fn push_sample(&mut self, sample: Vec<u8>, with_configuration: bool, user_data: u32) -> Result<()> {
        self.state()?;
        let data = if with_configuration {
            let mut data = self.configuration_data()?;
            data.extend_from_slice(&sample);
            data
        } else {
            sample
        };
        if data.is_empty() {
            bail!("Input with empty data extent.");
        }
        match self.state()? {
            DecoderState::Hevc(decoder) => push_hevc_units(decoder, &data, user_data),
            #[cfg(not(target_arch = "wasm32"))]
            DecoderState::Av1(decoder) => decoder.push(&data, user_data),
            DecoderState::Avc(decoder) => decoder.push(&data, user_data),
        }
    }

    /// `flush_decoder`.
    fn flush(&mut self) -> Result<()> {
        match self.state()? {
            DecoderState::Hevc(decoder) => decoder.flush_data(),
            #[cfg(not(target_arch = "wasm32"))]
            DecoderState::Av1(_) => {}
            DecoderState::Avc(_) => {}
        }
        Ok(())
    }

    /// `get_decoded_frame`: the next picture, with the user data (sample
    /// index) the decoder passed through.
    fn decoded_frame(&mut self) -> Result<Option<(PixelImage, u32)>> {
        match self.state()? {
            DecoderState::Hevc(decoder) => match decoder.decode_next_picture() {
                Some(picture) => Ok(Some((super::context::hevc_picture_to_image(&picture)?, picture.user_data))),
                None => Ok(None),
            },
            #[cfg(not(target_arch = "wasm32"))]
            DecoderState::Av1(decoder) => Ok(decoder.next_image()),
            DecoderState::Avc(decoder) => Ok(decoder.nextImage()),
        }
    }
}

/// libde265's `push_data2`: length-prefixed NAL units, queued one by one
/// until the framing breaks.
fn push_hevc_units(decoder: &mut nerthus_hevc::Decoder, data: &[u8], user_data: u32) -> Result<()> {
    let mut position = 0;
    while position < data.len() {
        if data.len() - position < 4 {
            bail!("End of data");
        }
        let size_bytes = [data[position], data[position + 1], data[position + 2], data[position + 3]];
        let size = u32::from_be_bytes(size_bytes) as usize;
        position += 4;
        if size > data.len() - position {
            bail!("End of data");
        }
        decoder.push_nal(&data[position..position + size], user_data);
        position += size;
    }
    Ok(())
}

/// `Track_Visual::decode_next_image_sample`, with the alpha track's
/// sample moved in as the alpha plane.
pub fn decode_next_image_sample(tracks: &mut BTreeMap<u32, Track>, id: u32, stream: &mut FileStream) -> Result<PixelImage> {
    let Some(mut track) = tracks.remove(&id) else {
        bail!("Invalid track id");
    };
    let result = track.decode_next_sample(stream);
    let alpha_track = track.alpha_track;
    tracks.insert(id, track);
    let (mut image, decoded_sample) = result?;

    if let Some(alpha_id) = alpha_track {
        let alpha = decode_next_image_sample(tracks, alpha_id, stream)?;
        let plane = alpha.planes.get(&Channel::Y).cloned().ok_or_else(|| anyhow!("alpha image without luma"))?;
        image.planes.insert(Channel::Alpha, plane);
    }

    let track = &tracks[&id];
    if let Some(reader) = &track.content_ids {
        let info = reader.sample_info(stream, decoded_sample);
        if let Some((&last, rest)) = info.split_last() {
            if last != 0 {
                bail!("utf8string not null-terminated");
            }
            if rest.contains(&0) {
                bail!("utf8string with null character");
            }
        }
    }
    if let Some(reader) = &track.tai_timestamps {
        let info = reader.sample_info(stream, decoded_sample);
        if !info.is_empty() && info.len() != 9 {
            bail!("Wrong size of TAI timestamp data");
        }
    }
    if let Some(track) = tracks.get_mut(&id) {
        track.next_sample_to_be_output += 1;
    }
    Ok(image)
}

impl Track {
    /// The push/decode loop of `decode_next_image_sample`: the picture and
    /// the sample index its decoder reports.
    fn decode_next_sample(&mut self, stream: &mut FileStream) -> Result<(PixelImage, u32)> {
        if self.end_of_sequence_reached() {
            bail!("End of sequence");
        }
        if self.timeline.is_empty() {
            bail!("Sequence track without samples");
        }
        loop {
            let timing = self.timeline[(self.next_sample_to_be_decoded % self.timeline.len() as u64) as usize];
            let Some(chunk) = self.chunks.get(timing.chunk) else {
                bail!("Sample outside the track's chunks");
            };
            let Some(decoder_index) = chunk.decoder else {
                bail!("Sample without a decoder");
            };

            if self.next_sample_to_be_decoded != 0 {
                if let Some((image, decoded_sample)) = self.decoders[decoder_index].decoded_frame()? {
                    if (self.next_sample_to_be_output + 1).is_multiple_of(self.timeline.len() as u64) {
                        self.decoder_is_flushed = false;
                    }
                    return Ok((image, decoded_sample));
                }
                if self.decoder_is_flushed {
                    bail!("Did not decode all frames");
                }
            }

            if self.next_sample_to_be_decoded < self.num_output_samples {
                let (offset, size) = chunk.ranges[(timing.sample - chunk.first_sample) as usize];
                let sample = stream.read_range(offset, size);
                self.next_sample_to_be_decoded += 1;
                let with_configuration = timing.sample == 0;
                self.decoders[decoder_index].push_sample(sample, with_configuration, timing.sample)?;
            } else {
                self.decoders[decoder_index].flush()?;
                self.decoder_is_flushed = true;
            }
        }
    }
}
