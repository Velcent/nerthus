//! Open3DGC (`o3dgc::SC3DMCDecoder<unsigned short>`), the mesh compression
//! glTF 1's "Open3DGC-compression" extension uses: a header of counts and
//! ranges, then the triangles (as fans), coordinates, normals and float
//! attributes, each a block of prediction residuals over quantized values.
//! Everything is written into the decoded data at the offsets the importer
//! picks, which can overlap. The C++'s `long` is 32-bit (Windows), and its
//! arrays are read and written unchecked: past an array is an error here,
//! values never written are carried along as unset until they decide
//! something.

mod codec;
mod connectivity;
mod stream;

use std::ops::Range;

use codec::{AdaptiveBitModel, AdaptiveDataModel, Decoder, StaticBitModel, decode_int_egc, decode_uint_egc, uint_to_int, undefined};
use connectivity::TriangleListDecoder;
use stream::{Stream, StreamType};

use crate::scene::{self, ImportError, check_allocation};

/// `O3DGC_SC3DMC_START_CODE`.
const START_CODE: u32 = 0x1F1;

/// `O3DGC_SC3DMC_MAX_NUM_FLOAT_ATTRIBUTES` (and int), and
/// `O3DGC_SC3DMC_MAX_DIM_ATTRIBUTES`.
const MAX_ATTRIBUTES: usize = 256;
const MAX_DIMENSIONS: usize = 32;

/// `O3DGC_SC3DMC_MAX_PREDICTION_NEIGHBORS`.
const MAX_NEIGHBORS: usize = 2;

/// `O3DGCSC3DMCBinarization`'s arithmetic-coded and ASCII forms.
const BINARIZATION_AC_EGC: u8 = 4;
const BINARIZATION_ASCII: u8 = 5;

/// `O3DGCSC3DMCPredictionMode`s.
const NO_PREDICTION: u8 = 0;
const DIFFERENTIAL_PREDICTION: u8 = 1;
const PARALLELOGRAM_PREDICTION: u8 = 5;
const SURF_NORMALS_PREDICTION: u8 = 6;

/// `sizeof(long)` on Windows, and `sizeof(float)`.
const LONG_SIZE: u64 = 4;
const FLOAT_SIZE: u64 = 4;

/// `O3DGC_DEFAULT_VECTOR_SIZE`: a `Vector`'s first allocation.
const DEFAULT_VECTOR_SIZE: usize = 32;

/// The decoded data (`new uint8_t[n]`): arrays are written at byte
/// offsets, which must lie in it, and bytes never written are garbage.
pub(crate) struct Memory {
    bytes: Vec<u8>,
    written: Vec<bool>,
}

impl Memory {
    pub fn new(size: usize) -> Self {
        Memory { bytes: vec![0; size], written: vec![false; size] }
    }

    /// The bytes of element `index` of `size` bytes past `base`.
    fn range(&self, base: Option<usize>, index: i64, size: usize) -> Result<Range<usize>, ImportError> {
        let Some(base) = base else {
            return undefined("Open3DGC data reached through a null pointer");
        };
        let offset = index.checked_mul(size as i64).and_then(|offset| (base as i64).checked_add(offset));
        match offset {
            Some(start) if start >= 0 && start as usize + size <= self.bytes.len() => Ok(start as usize..start as usize + size),
            _ => undefined("Open3DGC data reached outside the decoded buffer"),
        }
    }

    /// The bytes, unless any was never written.
    fn load(&self, range: Range<usize>) -> Option<&[u8]> {
        let written = self.written[range.clone()].iter().all(|&written| written);
        written.then(|| &self.bytes[range])
    }

    /// The bytes written, or garbage (unset).
    fn store(&mut self, range: Range<usize>, bytes: Option<&[u8]>) {
        match bytes {
            Some(bytes) => {
                self.bytes[range.clone()].copy_from_slice(bytes);
                self.written[range].fill(true);
            }
            None => self.written[range].fill(false),
        }
    }

    pub fn read_u16(&self, base: Option<usize>, index: i64) -> Result<Option<u16>, ImportError> {
        let range = self.range(base, index, 2)?;
        let value = self.load(range).map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        Ok(value)
    }

    pub fn write_u16(&mut self, base: Option<usize>, index: i64, value: Option<u16>) -> Result<(), ImportError> {
        let range = self.range(base, index, 2)?;
        let bytes = value.map(u16::to_le_bytes);
        self.store(range, bytes.as_ref().map(|bytes| bytes.as_slice()));
        Ok(())
    }

    fn read_f32(&self, base: Option<usize>, index: i64) -> Result<Option<f32>, ImportError> {
        let range = self.range(base, index, 4)?;
        let value = self.load(range).map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        Ok(value)
    }

    fn write_f32(&mut self, base: Option<usize>, index: i64, value: Option<f32>) -> Result<(), ImportError> {
        let range = self.range(base, index, 4)?;
        let bytes = value.map(f32::to_le_bytes);
        self.store(range, bytes.as_ref().map(|bytes| bytes.as_slice()));
        Ok(())
    }

    /// The importer's read of `length` bytes from `start` (past the buffer
    /// is undefined), and whether the decoder wrote them all: those it
    /// did not are garbage.
    pub fn read(&self, start: usize, length: usize) -> Result<(&[u8], bool), ImportError> {
        let end = start.checked_add(length).filter(|&end| end <= self.bytes.len());
        let Some(end) = end else {
            return undefined("decoded Open3DGC data read past its end");
        };
        let written = self.written[start..end].iter().all(|&written| written);
        Ok((&self.bytes[start..end], written))
    }
}

