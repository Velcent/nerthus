//! Assimp's Blender importer (`BlenderImporter`): a .blend file (or its
//! gzip stream) read through its own DNA, the scene's objects walked from
//! its master collection or object list, and every mesh object's faces and
//! polygons split by material into meshes, with their modifiers applied.
//!
//! Parented objects are kept in a `std::set` ordered by a comparison of
//! names up to the left one's length, so an object whose name another's
//! begins with (`Cube` and `Cube.001`) counts as the same object and is
//! left out, and the order is MSVC's tree's. What the C++ leaves undefined
//! (reads past an array, null materials, type confusion) is an error.

mod dna;
mod modifiers;
mod structures;

use std::collections::{BTreeMap, HashSet};

use dna::{Block, CustomArray, Database, Failure, Outcome, STACK_BUDGET, block_capacity, import_error, next_block, parse_dna, text, undefined};
use structures::{CD_MLOOPUV, Element, Kind, MLoopUV, Mesh as BlendMesh, Scene as BlendScene};

use crate::compression::inflate_gzip;
use crate::msvc::{UniqueSet, sort};
use crate::postprocess::sort_would_split;
use crate::scene::{self, ImportError, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, check_allocation};
use crate::stream_reader::StreamReader;

/// `BLENDER` at the start of the file.
const TOKEN: &[u8] = b"BLENDER";

/// The file's header: the token, pointer size, byte order and version.
const HEADER_SIZE: usize = 12;

/// The conversion thread's stack: some twenty times what the port's
/// frames take at the C++'s budget (reserved, not committed).
const CONVERSION_STACK: usize = 64 << 20;

/// The stack frames of `ParseSubCollection` and `ConvertNode` (MSVC
/// x64), each nested level taking another.
const SUB_COLLECTION_FRAME: u64 = 400;
const NODE_FRAME: u64 = 416;

/// `Object::Type`s with data the importer looks at.
const TYPE_MESH: i32 = 1;
const TYPE_LAMP: i32 = 10;
const TYPE_CAMERA: i32 = 11;

/// `Tex::Type_IMAGE`; the procedural types become placeholders.
const TEX_IMAGE: i32 = 8;

/// `sizeof` on MSVC x64 of what mesh conversion allocates.
const VECTOR_SIZE: u64 = 12;
const COLOR_SIZE: u64 = 16;
const FACE_SIZE: u64 = 16;
const INDEX_SIZE: u64 = 4;
const NODE_POINTER_SIZE: u64 = 8;

/// The texture slots of a material (`mtex[18]`).
const TEXTURE_SLOTS: usize = 18;

/// `aiMesh::mTextureCoords` indices that land somewhere: sets 8 to 11 lie
/// over `mNumUVComponents` (nothing after the conversion reads them back
/// as coordinates, so they are dropped), set 12 on would be `mFaces`.
const OVERLAID_SETS: usize = 12;

/// `ParseMagicToken`: the stream to read, or the reason the file is not
/// a Blender file; a gzip stream that does not inflate throws.
fn parse_magic_token(bytes: &[u8]) -> Result<Result<Vec<u8>, &'static str>, ImportError> {
    let mut magic = [0u8; 8];
    let head = bytes.len().min(7);
    magic[..head].copy_from_slice(&bytes[..head]);
    if &magic[..7] == TOKEN {
        return Ok(Ok(bytes.to_vec()));
    }
    if magic[0] != 0x1f || magic[1] != 0x8b {
        return Ok(Err("BLENDER magic bytes are missing, couldn't find GZIP header either"));
    }
    if magic[2] != 8 {
        return Ok(Err("Unsupported GZIP compression method"));
    }
    let inflated = inflate_gzip(bytes)?;
    // A memory stream reads whole items only.
    if inflated.len() >= 7 && &inflated[..7] == TOKEN {
        return Ok(Ok(inflated));
    }
    Ok(Err("Found no BLENDER magic word in decompressed GZIP file"))
}

/// `BlenderImporter::CanRead`.
pub(crate) fn can_read(bytes: &[u8]) -> Result<bool, ImportError> {
    Ok(parse_magic_token(bytes)?.is_ok())
}

/// A failure that reached the top: a `Blender::Error` no field read
/// caught fails the import like any other.
fn finish(failure: Failure) -> ImportError {
    match failure {
        Failure::Dna(message) => ImportError(message),
        Failure::Import(error) => error,
    }
}

