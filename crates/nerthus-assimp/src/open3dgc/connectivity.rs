//! Open3DGC's connectivity (`TriangleListDecoder`, `CompressedTriangleFans`,
//! `AdjacencyInfo`): triangle fans around each vertex in turn, their
//! sizes, configurations, operations and indices coded in five lists.
//! The triangles are written as 16-bit indices into the decoded data, and
//! each vertex keeps a fixed number of adjacency slots. The adjacency,
//! fan and list reads assert their ranges (live in the build matched);
//! every other array is read and written unchecked in the C++: going
//! past one is an error here, and so is deciding anything by a value
//! never written (copying one is not).

use super::codec::{AdaptiveBitModel, AdaptiveDataModel, Decoder, StaticBitModel, uint_to_int, undefined};
use super::stream::{Stream, StreamType};
use super::Memory;
use crate::scene::{self, ImportError, check_allocation};

/// `sizeof(long)` on Windows, and the 16-bit triangle indices'.
const LONG_SIZE: u64 = 4;
const INDEX_SIZE: u64 = 2;

/// `TriangleFans`' first sizes (`O3DGC_TFANS_MIN_SIZE_*`).
const FAN_SIZES: i32 = 8;
const FAN_VERTICES: i32 = 128;

/// `O3DGC_MIN_NUM_NEIGHBORS_SIZE`: the adjacency arrays' first size.
const MIN_NUM_NEIGHBORS_SIZE: usize = 16;

/// `O3DGC_BINARY_STREAM_BITS_PER_SYMBOL0`.
const BITS_PER_SYMBOL: u32 = 7;

/// A `new long[n]` of values not yet written.
fn unset(count: usize) -> Vec<Option<i32>> {
    vec![None; count]
}

fn read(array: &[Option<i32>], index: i64, what: &str) -> Result<i32, ImportError> {
    match usize::try_from(index).ok().and_then(|index| array.get(index)).copied().flatten() {
        Some(value) => Ok(value),
        None => undefined(what),
    }
}

fn write(array: &mut [Option<i32>], index: i64, value: i32, what: &str) -> Result<(), ImportError> {
    match usize::try_from(index).ok().and_then(|index| array.get_mut(index)) {
        Some(slot) => {
            *slot = Some(value);
            Ok(())
        }
        None => undefined(what),
    }
}

/// `AdjacencyInfo`: for each element, a run of neighbour slots (-1 when
/// free) ending at its prefix sum.
pub(super) struct Adjacency {
    num_neighbors: Vec<Option<i32>>,
    neighbors: Vec<Option<i32>>,
    num_elements: i32,
}

impl Adjacency {
    fn new() -> Self {
        Adjacency { num_neighbors: unset(MIN_NUM_NEIGHBORS_SIZE), neighbors: unset(MIN_NUM_NEIGHBORS_SIZE), num_elements: 0 }
    }

    /// The assertions of `Begin` and `End`.
    fn check_element(&self, element: i32) -> Result<(), ImportError> {
        scene::ai_assert(element < self.num_elements, "element < m_numElements")?;
        scene::ai_assert(element >= 0, "element >= 0")
    }

    /// `Begin`.
    pub fn begin(&self, element: i32) -> Result<i32, ImportError> {
        self.check_element(element)?;
        if element > 0 {
            return read(&self.num_neighbors, element as i64 - 1, "an Open3DGC adjacency count read where none was set");
        }
        Ok(0)
    }

    /// `End`.
    pub fn end(&self, element: i32) -> Result<i32, ImportError> {
        self.check_element(element)?;
        read(&self.num_neighbors, element as i64, "an Open3DGC adjacency count read where none was set")
    }

    /// `GetNumNeighbors`.
    pub fn count(&self, element: i32) -> Result<i32, ImportError> {
        let end = self.end(element)?;
        let begin = self.begin(element)?;
        Ok(end.wrapping_sub(begin))
    }