/// A float attribute's header.
#[derive(Clone, Copy, Default)]
pub(crate) struct FloatAttribute {
    pub count: u32,
    pub dimension: u32,
    pub kind: u8,
}

/// An int attribute's header.
#[derive(Clone, Copy, Default)]
pub(crate) struct IntAttribute {
    pub count: u32,
    pub dimension: u32,
    pub kind: u8,
}

/// `IndexedFaceSet` and the quantization bits, as the header sets them.
pub(crate) struct Header {
    pub coord_index_count: u32,
    pub coord_count: u32,
    pub normal_count: u32,
    pub float_attributes: Vec<FloatAttribute>,
    pub int_attributes: Vec<IntAttribute>,
    coord_min: [f32; 3],
    coord_max: [f32; 3],
    normal_min: [f32; 3],
    normal_max: [f32; 3],
    /// `m_minFloatAttribute` and `m_maxFloatAttribute`: 32 slots each.
    attribute_min: Vec<f32>,
    attribute_max: Vec<f32>,
    coord_bits: u32,
    normal_bits: u32,
    attribute_bits: Vec<u32>,
}

/// Where the arrays go in the decoded data (`SetCoordIndex`, `SetCoord`,
/// …): byte offsets, or none (a null pointer).
pub(crate) struct Layout {
    pub coord_index: Option<usize>,
    pub coord: Option<usize>,
    pub normal: Option<usize>,
    pub float_attributes: Vec<Option<usize>>,
}

/// `SC3DMCPredictor`: a neighbour's id triplet and prediction (garbage
/// components unset).
#[derive(Clone, Copy)]
struct Predictor {
    id: [i32; 3],
    prediction: [Option<i32>; MAX_DIMENSIONS],
}

/// `operator<` of `SC3DMCTriplet`: by c, then b, then a.
fn triplet_less(left: &[i32; 3], right: &[i32; 3]) -> bool {
    if left[2] != right[2] {
        return left[2] < right[2];
    }
    if left[1] != right[1] {
        return left[1] < right[1];
    }
    left[0] < right[0]
}

/// `Insert`: the id into the sorted list of at most two; its slot, or
/// none when it is there already or sorts past a full list. A slot keeps
/// its old prediction until the caller writes it.
fn insert(id: [i32; 3], count: &mut usize, list: &mut [Option<Predictor>; MAX_NEIGHBORS]) -> Option<usize> {
    for j in 0..*count {
        let present = list[j].map(|entry| entry.id).unwrap_or([0; 3]);
        if id == present {
            return None;
        }
        if triplet_less(&id, &present) {
            if *count < MAX_NEIGHBORS {
                *count += 1;
            }
            let mut h = *count - 1;
            while h > j {
                list[h] = list[h - 1];
                h -= 1;
            }
            let prediction = list[j].map(|entry| entry.prediction).unwrap_or([None; MAX_DIMENSIONS]);
            list[j] = Some(Predictor { id, prediction });
            return Some(j);
        }
    }
    if *count < MAX_NEIGHBORS {
        let slot = *count;
        let prediction = list[slot].map(|entry| entry.prediction).unwrap_or([None; MAX_DIMENSIONS]);
        list[slot] = Some(Predictor { id, prediction });
        *count += 1;
        return Some(slot);
    }
    None
}

fn set_prediction(neighbors: &mut [Option<Predictor>; MAX_NEIGHBORS], position: usize, component: usize, value: Option<i32>) -> Result<(), ImportError> {
    let Some(Some(entry)) = neighbors.get_mut(position) else {
        return undefined("an Open3DGC prediction written past its neighbours");
    };
    let Some(slot) = entry.prediction.get_mut(component) else {
        return undefined("an Open3DGC prediction written past its components");
    };
    *slot = value;
    Ok(())
}

/// `SphereToCube` on `long` normals: the face of the cube (by the largest
/// component) and the other two components, signed for it.
fn sphere_to_cube(x: i32, y: i32, z: i32) -> (i32, i32, i8) {
    let ax = x.wrapping_abs();
    let ay = y.wrapping_abs();
    let az = z.wrapping_abs();
    if az >= ax && az >= ay {
        if z >= 0 { (x, y, 0) } else { (x.wrapping_neg(), y.wrapping_neg(), 1) }
    } else if ay >= ax && ay >= az {
        if y >= 0 { (z, x, 2) } else { (z.wrapping_neg(), x.wrapping_neg(), 3) }
    } else if x >= 0 {
        (y, z, 4)
    } else {
        (y.wrapping_neg(), z.wrapping_neg(), 5)
    }
}

/// `sqrt(max(0.0, 1.0 - u*u - v*v))` (the squares in float) as a float.
fn remaining(u: f32, v: f32) -> f32 {
    let value = 1.0 - (u * u) as f64 - (v * v) as f64;
    let clamped = if value > 0.0 { value } else { 0.0 };
    clamped.sqrt() as f32
}

/// `CubeToSphere`: the face and two components back to a unit normal
/// (an orientation past 5 leaves the three values as they were).
fn cube_to_sphere(a: f32, b: f32, index: i8) -> Option<[f32; 3]> {
    let normal = match index {
        0 => [a, b, remaining(a, b)],
        1 => [-a, -b, -remaining(-a, -b)],
        2 => [b, remaining(b, a), a],
        3 => [-b, -remaining(-b, -a), -a],
        4 => [remaining(a, b), a, b],
        5 => [-remaining(-a, -b), -a, -b],
        _ => return None,
    };
    Some(normal)
}

