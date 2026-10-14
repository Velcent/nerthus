//! Assimp's glTF 1.0 importer (`glTFImporter` over `glTF::Asset`), which
//! claims a .gltf/.glb file before glTF 2 when the asset version starts
//! with '1'. Objects are keyed by string id and read lazily from the
//! scene's nodes (a node's children before its meshes); every primitive
//! becomes an `aiMesh` with its accessors copied raw (texture V flipped)
//! and faces from its mode, without glTF 2's vertex remapping.
//!
//! A mesh compressed with Open3DGC is decoded while it is read: the
//! decoded arrays are written at their accessors' offsets from the start
//! of a new buffer, which then stands in for the compressed bytes from
//! their offset on (so only data compressed at offset 0 reads back where
//! it was written), for every mesh after one selects it.
//!
//! What the C++ leaves undefined (reads past a buffer, null references,
//! strips and fans past three points) is reported as an error.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use crate::base64;
use crate::gltf2::asset::{
    MODE_LINE_LOOP, MODE_LINE_STRIP, MODE_LINES, MODE_POINTS, MODE_TRIANGLE_FAN, MODE_TRIANGLE_STRIP, MODE_TRIANGLES, component_size,
    find_array, find_object, find_string, is_binary, parse_data_uri,
};
use crate::io::FileSystem;
use crate::json::{self, Value};
use crate::open3dgc::{Layout, Memory, SC3DMCDecoder};
use crate::postprocess::join_vertices::positions_may_join;
use crate::scene::{self, ImportError, MAX_TEXTURE_COORDS, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};

const COMPONENT_BYTE: i32 = 5120;
const COMPONENT_UNSIGNED_BYTE: i32 = 5121;

/// `O3DGC_IFS_FLOAT_ATTRIBUTE_TYPE_TEXCOORD`, and the last int attribute
/// type (`O3DGC_IFS_INT_ATTRIBUTE_TYPE_INDEX_BUFFER_ID`).
const FLOAT_ATTRIBUTE_TEXCOORD: u8 = 4;
const INT_ATTRIBUTE_LAST: u8 = 3;

/// The decoded arrays' element sizes: 16-bit indices, floats, and 32-bit
/// `long`s for int attributes.
const INDEX_SIZE: usize = 2;
const FLOAT_SIZE: usize = 4;
const LONG_SIZE: usize = 4;

/// Face counts past which `new aiFace[n]` throws `bad_alloc`.
const MAX_FACES: u32 = 1 << 28;

/// Attribute set indices past which the list's `resize` throws.
const MAX_ATTRIBUTE_SETS: i32 = 1 << 24;

/// The JSON text and GLB v1 body of a file (`Asset::Load` up to the
/// parse).
struct Document {
    json: Vec<u8>,
    body: Option<Vec<u8>>,
    binary: bool,
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// `ReadBinaryHeader` and the scene/body reads of `Asset::Load`.
fn read_document(bytes: &[u8]) -> Result<Document, ImportError> {
    if !is_binary(bytes) {
        if bytes.len() < 2 {
            return fail("GLTF: No JSON file contents");
        }
        if bytes.len() >= u32::MAX as usize {
            return fail("GLTF: JSON size greater than 4GB");
        }
        return Ok(Document { json: bytes.to_vec(), body: None, binary: false });
    }
    if bytes.len() < 20 {
        return fail("GLTF: Unable to read the file header");
    }
    if &bytes[..4] != b"glTF" {
        return fail("GLTF: Invalid binary glTF file");
    }
    if read_u32(bytes, 4) != 1 {
        return fail("GLTF: Unsupported binary glTF version");
    }
    if read_u32(bytes, 16) != 0 {
        return fail("GLTF: Unsupported binary glTF scene format");
    }
    let total_length = read_u32(bytes, 8) as usize;
    let scene_length = read_u32(bytes, 12) as usize;
    let body_offset = (20 + scene_length + 3) & !3;
    let body_length = total_length.wrapping_sub(body_offset);
    if scene_length < 2 {
        return fail("GLTF: No JSON file contents");
    }
    if scene_length >= u32::MAX as usize {
        return fail("GLTF: JSON size greater than 4GB");
    }
    let Some(json) = bytes.get(20..20 + scene_length) else {
        return fail("GLTF: Could not read the file contents");
    };
    let mut body = None;
    if body_length > 0 {
        let Some(data) = bytes.get(body_offset..body_offset.saturating_add(body_length)) else {
            return fail("GLTF: Unable to read gltf file");
        };
        body = Some(data.to_vec());
    }
    Ok(Document { json: json.to_vec(), body, binary: true })
}

/// `ParseInsitu` and the root check.
fn parse_json(json: &[u8]) -> Result<Value, ImportError> {
    let value = match json::parse(json) {
        Ok(value) => value,
        Err(error) => return fail(format!("GLTF: JSON parse error, offset {}: {}", error.offset, error.code)),
    };
    if !value.is_object() {
        return fail("GLTF: JSON document root must be a JSON object");
    }
    Ok(value)
}

/// `AssetMetadata::Read`: the version string (a number prints as "%.1f"
/// in four bytes), after the GLB header's.
fn read_version(document: &Value, binary: bool) -> Result<Vec<u8>, ImportError> {
    let mut version = if binary { b"1".to_vec() } else { Vec::new() };
    if let Some(asset) = find_object(document, "asset")? {
        match asset.member("version") {
            Some(text) if text.is_string() => version = text.c_str().to_vec(),
            Some(number) if number.is_number() => {
                let mut printed = format!("{:.1}", number.get_double()).into_bytes();
                printed.truncate(3);
                version = printed;
            }
            _ => {}
        }
    }
    Ok(version)
}

/// `glTFImporter::CanRead`: the file loads as far as its metadata, and
/// its version starts with '1'.
pub fn can_read(bytes: &[u8]) -> bool {
    let Ok(document) = read_document(bytes) else {
        return false;
    };
    let Ok(json) = parse_json(&document.json) else {
        return false;
    };
    matches!(read_version(&json, document.binary), Ok(version) if version.first() == Some(&b'1'))
}

/// A member lookup through the `Value` overloads, which only work on
/// objects.
fn member_of<'a>(value: &'a Value, name: &str, check: fn(&Value) -> bool) -> Result<Option<&'a Value>, ImportError> {
    if !value.is_object() {
        return Err(scene::assertion_failed("IsObject()"));
    }
    Ok(value.member(name).filter(|found| check(found)))
}