    /// `GetNeighbor` (the slots array's size is its length here).
    pub fn neighbor(&self, slot: i32) -> Result<i32, ImportError> {
        scene::ai_assert((slot as i64) < self.neighbors.len() as i64, "element < m_neighborsSize")?;
        scene::ai_assert(slot >= 0, "element >= 0")?;
        read(&self.neighbors, slot as i64, "an Open3DGC adjacency slot read where none was set")
    }

    /// `AddNeighbor`: the element's first free slot (none left: dropped).
    /// It first asserts the element's run ends by the last one's, reading
    /// its count unchecked: for an element past the counts set, that
    /// compares a count never written, and `Begin`'s assertion fails
    /// if it doesn't.
    fn add(&mut self, element: i32, neighbor: i32) -> Result<(), ImportError> {
        let in_array = usize::try_from(element).is_ok_and(|index| index < self.num_neighbors.len());
        if !in_array {
            return undefined("an Open3DGC adjacency count read past its array");
        }
        if element >= self.num_elements {
            return Err(scene::assertion_failed("element < m_numElements"));
        }
        let own_end = read(&self.num_neighbors, element as i64, "an Open3DGC adjacency count read where none was set")?;
        let last_end = read(&self.num_neighbors, self.num_elements as i64 - 1, "an Open3DGC adjacency count read where none was set")?;
        scene::ai_assert(own_end <= last_end, "m_numNeighbors[element] <= m_numNeighbors[m_numElements-1]")?;
        let begin = self.begin(element)?;
        let end = self.end(element)?;
        let mut slot = begin;
        while slot < end {
            if self.neighbor(slot)? == -1 {
                write(&mut self.neighbors, slot as i64, neighbor, "an Open3DGC adjacency slot written past its array")?;
                return Ok(());
            }
            slot += 1;
        }
        Ok(())
    }
}

/// A `Vector<long>` read back by an iterator (`Read*` of
/// `CompressedTriangleFans`): past what was pushed it is unset memory.
#[derive(Default)]
struct List {
    values: Vec<i32>,
}

impl List {
    /// The `Read*` of the list, which asserts the iterator is within what
    /// was pushed (`expression`, for the message).
    fn next(&self, iterator: &mut u32, expression: &str) -> Result<i32, ImportError> {
        let Some(&value) = self.values.get(*iterator as usize) else {
            return Err(scene::assertion_failed(expression));
        };
        *iterator = iterator.wrapping_add(1);
        Ok(value)
    }
}

/// `CompressedTriangleFans`' lists.
#[derive(Default)]
struct CompressedFans {
    num_tfans: List,
    degrees: List,
    configs: List,
    operations: List,
    indices: List,
    triangles_order: List,
}

/// `LoadIntACEGC`: values Exp-Golomb coded above `m`, offset by a stored
/// minimum (biased by `O3DGC_MAX_LONG`).
fn load_int_egc(list: &mut List, m: u32, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
    let size_size = stream.read_u32_binary(iterator)?.wrapping_sub(12);
    let size = stream.read_u32_binary(iterator)?;
    if size == 0 {
        return Ok(());
    }
    let min_value = stream.read_u32_binary(iterator)?.wrapping_sub(0x7FFF_FFFF) as i32;
    let start = *iterator as usize;
    *iterator = iterator.wrapping_add(size_size);
    check_allocation(size as u64, LONG_SIZE)?;
    let mut decoder = Decoder::start(stream.bytes(), start, size_size)?;
    let mut values = AdaptiveDataModel::new(m + 2)?;
    let static_model = StaticBitModel::new();
    let mut adaptive_model = AdaptiveBitModel::new();
    for _ in 0..size {
        let mut value = decoder.decode_data(&mut values)?;
        if value == m {
            value = value.wrapping_add(decoder.exp_golomb(0, &static_model, &mut adaptive_model)?);
        }
        list.values.push(value.wrapping_add(min_value as u32) as i32);
    }
    Ok(())
}