fn cross(u: [i32; 3], w: [i32; 3]) -> [i32; 3] {
    [
        u[1].wrapping_mul(w[2]).wrapping_sub(u[2].wrapping_mul(w[1])),
        u[2].wrapping_mul(w[0]).wrapping_sub(u[0].wrapping_mul(w[2])),
        u[0].wrapping_mul(w[1]).wrapping_sub(u[1].wrapping_mul(w[0])),
    ]
}

fn difference(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0].wrapping_sub(b[0]), a[1].wrapping_sub(b[1]), a[2].wrapping_sub(b[2])]
}

/// `Vector<char>` (the normals' cube orientations): its buffer, with
/// slots never written unset, and its size.
#[derive(Default)]
struct OrientationVector {
    buffer: Vec<Option<i8>>,
    size: usize,
}

impl OrientationVector {
    /// A larger buffer, keeping the first `size` values.
    fn grow(&mut self, allocated: usize) -> Result<(), ImportError> {
        check_allocation(allocated as u64, 1)?;
        let mut buffer = vec![None; allocated];
        buffer[..self.size].copy_from_slice(&self.buffer[..self.size]);
        self.buffer = buffer;
        Ok(())
    }

    /// `Allocate`.
    fn allocate(&mut self, size: u32) -> Result<(), ImportError> {
        if size as usize > self.buffer.len() {
            self.grow(size as usize)?;
        }
        Ok(())
    }

    /// `PushBack`: a full buffer doubles (to at least 32).
    fn push(&mut self, value: i8) -> Result<(), ImportError> {
        if self.size == self.buffer.len() {
            let allocated = (self.buffer.len() * 2).max(DEFAULT_VECTOR_SIZE);
            self.grow(allocated)?;
        }
        self.buffer[self.size] = Some(value);
        self.size += 1;
        Ok(())
    }

    /// `operator[]`, which reads on past the size into the buffer.
    fn get(&self, index: usize) -> Result<Option<i8>, ImportError> {
        match self.buffer.get(index) {
            Some(&value) => Ok(value),
            None => undefined("an Open3DGC normal orientation read past its array"),
        }
    }

    fn set(&mut self, index: usize, value: Option<i8>) -> Result<(), ImportError> {
        let Some(slot) = self.buffer.get_mut(index) else {
            return undefined("an Open3DGC normal orientation written past its array");
        };
        *slot = value;
        Ok(())
    }
}

/// One float array's place, shape and range.
struct FloatArray<'r> {
    target: Option<usize>,
    count: u32,
    dimension: u32,
    stride: u32,
    minimum: &'r [f32],
    maximum: &'r [f32],
    bits: u32,
}

/// A block's decoder and models (binary streams).
struct BlockDecoder<'a> {
    decoder: Decoder<'a>,
    static_model: StaticBitModel,
    adaptive_model: AdaptiveBitModel,
    predictions: AdaptiveDataModel,
}

/// A block being read: its decoder (none for ASCII), the values model and
/// Exp-Golomb parameters, its prediction mode, and where its predictors
/// (ASCII) and the next block start.
struct Block<'a> {
    decoder: Option<BlockDecoder<'a>>,
    values: AdaptiveDataModel,
    exp_k: u32,
    m: u32,
    prediction: u8,
    predictors: u32,
}

/// `SC3DMCDecoder<unsigned short>`.
pub(crate) struct SC3DMCDecoder<'a> {
    stream: Stream<'a>,
    iterator: u32,
    stream_type: StreamType,
    header: Header,
    triangles: TriangleListDecoder,
    /// `m_quantFloatArray`: slots never written, or made from garbage,
    /// are unset.
    quantized: Vec<Option<i32>>,
    orientation: OrientationVector,
    /// `m_normals`: the predicted normals' cube components.
    normals: Vec<Option<f32>>,
}

