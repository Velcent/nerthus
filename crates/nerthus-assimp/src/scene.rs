//! `aiScene` and `aiMesh`, as far as importing meshes needs them.

pub const MAX_COLOR_SETS: usize = 8;
pub const MAX_TEXTURE_COORDS: usize = 8;

/// `aiPrimitiveType` bits.
pub const PRIMITIVE_POINT: u32 = 0x1;
pub const PRIMITIVE_LINE: u32 = 0x2;
pub const PRIMITIVE_TRIANGLE: u32 = 0x4;
pub const PRIMITIVE_POLYGON: u32 = 0x8;
pub const PRIMITIVE_NGON_ENCODING: u32 = 0x10;

/// An `aiMesh`: per-vertex streams (an empty stream stands for a null
/// pointer) and faces as index lists.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub name: String,
    pub primitive_types: u32,
    pub vertex_count: usize,
    pub vertices: Option<Vec<[f32; 3]>>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 3]>>,
    pub bitangents: Option<Vec<[f32; 3]>>,
    pub colors: [Option<Vec<[f32; 4]>>; MAX_COLOR_SETS],
    pub texture_coords: [Option<Vec<[f32; 3]>>; MAX_TEXTURE_COORDS],
    pub uv_components: [u32; MAX_TEXTURE_COORDS],
    pub faces: Option<Vec<Vec<u32>>>,
    pub material_index: u32,
    /// Each bone's weights, as the vertices they weigh (their values make
    /// no mesh here, but the steps index vertices with them).
    pub bones: Vec<Vec<u32>>,
}

impl Mesh {
    pub fn face_count(&self) -> usize {
        self.faces.as_ref().map_or(0, Vec::len)
    }

    /// `aiMesh::HasPositions`.
    pub fn has_positions(&self) -> bool {
        self.vertices.is_some() && self.vertex_count > 0
    }

    /// `aiMesh::HasFaces`.
    pub fn has_faces(&self) -> bool {
        self.faces.is_some() && self.face_count() > 0
    }

    /// `aiMesh::HasNormals`.
    pub fn has_normals(&self) -> bool {
        self.normals.is_some() && self.vertex_count > 0
    }

    /// `aiMesh::HasTangentsAndBitangents`.
    pub fn has_tangents_and_bitangents(&self) -> bool {
        self.tangents.is_some() && self.bitangents.is_some() && self.vertex_count > 0
    }

    /// `aiMesh::HasTextureCoords`.
    pub fn has_texture_coords(&self, index: usize) -> bool {
        index < MAX_TEXTURE_COORDS && self.texture_coords[index].is_some() && self.vertex_count > 0
    }

    /// `aiMesh::HasVertexColors`.
    pub fn has_vertex_colors(&self, index: usize) -> bool {
        index < MAX_COLOR_SETS && self.colors[index].is_some() && self.vertex_count > 0
    }

    /// `aiMesh::GetNumUVChannels`: every present set, gaps or not.
    pub fn uv_channel_count(&self) -> usize {
        self.texture_coords.iter().filter(|set| set.is_some()).count()
    }

    /// `aiMesh::GetNumColorChannels`: the sets present from 0 on.
    pub fn color_channel_count(&self) -> usize {
        self.colors.iter().take_while(|set| set.is_some()).count()
    }
}

/// An `aiScene`: only its meshes matter here.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
}

/// A failed import (`DeadlyImportError`, or a standard exception Assimp's
/// `ReadFile` catches).
#[derive(Clone, Debug)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

impl ImportError {
    /// Whether the native code would have crashed, hung or exited rather
    /// than thrown: undefined behaviour, endless loops and `exit` end the
    /// whole process, so such a failure in a file a scene loads as one of
    /// its objects fails the scene too.
    pub fn ends_process(&self) -> bool {
        let message = &self.0;
        message.starts_with("undefined behaviour") || message.starts_with("endless loop") || message.starts_with("process exit")
    }
}

/// Shorthand for an import failure.
pub fn fail<T>(message: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(message.into()))
}

/// `ai_assert` (and the CRT's `assert`): Gaea's Assimp is built without
/// `NDEBUG`, so both are live, and a failure ends the process (an abort,
/// or the CRT's assertion dialog).
pub(crate) fn ai_assert(condition: bool, expression: &str) -> Result<(), ImportError> {
    if condition {
        return Ok(());
    }
    Err(assertion_failed(expression))
}

/// The failure of an `ai_assert` or `assert` that failed.
pub(crate) fn assertion_failed(expression: &str) -> ImportError {
    ImportError(format!("process exit: assertion failed: {expression}"))
}

/// The stack an importer's recursion has: a 1.5 MiB thread stack (what a
/// .NET program's threads get on Windows) less 64 KiB for the frames
/// above the importer's (some 6 KiB of native ones, from the loader to the
/// scene's converter) and what the runtime keeps back for overflows.
pub(crate) const STACK_BUDGET: u64 = 1_572_864 - 65_536;

/// Allocations above this fail as `std::bad_alloc` does on Windows (an
/// internal error once `ReadFile` catches it).
pub(crate) const ALLOCATION_LIMIT: u64 = 1 << 31;

/// `std::bad_alloc`, caught as an internal error.
pub(crate) fn bad_alloc<T>() -> Result<T, ImportError> {
    fail("Internal error")
}

/// `new T[count]` for a `T` of `element_size` bytes: `bad_alloc` past the
/// allocation limit.
pub(crate) fn check_allocation(count: u64, element_size: u64) -> Result<(), ImportError> {
    let size = count.saturating_mul(element_size);
    if size > ALLOCATION_LIMIT {
        return bad_alloc();
    }
    Ok(())
}