/// `LoadUIntAC`: values below `m + 1`, offset by a stored minimum.
fn load_uint_ac(list: &mut List, m: u32, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
    let size_size = stream.read_u32_binary(iterator)?.wrapping_sub(12);
    let size = stream.read_u32_binary(iterator)?;
    if size == 0 {
        return Ok(());
    }
    let min_value = stream.read_u32_binary(iterator)?;
    let start = *iterator as usize;
    *iterator = iterator.wrapping_add(size_size);
    check_allocation(size as u64, LONG_SIZE)?;
    let mut decoder = Decoder::start(stream.bytes(), start, size_size)?;
    let mut values = AdaptiveDataModel::new(m + 1)?;
    for _ in 0..size {
        let value = decoder.decode_data(&mut values)?;
        list.values.push(value.wrapping_add(min_value) as i32);
    }
    Ok(())
}

/// `LoadBinAC`: bits.
fn load_bin_ac(list: &mut List, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
    let size_size = stream.read_u32_binary(iterator)?.wrapping_sub(8);
    let size = stream.read_u32_binary(iterator)?;
    if size == 0 {
        return Ok(());
    }
    let start = *iterator as usize;
    *iterator = iterator.wrapping_add(size_size);
    check_allocation(size as u64, LONG_SIZE)?;
    let mut decoder = Decoder::start(stream.bytes(), start, size_size)?;
    let mut model = AdaptiveBitModel::new();
    for _ in 0..size {
        let bit = decoder.decode_adaptive_bit(&mut model)?;
        list.values.push(bit as i32);
    }
    Ok(())
}

/// `LoadUIntData` and `LoadIntData` (ASCII).
fn load_ascii_values(list: &mut List, signed: bool, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
    stream.read_u32_ascii(iterator)?;
    let size = stream.read_u32_ascii(iterator)?;
    check_allocation(size as u64, LONG_SIZE)?;
    for _ in 0..size {
        let value = if signed { stream.read_int_ascii(iterator)? } else { stream.read_uint_ascii(iterator)? as i32 };
        list.values.push(value);
    }
    Ok(())
}

/// `LoadBinData` (ASCII): seven bits a symbol, whole symbols read.
fn load_ascii_bits(list: &mut List, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
    stream.read_u32_ascii(iterator)?;
    let size = stream.read_u32_ascii(iterator)?;
    check_allocation(size.wrapping_mul(BITS_PER_SYMBOL) as u64, LONG_SIZE)?;
    let mut read = 0u32;
    while read < size {
        let mut symbol = stream.read_uchar(iterator)? as i32;
        for _ in 0..BITS_PER_SYMBOL {
            list.values.push(symbol & 1);
            symbol >>= 1;
            read = read.wrapping_add(1);
        }
    }
    Ok(())
}

/// `TriangleListDecoder` with the state its steps share.
pub(super) struct TriangleListDecoder {
    stream_type: StreamType,
    decode_triangles_order: bool,
    num_triangles: i32,
    num_vertices: i32,
    /// The byte offset of the triangles in the decoded data.
    triangles: Option<usize>,
    vertex_count: i32,
    triangle_count: i32,
    num_conquered: i32,
    num_visited: i32,
    visited: Vec<Option<i32>>,
    valence: Vec<Option<i32>>,
    adjacency: Adjacency,
    fans: CompressedFans,
    /// `m_tfans`' vertices (its fan sizes decide nothing here): unset
    /// ones were copied from visited slots never written.
    fan_vertices: Vec<Option<i32>>,
    iterators: [u32; 5],
}

impl TriangleListDecoder {
    pub fn new(stream_type: StreamType) -> Self {
        TriangleListDecoder {
            stream_type,
            decode_triangles_order: false,
            num_triangles: 0,
            num_vertices: 0,
            triangles: None,
            vertex_count: 0,
            triangle_count: 0,
            num_conquered: 0,
            num_visited: 0,
            visited: Vec::new(),
            valence: Vec::new(),
            adjacency: Adjacency::new(),
            fans: CompressedFans::default(),
            fan_vertices: Vec::new(),
            iterators: [0; 5],
        }
    }

    /// `GetVertexToTriangle`: unset when the decode stopped early.
    pub fn adjacency(&self) -> &Adjacency {
        &self.adjacency
    }