impl<'a> SC3DMCDecoder<'a> {
    /// `DecodeHeader`: none when the start code is neither binary nor
    /// ASCII.
    pub fn decode_header(bytes: &'a [u8]) -> Result<Option<Self>, ImportError> {
        let stream = Stream::new(bytes);
        let mut iterator = 0u32;
        let stream_type = if stream.read_u32_binary(&mut iterator)? == START_CODE {
            StreamType::Binary
        } else {
            iterator = 0;
            if stream.read_u32_ascii(&mut iterator)? != START_CODE {
                return Ok(None);
            }
            StreamType::Ascii
        };
        let kind = stream_type;
        // The stream size, encode mode, crease angle and mesh flags.
        stream.read_u32(&mut iterator, kind)?;
        stream.read_uchar(&mut iterator)?;
        stream.read_f32(&mut iterator, kind)?;
        stream.read_uchar(&mut iterator)?;
        let coord_count = stream.read_u32(&mut iterator, kind)?;
        let normal_count = stream.read_u32(&mut iterator, kind)?;
        let mut header = Header {
            coord_index_count: 0,
            coord_count,
            normal_count,
            float_attributes: Vec::new(),
            int_attributes: Vec::new(),
            coord_min: [0.0; 3],
            coord_max: [0.0; 3],
            normal_min: [0.0; 3],
            normal_max: [0.0; 3],
            attribute_min: vec![0.0; MAX_ATTRIBUTES * MAX_DIMENSIONS],
            attribute_max: vec![0.0; MAX_ATTRIBUTES * MAX_DIMENSIONS],
            coord_bits: 0,
            normal_bits: 0,
            attribute_bits: vec![0; MAX_ATTRIBUTES],
        };
        let float_count = stream.read_u32(&mut iterator, kind)?;
        let float_count_fits = (float_count as usize) < MAX_ATTRIBUTES;
        scene::ai_assert(float_count_fits, "numFloatAttributes < O3DGC_SC3DMC_MAX_NUM_FLOAT_ATTRIBUTES")?;
        let int_count = stream.read_u32(&mut iterator, kind)?;
        let int_count_fits = (int_count as usize) < MAX_ATTRIBUTES;
        scene::ai_assert(int_count_fits, "numIntAttributes < O3DGC_SC3DMC_MAX_NUM_INT_ATTRIBUTES")?;
        if header.coord_count > 0 {
            header.coord_index_count = stream.read_u32(&mut iterator, kind)?;
            for axis in 0..3 {
                header.coord_min[axis] = stream.read_f32(&mut iterator, kind)?;
                header.coord_max[axis] = stream.read_f32(&mut iterator, kind)?;
            }
            header.coord_bits = stream.read_uchar(&mut iterator)? as u32;
        }
        if header.normal_count > 0 {
            // The normal index count, then the ranges, per-vertex flag
            // and bits.
            stream.read_u32(&mut iterator, kind)?;
            for axis in 0..3 {
                header.normal_min[axis] = stream.read_f32(&mut iterator, kind)?;
                header.normal_max[axis] = stream.read_f32(&mut iterator, kind)?;
            }
            stream.read_uchar(&mut iterator)?;
            header.normal_bits = stream.read_uchar(&mut iterator)? as u32;
        }
        for attribute in 0..float_count as usize {
            let count = stream.read_u32(&mut iterator, kind)?;
            let mut entry = FloatAttribute { count, ..FloatAttribute::default() };
            if count > 0 {
                stream.read_u32(&mut iterator, kind)?;
                let dimension = stream.read_uchar(&mut iterator)?;
                entry.dimension = dimension as u32;
                for component in 0..dimension as usize {
                    // Each range is read, then its setter asserts the
                    // component.
                    let minimum = stream.read_f32(&mut iterator, kind)?;
                    scene::ai_assert(component < MAX_DIMENSIONS, "dim < O3DGC_SC3DMC_MAX_DIM_ATTRIBUTES")?;
                    let slot = attribute * MAX_DIMENSIONS + component;
                    header.attribute_min[slot] = minimum;
                    header.attribute_max[slot] = stream.read_f32(&mut iterator, kind)?;
                }
                stream.read_uchar(&mut iterator)?;
                entry.kind = stream.read_uchar(&mut iterator)?;
                header.attribute_bits[attribute] = stream.read_uchar(&mut iterator)? as u32;
            }
            header.float_attributes.push(entry);
        }
        for _ in 0..int_count {
            let count = stream.read_u32(&mut iterator, kind)?;
            let mut entry = IntAttribute { count, ..IntAttribute::default() };
            if count > 0 {
                stream.read_u32(&mut iterator, kind)?;
                entry.dimension = stream.read_uchar(&mut iterator)? as u32;
                stream.read_uchar(&mut iterator)?;
                entry.kind = stream.read_uchar(&mut iterator)?;
            }
            header.int_attributes.push(entry);
        }
        let decoder = SC3DMCDecoder {
            stream,
            iterator,
            stream_type,
            header,
            triangles: TriangleListDecoder::new(stream_type),
            quantized: Vec::new(),
            orientation: OrientationVector::default(),
            normals: Vec::new(),
        };
        Ok(Some(decoder))
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// `SetNFloatAttribute(a, 0)`: an attribute left out (its block is
    /// still read).
    pub fn disable_float_attribute(&mut self, attribute: usize) {
        self.header.float_attributes[attribute].count = 0;
    }

    /// `DecodePayload`: the triangles (whose result is ignored), the
    /// coordinates (a corrupt block is the result), the normals and
    /// float attributes (corrupt blocks ignored), the int attributes,
    /// then the triangle order.
    pub fn decode_payload(&mut self, memory: &mut Memory, layout: &Layout) -> Result<bool, ImportError> {
        let coord_index_count = self.header.coord_index_count as i32;
        let coord_count = self.header.coord_count as i32;
        let mut iterator = self.iterator;
        self.triangles.decode(memory, layout.coord_index, coord_index_count, coord_count, &self.stream, &mut iterator)?;
        self.iterator = iterator;
        if self.header.coord_count > 0 {
            let minimum = self.header.coord_min;
            let maximum = self.header.coord_max;
            let array = FloatArray {
                target: layout.coord,
                count: self.header.coord_count,
                dimension: 3,
                stride: 3,
                minimum: &minimum,
                maximum: &maximum,
                bits: self.header.coord_bits,
            };
            if !self.decode_float_array(memory, &array)? {
                return Ok(false);
            }
        }
        if self.header.normal_count > 0 {
            let minimum = self.header.normal_min;
            let maximum = self.header.normal_max;
            let array = FloatArray {
                target: layout.normal,
                count: self.header.normal_count,
                dimension: 3,
                stride: 3,
                minimum: &minimum,
                maximum: &maximum,
                bits: self.header.normal_bits,
            };
            self.decode_float_array(memory, &array)?;
        }
        for attribute in 0..self.header.float_attributes.len() {
            let entry = self.header.float_attributes[attribute];
            let first = attribute * MAX_DIMENSIONS;
            let minimum = self.header.attribute_min[first..].to_vec();
            let maximum = self.header.attribute_max[first..].to_vec();
            let array = FloatArray {
                target: layout.float_attributes.get(attribute).copied().flatten(),
                count: entry.count,
                dimension: entry.dimension,
                stride: entry.dimension,
                minimum: &minimum,
                maximum: &maximum,
                bits: self.header.attribute_bits[attribute],
            };
            self.decode_float_array(memory, &array)?;
        }
        for attribute in 0..self.header.int_attributes.len() {
            let entry = self.header.int_attributes[attribute];
            self.decode_int_array(memory, entry.count, entry.dimension)?;
        }
        self.triangles.reorder(memory)?;
        Ok(true)
    }

    /// A block's start: its size, binarization and prediction mode, then
    /// (binary) the decoder and its Exp-Golomb parameters, or (ASCII) the
    /// predictors' position. `None` when the binarization does not match
    /// the stream.
    fn start_block(&mut self) -> Result<Option<Block<'a>>, ImportError> {
        let start = self.iterator;
        let declared_size = self.stream.read_u32(&mut self.iterator, self.stream_type)?;
        let mask = self.stream.read_uchar(&mut self.iterator)?;
        let binarization = (mask >> 4) & 7;
        let prediction = mask & 7;
        let stream_size = declared_size.wrapping_sub(self.iterator.wrapping_sub(start));
        let mut predictors = self.iterator.wrapping_add(stream_size);
        if self.stream_type == StreamType::Ascii {
            if binarization != BINARIZATION_ASCII {
                return Ok(None);
            }
            // The predictors' size.
            self.stream.read_u32(&mut predictors, self.stream_type)?;
            let values = AdaptiveDataModel::new(2)?;
            let block = Block { decoder: None, values, exp_k: 0, m: 0, prediction, predictors };
            return Ok(Some(block));
        }
        if binarization != BINARIZATION_AC_EGC {
            return Ok(None);
        }
        let position = self.iterator as usize;
        self.iterator = self.iterator.wrapping_add(stream_size);
        let mut decoder = Decoder::start(self.stream.bytes(), position, stream_size)?;
        let static_model = StaticBitModel::new();
        let mut adaptive_model = AdaptiveBitModel::new();
        let exp_k = decoder.exp_golomb(0, &static_model, &mut adaptive_model)?;
        let m = decoder.exp_golomb(0, &static_model, &mut adaptive_model)?;
        let values = AdaptiveDataModel::new(m.wrapping_add(2))?;
        let predictions = AdaptiveDataModel::new(MAX_NEIGHBORS as u32 + 1)?;
        let coder = BlockDecoder { decoder, static_model, adaptive_model, predictions };
        let block = Block { decoder: Some(coder), values, exp_k, m, prediction, predictors };
        Ok(Some(block))
    }

