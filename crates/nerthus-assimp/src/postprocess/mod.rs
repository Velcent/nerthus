//! The steps `Importer::ReadFile(path, 0x800a)` runs after the importer:
//! `ScenePreprocessor`, then (in registry order) Triangulate, SortByPType
//! and JoinIdenticalVertices.

pub(crate) mod join_vertices;
mod preprocess;
mod sort_by_primitive_type;
mod triangulate;

use crate::scene::{ImportError, Mesh, Scene};

/// `Exporter::Export`'s steps for an exporter that enforces
/// JoinIdenticalVertices, Triangulate and SortByPType (glTF 2): the same
/// steps in registry order, without the importer's `ScenePreprocessor`.
pub fn run_export(scene: &mut Scene) -> Result<(), ImportError> {
    for mesh in &mut scene.meshes {
        triangulate::triangulate_mesh(mesh)?;
    }
    sort_by_primitive_type::execute(scene)?;
    for mesh in &mut scene.meshes {
        join_vertices::process_mesh(mesh)?;
    }
    Ok(())
}

/// Assimp's steps index the vertex streams with the face indices without
/// checking them (JoinIdenticalVertices reads every face of every mesh
/// with positions), so an index past the vertices reads past the arrays:
/// undefined behaviour, an import failure here.
fn check_face_indices(scene: &Scene) -> Result<(), ImportError> {
    for mesh in &scene.meshes {
        if !mesh.has_positions() || !mesh.has_faces() {
            continue;
        }
        let in_range = mesh.faces.iter().flatten().flatten().all(|&index| (index as usize) < mesh.vertex_count);
        if !in_range {
            return Err(ImportError("undefined behaviour: a face index past the vertices".to_string()));
        }
    }
    Ok(())
}

/// `ScenePreprocessor` alone: what `ReadFile` runs for an import that
/// asks for no steps (a file a scene loads as one of its objects).
pub(crate) fn preprocess_meshes(meshes: &mut [Mesh]) {
    for mesh in meshes {
        preprocess::process_mesh(mesh);
    }
}

/// Whether SortByPType would split any of these meshes (several
/// primitive types after the preprocessing and triangulation before it),
/// which makes it walk the node graph.
pub(crate) fn sort_would_split(meshes: &[Mesh]) -> Result<bool, ImportError> {
    let mut scene = Scene { meshes: meshes.to_vec() };
    check_face_indices(&scene)?;
    preprocess::process_scene(&mut scene);
    for mesh in &mut scene.meshes {
        triangulate::triangulate_mesh(mesh)?;
    }
    let splits = scene.meshes.iter().any(|mesh| (mesh.primitive_types & 0xF).count_ones() > 1);
    Ok(splits)
}

pub fn run(scene: &mut Scene) -> Result<(), ImportError> {
    check_face_indices(scene)?;
    preprocess::process_scene(scene);
    for mesh in &mut scene.meshes {
        triangulate::triangulate_mesh(mesh)?;
    }
    sort_by_primitive_type::execute(scene)?;
    for mesh in &mut scene.meshes {
        join_vertices::process_mesh(mesh)?;
    }
    Ok(())
}
