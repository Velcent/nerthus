//! `BlenderModifierShowcase`: an object's modifiers applied to its node
//! once its children are converted. Only Mirror and Subdivision have
//! handlers; Mirror copies the node's meshes (numbering the copies as if
//! the node's meshes were the last ones made) and Subdivision refines the
//! last meshes made, as many as the node has.

use std::collections::HashSet;

use super::Conversion;
use super::dna::{Database, Outcome, import_error, undefined};
use super::structures::{Element, Kind, MirrorModifierData, ModifierData, Object, SubsurfModifierData};
use crate::subdivision::catmull_clark;

/// `eModifierType_Subsurf` and `eModifierType_Mirror`.
const SUBSURF: i32 = 1;
const MIRROR: i32 = 5;

/// `MirrorModifierData::Flags`.
const MIRROR_U: i16 = 1 << 1;
const MIRROR_V: i16 = 1 << 2;
const AXIS_X: i16 = 1 << 3;
const AXIS_Y: i16 = 1 << 4;
const AXIS_Z: i16 = 1 << 5;

/// The common part of a list element read through the
/// `SharedModifierData` layout, which only the two modifier types have.
fn common(db: &Database, element: Element) -> Outcome<ModifierData> {
    match element.kind {
        Kind::SubsurfModifierData => Ok(db.store.subsurf_modifiers[element.index].modifier.clone()),
        Kind::MirrorModifierData => Ok(db.store.mirror_modifiers[element.index].modifier.clone()),
        _ => undefined("a Blender modifier list element read as a modifier it is not"),
    }
}

/// `cur->modifier.next` of an element the walk skips: what lies there in
/// the element's own type (a plain `ModifierData`'s `prev`).
fn next_past(db: &Database, element: Element) -> Outcome<Option<Element>> {
    match element.kind {
        Kind::SubsurfModifierData => Ok(db.store.subsurf_modifiers[element.index].modifier.next),
        Kind::MirrorModifierData => Ok(db.store.mirror_modifiers[element.index].modifier.next),
        Kind::ModifierData => Ok(db.store.modifiers[element.index].prev),
        _ => undefined("a Blender modifier list through an element that is no modifier"),
    }
}

/// A list element as the Subdivision handler reads it: a mirror's
/// `axis`, `flag` and `tolerance` are a subdivision's `subdivType`,
/// `levels`, `renderLevels` and `flags`.
fn as_subsurf(db: &Database, element: Element) -> Outcome<SubsurfModifierData> {
    match element.kind {
        Kind::SubsurfModifierData => Ok(db.store.subsurf_modifiers[element.index].clone()),
        Kind::MirrorModifierData => {
            let mirror = &db.store.mirror_modifiers[element.index];
            let tolerance = mirror.tolerance.to_bits();
            let subsurf = SubsurfModifierData {
                modifier: mirror.modifier.clone(),
                subdiv_type: mirror.axis,
                levels: mirror.flag,
                render_levels: tolerance as u16 as i16,
            };
            Ok(subsurf)
        }
        _ => undefined("a Blender modifier read as a subdivision it is not"),
    }
}

/// A list element as the Mirror handler reads it (a subdivision is too
/// small to hold the `mirror_ob` the handler locks first).
fn as_mirror(db: &Database, element: Element) -> Outcome<MirrorModifierData> {
    match element.kind {
        Kind::MirrorModifierData => Ok(db.store.mirror_modifiers[element.index].clone()),
        _ => undefined("a Blender modifier read as a mirror it is not"),
    }
}

/// Whether the structure an element was read as has a `ModifierData`
/// called `modifier` at offset 0 (else the element is skipped).
fn has_modifier_member(db: &Database, element: Element) -> Outcome<bool> {
    let Some(&structure) = db.dna_types.get(&element) else {
        return undefined("a Blender modifier without its type name");
    };
    let name = &db.dna.structures[structure].name;
    let Some(&structure) = db.dna.indices.get(name) else {
        return Ok(false);
    };
    let structure = &db.dna.structures[structure];
    let Some(&field) = structure.indices.get(b"modifier".as_slice()) else {
        return Ok(false);
    };
    let field = &structure.fields[field];
    if field.offset != 0 {
        return Ok(false);
    }
    let Some(&member) = db.dna.indices.get(&field.type_name) else {
        return Ok(false);
    };
    Ok(db.dna.structures[member].name == b"ModifierData")
}

/// `ApplyModifiers`: every modifier in the list, each by the handler that
/// takes its type. A list that comes round again never ends (natively
/// it runs until memory does when a modifier keeps adding).
pub(super) fn apply_modifiers(db: &mut Database, conversion: &mut Conversion, node: usize, object: &Object) -> Outcome<()> {
    let mut current = object.modifiers.first;
    let mut seen = HashSet::new();
    let mut grows = false;
    while let Some(element) = current {
        if !seen.insert(element) {
            if grows {
                return import_error("bad allocation");
            }
            return import_error("endless loop: a Blender modifier list that comes round again");
        }
        if !has_modifier_member(db, element)? {
            current = next_past(db, element)?;
            continue;
        }
        let modifier = common(db, element)?;
        if modifier.modifier_type == MIRROR {
            let mirror = as_mirror(db, element)?;
            grows |= !conversion.nodes[node].is_empty();
            apply_mirror(db, conversion, node, &mirror)?;
        } else if modifier.modifier_type == SUBSURF {
            let subsurf = as_subsurf(db, element)?;
            grows |= apply_subdivision(conversion, node, &subsurf)?;
        }
        current = modifier.next;
    }
    Ok(())
}