/// `BlenderImporter::InternReadFile`. The conversion recurses as the
/// C++'s does, and up to the C++'s stack budget the port's frames take
/// about twice as much (more than a default thread has), so it runs on a
/// thread of its own.
pub(crate) fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let stream = match parse_magic_token(bytes)? {
        Ok(stream) => stream,
        Err(reason) => return Err(ImportError(format!("BLEND: {reason}"))),
    };
    let worker = std::thread::Builder::new().stack_size(CONVERSION_STACK);
    std::thread::scope(|scope| {
        let Ok(conversion) = worker.spawn_scoped(scope, || convert_stream(&stream)) else {
            return Err(ImportError("could not start a thread for the Blender conversion".to_string()));
        };
        match conversion.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

/// The file after its magic token: blocks, scene, conversion.
fn convert_stream(stream: &[u8]) -> Result<Scene, ImportError> {
    let pointer_64 = stream.get(7) == Some(&b'-');
    let little = stream.get(8) == Some(&b'v');
    let body = stream.get(HEADER_SIZE..).unwrap_or(&[]);
    let reader = StreamReader::new(body, !little)?;
    let mut db = Database::new(reader, pointer_64);
    let result = (|| {
        parse_blend_file(&mut db)?;
        let blend_scene = extract_scene(&mut db)?;
        convert_blend_file(&mut db, &blend_scene)
    })();
    result.map_err(finish)
}

/// `ParseBlendFile`: every block up to `ENDB`, the DNA parsed where it
/// is, the rest sorted by address (MSVC's sort).
fn parse_blend_file(db: &mut Database) -> Outcome<()> {
    let mut capacity = block_capacity();
    capacity.reserve(128)?;
    let mut found_dna = false;
    let mut previous = Block { start: 0, id: [0; 4], id_length: 0, size: 0, address: 0, dna_index: 0, num: 0 };
    loop {
        let block = next_block(&mut db.reader, db.pointer_64, &previous)?;
        previous = block;
        if block.id() == b"ENDB" {
            break;
        }
        if block.id() == b"DNA1" {
            parse_dna(&mut db.reader, db.pointer_64, &mut db.dna)?;
            found_dna = true;
            continue;
        }
        capacity.push_to(db.blocks.len() + 1)?;
        db.blocks.push(block);
    }
    if !found_dna {
        return import_error("SDNA not found");
    }
    db.dna.resolve_field_types();
    sort(&mut db.blocks, |a, b| a.address < b.address);
    Ok(())
}

/// `ExtractScene`: the first block (by address) of the `Scene` structure.
fn extract_scene(db: &mut Database) -> Outcome<BlendScene> {
    let Some(&structure) = db.dna.indices.get(b"Scene".as_slice()) else {
        return import_error("There is no `Scene` structure record");
    };
    let Some(block) = db.blocks.iter().find(|block| block.dna_index as usize == structure) else {
        return import_error("There is not a single `Scene` record to load");
    };
    let start = block.start;
    db.set_position(start)?;
    let mut blend_scene = BlendScene::default();
    db.convert_value(structure, &mut blend_scene, 0)?;
    Ok(blend_scene)
}

/// A material slot as `materials_raw` holds it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MaterialRef {
    Null,
    Material(usize),
    Default,
}

/// `ConversionData`: the meshes made, the materials they use, every
/// node's mesh indices, and the parented objects not yet placed.
pub(super) struct Conversion {
    pub meshes: Vec<scene::Mesh>,
    materials: Vec<MaterialRef>,
    pub nodes: Vec<Vec<u32>>,
    objects: Vec<usize>,
}

/// `ObjectCompare`: `strncmp(left, right, strlen(left)) < 0`.
fn name_less(db: &Database, left: usize, right: usize) -> Outcome<bool> {
    let left = db.store.objects[left].id.c_str(0);
    let right = db.store.objects[right].id.c_str(0);
    for (index, &byte) in left.iter().enumerate() {
        let other = right.get(index).copied().unwrap_or(0);
        if byte != other {
            return Ok(byte < other);
        }
    }
    Ok(false)
}

/// The elements of a list from its first element along `next`, which
/// must be of the list's type; a list that comes round again never ends.
fn walk_list(db: &Database, first: Option<Element>, kind: Kind, next: impl Fn(&Database, usize) -> Option<usize>) -> Outcome<Vec<usize>> {
    let Some(first) = first else {
        return Ok(Vec::new());
    };
    if first.kind != kind {
        return undefined("a Blender list's first element read as a type it is not");
    }
    walk_from(db, Some(first.index), next)
}

fn walk_from(db: &Database, first: Option<usize>, next: impl Fn(&Database, usize) -> Option<usize>) -> Outcome<Vec<usize>> {
    let mut elements = Vec::new();
    let mut seen = HashSet::new();
    let mut current = first;
    while let Some(index) = current {
        if !seen.insert(index) {
            return import_error("endless loop: a Blender list that comes round again");
        }
        elements.push(index);
        current = next(db, index);
    }
    Ok(elements)
}

/// `ConvertBlendFile`.
fn convert_blend_file(db: &mut Database, blend_scene: &BlendScene) -> Outcome<Scene> {
    let mut conversion = Conversion { meshes: Vec::new(), materials: Vec::new(), nodes: Vec::new(), objects: Vec::new() };
    if let Some(master) = blend_scene.master_collection {
        parse_sub_collection(db, &mut conversion, master, 0)?;
    } else {
        let bases = walk_list(db, blend_scene.base.first, Kind::Base, |db, index| db.store.bases[index].next)?;
        let mut no_parents = Vec::new();
        let mut set = UniqueSet::new();
        let mut less = |left: &usize, right: &usize| name_less(db, *left, *right);
        for &base in &bases {
            if let Some(object) = db.store.bases[base].object {
                if db.store.objects[object].parent.is_none() {
                    no_parents.push(object);
                } else {
                    set.insert(object, &mut less)?;
                }
            }
        }
        let active = walk_from(db, blend_scene.basact, |db, index| db.store.bases[index].next)?;
        for &base in &active {
            if let Some(object) = db.store.bases[base].object
                && db.store.objects[object].parent.is_some()
            {
                set.insert(object, &mut less)?;
            }
        }
        if no_parents.is_empty() {
            return import_error("Expected at least one object with no parent");
        }
        conversion.objects = set.in_order();
        check_allocation(no_parents.len() as u64, NODE_POINTER_SIZE)?;
        for object in no_parents {
            convert_node(db, &mut conversion, object, 0)?;
        }
    }
    build_materials(db, &conversion)?;
    // SortByPType walks the nodes when it splits a mesh; a node mesh index
    // past the meshes is read past its table there.
    let mesh_count = conversion.meshes.len() as u64;
    let broken = conversion.nodes.iter().flatten().any(|&index| index as u64 >= mesh_count);
    if broken && sort_would_split(&conversion.meshes)? {
        return undefined("a Blender node's mesh index past the meshes read when the meshes are sorted");
    }
    Ok(Scene { meshes: conversion.meshes })
}

/// `ParseSubCollection`: the collection's objects as nodes, then a node
/// for each child collection (numbered with the entries that have none,
/// so an empty entry before a full one writes past the node's children).
fn parse_sub_collection(db: &mut Database, conversion: &mut Conversion, collection: usize, stack: u64) -> Outcome<()> {
    let stack = stack + SUB_COLLECTION_FRAME;
    if stack > STACK_BUDGET {
        return undefined("Blender collections nested past the stack");
    }
    let objects_first = db.store.collections[collection].gobject.first;
    let children_first = db.store.collections[collection].children.first;
    let entries = walk_list(db, objects_first, Kind::CollectionObject, |db, index| db.store.collection_objects[index].next)?;
    let objects: Vec<usize> = entries.iter().filter_map(|&entry| db.store.collection_objects[entry].ob).collect();
    let children = walk_list(db, children_first, Kind::CollectionChild, |db, index| db.store.collection_children[index].next)?;
    let child_collections = children.iter().filter(|&&child| db.store.collection_children[child].collection.is_some()).count();
    let node_children = objects.len() + child_collections;
    check_allocation(node_children as u64, NODE_POINTER_SIZE)?;
    for &object in &objects {
        convert_node(db, conversion, object, stack)?;
    }
    for (slot, &child) in (objects.len()..).zip(children.iter()) {
        let Some(child_collection) = db.store.collection_children[child].collection else {
            continue;
        };
        if slot >= node_children {
            return undefined("a Blender collection's node written past its children");
        }
        parse_sub_collection(db, conversion, child_collection, stack)?;
    }
    Ok(())
}

/// `CheckActualType`: the data must have been read as the structure the
/// object type needs.
fn check_actual_type(db: &Database, data: Element, expected: &[u8]) -> Outcome<()> {
    let Some(&structure) = db.dna_types.get(&data) else {
        return undefined("a Blender object's data compared by a type name it never got");
    };
    let actual = &db.dna.structures[structure].name;
    if actual != expected {
        return import_error(format!(
            "Expected object at {:x} to be of type `{}`, but it claims to be a `{}`instead",
            data.index,
            text(expected),
            text(actual)
        ));
    }
    Ok(())
}

/// `ConvertNode`: the object's children (taken from the parented objects
/// in set order), its data, its children's nodes, then its modifiers.
fn convert_node(db: &mut Database, conversion: &mut Conversion, object: usize, stack: u64) -> Outcome<()> {
    let stack = stack + NODE_FRAME;
    if stack > STACK_BUDGET {
        return undefined("Blender nodes nested past the stack");
    }
    let mut children = Vec::new();
    conversion.objects.retain(|&other| {
        if db.store.objects[other].parent == Some(object) {
            children.push(other);
            return false;
        }
        true
    });
    let current = db.store.objects[object].clone();
    let node = conversion.nodes.len();
    conversion.nodes.push(Vec::new());
    if let Some(data) = current.data {
        match current.object_type {
            TYPE_MESH => {
                check_actual_type(db, data, b"Mesh")?;
                let old = conversion.meshes.len();
                let mesh = db.store.meshes[data.index].clone();
                convert_mesh(db, &mesh, conversion)?;
                conversion.nodes[node] = (old as u32..conversion.meshes.len() as u32).collect();
            }
            TYPE_LAMP => check_actual_type(db, data, b"Lamp")?,
            TYPE_CAMERA => check_actual_type(db, data, b"Camera")?,
            _ => {}
        }
    }
    check_allocation(children.len() as u64, NODE_POINTER_SIZE)?;
    for child in children {
        convert_node(db, conversion, child, stack)?;
    }
    modifiers::apply_modifiers(db, conversion, node, &current)
}

/// A mesh being made: its arrays as allocated, and how far each pass has
/// written them.
struct Building {
    mesh: scene::Mesh,
    vertices: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    faces: Vec<Vec<u32>>,
    texture_coords: Vec<Option<Vec<[f32; 3]>>>,
    colors: Option<Vec<[f32; 4]>>,
    vertex_count: usize,
    face_count: usize,
}

impl Building {
    /// The next face (`mFaces[mNumFaces++]`).
    fn next_face(&mut self) -> Outcome<usize> {
        if self.face_count >= self.faces.len() {
            return undefined("a Blender mesh's face written past its faces");
        }
        self.face_count += 1;
        Ok(self.face_count - 1)
    }

    /// A vertex at the next slot (`mVertices + mNumVertices`).
    fn push_vertex(&mut self, position: [f32; 3], normal: [f32; 3]) -> Outcome<u32> {
        let slot = self.vertex_count;
        if slot >= self.vertices.len() {
            return undefined("a Blender mesh's vertex written past its vertices");
        }
        self.vertices[slot] = position;
        self.normals[slot] = normal;
        self.vertex_count += 1;
        Ok(slot as u32)
    }

    /// A texture coordinate in set `set` at the vertex slot `slot`.
    fn write_coordinate(&mut self, set: usize, slot: usize, value: [f32; 2]) -> Outcome<()> {
        let target = self.texture_coords.get_mut(set).and_then(|set| set.as_mut()).and_then(|set| set.get_mut(slot));
        let Some(target) = target else {
            return undefined("a Blender texture coordinate written where no array is");
        };
        target[0] = value[0];
        target[1] = value[1];
        Ok(())
    }

    fn write_color(&mut self, slot: usize, color: [f32; 4]) -> Outcome<()> {
        let Some(target) = self.colors.as_mut().and_then(|colors| colors.get_mut(slot)) else {
            return undefined("a Blender color written past its array");
        };
        *target = color;
        Ok(())
    }

    /// `mTextureCoords[set] = new aiVector3D[mNumVertices]` (past the eight
    /// sets it overwrites the mesh's next members).
    fn allocate_coordinates(&mut self, set: usize) -> Outcome<()> {
        if set >= OVERLAID_SETS {
            return undefined("a Blender texture coordinate set over the mesh's faces");
        }
        check_allocation(self.vertex_count as u64, VECTOR_SIZE)?;
        self.texture_coords[set] = Some(vec![[0.0; 3]; self.vertex_count]);
        Ok(())
    }

    fn finish(self) -> scene::Mesh {
        let mut mesh = self.mesh;
        let count = self.vertex_count;
        let mut vertices = self.vertices;
        vertices.truncate(count);
        let mut normals = self.normals;
        normals.truncate(count);
        mesh.vertex_count = count;
        mesh.vertices = Some(vertices);
        mesh.normals = Some(normals);
        mesh.faces = Some(self.faces);
        let sets = self.texture_coords.into_iter().take(scene::MAX_TEXTURE_COORDS);
        for (set, coordinates) in sets.enumerate() {
            mesh.texture_coords[set] = coordinates.map(|mut coordinates| {
                coordinates.truncate(count);
                coordinates
            });
        }
        mesh.colors[0] = self.colors.map(|mut colors| {
            colors.truncate(count);
            colors
        });
        mesh
    }
}

/// `vector[index]` of a signed index: past the array is undefined.
fn element<'a, T>(array: &'a [T], index: i64, what: &str) -> Outcome<&'a T> {
    match usize::try_from(index).ok().and_then(|index| array.get(index)) {
        Some(element) => Ok(element),
        None => undefined(what),
    }
}

