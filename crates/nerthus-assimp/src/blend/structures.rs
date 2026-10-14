//! The structures Assimp's Blender importer converts (`BlenderScene.h`)
//! and their converters (`BlenderScene.cpp`), each field read in the
//! generated order with its error policy. Fields that only matter for
//! whether a read fails are read and dropped.

use super::dna::{ARRAY_FRAME, CustomArray, Convert, Database, Outcome, Policy, Typed, dna_error};
use crate::scene::check_allocation;

/// `sizeof` on MSVC x64 of the element types arrays are allocated for.
pub(super) const MFACE_SIZE: u64 = 40;
pub(super) const MTFACE_SIZE: u64 = 56;
pub(super) const TFACE_SIZE: u64 = 72;
pub(super) const MVERT_SIZE: u64 = 56;
pub(super) const MEDGE_SIZE: u64 = 32;
pub(super) const MLOOP_SIZE: u64 = 24;
pub(super) const MLOOPUV_SIZE: u64 = 32;
pub(super) const MLOOPCOL_SIZE: u64 = 24;
pub(super) const MPOLY_SIZE: u64 = 32;
pub(super) const MTEXPOLY_SIZE: u64 = 40;
pub(super) const MDEFORMVERT_SIZE: u64 = 48;
pub(super) const MDEFORMWEIGHT_SIZE: u64 = 24;
pub(super) const MCOL_SIZE: u64 = 24;

/// `CD_NUMTYPES`, and the custom data types `readCustomData` reads.
const CUSTOM_DATA_TYPES: i32 = 42;
pub(super) const CD_MLOOPUV: i32 = 16;

macro_rules! kinds {
    ($($kind:ident = $name:literal => $arena:ident),* $(,)?) => {
        /// A converter `DNA::RegisterConverters` registers, by structure
        /// name.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub(super) enum Kind {
            $($kind),*
        }

        /// The converted objects, by type.
        #[derive(Default)]
        pub(super) struct Store {
            $(pub $arena: Vec<$kind>),*
        }

        impl Kind {
            pub fn for_name(name: &[u8]) -> Option<Kind> {
                $(if name == $name.as_bytes() {
                    return Some(Kind::$kind);
                })*
                None
            }
        }

        impl Store {
            /// `Structure::Allocate`.
            pub fn allocate(&mut self, kind: Kind) -> Element {
                match kind {
                    $(Kind::$kind => {
                        self.$arena.push(<$kind>::default());
                        Element { kind, index: self.$arena.len() - 1 }
                    })*
                }
            }
        }

        /// The registered converter of an object's type, in place.
        pub(super) fn convert_element(db: &mut Database, structure: usize, element: Element, frames: u64) -> Outcome<()> {
            match element.kind {
                $(Kind::$kind => db.convert_slot::<$kind>(structure, element.index, frames)),*
            }
        }

        $(impl Typed for $kind {
            const KIND: Kind = Kind::$kind;
            fn arena(store: &mut Store) -> &mut Vec<Self> {
                &mut store.$arena
            }
        })*
    };
}

kinds! {
    Object = "Object" => objects,
    Group = "Group" => groups,
    MTex = "MTex" => mtexes,
    TFace = "TFace" => tfaces,
    SubsurfModifierData = "SubsurfModifierData" => subsurf_modifiers,
    MFace = "MFace" => mfaces,
    Lamp = "Lamp" => lamps,
    MDeformWeight = "MDeformWeight" => deform_weights,
    PackedFile = "PackedFile" => packed_files,
    Base = "Base" => bases,
    MTFace = "MTFace" => mtfaces,
    Material = "Material" => materials,
    MTexPoly = "MTexPoly" => mtexpolys,
    Mesh = "Mesh" => meshes,
    MDeformVert = "MDeformVert" => deform_verts,
    World = "World" => worlds,
    MLoopCol = "MLoopCol" => loop_colors,
    MVert = "MVert" => verts,
    MEdge = "MEdge" => edges,
    MLoopUV = "MLoopUV" => loop_uvs,
    GroupObject = "GroupObject" => group_objects,
    ListBase = "ListBase" => list_bases,
    MLoop = "MLoop" => loops,
    ModifierData = "ModifierData" => modifiers,
    Id = "ID" => ids,
    MCol = "MCol" => mcols,
    MPoly = "MPoly" => polys,
    Scene = "Scene" => scenes,
    Library = "Library" => libraries,
    Tex = "Tex" => texes,
    Camera = "Camera" => cameras,
    MirrorModifierData = "MirrorModifierData" => mirror_modifiers,
    Image = "Image" => images,
    CustomData = "CustomData" => custom_data,
    CustomDataLayer = "CustomDataLayer" => custom_data_layers,
    Collection = "Collection" => collections,
    CollectionChild = "CollectionChild" => collection_children,
    CollectionObject = "CollectionObject" => collection_objects,
}