    /// A signed residual (`ReadIntASCII` or `DecodeIntACEGC`).
    fn signed_residual(&mut self, block: &mut Block) -> Result<i32, ImportError> {
        match block.decoder.as_mut() {
            None => self.stream.read_int_ascii(&mut self.iterator),
            Some(coder) => decode_int_egc(&mut coder.decoder, &mut block.values, &coder.static_model, &mut coder.adaptive_model, block.exp_k, block.m),
        }
    }

    /// An unsigned value (`ReadUIntASCII` or `DecodeUIntACEGC`).
    fn unsigned_value(&mut self, block: &mut Block) -> Result<u32, ImportError> {
        match block.decoder.as_mut() {
            None => self.stream.read_uint_ascii(&mut self.iterator),
            Some(coder) => decode_uint_egc(&mut coder.decoder, &mut block.values, &coder.static_model, &mut coder.adaptive_model, block.exp_k, block.m),
        }
    }

    /// The predictor picked from two (`ReadUCharASCII` at the predictors'
    /// position, or the predictions model).
    fn best_predictor(&mut self, block: &mut Block) -> Result<u32, ImportError> {
        match block.decoder.as_mut() {
            None => Ok(self.stream.read_uchar(&mut block.predictors)? as u32),
            Some(coder) => coder.decoder.decode_data(&mut coder.predictions),
        }
    }

    /// A quantized value (unset when garbage).
    fn quantized_at(&self, index: u32) -> Result<Option<i32>, ImportError> {
        match self.quantized.get(index as usize) {
            Some(&value) => Ok(value),
            None => undefined("an Open3DGC quantized value read past its array"),
        }
    }

    fn set_quantized(&mut self, index: u32, value: Option<i32>) -> Result<(), ImportError> {
        let Some(slot) = self.quantized.get_mut(index as usize) else {
            return undefined("an Open3DGC quantized value written past its array");
        };
        *slot = value;
        Ok(())
    }