    /// The `index`th triangle corner, from the decoded data.
    pub fn corner(&self, memory: &Memory, index: i64) -> Result<i32, ImportError> {
        match memory.read_u16(self.triangles, index)? {
            Some(corner) => Ok(corner as i32),
            None => undefined("an Open3DGC triangle corner read where none was written"),
        }
    }

    /// `Decode`: the compression mask (vertex reordering is refused before
    /// anything is set up), the adjacency slots per vertex, the fan lists,
    /// then the fans.
    pub fn decode(&mut self, memory: &mut Memory, triangles: Option<usize>, num_triangles: i32, num_vertices: i32, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
        let mask = stream.read_uchar(iterator)?;
        self.decode_triangles_order = mask & 2 != 0;
        if mask & 1 != 0 {
            return Ok(());
        }
        let max_size = stream.read_u32(iterator, self.stream_type)?;
        self.init(triangles, num_triangles, num_vertices, max_size as i32)?;
        self.load_fans(stream, iterator)?;
        for focus in 0..self.num_vertices {
            if focus == self.vertex_count {
                self.vertex_count += 1;
            }
            self.local_connectivity(memory, focus)?;
            self.decompress_fan(memory, focus)?;
        }
        Ok(())
    }

    /// `Init`, which asserts there are vertices and triangles.
    fn init(&mut self, triangles: Option<usize>, num_triangles: i32, num_vertices: i32, max_size: i32) -> Result<(), ImportError> {
        scene::ai_assert(num_vertices > 0, "numVertices > 0")?;
        scene::ai_assert(num_triangles > 0, "numTriangles > 0")?;
        self.num_triangles = num_triangles;
        self.num_vertices = num_vertices;
        self.triangles = triangles;
        check_allocation(num_vertices as u64, LONG_SIZE)?;
        self.valence = unset(num_vertices as usize);
        self.visited = unset(num_vertices as usize);
        if self.decode_triangles_order {
            // `new T[3 * n]` in 32-bit `long`s, sign-extended.
            let corners = num_triangles.wrapping_mul(3) as i64 as u64;
            check_allocation(corners, INDEX_SIZE)?;
        }
        // `CompressedTriangleFans::Allocate` (sizes as unsigned longs).
        check_allocation(num_vertices as u32 as u64, LONG_SIZE)?;
        check_allocation(num_vertices.wrapping_mul(2) as u32 as u64, LONG_SIZE)?;
        check_allocation(num_triangles as u32 as u64, LONG_SIZE)?;
        // `TriangleFans::Allocate`: its sizes asserted positive (twice and
        // eight times the vertices, in 32-bit `int`s), then allocated
        // only past its first sizes.
        let fan_sizes = num_vertices.wrapping_mul(2);
        let fan_vertices = num_vertices.wrapping_mul(8);
        scene::ai_assert(fan_sizes > 0, "sizeTFAN > 0")?;
        scene::ai_assert(fan_vertices > 0, "verticesSize > 0")?;
        if fan_vertices > FAN_VERTICES {
            check_allocation(fan_vertices as u64, LONG_SIZE)?;
        }
        if fan_sizes > FAN_SIZES {
            check_allocation(fan_sizes as u64, LONG_SIZE)?;
        }
        // Vertex-to-triangle slots: `max_size` for each vertex.
        let adjacency = &mut self.adjacency;
        if num_vertices > MIN_NUM_NEIGHBORS_SIZE as i32 {
            check_allocation(num_vertices as u64, LONG_SIZE)?;
            adjacency.num_neighbors = unset(num_vertices as usize);
        }
        adjacency.num_elements = num_vertices;
        let mut total = 0i32;
        for element in 0..num_vertices as usize {
            total = total.wrapping_add(max_size);
            adjacency.num_neighbors[element] = Some(total);
        }
        if total > adjacency.neighbors.len() as i32 {
            check_allocation(total as u64, LONG_SIZE)?;
            adjacency.neighbors = unset(total as usize);
        }
        adjacency.neighbors.fill(Some(-1));
        Ok(())
    }