impl Kind {
    /// The stack frame of the C++'s `Structure::Convert<T>` for the type
    /// (MSVC x64, from the DLL's unwind data). A world's converter is
    /// inlined into the one pointer read that reaches it.
    pub fn convert_frame(self) -> u64 {
        match self {
            Kind::World => 0,
            Kind::Object | Kind::MirrorModifierData => 128,
            Kind::ListBase | Kind::ModifierData => 112,
            Kind::Group
            | Kind::MTex
            | Kind::PackedFile
            | Kind::Base
            | Kind::Material
            | Kind::MTexPoly
            | Kind::Mesh
            | Kind::MDeformVert
            | Kind::GroupObject
            | Kind::Scene
            | Kind::Library
            | Kind::Tex
            | Kind::Camera
            | Kind::Image
            | Kind::CustomDataLayer
            | Kind::CollectionChild
            | Kind::CollectionObject => 96,
            _ => 80,
        }
    }

    /// The stack frame of the registered converter (the wrapper taking an
    /// `ElemBase`) with the frame of the converter it calls; five of
    /// them have their converter inlined.
    pub fn registered_frame(self) -> u64 {
        match self {
            Kind::SubsurfModifierData | Kind::World => 80,
            Kind::Scene | Kind::Camera => 96,
            Kind::MirrorModifierData => 128,
            _ => 48 + self.convert_frame(),
        }
    }
}

/// A converted object of any type (`std::shared_ptr<ElemBase>`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct Element {
    pub kind: Kind,
    pub index: usize,
}

const NAME_LENGTH: usize = 1024;

/// `ID`: a name ("OB", "ME" … then the name) and flags.
#[derive(Clone)]
pub(super) struct Id {
    pub name: Vec<u8>,
    pub flag: i16,
}

impl Default for Id {
    fn default() -> Self {
        Id { name: vec![0; NAME_LENGTH], flag: 0 }
    }
}

impl Id {
    /// The name as a C string from `offset` (`id.name + 2`). A name that
    /// fills its array runs on into `flag`, then the zeroed padding.
    pub fn c_str(&self, offset: usize) -> Vec<u8> {
        let tail = self.name.get(offset..).unwrap_or(&[]);
        let flag = self.flag.to_le_bytes();
        let bytes = tail.iter().chain(flag.iter());
        bytes.take_while(|&&byte| byte != 0).copied().collect()
    }
}

impl Convert for Id {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        let name: [u8; NAME_LENGTH] = db.read_array(s, Policy::Warn, "name", Database::convert_u8)?;
        out.name = name.to_vec();
        out.flag = db.read_i16(s, Policy::Ignore, "flag")?;
        advance(db, s)
    }
}

/// The end of every converter: `IncPtr(size)`.
fn advance(db: &mut Database, s: usize) -> Outcome<()> {
    let size = db.dna.structures[s].size;
    db.advance(size)
}

/// `ListBase`: the first and last elements, of any type.
#[derive(Clone, Default)]
pub(super) struct ListBase {
    pub first: Option<Element>,
}