/// `BlenderModifier_Mirror::DoIt`: copies of the node's meshes mirrored on
/// the flagged axes (about the mirror object's position, if any), wound
/// the other way after an odd number of flips; the copies are numbered
/// the node's count past each original.
fn apply_mirror(db: &Database, conversion: &mut Conversion, node: usize, mirror: &MirrorModifierData) -> Outcome<()> {
    let center = mirror.mirror_ob.map(|object| {
        let matrix = db.store.objects[object].obmat;
        [matrix[3][0], matrix[3][1], matrix[3][2]]
    });
    let sign = |flag: i16| if mirror.flag & flag != 0 { -1.0f32 } else { 1.0 };
    let (xs, ys, zs) = (sign(AXIS_X), sign(AXIS_Y), sign(AXIS_Z));
    let (us, vs) = (sign(MIRROR_U), sign(MIRROR_V));
    let node_meshes = conversion.nodes[node].clone();
    for &index in &node_meshes {
        let Some(source) = conversion.meshes.get(index as usize) else {
            return undefined("a Blender node's mesh index past the meshes");
        };
        // `SceneCombiner::Copy` copies the colour and coordinate sets only
        // while `HasVertexColors`/`HasTextureCoords`, which want vertices:
        // without any, the copy keeps the original's arrays and both
        // meshes free them.
        let has_colors = source.colors.iter().any(Option::is_some);
        let has_coordinates = source.texture_coords.iter().any(Option::is_some);
        if source.vertex_count == 0 && (has_colors || has_coordinates) {
            return undefined("a Blender mesh without vertices mirrored with arrays its copy frees again");
        }
        let mut mesh = source.clone();
        let count = mesh.vertex_count;
        if let Some(vertices) = mesh.vertices.as_mut() {
            for vertex in vertices.iter_mut().take(count) {
                match center {
                    Some(center) => {
                        vertex[0] = center[0] + xs * (center[0] - vertex[0]);
                        vertex[1] = center[1] + ys * (center[1] - vertex[1]);
                        vertex[2] = center[2] + zs * (center[2] - vertex[2]);
                    }
                    None => {
                        vertex[0] *= xs;
                        vertex[1] *= ys;
                        vertex[2] *= zs;
                    }
                }
            }
        }
        for stream in [mesh.normals.as_mut(), mesh.tangents.as_mut(), mesh.bitangents.as_mut()].into_iter().flatten() {
            for vector in stream.iter_mut().take(count) {
                vector[0] *= xs;
                vector[1] *= ys;
                vector[2] *= zs;
            }
        }
        if count > 0 {
            for set in mesh.texture_coords.iter_mut() {
                let Some(coordinates) = set.as_mut() else {
                    break;
                };
                for coordinate in coordinates.iter_mut().take(count) {
                    coordinate[0] *= us;
                    coordinate[1] *= vs;
                }
            }
        }
        if xs * ys * zs < 0.0
            && let Some(faces) = mesh.faces.as_mut()
        {
            for face in faces {
                let length = face.len();
                for index in 0..length / 2 {
                    face.swap(index, length - 1 - index);
                }
            }
        }
        conversion.meshes.push(mesh);
    }
    let count = node_meshes.len() as u32;
    let mut indices = node_meshes.clone();
    indices.extend(node_meshes.iter().map(|&index| count.wrapping_add(index)));
    conversion.nodes[node] = indices;
    Ok(())
}

/// `BlenderModifier_Subdivision::DoIt`: Catmull-Clark (also for the
/// simple type, which is unimplemented) over the last meshes made, as
/// many as the node has, `max(renderLevels, levels)` times. Whether
/// anything grew.
fn apply_subdivision(conversion: &mut Conversion, node: usize, subsurf: &SubsurfModifierData) -> Outcome<bool> {
    if !(0..=1).contains(&subsurf.subdiv_type) {
        return Ok(false);
    }
    if conversion.meshes.is_empty() {
        return Ok(false);
    }
    let total = conversion.meshes.len();
    let count = conversion.nodes[node].len();
    let first = total.wrapping_sub(count);
    if first >= total {
        return Ok(false);
    }
    let levels = subsurf.render_levels.max(subsurf.levels) as i32 as u32;
    let meshes = conversion.meshes.split_off(first);
    let subdivided = catmull_clark(meshes, levels)?;
    conversion.meshes.extend(subdivided);
    Ok(levels > 0)
}