/// `MemberOrDefault` with the generic `ReadHelper` (an `Int` value cast).
fn read_int(object: &Value, name: &str, default: i32) -> i32 {
    match object.member(name) {
        Some(value) if object.is_object() && value.is_int() => value.get_int(),
        _ => default,
    }
}

/// `MemberOrDefault<size_t>`: `IsUint64` on Windows.
fn read_size(object: &Value, name: &str) -> usize {
    read_size_member(object, name).unwrap_or(0)
}

/// `ReadMember<size_t>`.
fn read_size_member(object: &Value, name: &str) -> Option<usize> {
    match object.member(name) {
        Some(value) if object.is_object() && value.is_uint64() => Some(value.get_uint64() as usize),
        _ => None,
    }
}

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The bytes up to the first NUL (a `const char*` view).
fn c_string(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// The "Open3DGC-compression" extension's `compressedData`: every member
/// required, then SCALAR unsigned bytes in binary or ASCII mode (which
/// the decoder finds out for itself).
fn read_compression(extension: &Value) -> Result<Compression, ImportError> {
    let Some(data) = member_of(extension, "compressedData", Value::is_object)? else {
        return fail("GLTF: \"Open3DGC-compression\" must has \"compressedData\".");
    };
    let missing = |name: &str| -> Result<Compression, ImportError> { fail(format!("GLTF: \"compressedData\" must has \"{name}\".")) };
    let Some(buffer) = data.member("buffer").filter(|value| value.is_string()) else {
        return missing("buffer");
    };
    let Some(offset) = read_size_member(data, "byteOffset") else {
        return missing("byteOffset");
    };
    let Some(component_type) = data.member("componentType").filter(|value| value.is_int()) else {
        return missing("componentType");
    };
    let Some(kind) = data.member("type").filter(|value| value.is_string()) else {
        return missing("type");
    };
    let Some(count) = read_size_member(data, "count") else {
        return missing("count");
    };
    let Some(mode) = data.member("mode").filter(|value| value.is_string()) else {
        return missing("mode");
    };
    for name in ["indicesCount", "verticesCount"] {
        if read_size_member(data, name).is_none() {
            return missing(name);
        }
    }
    if kind.c_str() != b"SCALAR" {
        return fail("GLTF: only \"SCALAR\" type is supported for compressed data.");
    }
    if component_type.get_int() != COMPONENT_UNSIGNED_BYTE {
        return fail("GLTF: only \"UNSIGNED_BYTE\" component type is supported for compressed data.");
    }
    let mode = mode.c_str();
    if mode != b"binary" && mode != b"ascii" {
        let mode = String::from_utf8_lossy(mode);
        return fail(format!("GLTF: for compressed data supported modes is: \"ascii\", \"binary\". Not the: \"{mode}\"."));
    }
    let compression = Compression { buffer: buffer.c_str().to_vec(), offset, count };
    Ok(compression)
}

/// `ReadMember<const char*>`.
fn read_id<'a>(object: &'a Value, name: &str) -> Option<&'a [u8]> {
    if !object.is_object() {
        return None;
    }
    object.member(name).filter(|value| value.is_string()).map(Value::c_str)
}

/// `LazyDict<T>` over a JSON object keyed by id.
struct Dict<'a, T> {
    name: &'static str,
    object: Option<&'a Value>,
    objects: Vec<T>,
    by_id: HashMap<Vec<u8>, usize>,
    in_progress: HashSet<Vec<u8>>,
}

enum Lookup<'a> {
    Loaded(usize),
    Read(&'a Value),
}

impl<'a, T> Dict<'a, T> {
    fn new(name: &'static str) -> Self {
        Dict { name, object: None, objects: Vec::new(), by_id: HashMap::new(), in_progress: HashSet::new() }
    }

    /// `AttachToDocument` for a dictionary at the document's top level.
    fn attach(&mut self, document: &'a Value) {
        self.object = document.member(self.name).filter(|value| value.is_object());
    }

    /// The first half of `Get`: the object read before, or its JSON (a
    /// reference back to an object being read recurses without end in
    /// the C++).
    fn lookup(&mut self, id: &[u8]) -> Result<Lookup<'a>, ImportError> {
        if let Some(&slot) = self.by_id.get(id) {
            return Ok(Lookup::Loaded(slot));
        }
        let Some(dictionary) = self.object else {
            return fail(format!("GLTF: Missing section \"{}\"", self.name));
        };
        let Some(object) = dictionary.member_bytes(id) else {
            return fail(format!("GLTF: Missing object with id \"{}\" in \"{}\"", String::from_utf8_lossy(id), self.name));
        };
        if !object.is_object() {
            return fail(format!("GLTF: Object with id \"{}\" is not a JSON object", String::from_utf8_lossy(id)));
        }
        if !self.in_progress.insert(id.to_vec()) {
            return undefined("a glTF object that references itself recurses past the stack");
        }
        Ok(Lookup::Read(object))
    }

    /// `Add`.
    fn add(&mut self, id: &[u8], value: T) -> usize {
        self.objects.push(value);
        let slot = self.objects.len() - 1;
        self.by_id.insert(id.to_vec(), slot);
        self.in_progress.remove(id);
        slot
    }
}

/// `SEncodedRegion`: decoded data standing in for the compressed bytes
/// from `offset` on, for the mesh with the id.
struct Region {
    offset: usize,
    decoded: Memory,
    decoded_length: usize,
    id: Vec<u8>,
}

struct Buffer {
    data: Option<Vec<u8>>,
    /// `byteLength`: the data's, grown by each region's decoded size less
    /// its compressed one.
    byte_length: usize,
    regions: Vec<Region>,
    /// `EncodedRegion_Current`, which `ImportMeshes` sets and nothing
    /// clears.
    current: Cell<Option<usize>>,
}

impl Buffer {
    fn new(data: Option<Vec<u8>>) -> Self {
        let byte_length = data.as_ref().map_or(0, Vec::len);
        Buffer { data, byte_length, regions: Vec::new(), current: Cell::new(None) }
    }

    /// `EncodedRegion_SetCurrent`: the first region with the id.
    fn select_region(&self, id: &[u8]) -> Result<(), ImportError> {
        if let Some(current) = self.current.get()
            && self.regions[current].id == id
        {
            return Ok(());
        }
        let Some(found) = self.regions.iter().position(|region| region.id == id) else {
            return fail(format!("GLTF: EncodedRegion with ID: \"{}\" not found.", String::from_utf8_lossy(id)));
        };
        self.current.set(Some(found));
        Ok(())
    }
}