    /// The neighbours' predictions of vertex `v` (parallelograms across
    /// its triangles' far edges, then its triangles' earlier vertices).
    fn predictions(
        &self,
        memory: &Memory,
        v: i32,
        dimension: u32,
        stride: u32,
        prediction: u8,
        neighbors: &mut [Option<Predictor>; MAX_NEIGHBORS],
    ) -> Result<usize, ImportError> {
        let mut count = 0usize;
        let adjacency = self.triangles.adjacency();
        let has_neighbors = adjacency.count(v)? > 0;
        if !has_neighbors || prediction == NO_PREDICTION {
            return Ok(0);
        }
        let begin = adjacency.begin(v)?;
        let end = adjacency.end(v)?;
        for slot in begin..end {
            let triangle = adjacency.neighbor(slot)?;
            if triangle < 0 {
                break;
            }
            let first = triangle as i64 * 3;
            if prediction == PARALLELOGRAM_PREDICTION {
                let (a, b) = if self.triangles.corner(memory, first)? == v {
                    (self.triangles.corner(memory, first + 1)?, self.triangles.corner(memory, first + 2)?)
                } else if self.triangles.corner(memory, first + 1)? == v {
                    (self.triangles.corner(memory, first)?, self.triangles.corner(memory, first + 2)?)
                } else {
                    (self.triangles.corner(memory, first)?, self.triangles.corner(memory, first + 1)?)
                };
                if a < v && b < v {
                    let a_begin = adjacency.begin(a)?;
                    let a_end = adjacency.end(a)?;
                    for a_slot in a_begin..a_end {
                        let other = adjacency.neighbor(a_slot)?;
                        if other < 0 {
                            break;
                        }
                        let mut c = -1;
                        let mut found_b = false;
                        for corner in 0..3 {
                            let x = self.triangles.corner(memory, other as i64 * 3 + corner)?;
                            if x == b {
                                found_b = true;
                            }
                            if x < v && x != a && x != b {
                                c = x;
                            }
                        }
                        if c == -1 || !found_b {
                            continue;
                        }
                        let id = [a.min(b), a.max(b), c.wrapping_neg().wrapping_sub(1)];
                        let Some(position) = insert(id, &mut count, neighbors) else {
                            continue;
                        };
                        for component in 0..dimension {
                            let at = |vertex: i32| (vertex as u32).wrapping_mul(stride).wrapping_add(component);
                            let a_value = self.quantized_at(at(a))?;
                            let b_value = self.quantized_at(at(b))?;
                            let c_value = self.quantized_at(at(c))?;
                            let value = a_value.zip(b_value).zip(c_value).map(|((a, b), c)| a.wrapping_add(b).wrapping_sub(c));
                            set_prediction(neighbors, position, component as usize, value)?;
                        }
                    }
                }
            }
            if matches!(prediction, SURF_NORMALS_PREDICTION | PARALLELOGRAM_PREDICTION | DIFFERENTIAL_PREDICTION) {
                for corner in 0..3 {
                    let w = self.triangles.corner(memory, first + corner)?;
                    if w >= v {
                        continue;
                    }
                    let Some(position) = insert([-1, -1, w], &mut count, neighbors) else {
                        continue;
                    };
                    for component in 0..dimension {
                        let index = (w as u32).wrapping_mul(stride).wrapping_add(component);
                        let value = self.quantized_at(index)?;
                        set_prediction(neighbors, position, component as usize, value)?;
                    }
                }
            }
        }
        Ok(count)
    }

    /// `DecodeFloatArray`: quantized values from their predictions and
    /// residuals, then back to floats (normals through the cube faces).
    fn decode_float_array(&mut self, memory: &mut Memory, array: &FloatArray) -> Result<bool, ImportError> {
        let count = array.count;
        let stride = array.stride;
        let size = count.wrapping_mul(array.dimension);
        let dimension_fits = (array.dimension as usize) < MAX_DIMENSIONS;
        scene::ai_assert(dimension_fits, "dimFloatArray < O3DGC_SC3DMC_MAX_DIM_ATTRIBUTES")?;
        let Some(mut block) = self.start_block()? else {
            return Ok(false);
        };
        let mut dimension = array.dimension;
        if block.prediction == SURF_NORMALS_PREDICTION {
            self.orientation.allocate(size)?;
            self.orientation.size = 0;
            match block.decoder.as_mut() {
                None => {
                    for _ in 0..count {
                        let value = self.stream.read_int_ascii(&mut self.iterator)?;
                        self.orientation.push(value as u8 as i8)?;
                    }
                }
                Some(coder) => {
                    let mut model = AdaptiveDataModel::new(12)?;
                    for _ in 0..count {
                        let value = coder.decoder.decode_data(&mut model)?;
                        self.orientation.push(uint_to_int(value) as u8 as i8)?;
                    }
                }
            }
            self.process_normals(memory)?;
            dimension = 2;
        }
        if self.quantized.len() < size as usize {
            check_allocation(size as u64, LONG_SIZE)?;
            self.quantized = vec![None; size as usize];
        }
        let mut neighbors: [Option<Predictor>; MAX_NEIGHBORS] = [None; MAX_NEIGHBORS];
        for v in 0..count as i32 {
            let predicted = self.predictions(memory, v, dimension, stride, block.prediction, &mut neighbors)?;
            let at = |vertex: i32, component: u32| (vertex as u32).wrapping_mul(stride).wrapping_add(component);
            if predicted > 1 {
                let best = self.best_predictor(&mut block)? as usize;
                for component in 0..dimension {
                    let residual = self.signed_residual(&mut block)?;
                    // Past the two neighbours (or their 32 components) the
                    // C++ reads on through its stack: garbage.
                    let neighbor = neighbors.get(best).copied().flatten();
                    let prediction = neighbor.and_then(|neighbor| neighbor.prediction.get(component as usize).copied().flatten());
                    self.set_quantized(at(v, component), prediction.map(|prediction| residual.wrapping_add(prediction)))?;
                }
            } else if v > 0 && block.prediction != NO_PREDICTION {
                for component in 0..dimension {
                    let residual = self.signed_residual(&mut block)?;
                    let previous = self.quantized_at(at(v - 1, component))?;
                    self.set_quantized(at(v, component), previous.map(|previous| residual.wrapping_add(previous)))?;
                }
            } else {
                for component in 0..dimension {
                    let value = self.unsigned_value(&mut block)? as i32;
                    self.set_quantized(at(v, component), Some(value))?;
                }
            }
        }
        self.iterator = block.predictors;
        if block.prediction != SURF_NORMALS_PREDICTION {
            self.inverse_quantize(memory, array, dimension, array.minimum, array.maximum, array.bits)?;
            return Ok(true);
        }
        let normal_min = [-2.0f32; 2];
        let normal_max = [2.0f32; 2];
        self.inverse_quantize(memory, array, dimension, &normal_min, &normal_max, array.bits.wrapping_add(1))?;
        for v in 0..count as i32 {
            let slot = 2 * v as usize;
            let (Some(&na0), Some(&nb0)) = (self.normals.get(slot), self.normals.get(slot + 1)) else {
                return undefined("an Open3DGC predicted normal read past its array");
            };
            let first = (v as u32).wrapping_mul(stride);
            let a = memory.read_f32(array.target, first as i64)?;
            let b = memory.read_f32(array.target, first.wrapping_add(1) as i64)?;
            let index = self.orientation.get(v as usize)?;
            let normal = match (index, a.zip(na0), b.zip(nb0)) {
                // Past the six faces nothing is written.
                (Some(index), _, _) if !(0..=5).contains(&index) => continue,
                (Some(index), Some((a, na0)), Some((b, nb0))) => cube_to_sphere(a + na0, b + nb0, index),
                // Garbage in: the three components come out garbage.
                _ => None,
            };
            for axis in 0..3 {
                let component = normal.map(|normal| normal[axis]);
                memory.write_f32(array.target, first.wrapping_add(axis as u32) as i64, component)?;
            }
        }
        Ok(true)
    }