/// A material's texture UV layers by texture slot (`TextureUVMapping`):
/// `operator[]` adds empty slots.
type UvMapping = BTreeMap<u32, Option<usize>>;

/// The UV layer of `ldata` with the texture's UV name (`getCustomDataLayerData`):
/// the first layer of the UV type with that name, if it has data.
fn uv_layer(db: &Database, mesh: &BlendMesh, name: &[u8]) -> Option<usize> {
    for &layer in &mesh.ldata.layers {
        let candidate = &db.store.custom_data_layers[layer];
        if candidate.layer_type != CD_MLOOPUV {
            continue;
        }
        // A layer name that fills its 64 bytes runs on into the data
        // pointer, longer than any UV name: never equal.
        let Some(end) = candidate.name.iter().position(|&byte| byte == 0) else {
            continue;
        };
        if &candidate.name[..end] != name {
            continue;
        }
        return candidate.data.as_ref().map(|_| layer);
    }
    None
}

fn layer_uvs<'d>(db: &'d Database, layer: usize) -> &'d [MLoopUV] {
    match &db.store.custom_data_layers[layer].data {
        Some(CustomArray::LoopUvs(uvs)) => uvs,
        _ => &[],
    }
}

/// `ConvertMesh`: faces and polygons split by material into meshes (one
/// vertex per corner), then texture coordinates and colours in passes of
/// their own.
fn convert_mesh(db: &Database, mesh: &BlendMesh, conversion: &mut Conversion) -> Outcome<()> {
    if (mesh.totface == 0 && mesh.totloop == 0) || mesh.totvert == 0 {
        return Ok(());
    }
    if mesh.totface as i64 as u64 > mesh.mface.len() as u64 {
        return import_error("Number of faces is larger than the corresponding array");
    }
    if mesh.totvert as i64 as u64 > mesh.mvert.len() as u64 {
        return import_error("Number of vertices is larger than the corresponding array");
    }
    if mesh.totloop as i64 as u64 > mesh.mloop.len() as u64 {
        return import_error("Number of vertices is larger than the corresponding array");
    }

    let mut per_material: BTreeMap<i32, (u64, u64)> = BTreeMap::new();
    for face in &mesh.mface[..mesh.totface as usize] {
        let entry = per_material.entry(face.mat_nr).or_default();
        entry.0 += 1;
        entry.1 += if face.v4 != 0 { 4 } else { 3 };
    }
    for index in 0..mesh.totpoly.max(0) as i64 {
        let poly = element(&mesh.mpoly, index, "a Blender polygon read past its array")?;
        let entry = per_material.entry(poly.mat_nr as i32).or_default();
        entry.0 += 1;
        entry.1 = entry.1.wrapping_add(poly.totloop as i64 as u64);
    }

    let mut building: Vec<Building> = Vec::new();
    let mut mesh_of_material: BTreeMap<u64, usize> = BTreeMap::new();
    for (&material, &(face_count, vertex_count)) in &per_material {
        mesh_of_material.insert(material as i64 as u64, building.len());
        check_allocation(vertex_count, VECTOR_SIZE)?;
        check_allocation(vertex_count, VECTOR_SIZE)?;
        check_allocation(face_count, FACE_SIZE)?;
        let name = text(&mesh.id.c_str(2));
        let mut output = scene::Mesh { name, ..scene::Mesh::default() };
        output.material_index = if mesh.mat.is_empty() {
            u32::MAX
        } else {
            if material as i64 as u64 >= mesh.mat.len() as u64 {
                return import_error("Material index is out of range");
            }
            let slot = match mesh.mat[material as usize] {
                Some(index) => MaterialRef::Material(index),
                None => MaterialRef::Null,
            };
            match conversion.materials.iter().position(|&known| known == slot) {
                Some(position) => position as u32,
                None => {
                    conversion.materials.push(slot);
                    (conversion.materials.len() - 1) as u32
                }
            }
        };
        building.push(Building {
            mesh: output,
            vertices: vec![[0.0; 3]; vertex_count as usize],
            normals: vec![[0.0; 3]; vertex_count as usize],
            faces: vec![Vec::new(); face_count as usize],
            texture_coords: vec![None; OVERLAID_SETS],
            colors: None,
            vertex_count: 0,
            face_count: 0,
        });
    }
    let target = |material: i64| mesh_of_material[&(material as u64)];

    let vertex = |index: i32, out_of_range: &'static str| -> Outcome<([f32; 3], [f32; 3])> {
        if index >= mesh.totvert {
            return import_error(out_of_range);
        }
        let vertex = element(&mesh.mvert, index as i64, "a Blender vertex read before its array")?;
        Ok((vertex.co, vertex.no))
    };
    for face in &mesh.mface[..mesh.totface as usize] {
        let out = &mut building[target(face.mat_nr as i64)];
        let slot = out.next_face()?;
        let mut indices = Vec::with_capacity(4);
        for (corner, message) in [(face.v1, "Vertex index v1 out of range"), (face.v2, "Vertex index v2 out of range"), (face.v3, "Vertex index v3 out of range")] {
            let (position, normal) = vertex(corner, message)?;
            indices.push(out.push_vertex(position, normal)?);
        }
        if face.v4 >= mesh.totvert {
            return import_error("Vertex index v4 out of range");
        }
        if face.v4 != 0 {
            let (position, normal) = vertex(face.v4, "Vertex index v4 out of range")?;
            indices.push(out.push_vertex(position, normal)?);
            out.mesh.primitive_types |= PRIMITIVE_POLYGON;
        } else {
            out.mesh.primitive_types |= PRIMITIVE_TRIANGLE;
        }
        out.faces[slot] = indices;
    }
    for poly in &mesh.mpoly[..mesh.totpoly.max(0) as usize] {
        let out = &mut building[target(poly.mat_nr as i64)];
        let slot = out.next_face()?;
        check_allocation(poly.totloop as u32 as u64, INDEX_SIZE)?;
        let mut indices = Vec::with_capacity(poly.totloop.max(0) as usize);
        for corner in 0..poly.totloop.max(0) {
            let Some(loop_index) = poly.loopstart.checked_add(corner) else {
                return undefined("a Blender loop index past the integers");
            };
            let corner_loop = element(&mesh.mloop, loop_index as i64, "a Blender loop read past its array")?;
            let (position, normal) = vertex(corner_loop.v, "Vertex index out of range")?;
            indices.push(out.push_vertex(position, normal)?);
        }
        out.mesh.primitive_types |= if poly.totloop == 3 { PRIMITIVE_TRIANGLE } else { PRIMITIVE_POLYGON };
        out.faces[slot] = indices;
    }

    let mut mappings: BTreeMap<u32, UvMapping> = BTreeMap::new();
    for (slot_index, &material) in mesh.mat.iter().enumerate() {
        let Some(material) = material else {
            return undefined("a null Blender material's textures read");
        };
        let mut mapping = UvMapping::new();
        for (texture, mtex) in db.store.materials[material].mtex.iter().enumerate() {
            let Some(mtex) = *mtex else {
                continue;
            };
            let uvname = db.store.mtexes[mtex].uvname_c_str();
            if uvname.is_empty() {
                continue;
            }
            if let Some(layer) = uv_layer(db, mesh, &uvname) {
                mapping.entry(texture as u32).or_insert(Some(layer));
            }
        }
        if !mapping.is_empty() {
            mappings.insert(slot_index as u32, mapping);
        }
    }

    if !mesh.mtface.is_empty() || !mesh.mloopuv.is_empty() {
        if mesh.totface > mesh.mtface.len() as i32 {
            return import_error("Number of UV faces is larger than the corresponding UV face array (#1)");
        }
        for out in building.iter_mut() {
            match mappings.get(&out.mesh.material_index) {
                None => out.allocate_coordinates(0)?,
                Some(mapping) => {
                    for set in 0..mapping.len() {
                        out.allocate_coordinates(set)?;
                    }
                }
            }
            out.face_count = 0;
            out.vertex_count = 0;
        }
        for (index, face) in mesh.mface[..mesh.totface as usize].iter().enumerate() {
            let coordinates = mesh.mtface[index].uv;
            let out = &mut building[target(face.mat_nr as i64)];
            let slot = out.next_face()?;
            let corners = out.faces[slot].len();
            for &coordinate in coordinates.iter().take(corners) {
                let vertex_slot = out.vertex_count;
                out.write_coordinate(0, vertex_slot, coordinate)?;
                out.vertex_count += 1;
            }
        }
        for poly in &mesh.mpoly[..mesh.totpoly.max(0) as usize] {
            let out = &mut building[target(poly.mat_nr as i64)];
            let slot = out.next_face()?;
            let corners = out.faces[slot].len();
            let key = poly.mat_nr as i32 as u32;
            match mappings.get_mut(&key) {
                None => {
                    for corner in 0..corners {
                        let loop_index = poly.loopstart as i64 + corner as i64;
                        let uv = element(&mesh.mloopuv, loop_index, "a Blender loop UV read past its array")?;
                        let vertex_slot = out.vertex_count;
                        out.write_coordinate(0, vertex_slot, uv.uv)?;
                        out.vertex_count += 1;
                    }
                }
                Some(mapping) => {
                    let mut set = 0u32;
                    while (set as usize) < mapping.len() {
                        let layer = *mapping.entry(set).or_insert(None);
                        for corner in 0..corners {
                            let Some(layer) = layer else {
                                return undefined("a Blender texture's missing UV layer read");
                            };
                            let loop_index = poly.loopstart as i64 + corner as i64;
                            let uv = element(layer_uvs(db, layer), loop_index, "a Blender layer UV read past its array")?;
                            let vertex_slot = out.vertex_count + corner;
                            out.write_coordinate(set as usize, vertex_slot, uv.uv)?;
                        }
                        if set as usize == mapping.len() - 1 {
                            out.vertex_count += corners;
                        }
                        set += 1;
                    }
                }
            }
        }
    }

    if !mesh.tface.is_empty() {
        if mesh.totface > mesh.tface.len() as i32 {
            return import_error("Number of faces is larger than the corresponding UV face array (#2)");
        }
        for out in building.iter_mut() {
            out.allocate_coordinates(0)?;
            out.face_count = 0;
            out.vertex_count = 0;
        }
        for (index, face) in mesh.mface[..mesh.totface as usize].iter().enumerate() {
            let coordinates = mesh.tface[index].uv;
            let out = &mut building[target(face.mat_nr as i64)];
            let slot = out.next_face()?;
            let corners = out.faces[slot].len();
            for &coordinate in coordinates.iter().take(corners) {
                let vertex_slot = out.vertex_count;
                out.write_coordinate(0, vertex_slot, coordinate)?;
                out.vertex_count += 1;
            }
        }
    }

    if !mesh.mcol.is_empty() || !mesh.mloopcol.is_empty() {
        if mesh.totface > (mesh.mcol.len() / 4) as i32 {
            return import_error("Number of faces is larger than the corresponding color face array");
        }
        for out in building.iter_mut() {
            check_allocation(out.vertex_count as u64, COLOR_SIZE)?;
            out.colors = Some(vec![[0.0; 4]; out.vertex_count]);
            out.face_count = 0;
            out.vertex_count = 0;
        }
        for (index, face) in mesh.mface[..mesh.totface as usize].iter().enumerate() {
            let out = &mut building[target(face.mat_nr as i64)];
            let slot = out.next_face()?;
            for corner in 0..out.faces[slot].len() {
                let color = mesh.mcol[(index << 2) + corner];
                let vertex_slot = out.vertex_count;
                out.write_color(vertex_slot, [color.r as f32, color.g as f32, color.b as f32, color.a as f32])?;
                out.vertex_count += 1;
            }
        }
        let scale = 1.0f32 / 255.0;
        for poly in &mesh.mpoly[..mesh.totpoly.max(0) as usize] {
            let out = &mut building[target(poly.mat_nr as i64)];
            let slot = out.next_face()?;
            for corner in 0..out.faces[slot].len() {
                let loop_index = poly.loopstart as i64 + corner as i64;
                let color = element(&mesh.mloopcol, loop_index, "a Blender loop colour read past its array")?;
                let rgba = [color.r as f32 * scale, color.g as f32 * scale, color.b as f32 * scale, color.a as f32 * scale];
                let vertex_slot = out.vertex_count;
                out.write_color(vertex_slot, rgba)?;
                out.vertex_count += 1;
            }
        }
    }

    for out in building {
        conversion.meshes.push(out.finish());
    }
    Ok(())
}

