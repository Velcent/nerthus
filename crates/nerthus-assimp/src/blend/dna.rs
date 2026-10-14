//! Blender's DNA (`BlenderDNA`): the structure definitions a .blend file
//! carries, its file blocks, and the reads Assimp's importer makes through
//! them: fields by name, converted between primitive types, with an error
//! policy each, and pointers resolved to the blocks they fall in.
//!
//! A `Blender::Error` (a missing field or structure, a pointer to a block
//! of another type) is caught by the nearest field read, whose policy then
//! substitutes a default or fails the import; any other failure ends the
//! import. Objects reached through pointers are cached before they are
//! converted, so cycles end, and an object whose conversion was cut short
//! by a caught error stays as far as it got.

use std::collections::HashMap;

use super::structures::{Element, Kind, Store, convert_element};
use crate::cast::to_int;
use crate::fast_atof::strtoul10;
use crate::msvc::VectorCapacity;
use crate::scene::{ImportError, check_allocation};
use crate::stream_reader::StreamReader;

pub(super) use crate::scene::STACK_BUDGET;

/// Stack frames of the C++'s readers, as MSVC lays them out on x64:
/// `ReadField` of a structure (with an ignored list or custom data, 288),
/// `ReadFieldPtr` of a static type with `ResolvePointer` inlined, the
/// `ElemBase` reader with its `ResolvePointer`, and the array readers.
const FIELD_FRAME: u64 = 240;
const LARGE_FIELD_FRAME: u64 = 288;
const POINTER_FRAME: u64 = 432;
const DYNAMIC_FRAME: u64 = 352 + 928;
pub(super) const ARRAY_FRAME: u64 = 416;
const POINTER_ARRAY_FRAME: u64 = 336 + 240;
const POINTERS_FRAME: u64 = 480 + 160;

/// `ReadFieldPtr` of a static type: the group object list's own reads
/// call `ResolvePointer`, and a world's converter is inlined.
fn pointer_frame(kind: Kind) -> u64 {
    match kind {
        Kind::GroupObject => 352 + 160,
        Kind::World => 448,
        _ => POINTER_FRAME,
    }
}

/// `sizeof` on MSVC x64 of the containers the parser fills.
const STRING_SIZE: u64 = 32;
const TYPE_SIZE: u64 = 40;
const STRUCTURE_SIZE: u64 = 88;
const BLOCK_SIZE: u64 = 72;
const SHARED_POINTER_SIZE: u64 = 16;

/// A failed read: `Blender::Error`, which field reads catch, or anything
/// else, which ends the import.
pub(super) enum Failure {
    Dna(String),
    Import(ImportError),
}

impl From<ImportError> for Failure {
    fn from(error: ImportError) -> Self {
        Failure::Import(error)
    }
}

pub(super) type Outcome<T> = Result<T, Failure>;

pub(super) fn dna_error<T>(message: String) -> Outcome<T> {
    Err(Failure::Dna(message))
}

pub(super) fn import_error<T>(message: impl Into<String>) -> Outcome<T> {
    Err(Failure::Import(ImportError(message.into())))
}

pub(super) fn undefined<T>(what: &str) -> Outcome<T> {
    import_error(format!("undefined behaviour: {what}"))
}

pub(super) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `ErrorPolicy`: what a caught `Blender::Error` leaves in a field.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Policy {
    Ignore,
    Warn,
    Fail,
}

impl Policy {
    /// `_defaultInitializer`: the default, or for `Fail` a failure.
    fn recover<T: Default>(self, message: String) -> Outcome<T> {
        if self == Policy::Fail {
            return import_error(format!("Constructing BlenderDNA Structure encountered an error: {message}"));
        }
        Ok(T::default())
    }
}

/// `Field`: a structure member, its type looked up once the DNA is read.
#[derive(Clone)]
pub(super) struct Field {
    pub name: Vec<u8>,
    pub type_name: Vec<u8>,
    pub type_structure: Option<usize>,
    pub size: u64,
    pub offset: u64,
    pub array_sizes: [u64; 2],
    pub pointer: bool,
    pub array: bool,
}

pub(super) struct Structure {
    pub name: Vec<u8>,
    pub fields: Vec<Field>,
    pub indices: HashMap<Vec<u8>, usize>,
    pub size: u64,
}

impl Structure {
    fn new(name: Vec<u8>, size: u64) -> Self {
        Structure { name, fields: Vec::new(), indices: HashMap::new(), size }
    }
}

#[derive(Default)]
pub(super) struct Dna {
    pub structures: Vec<Structure>,
    pub indices: HashMap<Vec<u8>, usize>,
    capacity: Option<VectorCapacity>,
}