impl Convert for ListBase {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_dynamic(s, Policy::Ignore, "*first", &mut out.first)?;
        let mut last = None;
        db.read_dynamic(s, Policy::Ignore, "*last", &mut last)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct PackedFile {
    pub size: i32,
    pub data: Option<u64>,
}

impl Convert for PackedFile {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.size = db.read_i32(s, Policy::Warn, "size")?;
        db.read_i32(s, Policy::Warn, "seek")?;
        db.read_file_offset(s, Policy::Warn, "*data", &mut out.data)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct GroupObject {
    pub prev: Option<usize>,
    pub next: Option<usize>,
    pub ob: Option<usize>,
}

impl Convert for GroupObject {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_pointer::<GroupObject>(s, Policy::Fail, "*prev", &mut out.prev)?;
        db.read_pointer::<GroupObject>(s, Policy::Fail, "*next", &mut out.next)?;
        db.read_pointer::<Object>(s, Policy::Ignore, "*ob", &mut out.ob)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Group {
    pub id: Id,
    pub gobject: Option<usize>,
}

impl Convert for Group {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.id = db.read_struct(s, Policy::Fail, "id")?;
        db.read_i32(s, Policy::Ignore, "layer")?;
        db.read_pointer::<GroupObject>(s, Policy::Ignore, "*gobject", &mut out.gobject)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct CollectionObject {
    pub next: Option<usize>,
    pub ob: Option<usize>,
}

impl Convert for CollectionObject {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_pointer::<CollectionObject>(s, Policy::Fail, "*next", &mut out.next)?;
        db.read_pointer::<Object>(s, Policy::Ignore, "*ob", &mut out.ob)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct CollectionChild {
    pub next: Option<usize>,
    pub collection: Option<usize>,
}

impl Convert for CollectionChild {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        let mut prev = None;
        db.read_pointer::<CollectionChild>(s, Policy::Fail, "*prev", &mut prev)?;
        db.read_pointer::<CollectionChild>(s, Policy::Fail, "*next", &mut out.next)?;
        db.read_pointer::<Collection>(s, Policy::Ignore, "*collection", &mut out.collection)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Collection {
    pub id: Id,
    pub gobject: ListBase,
    pub children: ListBase,
}

impl Convert for Collection {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.id = db.read_struct(s, Policy::Fail, "id")?;
        out.gobject = db.read_struct(s, Policy::Fail, "gobject")?;
        out.children = db.read_struct(s, Policy::Fail, "children")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct World;

impl Convert for World {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MVert {
    pub co: [f32; 3],
    pub no: [f32; 3],
}

impl Convert for MVert {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.co = db.read_array(s, Policy::Fail, "co", Database::convert_f32)?;
        out.no = db.read_array(s, Policy::Warn, "no", Database::convert_f32)?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        db.read_i32(s, Policy::Ignore, "bweight")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MEdge;

impl Convert for MEdge {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_i32(s, Policy::Fail, "v1")?;
        db.read_i32(s, Policy::Fail, "v2")?;
        db.read_i8(s, Policy::Ignore, "crease")?;
        db.read_i8(s, Policy::Ignore, "bweight")?;
        db.read_i16(s, Policy::Ignore, "flag")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MLoop {
    pub v: i32,
}

impl Convert for MLoop {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.v = db.read_i32(s, Policy::Ignore, "v")?;
        db.read_i32(s, Policy::Ignore, "e")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MLoopUV {
    pub uv: [f32; 2],
}

impl Convert for MLoopUV {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.uv = db.read_array(s, Policy::Ignore, "uv", Database::convert_f32)?;
        db.read_i32(s, Policy::Ignore, "flag")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MLoopCol {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Convert for MLoopCol {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.r = db.read_u8(s, Policy::Ignore, "r")?;
        out.g = db.read_u8(s, Policy::Ignore, "g")?;
        out.b = db.read_u8(s, Policy::Ignore, "b")?;
        out.a = db.read_u8(s, Policy::Ignore, "a")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MPoly {
    pub loopstart: i32,
    pub totloop: i32,
    pub mat_nr: i16,
}

impl Convert for MPoly {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.loopstart = db.read_i32(s, Policy::Ignore, "loopstart")?;
        out.totloop = db.read_i32(s, Policy::Ignore, "totloop")?;
        out.mat_nr = db.read_i16(s, Policy::Ignore, "mat_nr")?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MTexPoly;

impl Convert for MTexPoly {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        let mut tpage = None;
        db.read_pointer::<Image>(s, Policy::Ignore, "*tpage", &mut tpage)?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        db.read_i8(s, Policy::Ignore, "transp")?;
        db.read_i16(s, Policy::Ignore, "mode")?;
        db.read_i16(s, Policy::Ignore, "tile")?;
        db.read_i16(s, Policy::Ignore, "pad")?;
        advance(db, s)
    }
}

/// `MCol`: `char`s, signed.
#[derive(Clone, Copy, Default)]
pub(super) struct MCol {
    pub r: i8,
    pub g: i8,
    pub b: i8,
    pub a: i8,
}

impl Convert for MCol {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.r = db.read_i8(s, Policy::Fail, "r")?;
        out.g = db.read_i8(s, Policy::Fail, "g")?;
        out.b = db.read_i8(s, Policy::Fail, "b")?;
        out.a = db.read_i8(s, Policy::Fail, "a")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MFace {
    pub v1: i32,
    pub v2: i32,
    pub v3: i32,
    pub v4: i32,
    pub mat_nr: i32,
}

impl Convert for MFace {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.v1 = db.read_i32(s, Policy::Fail, "v1")?;
        out.v2 = db.read_i32(s, Policy::Fail, "v2")?;
        out.v3 = db.read_i32(s, Policy::Fail, "v3")?;
        out.v4 = db.read_i32(s, Policy::Fail, "v4")?;
        out.mat_nr = db.read_i32(s, Policy::Fail, "mat_nr")?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct TFace {
    pub uv: [[f32; 2]; 4],
}

impl Convert for TFace {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.uv = db.read_array2(s, Policy::Fail, "uv", Database::convert_f32)?;
        db.read_array::<4, i32>(s, Policy::Fail, "col", Database::convert_i32)?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        db.read_i16(s, Policy::Ignore, "mode")?;
        db.read_i16(s, Policy::Ignore, "tile")?;
        db.read_i16(s, Policy::Ignore, "unwrap")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MTFace {
    pub uv: [[f32; 2]; 4],
}

impl Convert for MTFace {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.uv = db.read_array2(s, Policy::Fail, "uv", Database::convert_f32)?;
        db.read_i8(s, Policy::Ignore, "flag")?;
        db.read_i16(s, Policy::Ignore, "mode")?;
        db.read_i16(s, Policy::Ignore, "tile")?;
        db.read_i16(s, Policy::Ignore, "unwrap")?;
        advance(db, s)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct MDeformWeight;

impl Convert for MDeformWeight {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_i32(s, Policy::Fail, "def_nr")?;
        db.read_f32(s, Policy::Fail, "weight")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct MDeformVert {
    dw: Vec<MDeformWeight>,
}

impl Convert for MDeformVert {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_array_pointer(s, Policy::Warn, "*dw", MDEFORMWEIGHT_SIZE, &mut out.dw)?;
        db.read_i32(s, Policy::Ignore, "totweight")?;
        advance(db, s)
    }
}

/// The floats `Convert<Material>` reads after the textures.
const MATERIAL_FLOATS: [&str; 50] = [
    "amb", "ang", "spectra", "spec", "zoffs", "add", "fresnel_mir", "fresnel_mir_i", "fresnel_tra", "fresnel_tra_i", "filter", "tx_limit",
    "tx_falloff", "gloss_mir", "gloss_tra", "adapt_thresh_mir", "adapt_thresh_tra", "aniso_gloss_mir", "dist_mir", "hasize", "flaresize",
    "subsize", "flareboost", "strand_sta", "strand_end", "strand_ease", "strand_surfnor", "strand_min", "strand_widthfade", "sbias",
    "lbias", "shad_alpha", "param", "rms", "rampfac_col", "rampfac_spec", "friction", "fh", "reflect", "fhdist", "xyfrict", "sss_radius",
    "sss_col", "sss_error", "sss_scale", "sss_ior", "sss_colfac", "sss_texfac", "sss_front", "sss_back",
];

/// Then its shorts (the shaders read a second time).
const MATERIAL_SHORTS: [&str; 29] = [
    "material_type", "flag", "ray_depth", "ray_depth_tra", "samp_gloss_mir", "samp_gloss_tra", "fadeto_mir", "shade_flag", "flarec",
    "starc", "linec", "ringc", "pr_lamp", "pr_texture", "ml_flag", "diff_shader", "spec_shader", "texco", "mapto", "ramp_show", "pad3",
    "dynamode", "pad2", "sss_flag", "sss_preset", "shadowonly_flag", "index", "vcol_alpha", "pad4",
];

/// `Material`: only its name, textures and whether its reads fail
/// matter here.
#[derive(Clone, Default)]
pub(super) struct Material {
    pub id: Id,
    pub mtex: [Option<usize>; 18],
}

impl Convert for Material {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.id = db.read_struct(s, Policy::Fail, "id")?;
        for name in ["r", "g", "b", "specr", "specg", "specb"] {
            db.read_f32(s, Policy::Warn, name)?;
        }
        db.read_i16(s, Policy::Ignore, "har")?;
        for name in ["ambr", "ambg", "ambb"] {
            db.read_f32(s, Policy::Warn, name)?;
        }
        for name in ["mirr", "mirg", "mirb"] {
            db.read_f32(s, Policy::Ignore, name)?;
        }
        db.read_f32(s, Policy::Warn, "emit")?;
        db.read_f32(s, Policy::Ignore, "ray_mirror")?;
        db.read_f32(s, Policy::Warn, "alpha")?;
        db.read_f32(s, Policy::Ignore, "ref")?;
        db.read_f32(s, Policy::Ignore, "translucency")?;
        db.read_i32(s, Policy::Ignore, "mode")?;
        for name in ["roughness", "darkness", "refrac"] {
            db.read_f32(s, Policy::Ignore, name)?;
        }
        let mut group = None;
        db.read_pointer::<Group>(s, Policy::Ignore, "*group", &mut group)?;
        db.read_i16(s, Policy::Warn, "diff_shader")?;
        db.read_i16(s, Policy::Warn, "spec_shader")?;
        db.read_pointers::<MTex, 18>(s, Policy::Ignore, "*mtex", &mut out.mtex)?;
        for name in MATERIAL_FLOATS {
            db.read_f32(s, Policy::Ignore, name)?;
        }
        for name in MATERIAL_SHORTS {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        db.read_i8(s, Policy::Ignore, "seed1")?;
        db.read_i8(s, Policy::Ignore, "seed2")?;
        advance(db, s)
    }
}

/// `CustomDataLayer`: its type, name and (read as its type says) data.
#[derive(Clone, Default)]
pub(super) struct CustomDataLayer {
    pub layer_type: i32,
    pub name: Vec<u8>,
    pub data: Option<CustomArray>,
}

impl Convert for CustomDataLayer {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.layer_type = db.read_i32(s, Policy::Fail, "type")?;
        for name in ["offset", "flag", "active", "active_rnd"] {
            db.read_i32(s, Policy::Fail, name)?;
        }
        for name in ["active_clone", "active_mask", "uid"] {
            db.read_i32(s, Policy::Warn, name)?;
        }
        let name: [u8; 64] = db.read_array(s, Policy::Warn, "name", Database::convert_u8)?;
        out.name = name.to_vec();
        let layer_type = out.layer_type;
        db.read_custom_data(s, Policy::Fail, "*data", layer_type, &mut out.data)?;
        advance(db, s)
    }
}

/// `CustomData`: its layers (the type map and counts are only read).
#[derive(Clone, Default)]
pub(super) struct CustomData {
    pub layers: Vec<usize>,
}

impl Convert for CustomData {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_array::<42, i32>(s, Policy::Warn, "typemap", Database::convert_i32)?;
        for name in ["totlayer", "maxlayer", "totsize"] {
            db.read_i32(s, Policy::Warn, name)?;
        }
        db.read_pointer_vector::<CustomDataLayer>(s, Policy::Warn, "*layers", &mut out.layers)?;
        advance(db, s)
    }
}

/// `readCustomData`: an array of the layer type's elements (types beyond
/// `CD_NUMTYPES` throw; types without a reader, or no elements, leave
/// what was there). A layer whose array fails is dropped, so only a whole
/// array is kept, and only UVs are looked at again.
pub(super) fn read_custom_array(db: &mut Database, layer_type: i32, count: u64, out: &mut Option<CustomArray>) -> Outcome<()> {
    if !(0..CUSTOM_DATA_TYPES).contains(&layer_type) {
        return dna_error(format!("CustomData.type {layer_type} out of index"));
    }
    let (element_size, name) = match layer_type {
        0 => (MVERT_SIZE, "MVert"),
        3 => (MEDGE_SIZE, "MEdge"),
        4 => (MFACE_SIZE, "MFace"),
        5 => (MTFACE_SIZE, "MTFace"),
        15 => (MTEXPOLY_SIZE, "MTexPoly"),
        CD_MLOOPUV => (MLOOPUV_SIZE, "MLoopUV"),
        17 => (MLOOPCOL_SIZE, "MLoopCol"),
        25 => (MPOLY_SIZE, "MPoly"),
        26 => (MLOOP_SIZE, "MLoop"),
        _ => return Ok(()),
    };
    if count == 0 {
        return Ok(());
    }
    check_allocation(count, element_size)?;
    let structure = db.dna.by_name(name.as_bytes())?;
    let array = match layer_type {
        CD_MLOOPUV => CustomArray::LoopUvs(read_elements::<MLoopUV>(db, structure, count)?),
        0 => discard(read_elements::<MVert>(db, structure, count)?),
        3 => discard(read_elements::<MEdge>(db, structure, count)?),
        4 => discard(read_elements::<MFace>(db, structure, count)?),
        5 => discard(read_elements::<MTFace>(db, structure, count)?),
        15 => discard(read_elements::<MTexPoly>(db, structure, count)?),
        17 => discard(read_elements::<MLoopCol>(db, structure, count)?),
        25 => discard(read_elements::<MPoly>(db, structure, count)?),
        _ => discard(read_elements::<MLoop>(db, structure, count)?),
    };
    *out = Some(array);
    Ok(())
}

/// `count` elements read one after another (each a copy converted).
fn read_elements<T: Typed>(db: &mut Database, structure: usize, count: u64) -> Outcome<Vec<T>> {
    let mut elements = Vec::with_capacity(count.min(1 << 20) as usize);
    for _ in 0..count {
        let mut element = T::default();
        db.convert_value(structure, &mut element, ARRAY_FRAME)?;
        elements.push(element);
    }
    Ok(elements)
}

/// An array whose contents are never looked at again.
fn discard<T>(_elements: Vec<T>) -> CustomArray {
    CustomArray::Other
}

/// `Mesh`: the counts and arrays the importer reads.
#[derive(Clone, Default)]
pub(super) struct Mesh {
    pub id: Id,
    pub totface: i32,
    pub totvert: i32,
    pub totloop: i32,
    pub totpoly: i32,
    pub mface: Vec<MFace>,
    pub mtface: Vec<MTFace>,
    pub tface: Vec<TFace>,
    pub mvert: Vec<MVert>,
    pub mloop: Vec<MLoop>,
    pub mloopuv: Vec<MLoopUV>,
    pub mloopcol: Vec<MLoopCol>,
    pub mpoly: Vec<MPoly>,
    pub mcol: Vec<MCol>,
    pub mat: Vec<Option<usize>>,
    pub ldata: CustomData,
}

impl Convert for Mesh {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.id = db.read_struct(s, Policy::Fail, "id")?;
        out.totface = db.read_i32(s, Policy::Fail, "totface")?;
        db.read_i32(s, Policy::Fail, "totedge")?;
        out.totvert = db.read_i32(s, Policy::Fail, "totvert")?;
        out.totloop = db.read_i32(s, Policy::Ignore, "totloop")?;
        out.totpoly = db.read_i32(s, Policy::Ignore, "totpoly")?;
        for name in ["subdiv", "subdivr", "subsurftype", "smoothresh"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        db.read_array_pointer(s, Policy::Fail, "*mface", MFACE_SIZE, &mut out.mface)?;
        db.read_array_pointer(s, Policy::Ignore, "*mtface", MTFACE_SIZE, &mut out.mtface)?;
        db.read_array_pointer(s, Policy::Ignore, "*tface", TFACE_SIZE, &mut out.tface)?;
        db.read_array_pointer(s, Policy::Fail, "*mvert", MVERT_SIZE, &mut out.mvert)?;
        let mut medge: Vec<MEdge> = Vec::new();
        db.read_array_pointer(s, Policy::Warn, "*medge", MEDGE_SIZE, &mut medge)?;
        db.read_array_pointer(s, Policy::Ignore, "*mloop", MLOOP_SIZE, &mut out.mloop)?;
        db.read_array_pointer(s, Policy::Ignore, "*mloopuv", MLOOPUV_SIZE, &mut out.mloopuv)?;
        db.read_array_pointer(s, Policy::Ignore, "*mloopcol", MLOOPCOL_SIZE, &mut out.mloopcol)?;
        db.read_array_pointer(s, Policy::Ignore, "*mpoly", MPOLY_SIZE, &mut out.mpoly)?;
        let mut mtpoly: Vec<MTexPoly> = Vec::new();
        db.read_array_pointer(s, Policy::Ignore, "*mtpoly", MTEXPOLY_SIZE, &mut mtpoly)?;
        let mut dvert: Vec<MDeformVert> = Vec::new();
        db.read_array_pointer(s, Policy::Ignore, "*dvert", MDEFORMVERT_SIZE, &mut dvert)?;
        db.read_array_pointer(s, Policy::Ignore, "*mcol", MCOL_SIZE, &mut out.mcol)?;
        db.read_pointer_array::<Material>(s, Policy::Fail, "**mat", &mut out.mat)?;
        for name in ["vdata", "edata", "fdata", "pdata"] {
            db.read_struct::<CustomData>(s, Policy::Ignore, name)?;
        }
        out.ldata = db.read_struct(s, Policy::Warn, "ldata")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Library;

impl Convert for Library {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        db.read_array::<240, u8>(s, Policy::Warn, "name", Database::convert_u8)?;
        db.read_array::<240, u8>(s, Policy::Fail, "filename", Database::convert_u8)?;
        let mut parent = None;
        db.read_pointer::<Library>(s, Policy::Warn, "*parent", &mut parent)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Camera;

impl Convert for Camera {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        db.read_i32(s, Policy::Warn, "type")?;
        db.read_i32(s, Policy::Warn, "flag")?;
        db.read_f32(s, Policy::Warn, "lens")?;
        db.read_f32(s, Policy::Warn, "sensor_x")?;
        db.read_f32(s, Policy::Ignore, "clipsta")?;
        db.read_f32(s, Policy::Ignore, "clipend")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Lamp;

impl Convert for Lamp {
    fn convert(db: &mut Database, s: usize, _out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        db.read_i32(s, Policy::Fail, "type")?;
        for name in ["flag", "colormodel", "totex"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        for name in ["r", "g", "b", "k"] {
            db.read_f32(s, Policy::Warn, name)?;
        }
        db.read_f32(s, Policy::Ignore, "energy")?;
        db.read_f32(s, Policy::Warn, "dist")?;
        db.read_f32(s, Policy::Ignore, "spotsize")?;
        db.read_f32(s, Policy::Ignore, "spotblend")?;
        for name in ["coeff_const", "coeff_lin", "coeff_quad"] {
            db.read_f32(s, Policy::Warn, name)?;
        }
        db.read_f32(s, Policy::Ignore, "att1")?;
        db.read_f32(s, Policy::Ignore, "att2")?;
        db.read_i32(s, Policy::Ignore, "falloff_type")?;
        for name in ["sun_brightness", "area_size", "area_sizey", "area_sizez"] {
            db.read_f32(s, Policy::Ignore, name)?;
        }
        db.read_i16(s, Policy::Ignore, "area_shape")?;
        advance(db, s)
    }
}

/// `ModifierData`: the next and previous modifiers, its type and name.
#[derive(Clone, Default)]
pub(super) struct ModifierData {
    pub next: Option<Element>,
    pub prev: Option<Element>,
    pub modifier_type: i32,
}

impl Convert for ModifierData {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_dynamic(s, Policy::Warn, "*next", &mut out.next)?;
        db.read_dynamic(s, Policy::Warn, "*prev", &mut out.prev)?;
        out.modifier_type = db.read_i32(s, Policy::Ignore, "type")?;
        db.read_i32(s, Policy::Ignore, "mode")?;
        db.read_array::<32, u8>(s, Policy::Ignore, "name", Database::convert_u8)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct SubsurfModifierData {
    pub modifier: ModifierData,
    pub subdiv_type: i16,
    pub levels: i16,
    pub render_levels: i16,
}

impl Convert for SubsurfModifierData {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.modifier = db.read_struct(s, Policy::Fail, "modifier")?;
        out.subdiv_type = db.read_i16(s, Policy::Warn, "subdivType")?;
        out.levels = db.read_i16(s, Policy::Fail, "levels")?;
        out.render_levels = db.read_i16(s, Policy::Ignore, "renderLevels")?;
        db.read_i16(s, Policy::Ignore, "flags")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct MirrorModifierData {
    pub modifier: ModifierData,
    pub axis: i16,
    pub flag: i16,
    pub tolerance: f32,
    pub mirror_ob: Option<usize>,
}

impl Convert for MirrorModifierData {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.modifier = db.read_struct(s, Policy::Fail, "modifier")?;
        out.axis = db.read_i16(s, Policy::Ignore, "axis")?;
        out.flag = db.read_i16(s, Policy::Ignore, "flag")?;
        out.tolerance = db.read_f32(s, Policy::Ignore, "tolerance")?;
        db.read_pointer::<Object>(s, Policy::Ignore, "*mirror_ob", &mut out.mirror_ob)?;
        advance(db, s)
    }
}

/// `Object`: its type, world matrix, parent, data and modifiers.
#[derive(Clone, Default)]
pub(super) struct Object {
    pub id: Id,
    pub object_type: i32,
    pub obmat: [[f32; 4]; 4],
    pub parent: Option<usize>,
    pub data: Option<Element>,
    pub modifiers: ListBase,
}

impl Convert for Object {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.id = db.read_struct(s, Policy::Fail, "id")?;
        out.object_type = db.read_i32(s, Policy::Fail, "type")?;
        out.obmat = db.read_array2(s, Policy::Warn, "obmat", Database::convert_f32)?;
        db.read_array2::<4, 4, f32>(s, Policy::Warn, "parentinv", Database::convert_f32)?;
        db.read_array::<32, u8>(s, Policy::Warn, "parsubstr", Database::convert_u8)?;
        db.read_pointer::<Object>(s, Policy::Warn, "*parent", &mut out.parent)?;
        for name in ["*track", "*proxy", "*proxy_from", "*proxy_group"] {
            let mut other = None;
            db.read_pointer::<Object>(s, Policy::Warn, name, &mut other)?;
        }
        let mut dup_group = None;
        db.read_pointer::<Group>(s, Policy::Warn, "*dup_group", &mut dup_group)?;
        db.read_dynamic(s, Policy::Fail, "*data", &mut out.data)?;
        out.modifiers = db.read_struct(s, Policy::Ignore, "modifiers")?;
        advance(db, s)
    }
}

/// `Base`: the scene's object list, read iteratively along `next` (the
/// back links are left unresolved).
#[derive(Clone, Default)]
pub(super) struct Base {
    pub next: Option<usize>,
    pub object: Option<usize>,
}

impl Convert for Base {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        let initial = db.reader.current_pos();
        let mut current: Option<usize> = None;
        let mut position = initial as i64 as u64;
        loop {
            db.set_position(position)?;
            let mut object = None;
            let mut next = None;
            let object_result = db.read_pointer::<Object>(s, Policy::Warn, "*object", &mut object);
            store_base(db, out, current, |base| base.object = object);
            object_result?;
            let next_result = db.read_pointer_with::<Base>(s, Policy::Warn, "*next", &mut next, true);
            store_base(db, out, current, |base| base.next = next);
            let cached = next_result?;
            match next {
                Some(index) if !cached => {
                    current = Some(index);
                    position = db.position();
                }
                _ => break,
            }
        }
        let end = (initial as i64 as u64).wrapping_add(db.dna.structures[s].size);
        db.set_position(end)
    }
}

/// Writes a field of the Base being converted: the first is `out`, later
/// ones (along `next`) live in the arena.
fn store_base(db: &mut Database, out: &mut Base, current: Option<usize>, write: impl FnOnce(&mut Base)) {
    match current {
        None => write(out),
        Some(index) => write(&mut db.store.bases[index]),
    }
}

/// `Scene`: where the objects are found.
#[derive(Clone, Default)]
pub(super) struct Scene {
    pub basact: Option<usize>,
    pub master_collection: Option<usize>,
    pub base: ListBase,
}

impl Convert for Scene {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        let mut camera = None;
        db.read_pointer::<Object>(s, Policy::Warn, "*camera", &mut camera)?;
        let mut world = None;
        db.read_pointer::<World>(s, Policy::Warn, "*world", &mut world)?;
        db.read_pointer::<Base>(s, Policy::Warn, "*basact", &mut out.basact)?;
        db.read_pointer::<Collection>(s, Policy::Warn, "*master_collection", &mut out.master_collection)?;
        out.base = db.read_struct(s, Policy::Ignore, "base")?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct Image {
    pub name: Vec<u8>,
    pub packedfile: Option<usize>,
}

impl Convert for Image {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        db.read_struct::<Id>(s, Policy::Fail, "id")?;
        let name: [u8; 240] = db.read_array(s, Policy::Warn, "name", Database::convert_u8)?;
        out.name = name.to_vec();
        for name in ["ok", "flag", "source", "type", "pad", "pad1"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        db.read_i32(s, Policy::Ignore, "lastframe")?;
        for name in ["tpageflag", "totbind", "xrep", "yrep", "twsta", "twend"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        db.read_pointer::<PackedFile>(s, Policy::Ignore, "*packedfile", &mut out.packedfile)?;
        db.read_f32(s, Policy::Ignore, "lastupdate")?;
        db.read_i32(s, Policy::Ignore, "lastused")?;
        for name in ["animspeed", "gen_x", "gen_y", "gen_type"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        advance(db, s)
    }
}

/// `Tex` (its `id` is never read).
#[derive(Clone, Default)]
pub(super) struct Tex {
    pub imaflag: i16,
    pub tex_type: i32,
    pub ima: Option<usize>,
}

impl Convert for Tex {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.imaflag = db.read_i16(s, Policy::Ignore, "imaflag")?;
        out.tex_type = db.read_i32(s, Policy::Fail, "type")?;
        db.read_pointer::<Image>(s, Policy::Warn, "*ima", &mut out.ima)?;
        advance(db, s)
    }
}

#[derive(Clone, Default)]
pub(super) struct MTex {
    pub mapto: i32,
    pub tex: Option<usize>,
    pub uvname: Vec<u8>,
    pub projection: [i32; 3],
    pub mapping: i8,
}

impl MTex {
    /// `uvname` as a C string: a name that fills its 32 bytes runs on
    /// into `projx`, `projy`, `projz` and `mapping`, then the zeroed
    /// padding.
    pub fn uvname_c_str(&self) -> Vec<u8> {
        let mut following = Vec::new();
        for projection in self.projection {
            following.extend(projection.to_le_bytes());
        }
        following.push(self.mapping as u8);
        let bytes = self.uvname.iter().chain(following.iter());
        bytes.take_while(|&&byte| byte != 0).copied().collect()
    }
}

impl Convert for MTex {
    fn convert(db: &mut Database, s: usize, out: &mut Self) -> Outcome<()> {
        out.mapto = db.read_i32(s, Policy::Ignore, "mapto")?;
        db.read_i32(s, Policy::Ignore, "blendtype")?;
        let mut object = None;
        db.read_pointer::<Object>(s, Policy::Ignore, "*object", &mut object)?;
        db.read_pointer::<Tex>(s, Policy::Ignore, "*tex", &mut out.tex)?;
        let uvname: [u8; 32] = db.read_array(s, Policy::Ignore, "uvname", Database::convert_u8)?;
        out.uvname = uvname.to_vec();
        for (slot, name) in ["projx", "projy", "projz"].into_iter().enumerate() {
            out.projection[slot] = db.read_i32(s, Policy::Ignore, name)?;
        }
        out.mapping = db.read_i8(s, Policy::Ignore, "mapping")?;
        db.read_array::<3, f32>(s, Policy::Ignore, "ofs", Database::convert_f32)?;
        db.read_array::<3, f32>(s, Policy::Ignore, "size", Database::convert_f32)?;
        db.read_f32(s, Policy::Ignore, "rot")?;
        db.read_i32(s, Policy::Ignore, "texflag")?;
        for name in ["colormodel", "pmapto", "pmaptoneg"] {
            db.read_i16(s, Policy::Ignore, name)?;
        }
        for name in ["r", "g", "b", "k"] {
            db.read_f32(s, Policy::Warn, name)?;
        }
        for name in ["colspecfac", "mirrfac", "alphafac", "difffac", "specfac", "emitfac", "hardfac", "norfac"] {
            db.read_f32(s, Policy::Ignore, name)?;
        }
        advance(db, s)
    }
}