    /// `ProcessNormals` (over the header's normal count, whatever array
    /// is being read): each vertex's summed quantized triangle normal on
    /// its cube face, with the coded orientation changes applied. Unset
    /// values make unset results.
    fn process_normals(&mut self, memory: &Memory) -> Result<(), ImportError> {
        let vertex_count = self.header.normal_count as i32;
        let size = self.header.normal_count.wrapping_mul(2) as usize;
        if self.normals.len() < size {
            check_allocation(size as u64, FLOAT_SIZE)?;
            self.normals = vec![None; size];
        }
        for v in 0..vertex_count {
            let adjacency = self.triangles.adjacency();
            let begin = adjacency.begin(v)?;
            let end = adjacency.end(v)?;
            let mut sum = Some([0i32; 3]);
            for slot in begin..end {
                let triangle = adjacency.neighbor(slot)?;
                if triangle == -1 {
                    break;
                }
                let first = triangle as i64 * 3;
                let mut corners = [0i32; 3];
                for (corner, vertex) in corners.iter_mut().enumerate() {
                    *vertex = self.triangles.corner(memory, first + corner as i64)?;
                }
                let mut points = [Some([0i32; 3]); 3];
                for (point, &vertex) in points.iter_mut().zip(&corners) {
                    for axis in 0..3 {
                        let index = vertex.wrapping_mul(3).wrapping_add(axis as i32);
                        let value = match usize::try_from(index).ok().and_then(|index| self.quantized.get(index)) {
                            Some(&value) => value,
                            None => return undefined("an Open3DGC quantized coordinate read past its array"),
                        };
                        *point = point.zip(value).map(|(mut point, value)| {
                            point[axis] = value;
                            point
                        });
                    }
                }
                let [Some(p1), Some(p2), Some(p3)] = points else {
                    sum = None;
                    continue;
                };
                let normal = cross(difference(p2, p1), difference(p3, p1));
                sum = sum.map(|sum| [sum[0].wrapping_add(normal[0]), sum[1].wrapping_add(normal[1]), sum[2].wrapping_add(normal[2])]);
            }
            let orientation = self.orientation.get(v as usize)?;
            let result = sum.zip(orientation).map(|(sum, orientation)| {
                let squares: f64 = sum.iter().map(|&component| component as f64 * component as f64).sum();
                let mut norm = squares.sqrt() as f32;
                if norm == 0.0 {
                    norm = 1.0;
                }
                let (na0, nb0, ni0) = sphere_to_cube(sum[0], sum[1], sum[2]);
                let ni1 = ni0.wrapping_add(orientation);
                if (ni1 >> 1) != (ni0 >> 1) {
                    return (0.0, 0.0, ni1);
                }
                (na0 as f32 / norm, nb0 as f32 / norm, ni1)
            });
            self.orientation.set(v as usize, result.map(|result| result.2))?;
            let slot = 2 * v as usize;
            self.normals[slot] = result.map(|result| result.0);
            self.normals[slot + 1] = result.map(|result| result.1);
        }
        Ok(())
    }

    /// `IQuantizeFloatArray`: `min + q × range / (2^bits − 1)` (a shift of
    /// 32 or more is masked, as x86 does).
    fn inverse_quantize(&self, memory: &mut Memory, array: &FloatArray, dimension: u32, minimum: &[f32], maximum: &[f32], bits: u32) -> Result<(), ImportError> {
        let mut step = [0.0f32; MAX_DIMENSIONS];
        let mut low = [0.0f32; MAX_DIMENSIONS];
        for component in 0..dimension as usize {
            if component >= MAX_DIMENSIONS {
                return undefined("an Open3DGC attribute wider than its step array");
            }
            let (Some(&minimum), Some(&maximum)) = (minimum.get(component), maximum.get(component)) else {
                return undefined("an Open3DGC range read past its array");
            };
            let range = maximum - minimum;
            let levels = 1i32.wrapping_shl(bits).wrapping_sub(1);
            step[component] = if range > 0.0 { range / levels as f32 } else { 1.0 };
            low[component] = minimum;
        }
        for v in 0..array.count {
            for component in 0..dimension {
                let index = v.wrapping_mul(array.stride).wrapping_add(component);
                let quantized = self.quantized_at(index)?;
                let value = quantized.map(|quantized| quantized as f32 * step[component as usize] + low[component as usize]);
                memory.write_f32(array.target, index as i64, value)?;
            }
        }
        Ok(())
    }