/// Where an accessor's bytes come from.
enum Source<'s> {
    Raw(&'s [u8]),
    Decoded(&'s Memory),
}

impl Source<'_> {
    /// `length` bytes from `start`, and whether they were all written
    /// (decoded data the decoder never wrote is garbage).
    fn read(&self, start: usize, length: usize, past_end: &str) -> Result<(&[u8], bool), ImportError> {
        match self {
            Source::Raw(data) => match start.checked_add(length).and_then(|end| data.get(start..end)) {
                Some(bytes) => Ok((bytes, true)),
                None => fail(past_end),
            },
            Source::Decoded(memory) => memory.read(start, length),
        }
    }
}

/// An accessor's elements, and which of them hold garbage.
struct Elements {
    values: Vec<[f32; 3]>,
    garbage: Vec<bool>,
}

/// Past this many vertex comparisons garbage is taken to decide.
const COMPARISON_LIMIT: usize = 100_000_000;

/// Whether garbage normals or texture coordinates (decoded data the
/// decoder never wrote) decide anything: only JoinVertices reads them,
/// comparing a used vertex's streams with another used vertex's only
/// when their positions may join.
fn garbage_decides(vertices: &[[f32; 3]], faces: &[Vec<u32>], garbage: &[bool]) -> bool {
    let mut used = vec![false; vertices.len()];
    for &index in faces.iter().flatten() {
        if let Some(slot) = used.get_mut(index as usize) {
            *slot = true;
        }
    }
    let used_vertices: Vec<usize> = (0..vertices.len()).filter(|&vertex| used[vertex]).collect();
    let garbage_vertices: Vec<usize> = used_vertices.iter().copied().filter(|&vertex| garbage[vertex]).collect();
    if garbage_vertices.len().saturating_mul(used_vertices.len()) > COMPARISON_LIMIT {
        return true;
    }
    garbage_vertices.iter().any(|&vertex| {
        let position = vertices[vertex];
        used_vertices.iter().any(|&other| other != vertex && positions_may_join(position, vertices[other]))
    })
}

struct BufferView {
    buffer: Option<usize>,
    byte_offset: usize,
    byte_length: usize,
}

struct Accessor {
    buffer_view: Option<usize>,
    byte_offset: u32,
    byte_stride: u32,
    component_type: i32,
    count: u32,
    components: u32,
}

struct Primitive {
    mode: i32,
    positions: Vec<Option<usize>>,
    normals: Vec<Option<usize>>,
    texture_coords: Vec<Option<usize>>,
    indices: Option<usize>,
}

/// `SCompression_Open3DGC`: the compressed data's place.
struct Compression {
    buffer: Vec<u8>,
    offset: usize,
    count: usize,
}

struct Mesh {
    id: Vec<u8>,
    primitives: Vec<Primitive>,
    compressions: Vec<Compression>,
}

struct Image {
    has_data: bool,
    media_type: Vec<u8>,
}

/// `glTF::Asset`, with only what reaches the meshes or can fail kept.
struct Asset<'a> {
    directory: String,
    file_system: FileSystem,
    binary_extension: bool,
    materials_common: bool,
    accessors: Dict<'a, Accessor>,
    buffers: Dict<'a, Buffer>,
    buffer_views: Dict<'a, BufferView>,
    cameras: Dict<'a, ()>,
    images: Dict<'a, Image>,
    materials: Dict<'a, ()>,
    meshes: Dict<'a, Mesh>,
    nodes: Dict<'a, ()>,
    samplers: Dict<'a, ()>,
    scenes: Dict<'a, ()>,
    textures: Dict<'a, ()>,
    lights: Dict<'a, ()>,
}

/// `getCurrentAssetDir`.
fn asset_directory(path: &str) -> String {
    let slash = path.rfind('/');
    let backslash = path.rfind('\\');
    let position = match (slash, backslash) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    match position {
        Some(position) => path[..=position].to_string(),
        None => String::new(),
    }
}

/// The ASCII values an attribute name starts with (`GetAttribVector`).
enum AttributeKind {
    Position,
    Normal,
    TextureCoord,
    Other,
}

impl<'a> Asset<'a> {
    /// `Asset::Load` after the metadata: extensions, dictionaries, then the
    /// scene and everything it reaches.
    fn load(path: &str, files: &FileSystem, document: &'a Value, body: Option<Vec<u8>>, binary: bool) -> Result<Self, ImportError> {
        let mut asset = Asset {
            directory: asset_directory(path),
            file_system: files.clone(),
            binary_extension: binary,
            materials_common: false,
            accessors: Dict::new("accessors"),
            buffers: Dict::new("buffers"),
            buffer_views: Dict::new("bufferViews"),
            cameras: Dict::new("cameras"),
            images: Dict::new("images"),
            materials: Dict::new("materials"),
            meshes: Dict::new("meshes"),
            nodes: Dict::new("nodes"),
            samplers: Dict::new("samplers"),
            scenes: Dict::new("scenes"),
            textures: Dict::new("textures"),
            lights: Dict::new("lights"),
        };
        if binary {
            // `SetAsBinary`: the body buffer.
            asset.buffers.add(b"binary_glTF", Buffer::new(body));
        }
        if let Some(used) = find_array(document, "extensionsUsed")? {
            for name in used.elements().iter().filter(|value| value.is_string()) {
                match name.c_str() {
                    b"KHR_binary_glTF" => asset.binary_extension = true,
                    b"KHR_materials_common" => asset.materials_common = true,
                    _ => {}
                }
            }
        }
        asset.accessors.attach(document);
        asset.buffers.attach(document);
        asset.buffer_views.attach(document);
        asset.cameras.attach(document);
        asset.images.attach(document);
        asset.materials.attach(document);
        asset.meshes.attach(document);
        asset.nodes.attach(document);
        asset.samplers.attach(document);
        asset.scenes.attach(document);
        asset.textures.attach(document);
        if let Some(extensions) = find_object(document, "extensions")?
            && let Some(container) = extensions.member("KHR_materials_common").filter(|value| value.is_object())
        {
            asset.lights.object = container.member("lights").filter(|value| value.is_object());
        }
        if let Some(scene) = find_string(document, "scene")? {
            asset.retrieve_scene(scene.c_str())?;
        }
        Ok(asset)
    }