impl Dna {
    /// `DNA::operator[](name)`.
    pub fn by_name(&self, name: &[u8]) -> Outcome<usize> {
        match self.indices.get(name) {
            Some(&index) => Ok(index),
            None => dna_error(format!("BlendDNA: Did not find a structure named `{}`", text(name))),
        }
    }

    /// `DNA::operator[](index)`.
    pub fn by_index(&self, index: u32) -> Outcome<usize> {
        if index as usize >= self.structures.len() {
            return dna_error(format!("BlendDNA: There is no structure with index `{index}`"));
        }
        Ok(index as usize)
    }

    /// `AddPrimitiveStructures`: dummies whose names pick the primitive
    /// conversions.
    fn add_primitive_structures(&mut self) {
        let primitives: [(&[u8], u64); 5] = [(b"int", 4), (b"short", 2), (b"char", 1), (b"float", 4), (b"double", 8)];
        for (name, size) in primitives {
            self.indices.insert(name.to_vec(), self.structures.len());
            self.structures.push(Structure::new(name.to_vec(), size));
        }
    }

    /// Each field's type structure, as `db.dna[f.type]` finds it once all
    /// the DNA is read.
    pub fn resolve_field_types(&mut self) {
        let indices = &self.indices;
        for structure in &mut self.structures {
            for field in &mut structure.fields {
                field.type_structure = indices.get(&field.type_name).copied();
            }
        }
    }
}

/// `FileBlockHead`.
#[derive(Clone, Copy)]
pub(super) struct Block {
    pub start: u64,
    pub id: [u8; 4],
    pub id_length: usize,
    pub size: u64,
    pub address: u64,
    pub dna_index: u32,
    pub num: u64,
}

impl Block {
    pub fn id(&self) -> &[u8] {
        &self.id[..self.id_length]
    }
}

fn match4(reader: &mut StreamReader, tag: &[u8; 4]) -> Outcome<bool> {
    let mut bytes = [0u8; 4];
    for byte in &mut bytes {
        *byte = reader.get_i1()? as u8;
    }
    Ok(&bytes == tag)
}

/// A NUL-terminated name.
fn read_name(reader: &mut StreamReader) -> Outcome<Vec<u8>> {
    let mut name = Vec::new();
    loop {
        let byte = reader.get_i1()? as u8;
        if byte == 0 {
            return Ok(name);
        }
        name.push(byte);
    }
}

fn align4(reader: &mut StreamReader) -> Outcome<()> {
    while reader.current_pos() & 3 != 0 {
        reader.get_i1()?;
    }
    Ok(())
}

/// `std::vector(n)` or `reserve(n)` of `count` from an `int`: past the
/// maximum size `length_error`, past the limit `bad_alloc`.
fn check_count(count: u64, element_size: u64) -> Outcome<()> {
    check_allocation(count, element_size)?;
    Ok(())
}

/// `DNA::ExtractArraySize`: up to two `[n]` dimensions.
fn array_sizes(name: &[u8]) -> [u64; 2] {
    let mut sizes = [1u64; 2];
    let Some(first) = name.iter().position(|&byte| byte == b'[') else {
        return sizes;
    };
    sizes[0] = strtoul10(name, first + 1).0 as u64;
    let Some(second) = name[first + 1..].iter().position(|&byte| byte == b'[') else {
        return sizes;
    };
    sizes[1] = strtoul10(name, first + 1 + second + 1).0 as u64;
    sizes
}

