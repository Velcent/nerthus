//! libheif's file and context (`file_layout.cc`, `file.cc`, `context.cc`,
//! `image-items/*.cc`): the boxes a file is read from, the image items it
//! describes, and their decoding into pixel images.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Result};

use super::boxes::{fourcc, read_box, Av1C, AvcC, Clap, ColorProfile, Content, HeifBox, HvcC, IlocItem, Infe, IpmaEntry, Range, Reference};
use super::image::{Channel, Chroma, Colorspace, PixelImage};
use super::nclx::{known_colour_primaries, known_matrix_coefficients, known_transfer_characteristics, Nclx};
use super::sequence::{assign_alpha_tracks, decode_next_image_sample, FileStream, Track};

const INITIAL_FTYP_REQUEST: usize = 1024;
const MAXIMUM_BOX_HEADER_SIZE: usize = 32;
const MAX_FILE_POS: u64 = 0x007F_FFFF_FFFF_FFFF;
const MAX_IMAGE_SIZE_PIXELS: u64 = 32768 * 32768;

/// What an image item is.
#[derive(Clone, Debug)]
pub enum ItemKind {
    Hevc(HvcC),
    Av1(Av1C),
    Avc(AvcC),
    Jpeg,
    Jpeg2000,
    Vvc,
    Mask,
    Grid { rows: u32, columns: u32, width: u32, height: u32, tiles: Vec<u32> },
    Overlay { references: Vec<u32>, data: Vec<u8> },
    Iden,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u32,
    pub item_type: u32,
    pub kind: Option<ItemKind>,
    /// `ImageItem_Error`'s error.
    pub error: Option<String>,
    pub properties: Vec<HeifBox>,
    pub width: u32,
    pub height: u32,
    pub nclx: Nclx,
    pub icc: Option<Vec<u8>>,
    pub alpha: Option<u32>,
    pub is_thumbnail: bool,
    pub premultiplied: bool,
}

impl Item {
    fn property(&self, kind: &[u8; 4]) -> Option<&HeifBox> {
        let wanted = fourcc(kind);
        self.properties.iter().find(|property| property.kind() == wanted)
    }

    pub fn ispe(&self) -> Option<(u32, u32)> {
        self.properties.iter().find_map(|property| match property.content {
            Content::Ispe { width, height } => Some((width, height)),
            _ => None,
        })
    }

    fn aux_type(&self) -> Option<String> {
        self.properties.iter().find_map(|property| match &property.content {
            Content::AuxC { aux_type } => Some(aux_type.clone()),
            _ => None,
        })
    }
}

pub struct Context<'a> {
    data: &'a [u8],
    ipco: Vec<HeifBox>,
    ipma: Vec<IpmaEntry>,
    iloc: Vec<IlocItem>,
    idat: Option<(usize, u64)>,
    iref: Option<Vec<Reference>>,
    infe: BTreeMap<u32, Infe>,
    primary_id: u32,
    pub items: BTreeMap<u32, Item>,
    pub top_level: Vec<u32>,
    pub primary: Option<u32>,
    /// The sequence tracks by id, and the first visual one's id (0 when
    /// there is none).
    pub tracks: BTreeMap<u32, Track>,
    visual_track_id: u32,
    /// The file stream's position after the last read.
    stream_position: Cell<u64>,
}

/// `check_for_valid_image_size`.
fn check_image_size(width: u32, height: u32) -> Result<()> {
    let max_side = i32::MAX as u32;
    if width > max_side || height > max_side || (height != 0 && width as u64 > MAX_IMAGE_SIZE_PIXELS / height as u64) {
        bail!("Image size {width}x{height} exceeds the maximum image size {MAX_IMAGE_SIZE_PIXELS}");
    }
    if width == 0 || height == 0 {
        bail!("zero width or height");
    }
    Ok(())
}

fn has_brand(brands: &[u32], brand: &[u8; 4]) -> bool {
    brands.contains(&fourcc(brand))
}