    /// `DecodeIntArray`: the block is read, but values go through the int
    /// attribute's pointer, which the importer never sets; without
    /// components only the predictor choices are read.
    fn decode_int_array(&mut self, memory: &Memory, count: u32, dimension: u32) -> Result<(), ImportError> {
        let dimension_fits = (dimension as usize) < MAX_DIMENSIONS;
        scene::ai_assert(dimension_fits, "dimIntArray < O3DGC_SC3DMC_MAX_DIM_ATTRIBUTES")?;
        let Some(mut block) = self.start_block()? else {
            return Ok(());
        };
        let vertex_count = count as i32;
        let mut neighbors: [Option<Predictor>; MAX_NEIGHBORS] = [None; MAX_NEIGHBORS];
        for v in 0..vertex_count {
            let mut predicted = 0usize;
            let adjacency = self.triangles.adjacency();
            let has_neighbors = adjacency.count(v)? > 0;
            if has_neighbors && block.prediction != NO_PREDICTION {
                let begin = adjacency.begin(v)?;
                let end = adjacency.end(v)?;
                for slot in begin..end {
                    let triangle = adjacency.neighbor(slot)?;
                    if triangle < 0 {
                        break;
                    }
                    for corner in 0..3 {
                        let w = self.triangles.corner(memory, triangle as i64 * 3 + corner)?;
                        if w < v {
                            insert([-1, -1, w], &mut predicted, &mut neighbors);
                        }
                    }
                }
            }
            if predicted > 1 {
                self.best_predictor(&mut block)?;
            }
            // The first vertex's values are written through the pointer.
            if dimension > 0 {
                return undefined("Open3DGC int attribute values written through a null pointer");
            }
        }
        self.iterator = block.predictors;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad (two triangles) with normals and a texture coordinate set,
    /// as Open3DGC's encoder writes it in binary and in ASCII form.
    const QUAD_BINARY: &str = "f1010000f6000000020000f0410f0400000004000000010000000000000002000000000000000000803f000000000000803f000000000000803f0c00000000000000000000000000000000000000000000803f0000803f010a040000000000000002000000000000803f000000000000803f01040a00020000000e00000004000000ffffff7f31c70e00000002000000ffffff7f00cb0d00000002000000020000005e0800000000000000080000000000000019000000457de493be005f5f4fb2c00005cda3dcc60000000116000000467de493bf341b475e767afa00000000008014000000457de493be601c6999223113c4803c01";
    const QUAD_ASCII: &str = "71030000002502000000020000400f040f0400000000040000000001000000000000000000020000000000000000000000007c0300000000000000007c0300000000000000007c030c000000000000000000000000000000000000000000000000000000007c030000007c03010a040000000000000000000200000000000000007c0300000000000000007c0301040a0002000000000e000000000400000000010100000c00000000020000000000010c00000000020000000006020a0000000000000000000a0000000000000000001800000000550000007f7f7b020000007f7f7b020000000007000000000000160000000056000000007f031c7f031c0000000000000700000000000012000000005500007f7f3a00007f7f3a000007000000000000";

    fn bytes(hex: &str) -> Vec<u8> {
        let digits = hex.as_bytes();
        let pairs = digits.chunks(2);
        pairs.map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
    }

    /// The quad decoded into positions (0), normals (48), texture
    /// coordinates (96) and indices (128).
    fn decode_quad(stream: &[u8]) -> (Vec<f32>, Vec<u16>) {
        let mut decoder = SC3DMCDecoder::decode_header(stream).unwrap().unwrap();
        let mut memory = Memory::new(140);
        let layout = Layout { coord_index: Some(128), coord: Some(0), normal: Some(48), float_attributes: vec![Some(96)] };
        assert!(decoder.decode_payload(&mut memory, &layout).unwrap());
        let (decoded, written) = memory.read(0, 140).unwrap();
        assert!(written);
        let (floats, indices) = decoded.split_at(128);
        let floats = floats.chunks(4).map(|value| f32::from_le_bytes(value.try_into().unwrap())).collect();
        let indices = indices.chunks(2).map(|value| u16::from_le_bytes(value.try_into().unwrap())).collect();
        (floats, indices)
    }

    #[test]
    fn a_quad_decodes_from_binary_and_ascii() {
        // The normals come back through the cube faces, quantized.
        let normal = [0.000_977_039_3, 0.000_977_039_3, 0.999_999_05];
        let mut expected = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0];
        for _ in 0..4 {
            expected.extend_from_slice(&normal);
        }
        expected.extend_from_slice(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        for stream in [QUAD_BINARY, QUAD_ASCII] {
            let (floats, indices) = decode_quad(&bytes(stream));
            assert_eq!(floats, expected);
            assert_eq!(indices, vec![0, 1, 2, 1, 3, 2]);
        }
    }

    #[test]
    fn a_stream_without_its_start_code_has_no_header() {
        assert!(SC3DMCDecoder::decode_header(&[0; 16]).unwrap().is_none());
    }

    #[test]
    fn reads_past_the_stream_and_its_zeros_are_undefined() {
        let stream = bytes(QUAD_BINARY);
        let Err(error) = SC3DMCDecoder::decode_header(&stream[..20]) else {
            panic!("a header cut short decoded");
        };
        assert!(error.ends_process());
    }
}