/// `DNAParser::Parse`: names, types, their sizes and the structures, from
/// the stream's position (a DNA1 block), into the DNA so far.
pub(super) fn parse_dna(reader: &mut StreamReader, pointer_64: bool, dna: &mut Dna) -> Outcome<()> {
    if !match4(reader, b"SDNA")? {
        return import_error("BlenderDNA: Expected SDNA chunk");
    }
    if !match4(reader, b"NAME")? {
        return import_error("BlenderDNA: Expected NAME field");
    }
    let name_count = reader.get_i4()? as i64 as u64;
    check_count(name_count, STRING_SIZE)?;
    let mut names = Vec::with_capacity(name_count.min(1 << 20) as usize);
    for _ in 0..name_count {
        names.push(read_name(reader)?);
    }

    align4(reader)?;
    if !match4(reader, b"TYPE")? {
        return import_error("BlenderDNA: Expected TYPE field");
    }
    let type_count = reader.get_i4()? as i64 as u64;
    check_count(type_count, TYPE_SIZE)?;
    let mut type_names = Vec::with_capacity(type_count.min(1 << 20) as usize);
    for _ in 0..type_count {
        type_names.push(read_name(reader)?);
    }

    align4(reader)?;
    if !match4(reader, b"TLEN")? {
        return import_error("BlenderDNA: Expected TLEN field");
    }
    let mut type_sizes = Vec::with_capacity(type_names.len());
    for _ in 0..type_names.len() {
        type_sizes.push(reader.get_i2()? as i64 as u64);
    }

    align4(reader)?;
    if !match4(reader, b"STRC")? {
        return import_error("BlenderDNA: Expected STRC field");
    }
    let structure_count = reader.get_i4()? as i64 as u64;
    let capacity = dna.capacity.get_or_insert_with(|| VectorCapacity::new(STRUCTURE_SIZE));
    capacity.reserve(structure_count.min(usize::MAX as u64) as usize)?;
    let mut index = 0u64;
    while index != structure_count {
        let type_index = reader.get_i2()? as u16 as usize;
        if type_index >= type_names.len() {
            return import_error(format!(
                "BlenderDNA: Invalid type index in structure name{type_index} (there are only {} entries)",
                type_names.len()
            ));
        }
        dna.indices.insert(type_names[type_index].clone(), dna.structures.len());
        if let Some(capacity) = dna.capacity.as_mut() {
            capacity.push_to(dna.structures.len() + 1)?;
        }
        let mut structure = Structure::new(type_names[type_index].clone(), 0);
        let field_count = reader.get_i2()? as u16;
        let mut offset = 0u64;
        for _ in 0..field_count {
            let field_type = reader.get_i2()? as u16 as usize;
            if field_type >= type_names.len() {
                return import_error(format!(
                    "BlenderDNA: Invalid type index in structure field {field_type} (there are only {} entries)",
                    type_names.len()
                ));
            }
            let name_index = reader.get_i2()? as u16 as usize;
            if name_index >= names.len() {
                return import_error(format!(
                    "BlenderDNA: Invalid name index in structure field {name_index} (there are only {} entries)",
                    names.len()
                ));
            }
            let mut field = Field {
                name: names[name_index].clone(),
                type_name: type_names[field_type].clone(),
                type_structure: None,
                size: type_sizes[field_type],
                offset,
                array_sizes: [1, 1],
                pointer: false,
                array: false,
            };
            // Pointers give the pointee's size, not their own.
            if field.name.first() == Some(&b'*') {
                field.size = if pointer_64 { 8 } else { 4 };
                field.pointer = true;
            }
            let Some(&last) = field.name.last() else {
                return undefined("a Blender DNA field without a name read from its end");
            };
            if last == b']' {
                let Some(bracket) = field.name.iter().position(|&byte| byte == b'[') else {
                    return import_error(format!("BlenderDNA: Encountered invalid array declaration {}", text(&field.name)));
                };
                field.array = true;
                field.array_sizes = array_sizes(&field.name);
                field.name.truncate(bracket);
                let elements = field.array_sizes[0].wrapping_mul(field.array_sizes[1]);
                field.size = field.size.wrapping_mul(elements);
            }
            offset = offset.wrapping_add(field.size);
            structure.indices.insert(field.name.clone(), structure.fields.len());
            structure.fields.push(field);
        }
        structure.size = offset;
        dna.structures.push(structure);
        index = index.wrapping_add(1);
    }
    dna.add_primitive_structures();
    Ok(())
}

/// `SectionParser::Next`: the block header after the previous block.
pub(super) fn next_block(reader: &mut StreamReader, pointer_64: bool, previous: &Block) -> Outcome<Block> {
    let position = previous.start.wrapping_add(previous.size);
    reader.set_ptr(position as i64)?;
    let mut id = [0u8; 4];
    for byte in &mut id {
        *byte = reader.get_i1()? as u8;
    }
    let id_length = if id[3] != 0 {
        4
    } else if id[2] != 0 {
        3
    } else if id[1] != 0 {
        2
    } else {
        1
    };
    let size = reader.get_i4()? as i64 as u64;
    let address = if pointer_64 { reader.get_u8()? } else { reader.get_u4()? as u64 };
    let dna_index = reader.get_i4()? as u32;
    let num = reader.get_i4()? as i64 as u64;
    let start = reader.current_pos() as i64 as u64;
    if (reader.remaining_size_to_limit() as u64) < size {
        return import_error("BLEND: invalid size of file block");
    }
    Ok(Block { start, id, id_length, size, address, dna_index, num })
}

/// A pointer field's value and its declared type.
pub(super) struct PointerField {
    pub value: u64,
    type_structure: Option<usize>,
    type_name: Vec<u8>,
}

/// A custom data layer's array (`readCustomData`): only UVs are looked at
/// again.
#[derive(Clone)]
pub(super) enum CustomArray {
    LoopUvs(Vec<super::structures::MLoopUV>),
    Other,
}