    fn retrieve_scene(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.scenes.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        if let Some(nodes) = member_of(object, "nodes", Value::is_array)? {
            for node in nodes.elements().iter().filter(|value| value.is_string()) {
                self.retrieve_node(node.c_str())?;
            }
        }
        self.scenes.add(id, ());
        Ok(())
    }

    /// `Node::Read`: children, meshes, camera, then the light.
    fn retrieve_node(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.nodes.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        if let Some(children) = member_of(object, "children", Value::is_array)? {
            for child in children.elements().iter().filter(|value| value.is_string()) {
                self.retrieve_node(child.c_str())?;
            }
        }
        if let Some(meshes) = member_of(object, "meshes", Value::is_array)? {
            for mesh in meshes.elements().iter().filter(|value| value.is_string()) {
                self.retrieve_mesh(mesh.c_str())?;
            }
        }
        if let Some(camera) = member_of(object, "camera", Value::is_string)? {
            self.retrieve_camera(camera.c_str())?;
        }
        if let Some(extensions) = member_of(object, "extensions", Value::is_object)?
            && self.materials_common
            && let Some(extension) = member_of(extensions, "KHR_materials_common", Value::is_object)?
            && let Some(light) = member_of(extension, "light", Value::is_string)?
        {
            let object = match self.lights.lookup(light.c_str())? {
                Lookup::Loaded(_) => None,
                Lookup::Read(object) => Some(object),
            };
            if object.is_some() {
                self.lights.add(light.c_str(), ());
            }
        }
        self.nodes.add(id, ());
        Ok(())
    }

    /// `Camera::Read`: the type is read as an integer, so a camera needs
    /// its "perspective" object unless the type is 1.
    fn retrieve_camera(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.cameras.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        let orthographic = read_int(object, "type", 0) == 1;
        let parameters = if orthographic { "orthographic" } else { "perspective" };
        if member_of(object, parameters, Value::is_object)?.is_none() {
            return fail("GLTF: Camera missing its parameters");
        }
        self.cameras.add(id, ());
        Ok(())
    }

    /// `Mesh::Read`: the primitives' accessors (attributes in member
    /// order), indices and material, then its extensions: Open3DGC
    /// compressed data is decoded, anything else fails.
    fn retrieve_mesh(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.meshes.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        let mut primitives = Vec::new();
        if let Some(list) = member_of(object, "primitives", Value::is_array)? {
            for value in list.elements() {
                let mut primitive = Primitive {
                    mode: read_int(value, "mode", MODE_TRIANGLES),
                    positions: Vec::new(),
                    normals: Vec::new(),
                    texture_coords: Vec::new(),
                    indices: None,
                };
                if let Some(attributes) = member_of(value, "attributes", Value::is_object)? {
                    for (name, accessor) in attributes.members() {
                        if !accessor.is_string() {
                            continue;
                        }
                        self.read_attribute(&mut primitive, name, accessor.c_str())?;
                    }
                }
                if let Some(indices) = member_of(value, "indices", Value::is_string)? {
                    primitive.indices = Some(self.retrieve_accessor(indices.c_str())?);
                }
                if let Some(material) = member_of(value, "material", Value::is_string)? {
                    self.retrieve_material(material.c_str())?;
                }
                primitives.push(primitive);
            }
        }
        let mut compressions = Vec::new();
        if let Some(extensions) = member_of(object, "extensions", Value::is_object)? {
            for (name, extension) in extensions.members() {
                let name = c_string(name);
                if name != b"Open3DGC-compression" {
                    return fail(format!("GLTF: Unknown mesh extension: \"{}\".", String::from_utf8_lossy(name)));
                }
                let compression = read_compression(extension)?;
                self.decode_open3dgc(id, &primitives, &compression)?;
                compressions.push(compression);
            }
        }
        let mesh = Mesh { id: id.to_vec(), primitives, compressions };
        self.meshes.add(id, mesh);
        Ok(())
    }

    /// `get_buf_offset`: the accessor's offset in its buffer, through a
    /// buffer view that must be there.
    fn buffer_offset(&self, slot: usize) -> Result<usize, ImportError> {
        let accessor = &self.accessors.objects[slot];
        let Some(view) = accessor.buffer_view else {
            return undefined("an Open3DGC accessor's missing buffer view used");
        };
        let view_offset = self.buffer_views.objects[view].byte_offset;
        Ok((accessor.byte_offset as usize).wrapping_add(view_offset))
    }

    fn accessor_count(&self, slot: Option<usize>) -> Result<usize, ImportError> {
        match slot {
            Some(slot) => Ok(self.accessors.objects[slot].count as usize),
            None => undefined("a missing Open3DGC accessor used"),
        }
    }