impl<'a> Context<'a> {
    /// `FileLayout::read`, `HeifFile::parse_heif_file` and
    /// `HeifContext::interpret_heif_file`.
    pub fn read(data: &'a [u8]) -> Result<Context<'a>> {
        let mut max_length = data.len().min(INITIAL_FTYP_REQUEST);
        if max_length < MAXIMUM_BOX_HEADER_SIZE {
            bail!("File size too small.");
        }
        let mut header_range = Range::new(data, 0, max_length);
        let size = header_range.read32() as u64;
        let kind = header_range.read32();
        if kind != fourcc(b"ftyp") {
            bail!("File does not start with 'ftyp' box.");
        }
        if size == 0 {
            bail!("ftyp box shall not be the only box in the file");
        }
        if size > max_length as u64 {
            bail!("ftyp box larger than initial read range");
        }
        // A broken ftyp is noted but not reported here.
        let mut ftyp_range = Range::new(data, 0, size as usize);
        let ftyp = read_box(&mut ftyp_range).ok();

        // Where libheif's file stream is left, for reads that seek past
        // the end of the file (which fail silently, so the next read starts
        // there instead).
        let mut stream_position = size;
        let mut next_box_start = size as usize;
        let mut meta: Option<HeifBox> = None;
        let mut moov: Option<HeifBox> = None;
        loop {
            let header_end = next_box_start + MAXIMUM_BOX_HEADER_SIZE;
            if header_end > max_length {
                max_length = data.len().min(header_end.max(max_length));
            }
            if header_end > max_length {
                if meta.is_some() || moov.is_some() {
                    break;
                }
                bail!("Insufficient input data");
            }
            let mut box_range = Range::new(data, next_box_start, max_length);
            let box_size = box_range.read32() as u64;
            let box_kind = box_range.read32();
            let mut full_size = box_size;
            let mut header_size = 8;
            if box_size == 1 {
                let high = box_range.read32() as u64;
                let low = box_range.read32() as u64;
                full_size = (high << 32) | low;
                header_size += 8;
            }
            if box_kind == fourcc(b"uuid") {
                header_size += 16;
            }
            stream_position = next_box_start as u64 + header_size;
            if box_kind == fourcc(b"meta") || box_kind == fourcc(b"moov") {
                if full_size == 0 {
                    bail!("Cannot read {} box with unspecified size", if box_kind == fourcc(b"meta") { "meta" } else { "moov" });
                }
                let end = next_box_start as u64 + full_size;
                if end > data.len() as u64 {
                    bail!("Cannot read full {} box", if box_kind == fourcc(b"meta") { "meta" } else { "moov" });
                }
                let mut parse_range = Range::new(data, next_box_start, end as usize);
                let parsed = read_box(&mut parse_range).map_err(|error| anyhow!("{error}"))?;
                stream_position = end;
                if box_kind == fourcc(b"meta") {
                    meta = Some(parsed);
                } else {
                    moov = Some(parsed);
                }
            }
            if full_size == 0 {
                if meta.is_some() || moov.is_some() {
                    break;
                }
                bail!("No meta box found");
            }
            next_box_start = next_box_start.checked_add(full_size as usize).ok_or_else(|| anyhow!("Box size too large, integer overflow"))?;
        }

        let Some(HeifBox { content: Content::Ftyp { major, brands }, .. }) = ftyp else {
            bail!("No ftyp box");
        };
        let _ = major;
        let sequence_brand = has_brand(&brands, b"msf1") || has_brand(&brands, b"isom") || has_brand(&brands, b"mp41") || has_brand(&brands, b"mp42");
        let supported = [b"heic", b"heix", b"mif1", b"avif", b"1pic", b"jpeg", b"isom", b"mp42", b"mp41", b"msf1"];
        if !supported.iter().any(|brand| has_brand(&brands, brand)) {
            bail!("File does not include any supported brands.");
        }
        if meta.is_none() && !sequence_brand {
            bail!("No meta box");
        }
        if moov.is_none() && sequence_brand {
            bail!("No moov box");
        }

        let mut context = Context {
            data,
            ipco: Vec::new(),
            ipma: Vec::new(),
            iloc: Vec::new(),
            idat: None,
            iref: None,
            infe: BTreeMap::new(),
            primary_id: 0,
            items: BTreeMap::new(),
            top_level: Vec::new(),
            primary: None,
            tracks: BTreeMap::new(),
            visual_track_id: 0,
            stream_position: Cell::new(stream_position),
        };
        if let Some(meta) = &meta {
            context.parse_images(meta)?;
        }
        let mut mvhd = None;
        if let Some(moov) = &moov {
            let Some(HeifBox { content: Content::Mvhd { timescale, duration }, .. }) = moov.child(b"mvhd") else {
                bail!("No mvhd box in image sequence.");
            };
            mvhd = Some((*timescale, *duration));
        }
        if meta.is_some() {
            context.interpret_images()?;
        }
        if let Some(moov) = &moov {
            context.interpret_sequences(moov, mvhd)?;
        }
        Ok(context)
    }

    /// `HeifContext::interpret_heif_file_sequences`.
    fn interpret_sequences(&mut self, moov: &HeifBox, mvhd: Option<(u32, u64)>) -> Result<()> {
        let trak = fourcc(b"trak");
        for track_box in moov.children().iter().filter(|child| child.kind() == trak) {
            let Some(track) = Track::alloc(track_box, mvhd)? else {
                continue;
            };
            let id = track.id;
            if track.is_visual() && self.visual_track_id == 0 {
                self.visual_track_id = id;
            }
            self.tracks.entry(id).or_insert(track);
        }
        assign_alpha_tracks(&mut self.tracks)
    }

    /// `heif_context_has_sequence`.
    pub fn has_sequence(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// `HeifContext::get_track(0)`: the first visual track, or the track
    /// with the lowest id.
    pub fn default_track(&self) -> Option<u32> {
        if self.visual_track_id != 0 {
            return Some(self.visual_track_id);
        }
        self.tracks.keys().next().copied()
    }

    /// `heif_track_decode_next_image` before the colour conversion.
    pub fn decode_next_track_image(&mut self, id: u32) -> Result<PixelImage> {
        let mut stream = FileStream { data: self.data, position: self.stream_position.get() };
        let result = decode_next_image_sample(&mut self.tracks, id, &mut stream);
        self.stream_position.set(stream.position);
        result
    }

    /// `HeifFile::parse_heif_images`.
    fn parse_images(&mut self, meta: &HeifBox) -> Result<()> {
        if let Some(hdlr) = meta.child(b"hdlr")
            && let Content::Hdlr { handler } = hdlr.content
            && handler != fourcc(b"pict")
        {
            bail!("No pict handler");
        }
        let Some(pitm) = meta.child(b"pitm") else {
            bail!("No pitm box");
        };
        if let Content::Pitm { item_id } = pitm.content {
            self.primary_id = item_id;
        }
        let Some(iprp) = meta.child(b"iprp") else {
            bail!("No iprp box");
        };
        let Some(ipco) = iprp.child(b"ipco") else {
            bail!("No ipco box");
        };
        self.ipco = ipco.children().to_vec();
        let mut ipma_found = false;
        for child in iprp.children() {
            if let Content::Ipma(entries) = &child.content {
                ipma_found = true;
                self.ipma.extend(entries.iter().cloned());
            }
        }
        if !ipma_found {
            bail!("No ipma box");
        }
        let Some(iloc) = meta.child(b"iloc") else {
            bail!("No iloc box");
        };
        if let Content::Iloc(items) = &iloc.content {
            self.iloc = items.clone();
        }
        if let Some(idat) = meta.child(b"idat")
            && let Content::Idat { start, size } = idat.content
        {
            self.idat = Some((start, size));
        }
        if let Some(iref) = meta.child(b"iref")
            && let Content::Iref(references) = &iref.content
        {
            self.iref = Some(references.clone());
            let mut parents = BTreeSet::new();
            self.check_for_ref_cycle(self.primary_id, &mut parents)?;
        }
        let Some(iinf) = meta.child(b"iinf") else {
            bail!("No iinf box");
        };
        for child in iinf.children() {
            if let Content::Infe(infe) = &child.content {
                self.infe.entry(infe.item_id).or_insert_with(|| infe.clone());
            }
        }
        Ok(())
    }

    fn check_for_ref_cycle(&self, id: u32, parents: &mut BTreeSet<u32>) -> Result<()> {
        if parents.contains(&id) {
            bail!("Image reference cycle");
        }
        parents.insert(id);
        for reference in self.references(id, b"dimg") {
            self.check_for_ref_cycle(reference, parents)?;
        }
        parents.remove(&id);
        Ok(())
    }

    /// `Box_iref::get_references`: the first reference of the type.
    pub fn references(&self, from: u32, kind: &[u8; 4]) -> Vec<u32> {
        let wanted = fourcc(kind);
        let Some(references) = &self.iref else {
            return Vec::new();
        };
        references.iter().find(|reference| reference.from == from && reference.kind == wanted).map(|reference| reference.to.clone()).unwrap_or_default()
    }

    fn references_from(&self, from: u32) -> Vec<Reference> {
        let Some(references) = &self.iref else {
            return Vec::new();
        };
        references.iter().filter(|reference| reference.from == from).cloned().collect()
    }

    /// `Box_ipco::get_properties_for_item_ID`.
    fn properties(&self, id: u32) -> Result<Vec<HeifBox>> {
        let Some(entry) = self.ipma.iter().find(|entry| entry.item_id == id) else {
            bail!("Item (ID={id}) has no properties assigned to it in ipma box");
        };
        let mut properties = Vec::new();
        for &(_, index) in &entry.associations {
            if index as usize > self.ipco.len() {
                bail!("Nonexisting property (index={index}) for item  ID={id} referenced in ipma box");
            }
            if index > 0 {
                properties.push(self.ipco[index as usize - 1].clone());
            }
        }
        Ok(properties)
    }

    fn is_property_essential(&self, id: u32, property_index: usize) -> bool {
        let Some(entry) = self.ipma.iter().find(|entry| entry.item_id == id) else {
            return false;
        };
        entry.associations.iter().find(|&&(_, index)| index as usize == property_index + 1).is_some_and(|&(essential, _)| essential)
    }

    /// `ImageItem::alloc_for_infe_box` and `initialize_decoder`.
    fn allocate_item(&self, infe: &Infe) -> Option<Item> {
        let item_type = infe.item_type;
        let name = item_type.to_be_bytes();
        let jpeg_mime = item_type == fourcc(b"mime") && infe.content_type == "image/jpeg";
        let image = jpeg_mime || matches!(&name, b"hvc1" | b"av01" | b"vvc1" | b"avc1" | b"unci" | b"j2k1" | b"lhv1" | b"mski" | b"grid" | b"iovl" | b"iden" | b"tili");
        if !image {
            return None;
        }
        let mut item = Item {
            id: infe.item_id,
            item_type,
            kind: None,
            error: None,
            properties: Vec::new(),
            width: 0,
            height: 0,
            nclx: Nclx::undefined(),
            icc: None,
            alpha: None,
            is_thumbnail: false,
            premultiplied: false,
        };
        match &name {
            b"unci" => item.error = Some("Image item of type 'unci' is not supported.".into()),
            b"lhv1" => item.error = Some("Layered HEVC images (lhv1) are not supported yet".into()),
            _ => {}
        }
        Some(item)
    }

    fn initialize_decoder(&self, item: &mut Item) -> Result<()> {
        let name = item.item_type.to_be_bytes();
        let jpeg_mime = item.item_type == fourcc(b"mime");
        let kind = match &name {
            b"hvc1" => {
                let Some(HeifBox { content: Content::HvcC(hvcc), .. }) = item.property(b"hvcC") else {
                    bail!("No hvcC box");
                };
                ItemKind::Hevc(hvcc.clone())
            }
            b"av01" => {
                let Some(HeifBox { content: Content::Av1C(av1c), .. }) = item.property(b"av1C") else {
                    bail!("No av1C box");
                };
                ItemKind::Av1(av1c.clone())
            }
            b"avc1" => {
                let Some(HeifBox { content: Content::AvcC(configuration), .. }) = item.property(b"avcC") else {
                    bail!("No avcC box");
                };
                ItemKind::Avc(configuration.clone())
            }
            b"vvc1" => {
                if item.property(b"vvcC").is_none() {
                    bail!("No av1C box");
                }
                ItemKind::Vvc
            }
            b"j2k1" => {
                if item.property(b"j2kH").is_none() {
                    bail!("No j2kH box found.");
                }
                ItemKind::Jpeg2000
            }
            b"jpeg" => ItemKind::Jpeg,
            b"mski" => ItemKind::Mask,
            b"tili" => bail!("Tiled image without 'tilC' property box."),
            b"grid" => self.read_grid_spec(item.id)?,
            b"iovl" => self.read_overlay_spec(item.id)?,
            b"iden" => ItemKind::Iden,
            _ if jpeg_mime => ItemKind::Jpeg,
            _ => bail!("unsupported item"),
        };
        item.kind = Some(kind);
        Ok(())
    }

    /// `ImageItem_Grid::read_grid_spec`.
    fn read_grid_spec(&self, id: u32) -> Result<ItemKind> {
        let data = self.uncompressed_item_data(id)?;
        if data.len() < 8 {
            bail!("Less than 8 bytes of data");
        }
        if data[0] != 0 {
            bail!("Grid image version {} is not supported", data[0]);
        }
        let large = data[1] & 1 != 0;
        let rows = data[2] as u32 + 1;
        let columns = data[3] as u32 + 1;
        let (width, height) = if large {
            if data.len() < 12 {
                bail!("Grid image data incomplete");
            }
            (u32::from_be_bytes([data[4], data[5], data[6], data[7]]), u32::from_be_bytes([data[8], data[9], data[10], data[11]]))
        } else {
            (u16::from_be_bytes([data[4], data[5]]) as u32, u16::from_be_bytes([data[6], data[7]]) as u32)
        };
        if self.iref.is_none() {
            bail!("No iref box available, but needed for grid image");
        }
        let tiles = self.references(id, b"dimg");
        if tiles.len() as u32 != rows * columns {
            bail!("Tiled image with {rows}x{columns}={} tiles, but only {} tile images in file", rows * columns, tiles.len());
        }
        Ok(ItemKind::Grid { rows, columns, width, height, tiles })
    }

    /// `ImageItem_Overlay::read_overlay_spec`.
    fn read_overlay_spec(&self, id: u32) -> Result<ItemKind> {
        if self.iref.is_none() {
            bail!("No iref box available, but needed for iovl image");
        }
        let references = self.references(id, b"dimg");
        let data = self.uncompressed_item_data(id)?;
        let count = references.len();
        if data.len() < 10 {
            bail!("Overlay image data incomplete");
        }
        if data[0] != 0 {
            bail!("Overlay image data version {} is not implemented yet", data[0]);
        }
        let field = if data[1] & 1 != 0 { 4 } else { 2 };
        if 2 + 4 * 2 + 2 * field + count * 2 * field > data.len() {
            bail!("Overlay image data incomplete");
        }
        let read = |position: usize| -> u32 {
            let mut value = 0u32;
            for byte in &data[position..position + field] {
                value = (value << 8) | *byte as u32;
            }
            value
        };
        let width = read(10);
        let height = read(10 + field);
        if width == 0 || height == 0 {
            bail!("Overlay image with zero width or height.");
        }
        Ok(ItemKind::Overlay { references, data })
    }

    /// `HeifContext::interpret_heif_file_images`.
    fn interpret_images(&mut self) -> Result<()> {
        let ids: Vec<u32> = self.infe.keys().copied().collect();
        for &id in &ids {
            let infe = self.infe[&id].clone();
            let Some(mut item) = self.allocate_item(&infe) else {
                continue;
            };
            match self.properties(id) {
                Ok(properties) => item.properties = properties,
                Err(error) => item.error = Some(error.to_string()),
            }
            if item.error.is_none()
                && let Err(error) = self.initialize_decoder(&mut item)
            {
                item.error = Some(error.to_string());
            }
            if !infe.hidden {
                if id == self.primary_id {
                    self.primary = Some(id);
                }
                self.top_level.push(id);
            }
            self.items.insert(id, item);
        }
        if self.primary.is_none() {
            bail!("'pitm' box references an unsupported or non-existing image");
        }

        for &id in &ids {
            let Some(item) = self.items.get(&id) else {
                continue;
            };
            if item.error.is_some() {
                continue;
            }
            let properties = self.properties(id)?;
            for (position, property) in properties.iter().enumerate() {
                if let Content::Other = property.content {
                    let index = self.ipco.iter().position(|candidate| candidate.header.start == property.header.start).unwrap_or(position);
                    if self.is_property_essential(id, index) {
                        bail!("could not parse item property '{}'", String::from_utf8_lossy(&property.kind().to_be_bytes()));
                    }
                }
            }
            for property in &properties {
                if let Content::Error { message, fatality } = &property.content
                    && *fatality != super::boxes::Fatality::Optional
                {
                    bail!("{message}");
                }
            }
            let mut width = 0;
            let mut height = 0;
            let mut ispe_read = false;
            for property in &properties {
                if let Content::Ispe { width: w, height: h } = property.content {
                    if w == 0 || h == 0 {
                        bail!("Zero image width or height");
                    }
                    width = w;
                    height = h;
                    ispe_read = true;
                }
            }
            let mut nclx = Nclx::undefined();
            let mut icc = None;
            for property in &properties {
                if let Content::Colr(profile) = &property.content {
                    match profile {
                        ColorProfile::Nclx(profile) => nclx = *profile,
                        ColorProfile::Raw(bytes) => icc = Some(bytes.clone()),
                    }
                }
            }
            for property in &properties {
                match &property.content {
                    Content::Clap(clap) => {
                        width = clap.width.round() as u32;
                        height = clap.height.round() as u32;
                    }
                    Content::Imir { .. } => {
                        if !ispe_read {
                            bail!("No ispe property");
                        }
                    }
                    Content::Irot { rotation }
                        if (*rotation == 90 || *rotation == 270) => {
                            if !ispe_read {
                                bail!("No ispe property");
                            }
                            std::mem::swap(&mut width, &mut height);
                        }
                    _ => {}
                }
            }
            let item = self.items.get_mut(&id).expect("item");
            item.width = width;
            item.height = height;
            item.nclx = nclx;
            item.icc = icc;
        }

        // Thumbnails and auxiliary images leave the top-level list.
        if self.iref.is_some() {
            for &id in &ids {
                if !self.items.contains_key(&id) {
                    continue;
                }
                for reference in self.references_from(id) {
                    if reference.kind == fourcc(b"thmb") {
                        for &master in &reference.to {
                            self.items.get_mut(&id).expect("item").is_thumbnail = true;
                            let Some(master_item) = self.items.get(&master) else {
                                bail!("Thumbnail references a non-existing image");
                            };
                            if master_item.is_thumbnail {
                                bail!("Thumbnail references another thumbnail");
                            }
                            if master == id {
                                bail!("Recursive thumbnail image detected");
                            }
                        }
                        self.top_level.retain(|&top| top != id);
                    } else if reference.kind == fourcc(b"auxl") {
                        let Some(aux_type) = self.items[&id].aux_type() else {
                            bail!("No auxC property for image {id}");
                        };
                        let alpha = aux_type == "urn:mpeg:avc:2015:auxid:1" || aux_type == "urn:mpeg:hevc:2015:auxid:1" || aux_type == "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";
                        if alpha {
                            for &master in &reference.to {
                                if !self.items.contains_key(&master) {
                                    if !self.infe.contains_key(&master) {
                                        bail!("Non-existing alpha image referenced");
                                    }
                                    continue;
                                }
                                if master == id {
                                    bail!("Recursive alpha image detected");
                                }
                                self.items.get_mut(&master).expect("master").alpha = Some(id);
                            }
                        }
                        let depth = aux_type == "urn:mpeg:hevc:2015:auxid:2" || aux_type == "urn:mpeg:mpegB:cicp:systems:auxiliary:depth";
                        if depth {
                            for &master in &reference.to {
                                if !self.items.contains_key(&master) {
                                    if !self.infe.contains_key(&master) {
                                        bail!("Non-existing depth image referenced");
                                    }
                                    continue;
                                }
                                if master == id {
                                    bail!("Recursive depth image detected");
                                }
                            }
                        }
                        for &master in &reference.to {
                            if !self.items.contains_key(&master) {
                                if !self.infe.contains_key(&master) {
                                    bail!("Non-existing aux image referenced");
                                }
                                continue;
                            }
                            if master == id {
                                bail!("Recursive aux image detected");
                            }
                            self.top_level.retain(|&top| top != id);
                        }
                    }
                }
            }
        }

        for (&id, item) in &self.items {
            if item.error.is_some() {
                continue;
            }
            if item.item_type == fourcc(b"hvc1") && item.property(b"hvcC").is_none() {
                bail!("No hvcC property in hvc1 type image");
            }
            if item.item_type == fourcc(b"vvc1") && item.property(b"vvcC").is_none() {
                bail!("No vvcC property in vvc1 type image");
            }
            let _ = id;
        }

        // Grids without a profile take their first tile's.
        if self.iref.is_some() {
            let grid_ids: Vec<u32> = self.items.iter().filter(|(_, item)| item.error.is_none() && item.item_type == fourcc(b"grid")).map(|(&id, _)| id).collect();
            for id in grid_ids {
                let tiles = self.references(id, b"dimg");
                let Some(&first) = tiles.first() else {
                    continue;
                };
                let Some(tile) = self.items.get(&first) else {
                    continue;
                };
                let (tile_icc, tile_nclx) = (tile.icc.clone(), tile.nclx);
                let grid = self.items.get_mut(&id).expect("grid");
                if grid.icc.is_none() && tile_icc.is_some() {
                    grid.icc = tile_icc;
                }
                if grid.nclx == Nclx::undefined() && tile_nclx != Nclx::undefined() {
                    grid.nclx = tile_nclx;
                }
            }
        }

        // Metadata items must have data (Exif, mime); its content only
        // reaches profiles.
        for &id in &ids {
            let infe = &self.infe[&id];
            let item_type = infe.item_type;
            let name = item_type.to_be_bytes();
            let is_image = (item_type == fourcc(b"mime") && infe.content_type == "image/jpeg") || matches!(&name, b"hvc1" | b"av01" | b"grid" | b"tili" | b"iden" | b"iovl" | b"avc1" | b"unci" | b"vvc1" | b"jpeg" | b"j2k1" | b"mski");
            if is_image || item_type == fourcc(b"rgan") {
                continue;
            }
            if let Err(error) = self.uncompressed_item_data(id)
                && (item_type == fourcc(b"Exif") || item_type == fourcc(b"mime"))
            {
                return Err(error);
            }
            for target in self.references(id, b"cdsc") {
                if !self.items.contains_key(&target) && !self.infe.contains_key(&target) {
                    bail!("Metadata assigned to non-existing image");
                }
            }
        }

        for &id in &ids {
            if !self.references(id, b"prem").is_empty() {
                let Some(item) = self.items.get_mut(&id) else {
                    bail!("`prem` link assigned to non-existing image");
                };
                item.premultiplied = true;
            }
        }
        Ok(())
    }

    /// `Box_iloc::read_data`.
    pub fn item_data(&self, id: u32) -> Result<Vec<u8>> {
        let Some(item) = self.iloc.iter().find(|item| item.item_id == id) else {
            bail!("Item with ID {id} has no compressed data");
        };
        let mut data = Vec::new();
        for extent in &item.extents {
            match item.construction_method {
                0 => {
                    if extent.offset > MAX_FILE_POS || item.base_offset > MAX_FILE_POS || extent.length > MAX_FILE_POS {
                        bail!("iloc data pointers out of allowed range");
                    }
                    let start = extent.offset + item.base_offset;
                    if start + extent.length > self.data.len() as u64 {
                        bail!("Extent in iloc box references data outside of file bounds (points to file position {start})");
                    }
                    if extent.length == 0 {
                        continue;
                    }
                    data.extend_from_slice(&self.data[start as usize..(start + extent.length) as usize]);
                    self.stream_position.set(start + extent.length);
                }
                1 => {
                    let Some((idat_start, idat_size)) = self.idat else {
                        bail!("idat box referenced in iref box is not present in file");
                    };
                    let start = extent.offset + item.base_offset;
                    let length = extent.length;
                    // A broken idat range is skipped silently, as in libheif.
                    if start <= idat_start as u64 + idat_size && length <= idat_size && start + length <= idat_size {
                        let from = idat_start as u64 + start;
                        if from + length <= self.data.len() as u64 {
                            data.extend_from_slice(&self.data[from as usize..(from + length) as usize]);
                            self.stream_position.set(from + length);
                        }
                    }
                }
                method => bail!("Item construction method {method} not implemented"),
            }
        }
        Ok(data)
    }

    /// `HeifFile::get_uncompressed_item_data`.
    fn uncompressed_item_data(&self, id: u32) -> Result<Vec<u8>> {
        let Some(infe) = self.infe.get(&id) else {
            bail!("Nonexisting item referenced");
        };
        if infe.item_type == fourcc(b"mime") {
            let encoding = infe.content_encoding.as_str();
            if encoding == "compress_zlib" || encoding == "deflate" {
                let compressed = self.item_data(id)?;
                let mut decoded = Vec::new();
                let result = if encoding == "deflate" {
                    std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(&compressed[..]), &mut decoded)
                } else {
                    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&compressed[..]), &mut decoded)
                };
                result.map_err(|error| anyhow!("{error}"))?;
                return Ok(decoded);
            }
            if !encoding.is_empty() {
                bail!("Unsupported header compression method {encoding}");
            }
        }
        self.item_data(id)
    }

    /// `HeifContext::has_alpha`.
    pub fn has_alpha(&self, id: u32) -> bool {
        let Some(item) = self.items.get(&id) else {
            return false;
        };
        if item.alpha.is_some() {
            return true;
        }
        if item.item_type == fourcc(b"grid") {
            let Ok(data) = self.uncompressed_item_data(id) else {
                return false;
            };
            if data.len() < 8 || data[0] != 0 {
                return false;
            }
            let rows = data[2] as usize + 1;
            let columns = data[3] as usize + 1;
            if data[1] & 1 != 0 && data.len() < 12 {
                return false;
            }
            if self.iref.is_none() {
                return false;
            }
            let tiles = self.references(id, b"dimg");
            if tiles.len() != rows * columns {
                return false;
            }
            if tiles.iter().any(|tile| !self.items.contains_key(tile)) {
                return false;
            }
            return tiles.iter().any(|tile| self.items[tile].alpha.is_some());
        }
        false
    }

    /// `get_id_of_non_virtual_child_image`.
    fn non_virtual_child(&self, id: u32) -> Result<u32> {
        let item_type = self.infe.get(&id).map_or(0, |infe| infe.item_type);
        if item_type == fourcc(b"grid") || item_type == fourcc(b"iden") || item_type == fourcc(b"iovl") {
            if self.iref.is_none() {
                bail!("Derived image does not reference any other image items");
            }
            let references = self.references(id, b"dimg");
            if references.is_empty() || references[0] == id {
                bail!("Derived image does not reference any other image items");
            }
            return self.non_virtual_child(references[0]);
        }
        if !self.items.contains_key(&id) {
            bail!("Image item {id} referenced, but it does not exist");
        }
        Ok(id)
    }

    /// `get_luma_bits_per_pixel` (-1 when unknown).
    pub fn luma_bits_per_pixel(&self, id: u32) -> i32 {
        let Some(item) = self.items.get(&id) else {
            return -1;
        };
        match &item.kind {
            Some(ItemKind::Hevc(hvcc)) => hvcc.bit_depth_luma as i32,
            Some(ItemKind::Av1(av1c)) => av1_bit_depth(av1c),
            Some(ItemKind::Avc(configuration)) => configuration.bit_depth_luma as i32,
            Some(ItemKind::Grid { .. }) | Some(ItemKind::Iden) | Some(ItemKind::Overlay { .. }) => match self.non_virtual_child(id) {
                Ok(child) if child != id => self.luma_bits_per_pixel(child),
                _ => -1,
            },
            _ => 8,
        }
    }

    /// `ImageItem::decode_image`.
    pub fn decode_image(&self, id: u32, processed: &BTreeSet<u32>) -> Result<PixelImage> {
        let Some(item) = self.items.get(&id) else {
            bail!("Nonexisting item referenced");
        };
        if let Some((width, height)) = item.ispe() {
            check_image_size(width, height)?;
        }
        let mut image = self.decode_compressed_image(item, processed)?;

        for property in &item.properties {
            match &property.content {
                Content::Irot { rotation } => image = image.rotate_ccw(*rotation)?,
                Content::Imir { horizontal } => image = image.mirror(*horizontal)?,
                Content::Clap(clap) => image = crop_to_clap(&image, clap)?,
                _ => {}
            }
        }

        if let Some(alpha_id) = item.alpha {
            let alpha_item = &self.items[&alpha_id];
            if let Some(error) = &alpha_item.error {
                bail!("{error}");
            }
            let mut alpha = self.decode_image(alpha_id, processed)?;
            let channel = match alpha.colorspace {
                Colorspace::YCbCr | Colorspace::Monochrome => Channel::Y,
                Colorspace::Rgb => Channel::R,
            };
            if alpha_item.width != image.width || alpha_item.height != image.height {
                alpha = alpha.scale_nearest_neighbor(image.width, image.height)?;
            }
            let plane = alpha.planes.remove(&channel).ok_or_else(|| anyhow!("alpha image without plane"))?;
            image.planes.insert(Channel::Alpha, plane);
            if item.premultiplied {
                image.premultiplied_alpha = true;
            }
        }

        if item.nclx != Nclx::undefined() {
            image.nclx = item.nclx;
        }
        Ok(image)
    }

    /// `decode_compressed_image` of each item type.
    fn decode_compressed_image(&self, item: &Item, processed: &BTreeSet<u32>) -> Result<PixelImage> {
        let mut processed = processed.clone();
        if processed.contains(&item.id) {
            bail!("'iref' has cyclic references");
        }
        processed.insert(item.id);
        match item.kind.as_ref().ok_or_else(|| anyhow!("item without decoder"))? {
            ItemKind::Hevc(hvcc) => {
                let data = self.item_data(item.id)?;
                decode_hevc(hvcc, &data)
            }
            #[cfg(not(target_arch = "wasm32"))]
            ItemKind::Av1(av1c) => {
                let mut data = av1c.config_obus.clone();
                data.extend_from_slice(&self.item_data(item.id)?);
                super::av1::decode_av1(&data)
            }
            // rav1d needs threads and the C allocator, which the web build
            // does not have.
            #[cfg(target_arch = "wasm32")]
            ItemKind::Av1(_) => bail!("AV1 images cannot be read in the web version"),
            ItemKind::Avc(configuration) => {
                let data = self.item_data(item.id)?;
                super::avc::decode(configuration, &data)
            }
            ItemKind::Jpeg | ItemKind::Jpeg2000 | ItemKind::Vvc => bail!("No decoding plugin installed for this compression format"),
            ItemKind::Mask => bail!("mask images are not supported"),
            ItemKind::Iden => {
                if self.iref.is_none() {
                    bail!("No iref box available, but needed for iden image");
                }
                let references = self.references(item.id, b"dimg");
                if references.len() > 1 {
                    bail!("'iden' image with more than one reference image");
                }
                let Some(&reference) = references.first() else {
                    bail!("'iden' image without 'dimg' reference");
                };
                if reference == item.id {
                    bail!("'iden' image referring to itself");
                }
                let Some(referenced) = self.items.get(&reference) else {
                    bail!("'iden' image references unavailable image");
                };
                if let Some(error) = &referenced.error {
                    bail!("{error}");
                }
                self.decode_image(reference, &processed)
            }
            ItemKind::Grid { rows, columns, width, height, tiles } => self.decode_grid(*rows, *columns, *width, *height, tiles, &processed),
            ItemKind::Overlay { references, data } => self.decode_overlay(item.id, references, data, &processed),
        }
    }

    /// `ImageItem_Overlay::decode_overlay_image`: an 8-bit RGB canvas in
    /// the background colour with each referenced image pasted (blended
    /// by its alpha) at its offset.
    fn decode_overlay(&self, id: u32, references: &[u32], data: &[u8], processed: &BTreeSet<u32>) -> Result<PixelImage> {
        let field = if data[1] & 1 != 0 { 4 } else { 2 };
        let read_unsigned = |position: usize, length: usize| -> u32 {
            let mut value = 0u32;
            for byte in &data[position..position + length] {
                value = (value << 8) | *byte as u32;
            }
            value
        };
        let read_signed = |position: usize, length: usize| -> i32 {
            let value = read_unsigned(position, length);
            let high_bit = 0x80u32 << ((length - 1) * 8);
            if value & high_bit != 0 { -(((!value) & 0x7fff_ffff) as i32) - 1 } else { value as i32 }
        };
        let mut background = [0u16; 4];
        for (index, colour) in background.iter_mut().enumerate() {
            *colour = read_unsigned(2 + index * 2, 2) as u16;
        }
        let width = read_unsigned(10, field);
        let height = read_unsigned(10 + field, field);
        check_image_size(width, height)?;
        let mut canvas = PixelImage::new(width, height, Colorspace::Rgb, Chroma::C444);
        for (index, channel) in [Channel::R, Channel::G, Channel::B].into_iter().enumerate() {
            canvas.add_plane(channel, width, height, 8);
            canvas.fill_plane(channel, background[index] >> 8);
        }
        for (index, &reference) in references.iter().enumerate() {
            if reference == id {
                bail!("Self-reference in 'iovl' image item.");
            }
            let Some(referenced) = self.items.get(&reference) else {
                bail!("'iovl' image references a non-existing item.");
            };
            if let Some(error) = &referenced.error {
                bail!("{error}");
            }
            let mut overlay = self.decode_image(reference, processed)?;
            if overlay.colorspace != Colorspace::Rgb || overlay.chroma != Chroma::C444 {
                let options = super::color::ConversionOptions::decoding_defaults();
                overlay = super::color::convert_colorspace(&overlay, Colorspace::Rgb, Chroma::C444, Nclx::undefined(), 0, &options)?;
            }
            let position = 10 + 2 * field + index * 2 * field;
            let dx = read_signed(position, field);
            let dy = read_signed(position + field, field);
            overlay_onto(&mut canvas, &overlay, dx, dy);
        }
        Ok(canvas)
    }

    /// `ImageItem_Grid::decode_full_grid_image`.
    fn decode_grid(&self, rows: u32, columns: u32, width: u32, height: u32, tiles: &[u32], processed: &BTreeSet<u32>) -> Result<PixelImage> {
        for tile in tiles {
            if !self.items.contains_key(tile) {
                bail!("Tile image ID={tile} is not a proper image.");
            }
        }
        check_image_size(width, height)?;
        let mut image: Option<PixelImage> = None;
        let mut tile_width = 0;
        let mut tile_height = 0;
        let mut y0 = 0;
        let mut reference_index = 0;
        for y in 0..rows {
            let mut x0 = 0;
            for x in 0..columns {
                let tile_id = tiles[reference_index];
                let tile = &self.items[&tile_id];
                if let Some(error) = &tile.error {
                    if reference_index != 0 {
                        reference_index += 1;
                        x0 += tile_width;
                        continue;
                    }
                    bail!("{error}");
                }
                check_image_size(tile.width, tile.height)?;
                if tile.width < width / columns || tile.height < height / rows {
                    bail!("Grid tiles do not cover whole image");
                }
                if x == 0 && y == 0 {
                    tile_width = tile.width;
                    tile_height = tile.height;
                } else if tile.width != tile_width || tile.height != tile_height {
                    bail!("Grid tiles have different sizes");
                }
                // A tile that fails to decode is left out (non-strict).
                if let Ok(tile_image) = self.decode_image(tile_id, processed) {
                    if image.is_none() {
                        let mut grid = PixelImage::clone_at_size(&tile_image, width, height);
                        if grid.has_channel(Channel::Alpha) {
                            let depth = grid.bits_per_pixel(Channel::Alpha);
                            grid.fill_plane(Channel::Alpha, ((1u32 << depth) - 1) as u16);
                        }
                        grid.nclx = tile_image.nclx;
                        grid.premultiplied_alpha = tile_image.premultiplied_alpha;
                        image = Some(grid);
                    }
                    let grid = image.as_mut().expect("grid image");
                    if grid.chroma != tile_image.chroma {
                        bail!("Image tile has different chroma format than combined image");
                    }
                    grid.copy_image_to(&tile_image, x0, y0)?;
                }
                x0 += tile.width;
                reference_index += 1;
            }
            y0 += tile_height;
        }
        image.ok_or_else(|| anyhow!("Decoder plugin error"))
    }
}