/// `FileDatabase` with the conversion's state: the reader, DNA, blocks
/// sorted by address, the converted objects, and the object cache.
pub(super) struct Database<'a> {
    pub reader: StreamReader<'a>,
    pub pointer_64: bool,
    pub dna: Dna,
    pub blocks: Vec<Block>,
    pub store: Store,
    /// `ObjectCache`: by structure and address.
    cache: HashMap<(usize, u64), Element>,
    /// `ElemBase::dna_type`: set on objects converted through a pointer
    /// of no static type, once converted.
    pub dna_types: HashMap<Element, usize>,
    /// Bytes of the C++'s stack the conversions under way take.
    stack: u64,
}

/// What a static type's objects are and how they convert.
pub(super) trait Convert: Default + Clone {
    fn convert(db: &mut Database, structure: usize, out: &mut Self) -> Outcome<()>;
}

/// A converted type with its own arena, for pointers.
pub(super) trait Typed: Convert {
    const KIND: Kind;
    fn arena(store: &mut Store) -> &mut Vec<Self>;
}

impl<'a> Database<'a> {
    pub fn new(reader: StreamReader<'a>, pointer_64: bool) -> Self {
        Database {
            reader,
            pointer_64,
            dna: Dna::default(),
            blocks: Vec::new(),
            store: Store::default(),
            cache: HashMap::new(),
            dna_types: HashMap::new(),
            stack: 0,
        }
    }

    /// `GetCurrentPos` (an `int`) as a position.
    pub fn position(&self) -> u64 {
        self.reader.current_pos() as i64 as u64
    }

    /// `SetCurrentPos`.
    pub fn set_position(&mut self, position: u64) -> Outcome<()> {
        self.reader.set_ptr(position as i64)?;
        Ok(())
    }

    /// `IncPtr`.
    pub fn advance(&mut self, by: u64) -> Outcome<()> {
        self.reader.inc_ptr(by as i64)?;
        Ok(())
    }

    /// `LocateFileBlockForAddress`: the first block starting at or after
    /// the address (so an address inside a block finds the next one),
    /// which must not end before it.
    pub fn locate(&self, pointer: u64) -> Outcome<usize> {
        let index = self.blocks.partition_point(|block| block.address < pointer);
        let Some(block) = self.blocks.get(index) else {
            return import_error(format!("Failure resolving pointer 0x{pointer:x}, no file block falls into this address range"));
        };
        let end = block.address.wrapping_add(block.size);
        if pointer >= end {
            return import_error(format!(
                "Failure resolving pointer 0x{pointer:x}, nearest file block starting at 0x{:x} ends at 0x{end:x}",
                block.address
            ));
        }
        Ok(index)
    }

    /// The address's place in its block's data (wrapping, as `size_t`).
    pub fn seek_block(&mut self, block: usize, pointer: u64) -> Outcome<()> {
        let block = self.blocks[block];
        let position = block.start.wrapping_add(pointer.wrapping_sub(block.address));
        self.set_position(position)
    }

    /// A conversion one level deeper in the C++'s recursion, its frames
    /// `frames` bytes of the stack.
    fn nested<T>(&mut self, frames: u64, convert: impl FnOnce(&mut Self) -> Outcome<T>) -> Outcome<T> {
        if self.stack + frames > STACK_BUDGET {
            return undefined("Blender objects nested past the stack");
        }
        self.stack += frames;
        let result = convert(self);
        self.stack -= frames;
        result
    }

    /// Converts an arena object in place (what a caught error interrupts
    /// stays), reached through a reader whose frames take `frames`.
    pub fn convert_slot<T: Typed>(&mut self, structure: usize, index: usize, frames: u64) -> Outcome<()> {
        let mut value = std::mem::take(&mut T::arena(&mut self.store)[index]);
        let result = self.nested(frames, |db| T::convert(db, structure, &mut value));
        T::arena(&mut self.store)[index] = value;
        result
    }

    /// Converts a value in place, reached through a reader whose frame
    /// takes `read_frame` (the converter's own is added).
    pub fn convert_value<T: Typed>(&mut self, structure: usize, out: &mut T, read_frame: u64) -> Outcome<()> {
        let frames = read_frame + T::KIND.convert_frame();
        self.nested(frames, |db| T::convert(db, structure, out))
    }

    /// `Structure::operator[]`.
    fn field(&self, structure: usize, name: &[u8]) -> Outcome<Field> {
        let owner = &self.dna.structures[structure];
        match owner.indices.get(name) {
            Some(&index) => Ok(owner.fields[index].clone()),
            None => dna_error(format!("BlendDNA: Did not find a field named `{}` in structure `{}`", text(name), text(&owner.name))),
        }
    }

    /// `db.dna[f.type]`.
    fn type_of(&self, field: &Field) -> Outcome<usize> {
        match field.type_structure {
            Some(index) => Ok(index),
            None => dna_error(format!("BlendDNA: Did not find a structure named `{}`", text(&field.type_name))),
        }
    }