    /// `Decode_O3DGC`: the header's counts checked against the first
    /// primitive's accessors, the decoded data sized (left-out texture
    /// coordinate sets still count), the arrays placed at their accessors'
    /// offsets, the payload decoded, and the region marked in the buffer.
    fn decode_open3dgc(&mut self, mesh_id: &[u8], primitives: &[Primitive], compression: &Compression) -> Result<(), ImportError> {
        let buffer_slot = self.retrieve_buffer(&compression.buffer)?;
        // `LoadFromBuffer` with the count as a 32-bit `unsigned long`.
        let count = compression.count as u32 as usize;
        let stream = match self.buffers.objects[buffer_slot].data.as_deref() {
            _ if count == 0 => Vec::new(),
            None => return undefined("Open3DGC data copied from a buffer without data"),
            Some(data) => match compression.offset.checked_add(count).and_then(|end| data.get(compression.offset..end)) {
                Some(bytes) => bytes.to_vec(),
                None => return undefined("Open3DGC data copied from past its buffer"),
            },
        };
        let Some(mut decoder) = SC3DMCDecoder::decode_header(&stream)? else {
            return fail("GLTF: can not decode Open3DGC header.");
        };
        let Some(primitive) = primitives.first() else {
            return undefined("the first primitive of a mesh without any used");
        };
        let header = decoder.header();

        let mut size_coord_index = header.coord_index_count.wrapping_mul(3) as usize;
        let indices_count = self.accessor_count(primitive.indices)?;
        if indices_count != size_coord_index {
            return fail(format!("GLTF: Open3DGC. Compressed indices count ({size_coord_index}) not equal to uncompressed ({indices_count})."));
        }
        size_coord_index = size_coord_index.wrapping_mul(INDEX_SIZE);

        let mut size_coord = header.coord_count as usize;
        let Some(&position) = primitive.positions.first() else {
            return undefined("the first position accessor of a primitive without any used");
        };
        let positions_count = self.accessor_count(position)?;
        if positions_count != size_coord {
            return fail(format!("GLTF: Open3DGC. Compressed positions count ({size_coord}) not equal to uncompressed ({positions_count})."));
        }
        size_coord = size_coord.wrapping_mul(3 * FLOAT_SIZE);

        let mut size_normal = header.normal_count as usize;
        let Some(&normal) = primitive.normals.first() else {
            return undefined("the first normal accessor of a primitive without any used");
        };
        let normals_count = self.accessor_count(normal)?;
        if normals_count != size_normal {
            return fail(format!("GLTF: Open3DGC. Compressed normals count ({size_normal}) not equal to uncompressed ({normals_count})."));
        }
        size_normal = size_normal.wrapping_mul(3 * FLOAT_SIZE);

        let float_attributes = header.float_attributes.clone();
        let int_attributes = header.int_attributes.clone();
        let mut decoded_size = size_coord_index.wrapping_add(size_coord).wrapping_add(size_normal);
        let mut texture_coords = 0usize;
        for (attribute, entry) in float_attributes.iter().enumerate() {
            let mut size = entry.count as usize;
            if entry.kind != FLOAT_ATTRIBUTE_TEXCOORD {
                return fail(format!("GLTF: Open3DGC. Unsupported type of float attribute: {}", entry.kind));
            }
            if texture_coords < primitive.texture_coords.len() {
                // The set is looked up by the attribute's index.
                let Some(&set) = primitive.texture_coords.get(attribute) else {
                    return undefined("a texture coordinate accessor past the primitive's used");
                };
                let set_count = self.accessor_count(set)?;
                if set_count != size {
                    return fail(format!("GLTF: Open3DGC. Compressed texture coordinates count ({size}) not equal to uncompressed ({set_count})."));
                }
                texture_coords += 1;
            } else {
                decoder.disable_float_attribute(attribute);
            }
            size = size.wrapping_mul(entry.dimension as usize * FLOAT_SIZE);
            decoded_size = decoded_size.wrapping_add(size);
        }
        for entry in &int_attributes {
            if entry.kind > INT_ATTRIBUTE_LAST {
                return fail(format!("GLTF: Open3DGC. Unsupported type of int attribute: {}", entry.kind));
            }
            let size = (entry.count as usize).wrapping_mul(entry.dimension as usize * LONG_SIZE);
            decoded_size = decoded_size.wrapping_add(size);
        }

        check_allocation(decoded_size as u64, 1)?;
        let mut memory = Memory::new(decoded_size);
        let Some(indices) = primitive.indices else {
            return undefined("a missing Open3DGC accessor used");
        };
        let Some(position) = position else {
            return undefined("a missing Open3DGC accessor used");
        };
        let mut layout = Layout {
            coord_index: Some(self.buffer_offset(indices)?),
            coord: Some(self.buffer_offset(position)?),
            normal: None,
            float_attributes: vec![None; float_attributes.len()],
        };
        if size_normal != 0 {
            let Some(normal) = normal else {
                return undefined("a missing Open3DGC accessor used");
            };
            layout.normal = Some(self.buffer_offset(normal)?);
        }
        let mut texture_coords = 0usize;
        for attribute in 0..float_attributes.len() {
            if texture_coords >= primitive.texture_coords.len() {
                continue;
            }
            let Some(&Some(set)) = primitive.texture_coords.get(attribute) else {
                return undefined("a missing Open3DGC accessor used");
            };
            layout.float_attributes[attribute] = Some(self.buffer_offset(set)?);
            texture_coords += 1;
        }

        if !decoder.decode_payload(&mut memory, &layout)? {
            return fail("GLTF: can not decode Open3DGC data.");
        }

        // `EncodedRegion_Mark`.
        let buffer = &mut self.buffers.objects[buffer_slot];
        let offset = compression.offset;
        if offset > buffer.byte_length {
            return fail(format!("GLTF: incorrect offset value ({offset}) for marking encoded region."));
        }
        if offset.wrapping_add(compression.count) > buffer.byte_length {
            return fail(format!("GLTF: encoded region with offset/length ({offset}/{}) is out of range.", compression.count));
        }
        let region = Region { offset, decoded: memory, decoded_length: decoded_size, id: mesh_id.to_vec() };
        buffer.regions.push(region);
        buffer.byte_length = buffer.byte_length.wrapping_add(decoded_size.wrapping_sub(compression.count));
        Ok(())
    }

    /// One attribute: its semantic prefix, then `_n` (MSVC `atoi`) as the
    /// set index.
    fn read_attribute(&mut self, primitive: &mut Primitive, name: &[u8], accessor_id: &[u8]) -> Result<(), ImportError> {
        let name = name.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let prefixes: [(&[u8], AttributeKind); 7] = [
            (b"POSITION", AttributeKind::Position),
            (b"NORMAL", AttributeKind::Normal),
            (b"TEXCOORD", AttributeKind::TextureCoord),
            (b"COLOR", AttributeKind::Other),
            (b"JOINT", AttributeKind::Other),
            (b"JOINTMATRIX", AttributeKind::Other),
            (b"WEIGHT", AttributeKind::Other),
        ];
        let Some((prefix, kind)) = prefixes.into_iter().find(|(prefix, _)| name.starts_with(prefix)) else {
            return Ok(());
        };
        let index = if name.get(prefix.len()) == Some(&b'_') { crate::gltf2::asset::msvc_atoi(&name[prefix.len() + 1..]) } else { 0 };
        if index < 0 {
            // `resize(idx + 1)` wraps to 0 and the store runs past the list.
            return undefined("a negative glTF attribute set index");
        }
        if index >= MAX_ATTRIBUTE_SETS {
            return fail("bad allocation");
        }
        let index = index as usize;
        let slot = self.retrieve_accessor(accessor_id)?;
        let list = match kind {
            AttributeKind::Position => &mut primitive.positions,
            AttributeKind::Normal => &mut primitive.normals,
            AttributeKind::TextureCoord => &mut primitive.texture_coords,
            AttributeKind::Other => return Ok(()),
        };
        if list.len() <= index {
            list.resize(index + 1, None);
        }
        list[index] = Some(slot);
        Ok(())
    }