/// `BuildMaterials`, for what can fail in it: a null material's name,
/// and an image's name or packed data.
fn build_materials(db: &mut Database, conversion: &Conversion) -> Outcome<()> {
    let mut materials = conversion.materials.clone();
    if conversion.meshes.iter().any(|mesh| mesh.material_index == u32::MAX) {
        materials.push(MaterialRef::Default);
    }
    for material in materials {
        let index = match material {
            MaterialRef::Null => return undefined("a null Blender material's name read"),
            MaterialRef::Default => continue,
            MaterialRef::Material(index) => index,
        };
        for slot in 0..TEXTURE_SLOTS {
            let Some(mtex) = db.store.materials[index].mtex[slot] else {
                continue;
            };
            resolve_texture(db, mtex)?;
        }
    }
    Ok(())
}

/// `ResolveTexture`: image textures name their image (and copy packed
/// data); procedural ones get a placeholder.
fn resolve_texture(db: &mut Database, mtex: usize) -> Outcome<()> {
    let Some(tex) = db.store.mtexes[mtex].tex else {
        return Ok(());
    };
    let tex = db.store.texes[tex].clone();
    if tex.tex_type != TEX_IMAGE {
        return Ok(());
    }
    let Some(image) = tex.ima else {
        return Ok(());
    };
    let image = db.store.images[image].clone();
    let Some(packed) = image.packedfile else {
        return Ok(());
    };
    let packed = db.store.packed_files[packed].clone();
    let size = packed.size as u32;
    check_allocation(size as u64, 1)?;
    let Some(offset) = packed.data else {
        return undefined("a Blender packed file's missing data read");
    };
    db.set_position(offset)?;
    db.reader.copy_and_advance(size as usize)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structures the test files declare: type, then fields as type
    /// and DNA name.
    const STRUCTURES: &[(&str, &[(&str, &str)])] = &[
        ("ID", &[("char", "name[66]"), ("short", "flag")]),
        ("ListBase", &[("void", "*first"), ("void", "*last")]),
        ("Object", &[("ID", "id"), ("short", "type"), ("Object", "*parent"), ("void", "*data")]),
        ("CollectionObject", &[("CollectionObject", "*next"), ("CollectionObject", "*prev"), ("Object", "*ob")]),
        ("Collection", &[("ID", "id"), ("ListBase", "gobject"), ("ListBase", "children")]),
        ("Scene", &[("ID", "id"), ("Collection", "*master_collection")]),
    ];
    const PRIMITIVES: &[(&str, u16)] = &[("char", 1), ("short", 2), ("void", 0)];
    const OBJECT: u32 = 2;
    const COLLECTION_OBJECT: u32 = 3;
    const COLLECTION: u32 = 4;
    const SCENE: u32 = 5;

    fn pad(bytes: &mut Vec<u8>) {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    }

    /// `SDNA`: names, types with their sizes, structures.
    fn dna() -> Vec<u8> {
        let mut types: Vec<(&str, u16)> = PRIMITIVES.to_vec();
        for (name, fields) in STRUCTURES {
            let mut size = 0;
            for (field_type, field_name) in fields.iter() {
                let element = if field_name.starts_with('*') { 8 } else { types.iter().find(|(name, _)| name == field_type).unwrap().1 };
                let count = field_name.split('[').nth(1).map_or(1, |count| count.trim_end_matches(']').parse().unwrap());
                size += element * count;
            }
            types.push((name, size));
        }
        let names: Vec<&str> = STRUCTURES.iter().flat_map(|(_, fields)| fields.iter().map(|(_, name)| *name)).collect();
        let mut bytes = b"SDNANAME".to_vec();
        bytes.extend((names.len() as u32).to_le_bytes());
        for name in &names {
            bytes.extend(name.as_bytes());
            bytes.push(0);
        }
        pad(&mut bytes);
        bytes.extend(b"TYPE");
        bytes.extend((types.len() as u32).to_le_bytes());
        for (name, _) in &types {
            bytes.extend(name.as_bytes());
            bytes.push(0);
        }
        pad(&mut bytes);
        bytes.extend(b"TLEN");
        for (_, size) in &types {
            bytes.extend(size.to_le_bytes());
        }
        pad(&mut bytes);
        bytes.extend(b"STRC");
        bytes.extend((STRUCTURES.len() as u32).to_le_bytes());
        let type_index = |name: &str| types.iter().position(|(type_name, _)| *type_name == name).unwrap() as u16;
        let mut name_index = 0u16;
        for (name, fields) in STRUCTURES {
            bytes.extend(type_index(name).to_le_bytes());
            bytes.extend((fields.len() as u16).to_le_bytes());
            for (field_type, _) in fields.iter() {
                bytes.extend(type_index(field_type).to_le_bytes());
                bytes.extend(name_index.to_le_bytes());
                name_index += 1;
            }
        }
        bytes
    }

    fn block(file: &mut Vec<u8>, code: &[u8; 4], address: u64, structure: u32, payload: &[u8]) {
        file.extend(code);
        file.extend((payload.len() as i32).to_le_bytes());
        file.extend(address.to_le_bytes());
        file.extend(structure.to_le_bytes());
        file.extend(1u32.to_le_bytes());
        file.extend(payload);
    }

    fn id(name: &str) -> Vec<u8> {
        let mut bytes = name.as_bytes().to_vec();
        bytes.resize(66 + 2, 0);
        bytes
    }

    /// A file whose master collection lists `entries` objects, the last
    /// of a chain of `objects` parents (each list entry and object block
    /// at its own address).
    fn chain_file(entries: u64, objects: u64) -> Vec<u8> {
        let mut file = b"BLENDER-v280".to_vec();
        block(&mut file, b"DNA1", 0, 0, &dna());
        let object_address = |index: u64| 0x10_0000 + index * 0x100;
        let entry_address = |index: u64| 0x80_0000 + index * 0x100;
        for index in 0..objects {
            let parent = if index == 0 { 0 } else { object_address(index - 1) };
            let mut object = id(&format!("OBO{index}"));
            object.extend(0i16.to_le_bytes());
            object.extend(parent.to_le_bytes());
            object.extend(0u64.to_le_bytes());
            block(&mut file, b"OB\0\0", object_address(index), OBJECT, &object);
        }
        for index in 0..entries {
            let next = if index + 1 < entries { entry_address(index + 1) } else { 0 };
            let mut entry = next.to_le_bytes().to_vec();
            entry.extend(0u64.to_le_bytes());
            entry.extend(object_address(objects - 1).to_le_bytes());
            block(&mut file, b"DATA", entry_address(index), COLLECTION_OBJECT, &entry);
        }
        let mut collection = id("GRMaster");
        for pointer in [entry_address(0), entry_address(entries - 1), 0, 0] {
            collection.extend(pointer.to_le_bytes());
        }
        block(&mut file, b"GR\0\0", 0x1000, COLLECTION, &collection);
        let mut scene = id("SCScene");
        scene.extend(0x1000u64.to_le_bytes());
        block(&mut file, b"SC\0\0", 0x2000, SCENE, &scene);
        block(&mut file, b"ENDB", 0, 0, &[]);
        file
    }

    #[test]
    fn a_collection_list_converts_within_the_stack() {
        let scene = read(&chain_file(2_000, 1)).unwrap();
        assert!(scene.meshes.is_empty());
    }

    /// Each list entry converts the next one: past the C++'s stack (about
    /// 2,850 entries) the import fails as the C++ would crash.
    #[test]
    fn a_collection_list_past_the_stack_fails() {
        let failure = read(&chain_file(3_000, 1)).unwrap_err();
        assert_eq!(failure.0, "undefined behaviour: Blender objects nested past the stack");
    }

    /// A parent chain the C++'s stack still holds takes the port's frames
    /// past a test thread's 2 MiB: the conversion runs on its own thread.
    #[test]
    fn a_long_parent_chain_converts_on_its_own_stack() {
        let scene = read(&chain_file(1, 2_600)).unwrap();
        assert!(scene.meshes.is_empty());
        let failure = read(&chain_file(1, 2_800)).unwrap_err();
        assert!(failure.ends_process());
    }
}