    /// `CompressedTriangleFans::Load`.
    fn load_fans(&mut self, stream: &Stream, iterator: &mut u32) -> Result<(), ImportError> {
        let fans = &mut self.fans;
        if self.stream_type == StreamType::Ascii {
            load_ascii_values(&mut fans.num_tfans, false, stream, iterator)?;
            load_ascii_values(&mut fans.degrees, false, stream, iterator)?;
            load_ascii_values(&mut fans.configs, false, stream, iterator)?;
            load_ascii_bits(&mut fans.operations, stream, iterator)?;
            load_ascii_values(&mut fans.indices, true, stream, iterator)?;
            if self.decode_triangles_order {
                load_ascii_values(&mut fans.triangles_order, false, stream, iterator)?;
            }
        } else {
            load_int_egc(&mut fans.num_tfans, 4, stream, iterator)?;
            load_int_egc(&mut fans.degrees, 16, stream, iterator)?;
            load_uint_ac(&mut fans.configs, 10, stream, iterator)?;
            load_bin_ac(&mut fans.operations, stream, iterator)?;
            load_int_egc(&mut fans.indices, 8, stream, iterator)?;
            if self.decode_triangles_order {
                load_int_egc(&mut fans.triangles_order, 16, stream, iterator)?;
            }
        }
        Ok(())
    }

    fn visited_at(&self, index: i64) -> Result<i32, ImportError> {
        read(&self.visited, index, "an Open3DGC visited vertex read where none was set")
    }

    /// A visited slot copied into a fan: one never written copies as
    /// unset.
    fn visited_copy(&self, index: i64) -> Result<Option<i32>, ImportError> {
        match usize::try_from(index).ok().and_then(|index| self.visited.get(index)) {
            Some(&value) => Ok(value),
            None => undefined("an Open3DGC visited vertex read past its array"),
        }
    }

    /// `CompueLocalConnectivityInfo`: the triangles around the vertex so
    /// far, and their later vertices sorted, then those of valence 1
    /// moved forward.
    fn local_connectivity(&mut self, memory: &Memory, focus: i32) -> Result<(), ImportError> {
        self.num_conquered = 0;
        self.num_visited = 0;
        let mut triangle = 0i32;
        let mut slot = self.adjacency.begin(focus)?;
        while triangle >= 0 && slot < self.adjacency.end(focus)? {
            triangle = self.adjacency.neighbor(slot)?;
            slot += 1;
            if triangle < 0 {
                continue;
            }
            self.num_conquered += 1;
            let first = triangle as i64 * 3;
            for corner in 0..3 {
                let vertex = self.corner(memory, first + corner)?;
                if vertex > focus {
                    self.insert_visited(vertex)?;
                }
            }
        }
        if self.num_visited > 2 {
            for x in 1..self.num_visited as i64 {
                if read(&self.valence, x, "an Open3DGC valence read where none was set")? != 1 {
                    continue;
                }
                let mut y = x;
                while y > 0 {
                    let current = read(&self.valence, y, "an Open3DGC valence read where none was set")?;
                    let previous = read(&self.valence, y - 1, "an Open3DGC valence read where none was set")?;
                    if current >= previous {
                        break;
                    }
                    self.valence.swap(y as usize, y as usize - 1);
                    self.visited.swap(y as usize, y as usize - 1);
                    y -= 1;
                }
            }
        }
        Ok(())
    }