    fn retrieve_accessor(&mut self, id: &[u8]) -> Result<usize, ImportError> {
        let object = match self.accessors.lookup(id)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut accessor = Accessor {
            buffer_view: None,
            byte_offset: read_int(object, "byteOffset", 0) as u32,
            byte_stride: read_int(object, "byteStride", 0) as u32,
            component_type: read_int(object, "componentType", COMPONENT_BYTE),
            count: read_int(object, "count", 0) as u32,
            components: 1,
        };
        if let Some(view) = read_id(object, "bufferView") {
            accessor.buffer_view = Some(self.retrieve_buffer_view(view)?);
        }
        accessor.components = match read_id(object, "type") {
            Some(b"VEC2") => 2,
            Some(b"VEC3") => 3,
            Some(b"VEC4") | Some(b"MAT2") => 4,
            Some(b"MAT3") => 9,
            Some(b"MAT4") => 16,
            _ => 1,
        };
        Ok(self.accessors.add(id, accessor))
    }

    fn retrieve_buffer_view(&mut self, id: &[u8]) -> Result<usize, ImportError> {
        let object = match self.buffer_views.lookup(id)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut view = BufferView { buffer: None, byte_offset: 0, byte_length: 0 };
        if let Some(buffer) = read_id(object, "buffer") {
            view.buffer = Some(self.retrieve_buffer(buffer)?);
        }
        view.byte_offset = read_int(object, "byteOffset", 0) as u32 as usize;
        view.byte_length = read_int(object, "byteLength", 0) as u32 as usize;
        Ok(self.buffer_views.add(id, view))
    }

    /// `Buffer::Read` (with `TranslateId`'s old binary id).
    fn retrieve_buffer(&mut self, id: &[u8]) -> Result<usize, ImportError> {
        let id: &[u8] = if self.binary_extension && id == b"KHR_binary_glTF" { b"binary_glTF" } else { id };
        let object = match self.buffers.lookup(id)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let stated_length = read_size(object, "byteLength");
        let mut data = None;
        match member_of(object, "uri", Value::is_string)? {
            None => {
                if stated_length > 0 {
                    return fail("GLTF: buffer with non-zero length missing the \"uri\" attribute");
                }
            }
            Some(uri) => {
                if let Some(data_uri) = parse_data_uri(uri.as_bytes()) {
                    if data_uri.base64 {
                        let decoded = base64::decode(data_uri.data)?;
                        if stated_length > 0 && decoded.len() != stated_length {
                            return fail("GLTF: buffer length differs from its byteLength");
                        }
                        if !decoded.is_empty() {
                            data = Some(decoded);
                        }
                    } else {
                        if stated_length != data_uri.data.len() {
                            return fail("GLTF: buffer length differs from its byteLength");
                        }
                        data = Some(data_uri.data.to_vec());
                    }
                } else if stated_length > 0 {
                    let mut directory = self.directory.clone();
                    if !directory.is_empty() && !directory.ends_with('/') {
                        directory.push('/');
                    }
                    let mut path = directory.into_bytes();
                    path.extend_from_slice(uri.c_str());
                    let Some(contents) = self.file_system.open_bytes(&path) else {
                        return fail("GLTF: could not open referenced file");
                    };
                    let Some(contents) = contents.get(..stated_length) else {
                        return fail("GLTF: error while reading referenced file");
                    };
                    data = Some(contents.to_vec());
                }
            }
        }
        Ok(self.buffers.add(id, Buffer::new(data)))
    }

    /// `buffers.Get` of a buffer read before.
    fn loaded_buffer(&self, id: &[u8]) -> Option<usize> {
        let id: &[u8] = if self.binary_extension && id == b"KHR_binary_glTF" { b"binary_glTF" } else { id };
        self.buffers.by_id.get(id).copied()
    }

    /// `Material::Read`: the textures its common or KHR_materials_common
    /// values name.
    fn retrieve_material(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.materials.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        if let Some(values) = member_of(object, "values", Value::is_object)? {
            self.read_material_values(values)?;
        }
        if let Some(extensions) = member_of(object, "extensions", Value::is_object)?
            && self.materials_common
            && let Some(extension) = member_of(extensions, "KHR_materials_common", Value::is_object)?
            && let Some(values) = member_of(extension, "values", Value::is_object)?
        {
            self.read_material_values(values)?;
        }
        self.materials.add(id, ());
        Ok(())
    }

    /// `ReadMaterialProperty` for ambient, diffuse and specular.
    fn read_material_values(&mut self, values: &Value) -> Result<(), ImportError> {
        for property in ["ambient", "diffuse", "specular"] {
            if let Some(value) = values.member(property)
                && value.is_string()
            {
                self.retrieve_texture(value.c_str())?;
            }
        }
        Ok(())
    }

    fn retrieve_texture(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.textures.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        if let Some(source) = read_id(object, "source") {
            self.retrieve_image(source)?;
        }
        if let Some(sampler) = read_id(object, "sampler") {
            let sampler_object = match self.samplers.lookup(sampler)? {
                Lookup::Loaded(_) => None,
                Lookup::Read(object) => Some(object),
            };
            if sampler_object.is_some() {
                self.samplers.add(sampler, ());
            }
        }
        self.textures.add(id, ());
        Ok(())
    }

    /// `Image::Read`: a KHR_binary_glTF buffer view, else a data URI.
    fn retrieve_image(&mut self, id: &[u8]) -> Result<(), ImportError> {
        let object = match self.images.lookup(id)? {
            Lookup::Loaded(_) => return Ok(()),
            Lookup::Read(object) => object,
        };
        let mut image = Image { has_data: false, media_type: Vec::new() };
        if let Some(extensions) = member_of(object, "extensions", Value::is_object)?
            && self.binary_extension
            && let Some(extension) = member_of(extensions, "KHR_binary_glTF", Value::is_object)?
        {
            if let Some(media_type) = extension.member("mimeType").filter(|value| value.is_string()) {
                image.media_type = media_type.as_bytes().to_vec();
            }
            if let Some(view_id) = read_id(extension, "bufferView") {
                let view = self.retrieve_buffer_view(view_id)?;
                let view = &self.buffer_views.objects[view];
                let Some(buffer) = view.buffer else {
                    return undefined("a glTF image buffer view without a buffer");
                };
                let data = self.buffers.objects[buffer].data.as_deref();
                if view.byte_length > 0 {
                    let end = view.byte_offset.checked_add(view.byte_length);
                    let in_range = matches!((data, end), (Some(data), Some(end)) if end <= data.len());
                    if !in_range {
                        return undefined("glTF image data copied from outside its buffer");
                    }
                }
                image.has_data = view.byte_length > 0;
            }
        }
        if !image.has_data
            && let Some(uri) = member_of(object, "uri", Value::is_string)?
            && let Some(data_uri) = parse_data_uri(uri.as_bytes())
        {
            image.media_type = data_uri.media_type.to_vec();
            if data_uri.base64 {
                let decoded = base64::decode(data_uri.data)?;
                image.has_data = !decoded.is_empty();
            }
        }
        self.images.add(id, image);
        Ok(())
    }