/// A plane's bytes as libheif holds them (little-endian 16-bit samples
/// above 8 bits).
fn plane_bytes(plane: &super::image::Plane) -> Vec<u8> {
    if plane.bit_depth <= 8 {
        return plane.samples.iter().map(|&value| value as u8).collect();
    }
    let mut bytes = Vec::with_capacity(plane.samples.len() * 2);
    for value in &plane.samples {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// `HeifPixelImage::overlay`, its clipping and loop bounds as they are
/// (the left and top cut-offs shorten the copy twice).
fn overlay_onto(canvas: &mut PixelImage, overlay: &PixelImage, dx: i32, dy: i32) {
    let alpha = overlay.planes.get(&Channel::Alpha).map(plane_bytes);
    for (&channel, plane) in &overlay.planes {
        if !canvas.has_channel(channel) {
            continue;
        }
        let input = plane_bytes(plane);
        let in_stride = plane.width as usize * if plane.bit_depth > 8 { 2 } else { 1 };
        let mut in_w = plane.width;
        let mut in_h = plane.height;
        let out_w = canvas.channel_width(channel);
        let out_h = canvas.channel_height(channel);
        let negate = |value: i32| -> u32 { if value == i32::MIN { i32::MAX as u32 + 1 } else { (-value) as u32 } };
        if (dx > 0 && dx as u32 >= out_w) || (dx < 0 && in_w <= negate(dx)) || (dy > 0 && dy as u32 >= out_h) || (dy < 0 && in_h <= negate(dy)) {
            return;
        }
        if dx as i64 + in_w as i64 > out_w as i64 {
            in_w = (out_w as i64 - dx as i64) as u32;
        }
        if dy as i64 + in_h as i64 > out_h as i64 {
            in_h = (out_h as i64 - dy as i64) as u32;
        }
        let (in_x0, out_x0) = if dx < 0 {
            let start = negate(dx);
            in_w -= start;
            (start, 0)
        } else {
            (0, dx as u32)
        };
        let (in_y0, out_y0) = if dy < 0 {
            let start = negate(dy);
            in_h -= start;
            (start, 0)
        } else {
            (0, dy as u32)
        };
        let target = canvas.plane_mut(channel);
        let out_stride = target.stride();
        for y in in_y0..in_h {
            let out_row = (out_y0 + y - in_y0) as usize * out_stride + out_x0 as usize;
            let in_row = in_x0 as usize + y as usize * in_stride;
            match &alpha {
                None => {
                    for x in 0..in_w as usize {
                        target.samples[out_row + x] = input[in_row + x] as u16;
                    }
                }
                Some(alpha) => {
                    for x in in_x0..in_w {
                        let index = out_row + x as usize;
                        let in_value = input[in_row + x as usize] as u32;
                        let alpha_value = alpha[in_row + x as usize] as u32;
                        let out_value = target.samples[index] as u32;
                        target.samples[index] = ((in_value * alpha_value + out_value * (255 - alpha_value)) / 255) as u16 & 0xff;
                    }
                }
            }
        }
    }
}

/// The AV1 configuration's bit depth.
pub fn av1_bit_depth(av1c: &Av1C) -> i32 {
    if !av1c.high_bitdepth {
        8
    } else if av1c.twelve_bit {
        12
    } else {
        10
    }
}

/// The `clap` crop, clamped as `ImageItem::decode_image` clamps it.
fn crop_to_clap(image: &PixelImage, clap: &Clap) -> Result<PixelImage> {
    let width = image.width;
    let height = image.height;
    let mut left = clap.left_rounded(width);
    let mut right = clap.right_rounded(width);
    let mut top = clap.top_rounded(height);
    let mut bottom = clap.bottom_rounded(height);
    if left < 0 {
        left = 0;
    }
    if top < 0 {
        top = 0;
    }
    if right as u32 >= width {
        right = width as i32 - 1;
    }
    if bottom as u32 >= height {
        bottom = height as i32 - 1;
    }
    if left > right || top > bottom {
        bail!("Invalid clean aperture");
    }
    image.crop(left as u32, right as u32, top as u32, bottom as u32)
}

/// The libde265 plugin: the hvcC NAL units and the item's length-prefixed
/// ones, decoded, as a YCbCr or monochrome image with the stream's nclx.
fn decode_hevc(hvcc: &HvcC, item_data: &[u8]) -> Result<PixelImage> {
    let mut stream = Vec::new();
    for unit in &hvcc.nal_units {
        stream.extend_from_slice(&(unit.len() as u32).to_be_bytes());
        stream.extend_from_slice(unit);
    }
    stream.extend_from_slice(item_data);
    if stream.is_empty() {
        bail!("Input with empty data extent.");
    }
    let mut units: Vec<&[u8]> = Vec::new();
    let mut position = 0;
    while position < stream.len() {
        if stream.len() - position < 4 {
            bail!("End of data");
        }
        let size = u32::from_be_bytes([stream[position], stream[position + 1], stream[position + 2], stream[position + 3]]) as usize;
        position += 4;
        if size > stream.len() - position {
            bail!("End of data");
        }
        units.push(&stream[position..position + size]);
        position += size;
    }
    let picture = nerthus_hevc::decode_nal_units(&units).map_err(|_| anyhow!("Decoding the input data did not give a decompressed image."))?;
    hevc_picture_to_image(&picture)
}

/// `convert_libde265_image_to_heif_image` with the picture's colour
/// description as its nclx profile.
pub fn hevc_picture_to_image(picture: &nerthus_hevc::DecodedPicture) -> Result<PixelImage> {
    let monochrome = picture.chroma_format == 0;
    let colorspace = if monochrome { Colorspace::Monochrome } else { Colorspace::YCbCr };
    let chroma = Chroma::from_format(picture.chroma_format);
    let mut image = PixelImage::new(picture.width as u32, picture.height as u32, colorspace, chroma);
    let depth = picture.bit_depth_luma as u8;
    let plane_count = if monochrome { 1 } else { 3 };
    let channels = [Channel::Y, Channel::Cb, Channel::Cr];
    for (index, channel) in channels.iter().enumerate().take(plane_count) {
        let plane_depth = if index == 0 { picture.bit_depth_luma } else { picture.bit_depth_chroma };
        if plane_depth as u8 != depth {
            bail!("Channels with different number of bits per pixel are not supported");
        }
        let (width, height) = if index == 0 { (picture.width, picture.height) } else { (picture.chroma_width, picture.chroma_height) };
        if width == 0 || height == 0 {
            bail!("Invalid image size");
        }
        image.add_plane(*channel, width as u32, height as u32, depth);
        image.plane_mut(*channel).samples = picture.planes[index].clone();
    }
    image.nclx = Nclx {
        colour_primaries: known_colour_primaries(picture.colour_primaries as u16),
        transfer_characteristics: known_transfer_characteristics(picture.transfer_characteristics as u16),
        matrix_coefficients: known_matrix_coefficients(picture.matrix_coefficients as u16),
        full_range: picture.full_range,
    };
    Ok(image)
}