    fn structure_name(&self, structure: usize) -> &[u8] {
        &self.dna.structures[structure].name
    }

    /// `ReadField`: the field converted by `convert` (from its type's
    /// structure), the stream left where it was.
    pub fn read_field<T: Default>(&mut self, structure: usize, policy: Policy, name: &str, convert: impl FnOnce(&mut Self, usize) -> Outcome<T>) -> Outcome<T> {
        let old = self.position();
        let attempt = (|| {
            let field = self.field(structure, name.as_bytes())?;
            let type_structure = self.type_of(&field)?;
            self.advance(field.offset)?;
            convert(self, type_structure)
        })();
        let value = match attempt {
            Ok(value) => value,
            Err(Failure::Dna(message)) => policy.recover(message)?,
            Err(failure) => return Err(failure),
        };
        self.set_position(old)?;
        Ok(value)
    }

    /// `ReadField` of a structure type.
    pub fn read_struct<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<T> {
        let large = T::KIND == Kind::CustomData || (T::KIND == Kind::ListBase && policy == Policy::Ignore);
        let read_frame = if large { LARGE_FIELD_FRAME } else { FIELD_FRAME };
        self.read_field(structure, policy, name, |db, type_structure| {
            let mut value = T::default();
            db.convert_value(type_structure, &mut value, read_frame)?;
            Ok(value)
        })
    }

    pub fn read_i32(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<i32> {
        self.read_field(structure, policy, name, Self::convert_i32)
    }

    pub fn read_i16(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<i16> {
        self.read_field(structure, policy, name, Self::convert_i16)
    }

    pub fn read_i8(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<i8> {
        self.read_field(structure, policy, name, Self::convert_i8)
    }

    pub fn read_u8(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<u8> {
        self.read_field(structure, policy, name, Self::convert_u8)
    }

    pub fn read_f32(&mut self, structure: usize, policy: Policy, name: &str) -> Outcome<f32> {
        self.read_field(structure, policy, name, Self::convert_f32)
    }

    /// `ReadFieldArray`: up to `M` elements of an array field, the rest
    /// defaults.
    pub fn read_array<const M: usize, T: Copy + Default>(
        &mut self,
        structure: usize,
        policy: Policy,
        name: &str,
        element: fn(&mut Self, usize) -> Outcome<T>,
    ) -> Outcome<[T; M]> {
        let old = self.position();
        let attempt = (|| {
            let field = self.field(structure, name.as_bytes())?;
            let type_structure = self.type_of(&field)?;
            if !field.array {
                let owner = text(self.structure_name(structure));
                return dna_error(format!("Field `{name}` of structure `{owner}` ought to be an array of size {M}"));
            }
            self.advance(field.offset)?;
            let mut out = [T::default(); M];
            let count = field.array_sizes[0].min(M as u64) as usize;
            for slot in out.iter_mut().take(count) {
                *slot = element(self, type_structure)?;
            }
            Ok(out)
        })();
        let value = match attempt {
            Ok(value) => value,
            Err(Failure::Dna(message)) => {
                policy.recover::<()>(message)?;
                [T::default(); M]
            }
            Err(failure) => return Err(failure),
        };
        self.set_position(old)?;
        Ok(value)
    }

    /// `ReadFieldArray2`.
    pub fn read_array2<const M: usize, const N: usize, T: Copy + Default>(
        &mut self,
        structure: usize,
        policy: Policy,
        name: &str,
        element: fn(&mut Self, usize) -> Outcome<T>,
    ) -> Outcome<[[T; N]; M]> {
        let old = self.position();
        let attempt = (|| {
            let field = self.field(structure, name.as_bytes())?;
            let type_structure = self.type_of(&field)?;
            if !field.array {
                let owner = text(self.structure_name(structure));
                return dna_error(format!("Field `{name}` of structure `{owner}` ought to be an array of size {M}*{N}"));
            }
            self.advance(field.offset)?;
            let mut out = [[T::default(); N]; M];
            let rows = field.array_sizes[0].min(M as u64) as usize;
            let columns = field.array_sizes[1].min(N as u64) as usize;
            for row in out.iter_mut().take(rows) {
                for slot in row.iter_mut().take(columns) {
                    *slot = element(self, type_structure)?;
                }
            }
            Ok(out)
        })();
        let value = match attempt {
            Ok(value) => value,
            Err(Failure::Dna(message)) => {
                policy.recover::<()>(message)?;
                [[T::default(); N]; M]
            }
            Err(failure) => return Err(failure),
        };
        self.set_position(old)?;
        Ok(value)
    }

    /// The pointer value of a pointer field (`ReadFieldPtr`'s guarded
    /// part); none when a caught error left the default.
    fn read_pointer_field(&mut self, structure: usize, policy: Policy, name: &str, check_pointer: bool) -> Outcome<Option<PointerField>> {
        let attempt = (|| {
            let field = self.field(structure, name.as_bytes())?;
            if check_pointer && !field.pointer {
                let owner = text(self.structure_name(structure));
                return dna_error(format!("Field `{name}` of structure `{owner}` ought to be a pointer"));
            }
            self.advance(field.offset)?;
            let value = self.convert_pointer()?;
            Ok(PointerField { value, type_structure: field.type_structure, type_name: field.type_name })
        })();
        match attempt {
            Ok(field) => Ok(Some(field)),
            Err(Failure::Dna(message)) => {
                policy.recover::<()>(message)?;
                Ok(None)
            }
            Err(failure) => Err(failure),
        }
    }

    /// `ReadFieldPtr` to an object of a static type: whether it came
    /// from the cache. Without recursion the object is only allocated, the
    /// stream left at it.
    pub fn read_pointer<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Option<usize>) -> Outcome<bool> {
        self.read_pointer_with::<T>(structure, policy, name, out, false)
    }

    pub fn read_pointer_with<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Option<usize>, non_recursive: bool) -> Outcome<bool> {
        let old = self.position();
        *out = None;
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(false);
        };
        let (index, cached) = self.resolve::<T>(&field, non_recursive, pointer_frame(T::KIND))?;
        *out = index;
        if !non_recursive {
            self.set_position(old)?;
        }
        Ok(cached)
    }

    /// `ReadFieldPtr` to an object of the type its block says.
    pub fn read_dynamic(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Option<Element>) -> Outcome<bool> {
        let old = self.position();
        *out = None;
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(false);
        };
        let (element, cached) = self.resolve_dynamic(field.value)?;
        *out = element;
        self.set_position(old)?;
        Ok(cached)
    }