    /// `GetPointer`: the buffer data and the accessor's start in it, or
    /// the selected region's decoded data when the start falls in it.
    fn pointer(&self, accessor: &Accessor) -> Option<(Source<'_>, usize)> {
        let view = &self.buffer_views.objects[accessor.buffer_view?];
        let buffer = &self.buffers.objects[view.buffer?];
        let data = buffer.data.as_deref()?;
        let start = (accessor.byte_offset as usize).wrapping_add(view.byte_offset);
        if let Some(current) = buffer.current.get() {
            let region = &buffer.regions[current];
            let end = region.offset.wrapping_add(region.decoded_length);
            if start >= region.offset && start < end {
                return Some((Source::Decoded(&region.decoded), start - region.offset));
            }
        }
        Some((Source::Raw(data), start))
    }

    /// `ExtractData` into 12-byte elements (`aiVector3D`): `None` without
    /// data; each element's bytes copied raw into the start of its slot.
    fn extract_elements(&self, slot: usize) -> Result<Option<Elements>, ImportError> {
        let accessor = &self.accessors.objects[slot];
        let Some((data, start)) = self.pointer(accessor) else {
            return Ok(None);
        };
        let element_size = accessor.components as usize * component_size(accessor.component_type)?;
        let stride = if accessor.byte_stride != 0 { accessor.byte_stride as usize } else { element_size };
        scene::ai_assert(element_size <= 12, "elemSize <= targetElemSize")?;
        let view_length = self.view_length(accessor);
        let fits = (accessor.count as usize).wrapping_mul(stride) <= view_length;
        scene::ai_assert(fits, "count * stride <= bufferView->byteLength")?;
        let mut values = Vec::with_capacity(accessor.count as usize);
        let mut garbage = Vec::with_capacity(accessor.count as usize);
        for index in 0..accessor.count as usize {
            let from = start.wrapping_add(index.wrapping_mul(stride));
            let (bytes, written) = data.read(from, element_size, "GLTF: accessor reads past its buffer")?;
            let mut raw = [0u8; 12];
            raw[..element_size].copy_from_slice(bytes);
            let component = |offset: usize| f32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
            values.push([component(0), component(4), component(8)]);
            garbage.push(!written);
        }
        Ok(Some(Elements { values, garbage }))
    }

    /// Elements that must all be defined (positions).
    fn extract(&self, slot: usize) -> Result<Option<Vec<[f32; 3]>>, ImportError> {
        let Some(elements) = self.extract_elements(slot)? else {
            return Ok(None);
        };
        if elements.garbage.contains(&true) {
            return undefined("decoded Open3DGC positions read where the decoder wrote nothing");
        }
        Ok(Some(elements.values))
    }

    /// The byte length of the accessor's buffer view (none: 0).
    fn view_length(&self, accessor: &Accessor) -> usize {
        accessor.buffer_view.map_or(0, |view| self.buffer_views.objects[view].byte_length)
    }

    /// `Indexer::GetUInt` (the indexer was checked valid on creation).
    fn index(&self, slot: usize, position: u32) -> Result<u32, ImportError> {
        let accessor = &self.accessors.objects[slot];
        let element_size = accessor.components as usize * component_size(accessor.component_type)?;
        let Some((data, start)) = self.pointer(accessor) else {
            return Err(scene::assertion_failed("data"));
        };
        let stride = if accessor.byte_stride != 0 { accessor.byte_stride as usize } else { element_size };
        let offset = (position as i32 as usize).wrapping_mul(stride);
        scene::ai_assert(offset < self.view_length(accessor), "i * stride < accessor.bufferView->byteLength")?;
        if element_size > 4 {
            return undefined("GLTF: index elements wider than the 4 bytes they are copied into");
        }
        let from = start.wrapping_add(offset);
        let (bytes, written) = data.read(from, element_size, "GLTF: indices read past their buffer")?;
        if !written {
            return undefined("decoded Open3DGC indices read where the decoder wrote nothing");
        }
        let mut raw = [0u8; 4];
        raw[..element_size].copy_from_slice(bytes);
        Ok(u32::from_le_bytes(raw))
    }
}

/// The faces of a primitive (`ImportMeshes`), from its indices or from
/// the vertex count: strips and fans only work with exactly three points.
fn build_faces(mode: i32, count: u32, index: &dyn Fn(u32) -> Result<u32, ImportError>) -> Result<Option<Vec<Vec<u32>>>, ImportError> {
    let mut faces: Vec<Vec<u32>> = Vec::new();
    match mode {
        MODE_POINTS => {
            for position in 0..count {
                faces.push(vec![index(position)?]);
            }
        }
        MODE_LINES => {
            let face_count = count / 2;
            for face in 0..face_count {
                faces.push(vec![index(face * 2)?, index(face * 2 + 1)?]);
            }
        }
        MODE_LINE_LOOP | MODE_LINE_STRIP => {
            let face_count = count.wrapping_sub(if mode == MODE_LINE_STRIP { 1 } else { 0 });
            if face_count > MAX_FACES {
                return fail("bad allocation");
            }
            if face_count == 0 || (mode == MODE_LINE_LOOP && count < 2) {
                return undefined("a glTF line primitive writes past its faces");
            }
            faces.push(vec![index(0)?, index(1)?]);
            for position in 2..count {
                let previous = faces[position as usize - 2][1];
                faces.push(vec![previous, index(position)?]);
            }
            if mode == MODE_LINE_LOOP {
                let last = faces[count as usize - 2][1];
                let first = faces[0][0];
                faces.push(vec![last, first]);
            }
        }
        MODE_TRIANGLES => {
            let face_count = count / 3;
            for face in 0..face_count {
                faces.push(vec![index(face * 3)?, index(face * 3 + 1)?, index(face * 3 + 2)?]);
            }
        }
        MODE_TRIANGLE_STRIP | MODE_TRIANGLE_FAN => {
            let face_count = count.wrapping_sub(2);
            if face_count > MAX_FACES {
                return fail("bad allocation");
            }
            if count != 3 {
                // Past the first face each one reads the next, unset face.
                return undefined("a glTF strip or fan of more than three points reads unset faces");
            }
            faces.push(vec![index(0)?, index(1)?, index(2)?]);
        }
        _ => return Ok(None),
    }
    Ok(Some(faces))
}