    /// A vertex into the sorted visited list (its valence counted).
    fn insert_visited(&mut self, vertex: i32) -> Result<(), ImportError> {
        for j in 0..self.num_visited as i64 {
            let present = self.visited_at(j)?;
            if vertex == present {
                let valence = read(&self.valence, j, "an Open3DGC valence read where none was set")?;
                write(&mut self.valence, j, valence.wrapping_add(1), "an Open3DGC valence written past its array")?;
                return Ok(());
            }
            if vertex < present {
                self.num_visited += 1;
                let mut h = self.num_visited as i64 - 1;
                while h > j {
                    let moved = self.visited_at(h - 1)?;
                    let moved_valence = read(&self.valence, h - 1, "an Open3DGC valence read where none was set")?;
                    write(&mut self.visited, h, moved, "an Open3DGC visited vertex written past its array")?;
                    write(&mut self.valence, h, moved_valence, "an Open3DGC valence written past its array")?;
                    h -= 1;
                }
                write(&mut self.visited, j, vertex, "an Open3DGC visited vertex written past its array")?;
                write(&mut self.valence, j, 1, "an Open3DGC valence written past its array")?;
                return Ok(());
            }
        }
        let end = self.num_visited as i64;
        write(&mut self.visited, end, vertex, "an Open3DGC visited vertex written past its array")?;
        write(&mut self.valence, end, 1, "an Open3DGC valence written past its array")?;
        self.num_visited += 1;
        Ok(())
    }

    /// A new vertex: visited, and in the fan.
    fn new_vertex(&mut self) -> Result<(), ImportError> {
        let end = self.num_visited as i64;
        write(&mut self.visited, end, self.vertex_count, "an Open3DGC visited vertex written past its array")?;
        self.num_visited = self.num_visited.wrapping_add(1);
        self.fan_vertices.push(Some(self.vertex_count));
        self.vertex_count = self.vertex_count.wrapping_add(1);
        Ok(())
    }

    /// An operation-coded vertex: an earlier one (a visited one by
    /// negative index, else relative to the focus), or a new one.
    fn coded_vertex(&mut self, focus: i32) -> Result<(), ImportError> {
        let operation = self.fans.operations.next(&mut self.iterators[3], "iterator < m_operations.GetSize()")?;
        if operation != 1 {
            return self.new_vertex();
        }
        let index = self.fans.indices.next(&mut self.iterators[4], "iterator < m_indices.GetSize()")?;
        if index < 0 {
            let position = index.wrapping_neg().wrapping_sub(1) as i64;
            let vertex = self.visited_copy(position)?;
            self.fan_vertices.push(vertex);
        } else {
            self.fan_vertices.push(Some(index.wrapping_add(focus)));
        }
        Ok(())
    }