    /// `ReadFieldPtr` to a file offset (`PackedFile::data`).
    pub fn read_file_offset(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Option<u64>) -> Outcome<()> {
        let old = self.position();
        *out = None;
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(());
        };
        if field.value != 0 {
            let block = self.locate(field.value)?;
            let block = self.blocks[block];
            *out = Some(block.start.wrapping_add(field.value.wrapping_sub(block.address)));
        }
        self.set_position(old)?;
        Ok(())
    }

    /// `ReadFieldPtr` to an array (a block of elements, uncached).
    pub fn read_array_pointer<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str, element_size: u64, out: &mut Vec<T>) -> Outcome<()> {
        let old = self.position();
        out.clear();
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(());
        };
        if field.value != 0 {
            let type_structure = self.type_for_pointer(&field)?;
            let block = self.locate(field.value)?;
            let block_structure = self.dna.by_index(self.blocks[block].dna_index)?;
            self.check_target(type_structure, block_structure)?;
            let previous = self.position();
            self.seek_block(block, field.value)?;
            let count = self.element_count(block, block_structure)?;
            check_allocation(count, element_size)?;
            out.resize(count as usize, T::default());
            for slot in out.iter_mut() {
                self.convert_value(type_structure, slot, ARRAY_FRAME)?;
            }
            self.set_position(previous)?;
        }
        self.set_position(old)?;
        Ok(())
    }

    /// `ReadFieldPtr` to an array of pointers to objects of a static type
    /// (`Mesh::mat`): the block's pointers, each resolved.
    pub fn read_pointer_array<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Vec<Option<usize>>) -> Outcome<()> {
        let old = self.position();
        out.clear();
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(());
        };
        if field.value != 0 {
            let block = self.locate(field.value)?;
            let pointer_size = if self.pointer_64 { 8 } else { 4 };
            let count = self.blocks[block].size / pointer_size;
            let previous = self.position();
            self.seek_block(block, field.value)?;
            check_allocation(count, SHARED_POINTER_SIZE)?;
            out.resize(count as usize, None);
            for slot in out.iter_mut() {
                let value = self.convert_pointer()?;
                *slot = None;
                let element = PointerField { value, type_structure: field.type_structure, type_name: field.type_name.clone() };
                let (index, _) = self.resolve::<T>(&element, false, POINTER_ARRAY_FRAME)?;
                *slot = index;
            }
            self.set_position(previous)?;
        }
        self.set_position(old)?;
        Ok(())
    }

    /// `ReadFieldPtr` to `N` pointers (`Material::mtex`).
    pub fn read_pointers<T: Typed, const N: usize>(&mut self, structure: usize, policy: Policy, name: &str, out: &mut [Option<usize>; N]) -> Outcome<()> {
        let old = self.position();
        let attempt = (|| {
            let field = self.field(structure, name.as_bytes())?;
            self.advance(field.offset)?;
            let mut values = [0u64; N];
            let count = field.array_sizes[0].min(N as u64) as usize;
            for value in values.iter_mut().take(count) {
                *value = self.convert_pointer()?;
            }
            Ok((values, field))
        })();
        let (values, field) = match attempt {
            Ok(read) => read,
            Err(Failure::Dna(message)) => {
                policy.recover::<()>(message)?;
                *out = [None; N];
                return Ok(());
            }
            Err(failure) => return Err(failure),
        };
        for (slot, &value) in out.iter_mut().zip(values.iter()) {
            *slot = None;
            let element = PointerField { value, type_structure: field.type_structure, type_name: field.type_name.clone() };
            let (index, _) = self.resolve::<T>(&element, false, POINTERS_FRAME)?;
            *slot = index;
        }
        self.set_position(old)?;
        Ok(())
    }

    /// `ReadFieldPtrVector` (`CustomData::layers`): as many new objects as
    /// the target block's count, uncached.
    pub fn read_pointer_vector<T: Typed>(&mut self, structure: usize, policy: Policy, name: &str, out: &mut Vec<usize>) -> Outcome<()> {
        out.clear();
        let old = self.position();
        let Some(field) = self.read_pointer_field(structure, policy, name, true)? else {
            return Ok(());
        };
        if field.value != 0 {
            let block = self.locate(field.value)?;
            self.seek_block(block, field.value)?;
            let type_structure = self.type_for_pointer(&field)?;
            let count = self.blocks[block].num;
            if self.dna.structures[type_structure].size == 0 && count > 1 << 24 {
                // Each element reads the same bytes: memory runs out first.
                return import_error("bad allocation");
            }
            for _ in 0..count {
                let slot = T::arena(&mut self.store).len();
                T::arena(&mut self.store).push(T::default());
                self.convert_slot::<T>(type_structure, slot, ARRAY_FRAME + T::KIND.convert_frame())?;
                out.push(slot);
            }
        }
        self.set_position(old)?;
        Ok(())
    }

    /// `ReadCustomDataPtr`: the layer's array, read as its layer type says.
    pub fn read_custom_data(&mut self, structure: usize, policy: Policy, name: &str, layer_type: i32, out: &mut Option<CustomArray>) -> Outcome<()> {
        let old = self.position();
        let value = match self.read_pointer_field(structure, policy, name, true)? {
            Some(field) => field.value,
            None => {
                *out = None;
                0
            }
        };
        if value != 0 {
            let block = self.locate(value)?;
            self.seek_block(block, value)?;
            let count = self.blocks[block].num;
            super::structures::read_custom_array(self, layer_type, count, out)?;
        }
        self.set_position(old)?;
        Ok(())
    }

    /// The structure a non-null pointer field's type names.
    fn type_for_pointer(&self, field: &PointerField) -> Outcome<usize> {
        match field.type_structure {
            Some(index) => Ok(index),
            None => dna_error(format!("BlendDNA: Did not find a structure named `{}`", text(&field.type_name))),
        }
    }

    /// The block's structure must be the field's (by name).
    fn check_target(&self, expected: usize, actual: usize) -> Outcome<()> {
        if self.dna.structures[expected].name != self.dna.structures[actual].name {
            let expected = text(&self.dna.structures[expected].name);
            let actual = text(&self.dna.structures[actual].name);
            return dna_error(format!("Expected target to be of type `{expected}` but seemingly it is a `{actual}` instead"));
        }
        Ok(())
    }

    /// `block->size / ss.size` (a structure of no size divides by zero).
    fn element_count(&self, block: usize, block_structure: usize) -> Outcome<u64> {
        let size = self.dna.structures[block_structure].size;
        if size == 0 {
            return undefined("a Blender block counted in elements of no size");
        }
        Ok(self.blocks[block].size / size)
    }

    /// `ResolvePointer` to an object of a static type: the field's type
    /// must be the block's, the cache first, then a new object converted
    /// (under a reader whose frame takes `read_frame`).
    pub fn resolve<T: Typed>(&mut self, field: &PointerField, non_recursive: bool, read_frame: u64) -> Outcome<(Option<usize>, bool)> {
        let pointer = field.value;
        if pointer == 0 {
            return Ok((None, false));
        }
        let structure = self.type_for_pointer(field)?;
        let block = self.locate(pointer)?;
        let block_structure = self.dna.by_index(self.blocks[block].dna_index)?;
        self.check_target(structure, block_structure)?;
        if let Some(&cached) = self.cache.get(&(structure, pointer)) {
            if cached.kind != T::KIND {
                return undefined("a cached Blender object taken as another type");
            }
            return Ok((Some(cached.index), true));
        }
        let previous = self.position();
        self.seek_block(block, pointer)?;
        self.element_count(block, block_structure)?;
        let arena = T::arena(&mut self.store);
        arena.push(T::default());
        let index = arena.len() - 1;
        self.cache.insert((structure, pointer), Element { kind: T::KIND, index });
        if !non_recursive {
            self.convert_slot::<T>(structure, index, read_frame + T::KIND.convert_frame())?;
            self.set_position(previous)?;
        }
        Ok((Some(index), false))
    }

    /// `ResolvePointer` to an `ElemBase`: the block's structure picks the
    /// converter; none leaves a null (and the stream at the block).
    pub fn resolve_dynamic(&mut self, pointer: u64) -> Outcome<(Option<Element>, bool)> {
        if pointer == 0 {
            return Ok((None, false));
        }
        let block = self.locate(pointer)?;
        let structure = self.dna.by_index(self.blocks[block].dna_index)?;
        if let Some(&cached) = self.cache.get(&(structure, pointer)) {
            return Ok((Some(cached), true));
        }
        let previous = self.position();
        self.seek_block(block, pointer)?;
        let Some(kind) = Kind::for_name(&self.dna.structures[structure].name) else {
            return Ok((None, false));
        };
        let element = self.store.allocate(kind);
        self.cache.insert((structure, pointer), element);
        convert_element(self, structure, element, DYNAMIC_FRAME + kind.registered_frame())?;
        self.set_position(previous)?;
        self.dna_types.insert(element, structure);
        Ok((Some(element), false))
    }

    /// `Convert<Pointer>`.
    pub fn convert_pointer(&mut self) -> Outcome<u64> {
        if self.pointer_64 {
            return Ok(self.reader.get_u8()?);
        }
        Ok(self.reader.get_u4()? as u64)
    }

    fn unknown_primitive<T>(&self, structure: usize) -> Outcome<T> {
        import_error(format!("Unknown source for conversion to primitive data type: {}", text(self.structure_name(structure))))
    }

    /// `Convert<int>`: other integers zero-extended, floats truncated.
    pub fn convert_i32(&mut self, structure: usize) -> Outcome<i32> {
        let value = match self.structure_name(structure) {
            b"int" => self.reader.get_u4()? as i32,
            b"short" => self.reader.get_u2()? as i32,
            b"char" => self.reader.get_u1()? as i32,
            b"float" => to_int(self.reader.get_f4()? as f64),
            b"double" => to_int(self.reader.get_f8()?),
            _ => return self.unknown_primitive(structure),
        };
        Ok(value)
    }

    /// `Convert<short>`: floats scaled by 32767 (clamped above 1).
    pub fn convert_i16(&mut self, structure: usize) -> Outcome<i16> {
        let value = match self.structure_name(structure) {
            b"float" => {
                let mut value = self.reader.get_f4()?;
                if value > 1.0 {
                    value = 1.0;
                }
                to_int((value * 32767.0) as f64) as i16
            }
            b"double" => to_int(self.reader.get_f8()? * 32767.0) as i16,
            b"int" => self.reader.get_u4()? as i16,
            b"short" => self.reader.get_u2()? as i16,
            b"char" => self.reader.get_u1()? as i16,
            _ => return self.unknown_primitive(structure),
        };
        Ok(value)
    }

    /// `Convert<char>`: floats scaled by 255.
    pub fn convert_i8(&mut self, structure: usize) -> Outcome<i8> {
        Ok(self.convert_byte(structure)? as i8)
    }

    /// `Convert<unsigned char>`.
    pub fn convert_u8(&mut self, structure: usize) -> Outcome<u8> {
        self.convert_byte(structure)
    }

    fn convert_byte(&mut self, structure: usize) -> Outcome<u8> {
        let value = match self.structure_name(structure) {
            b"float" => to_int((self.reader.get_f4()? * 255.0) as f64) as u8,
            b"double" => to_int(self.reader.get_f8()? * 255.0) as u8,
            b"int" => self.reader.get_u4()? as u8,
            b"short" => self.reader.get_u2()? as u8,
            b"char" => self.reader.get_u1()?,
            _ => return self.unknown_primitive(structure),
        };
        Ok(value)
    }

    /// `Convert<float>`: chars and shorts scaled to [-1, 1], ints as
    /// unsigned.
    pub fn convert_f32(&mut self, structure: usize) -> Outcome<f32> {
        let value = match self.structure_name(structure) {
            b"char" => self.reader.get_i1()? as f32 / 255.0,
            b"short" => self.reader.get_i2()? as f32 / 32767.0,
            b"int" => self.reader.get_u4()? as f32,
            b"float" => self.reader.get_f4()?,
            b"double" => self.reader.get_f8()? as f32,
            _ => return self.unknown_primitive(structure),
        };
        Ok(value)
    }
}

/// `std::vector<FileBlockHead>` growth for the blocks read.
pub(super) fn block_capacity() -> VectorCapacity {
    VectorCapacity::new(BLOCK_SIZE)
}