/// `ImportEmbeddedTextures`' format hint: a media type without '/' is
/// read through a null pointer.
fn check_embedded_textures(asset: &Asset) -> Result<(), ImportError> {
    for image in &asset.images.objects {
        let c_string = image.media_type.split(|&byte| byte == 0).next().unwrap_or(&[]);
        if image.has_data && !image.media_type.is_empty() && !c_string.contains(&b'/') {
            return undefined("an embedded glTF image media type without '/'");
        }
    }
    Ok(())
}

/// `ImportMeshes`: one mesh per primitive, in load order; a compressed
/// mesh first selects its decoded region in each compression's buffer.
fn import_meshes(asset: &Asset) -> Result<Vec<scene::Mesh>, ImportError> {
    let mut meshes = Vec::new();
    for mesh in &asset.meshes.objects {
        for compression in &mesh.compressions {
            if mesh.primitives.len() > 2 {
                return fail("GLTF: When using Open3DGC compression then only one primitive per mesh are allowed.");
            }
            let Some(buffer) = asset.loaded_buffer(&compression.buffer) else {
                return undefined("glTF compressed data's buffer not loaded");
            };
            asset.buffers.objects[buffer].select_region(&mesh.id)?;
        }
        for primitive in &mesh.primitives {
            let primitive_types = match primitive.mode {
                MODE_POINTS => PRIMITIVE_POINT,
                MODE_LINES | MODE_LINE_LOOP | MODE_LINE_STRIP => PRIMITIVE_LINE,
                MODE_TRIANGLES | MODE_TRIANGLE_STRIP | MODE_TRIANGLE_FAN => PRIMITIVE_TRIANGLE,
                _ => 0,
            };
            let mut output = scene::Mesh {
                primitive_types,
                ..scene::Mesh::default()
            };
            if let Some(Some(position)) = primitive.positions.first() {
                output.vertex_count = asset.accessors.objects[*position].count as usize;
                output.vertices = asset.extract(*position)?;
                if output.vertices.is_none() && output.vertex_count > 0 {
                    return undefined("glTF positions without data");
                }
            }
            let vertex_count = output.vertex_count;
            // Vertices with garbage normals or texture coordinates.
            let mut garbage = vec![false; vertex_count];
            let mut mark_garbage = |elements: &Elements| {
                for (vertex, &bad) in elements.garbage.iter().enumerate().take(vertex_count) {
                    garbage[vertex] |= bad;
                }
            };
            if let Some(Some(normal)) = primitive.normals.first()
                && let Some(elements) = asset.extract_elements(*normal)?
            {
                mark_garbage(&elements);
                output.normals = Some(elements.values);
            }
            for set in 0..primitive.texture_coords.len().min(MAX_TEXTURE_COORDS) {
                let Some(slot) = primitive.texture_coords[set] else {
                    return undefined("a glTF texture coordinate set missing before a later one");
                };
                let coordinates = asset.extract_elements(slot)?;
                output.uv_components[set] = asset.accessors.objects[slot].components;
                match coordinates {
                    None if vertex_count > 0 => return undefined("glTF texture coordinates without data"),
                    None => {}
                    Some(elements) => {
                        mark_garbage(&elements);
                        let mut values = elements.values;
                        if values.len() < vertex_count {
                            return undefined("fewer glTF texture coordinates than vertices");
                        }
                        values.truncate(vertex_count);
                        for value in values.iter_mut() {
                            value[1] = 1.0 - value[1];
                        }
                        output.texture_coords[set] = Some(values);
                    }
                }
            }

            let faces = match primitive.indices {
                Some(indices) => {
                    // The indexer sizes its elements up front, then the
                    // importer asserts it has data.
                    let accessor = &asset.accessors.objects[indices];
                    component_size(accessor.component_type)?;
                    scene::ai_assert(asset.pointer(accessor).is_some(), "data.IsValid()")?;
                    let count = accessor.count;
                    let read = |position: u32| asset.index(indices, position);
                    build_faces(primitive.mode, count, &read)?
                }
                None => {
                    let read = |position: u32| Ok(position);
                    build_faces(primitive.mode, vertex_count as u32, &read)?
                }
            };
            if let Some(face_list) = &faces {
                let valid = face_list.iter().flatten().all(|&index| (index as usize) < vertex_count);
                scene::ai_assert(valid, "validRes")?;
            }
            let has_faces = faces.as_ref().is_some_and(|faces| !faces.is_empty());
            if output.vertices.is_some() && vertex_count > 0 && has_faces {
                // Joining the vertices reads every stream (and the face
                // indices) within the vertex count.
                let short_normals = output.normals.as_ref().is_some_and(|normals| normals.len() < vertex_count);
                let index_out_of_range = faces.iter().flatten().flatten().any(|&index| index as usize >= vertex_count);
                if short_normals || index_out_of_range {
                    return undefined("glTF faces or normals beyond the vertices");
                }
                let vertices = output.vertices.as_deref().unwrap_or(&[]);
                let face_list = faces.as_deref().unwrap_or(&[]);
                if garbage.contains(&true) && garbage_decides(&vertices[..vertex_count.min(vertices.len())], face_list, &garbage) {
                    return undefined("garbage normals or texture coordinates decide which vertices join");
                }
            }
            if let Some(normals) = output.normals.as_mut() {
                normals.truncate(vertex_count);
            }
            if let Some(vertices) = output.vertices.as_mut() {
                vertices.truncate(vertex_count);
            }
            output.faces = faces;
            meshes.push(output);
        }
    }
    Ok(meshes)
}

/// `glTFImporter::InternReadFile`.
pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    let document = read_document(bytes)?;
    let json = parse_json(&document.json)?;
    let version = read_version(&json, document.binary)?;
    if version.first() != Some(&b'1') {
        return Ok(Scene::default());
    }
    let asset = Asset::load(path, files, &json, document.body, document.binary)?;
    check_embedded_textures(&asset)?;
    let meshes = import_meshes(&asset)?;
    Ok(Scene { meshes })
}