    /// `DecompressTFAN`: the vertex's fans, each made triangles.
    fn decompress_fan(&mut self, memory: &mut Memory, focus: i32) -> Result<(), ImportError> {
        let fan_count = self.fans.num_tfans.next(&mut self.iterators[0], "iterator < m_numTFANs.GetSize()")?;
        if fan_count <= 0 {
            return Ok(());
        }
        let mut fan = 0i32;
        while fan != fan_count {
            let degree_code = self.fans.degrees.next(&mut self.iterators[1], "iterator < m_degrees.GetSize()")?;
            let degree = degree_code.wrapping_add(2).wrapping_sub(self.num_conquered);
            let config = self.fans.configs.next(&mut self.iterators[2], "iterator < m_configs.GetSize()")?;
            let first_vertex = self.fan_vertices.len();
            self.fan_vertices.push(Some(focus));
            match config {
                0 | 7 => {
                    let (opening, closing) = if config == 0 { (0, 1) } else { (1, 0) };
                    let opening = self.visited_copy(opening)?;
                    self.fan_vertices.push(opening);
                    let mut u = 1i32;
                    while u < degree.wrapping_sub(1) {
                        self.new_vertex()?;
                        u += 1;
                    }
                    let closing = self.visited_copy(closing)?;
                    self.fan_vertices.push(closing);
                }
                1 | 8 => {
                    let (opening, closing) = if config == 1 { (0, 1) } else { (1, 0) };
                    let opening = self.visited_copy(opening)?;
                    self.fan_vertices.push(opening);
                    let mut u = 1i32;
                    while u < degree.wrapping_sub(1) {
                        self.coded_vertex(focus)?;
                        u += 1;
                    }
                    let closing = self.visited_copy(closing)?;
                    self.fan_vertices.push(closing);
                }
                2 | 3 => {
                    let mut u = 0i32;
                    while u < degree.wrapping_sub(1) {
                        self.new_vertex()?;
                        u += 1;
                    }
                    let closing = self.visited_copy(if config == 2 { 0 } else { 1 })?;
                    self.fan_vertices.push(closing);
                }
                4 | 5 => {
                    let opening = self.visited_copy(if config == 4 { 0 } else { 1 })?;
                    self.fan_vertices.push(opening);
                    let mut u = 1i32;
                    while u < degree {
                        self.new_vertex()?;
                        u += 1;
                    }
                }
                6 => {
                    let mut u = 0i32;
                    while u < degree {
                        self.new_vertex()?;
                        u += 1;
                    }
                }
                9 => {
                    let mut u = 0i32;
                    while u < degree {
                        self.coded_vertex(focus)?;
                        u += 1;
                    }
                }
                _ => {}
            }
            // The fan's second vertex is read (`GetVertex`, which asserts
            // it is there); a fan of fewer than three vertices makes no
            // triangle (the second, maybe never written, isn't used).
            let last_vertex = self.fan_vertices.len();
            if last_vertex < first_vertex + 2 {
                return Err(scene::assertion_failed("vertex < m_numVertices"));
            }
            if last_vertex < first_vertex + 3 {
                fan = fan.wrapping_add(1);
                continue;
            }
            let mut b = self.fan_vertices[first_vertex + 1];
            for k in first_vertex + 2..last_vertex {
                let c = self.fan_vertices[k];
                let corner = self.triangle_count.wrapping_mul(3);
                memory.write_u16(self.triangles, corner as i64, Some(focus as u16))?;
                memory.write_u16(self.triangles, corner.wrapping_add(1) as i64, b.map(|vertex| vertex as u16))?;
                memory.write_u16(self.triangles, corner.wrapping_add(2) as i64, c.map(|vertex| vertex as u16))?;
                let (Some(b_vertex), Some(c_vertex)) = (b, c) else {
                    return undefined("an Open3DGC fan vertex used where none was set");
                };
                self.adjacency.add(focus, self.triangle_count)?;
                self.adjacency.add(b_vertex, self.triangle_count)?;
                self.adjacency.add(c_vertex, self.triangle_count)?;
                b = c;
                self.triangle_count = self.triangle_count.wrapping_add(1);
            }
            fan = fan.wrapping_add(1);
        }
        Ok(())
    }

    /// `Reorder`: the triangles copied aside (`3 n` corners in 32-bit
    /// `long`s), then each moved to its coded position, relative to the
    /// one after the last.
    pub fn reorder(&mut self, memory: &mut Memory) -> Result<(), ImportError> {
        // Nothing moves without triangles (also when the decode stopped
        // before setting up).
        if !self.decode_triangles_order || self.num_triangles == 0 {
            return Ok(());
        }
        let copied = self.num_triangles.wrapping_mul(3);
        if copied < 0 {
            return undefined("Open3DGC triangles copied with a negative size");
        }
        let mut copy = Vec::with_capacity(copied as usize);
        for corner in 0..copied as i64 {
            copy.push(memory.read_u16(self.triangles, corner)?);
        }
        let mut iterator = 0u32;
        let mut previous = 0i32;
        for triangle in 0..self.num_triangles.max(0) as usize {
            let coded = self.fans.triangles_order.next(&mut iterator, "iterator < m_trianglesOrder.GetSize()")?;
            let target = uint_to_int(coded as u32).wrapping_add(previous);
            scene::ai_assert(target >= 0 && target < self.num_triangles, "t >= 0 && t < m_numTriangles")?;
            let first = target.wrapping_mul(3);
            for corner in 0..3usize {
                let Some(&value) = copy.get(triangle * 3 + corner) else {
                    return undefined("Open3DGC triangles moved from past their copy");
                };
                let index = first.wrapping_add(corner as i32);
                memory.write_u16(self.triangles, index as i64, value)?;
            }
            previous = target.wrapping_add(1);
        }
        Ok(())
    }
}
