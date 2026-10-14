//! `IFCGeometry`: representation items as meshes. Face sets become
//! polygons (inner bounds cut out of the largest one as openings),
//! extrusions are swept sides and caps (with the element's openings cut
//! into each), revolutions are rings of quads, swept disks are tubes
//! along a curve; booleans and openings have their own modules.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use super::curve::Curve;
use super::math::{Matrix3, Matrix4, Vector3, cross, dot};
use super::openings::generate_openings;
use super::profile::process_profile;
use super::schema::Class;
use super::schema::fields::{
    IfcConnectedFaceSet, IfcExtrudedAreaSolid, IfcFace, IfcFaceBasedSurfaceModel, IfcFaceBound, IfcManifoldSolidBrep,
    IfcPolyLoop, IfcProfileDef, IfcRevolvedAreaSolid, IfcShellBasedSurfaceModel, IfcSweptAreaSolid, IfcSweptDiskSolid,
};
use super::step::Object;
use super::util::{
    EPSILON, TempMesh, TempOpening, convert_axis_placement_1d, convert_axis_placement_3d, convert_cartesian_point, convert_direction,
};
use super::{Conversion, Failure, Outcome, boolean, material};
use crate::cast::to_unsigned;
use crate::scene::check_allocation;
use crate::zip::{IfcZip, ifc_member};

/// `sizeof(IfcVector3)`.
const VECTOR_SIZE: u64 = 24;

/// `AI_MATH_HALF_PI_F` and `AI_MATH_TWO_PI_F`: float constants
/// (`3.1415926538f` is the float `PI`).
pub(super) const HALF_PI_F: f32 = std::f32::consts::PI * 0.5;
pub(super) const TWO_PI_F: f32 = std::f32::consts::PI * 2.0;

/// The `.ifczip` member the importer reads instead of the file.
pub(super) fn read_ifczip(bytes: &[u8]) -> Outcome<Vec<u8>> {
    match ifc_member(bytes)? {
        IfcZip::Member(member) => Ok(member),
        IfcZip::Error(message) => Err(Failure::Std(format!("IFC: {message}"))),
        IfcZip::Endless => Err(Failure::endless("an ifczip whose first entry isn't .ifc, examined again and again")),
        IfcZip::Undefined(what) => Err(Failure::undefined(what)),
    }
}

/// `ProcessPolyloop`: the loop's points as a polygon; one of a single
/// point is dropped, an empty one kept.
fn process_polyloop(conversion: &Conversion, polyloop: &Object, mesh: &mut TempMesh) -> Outcome<bool> {
    let db = conversion.db;
    let mut count = 0u32;
    for point in polyloop.list(IfcPolyLoop::POLYGON) {
        let point = db.to(point.entity(), Class::IfcCartesianPoint)?;
        mesh.verts.push(convert_cartesian_point(&point));
        count += 1;
    }
    mesh.vertcnt.push(count);
    if count > 1 {
        return Ok(true);
    }
    if count == 1 {
        mesh.vertcnt.pop();
        mesh.verts.pop();
    }
    Ok(false)
}

/// `ProcessPolygonBoundaries`: the largest polygon (by Newell normal) is
/// the outer one; the others are cut out of it as openings.
fn process_polygon_boundaries(result: &mut TempMesh, input: &TempMesh) -> Outcome<()> {
    if input.vertcnt.is_empty() {
        return Ok(());
    }
    if input.vertcnt.len() == 1 {
        result.append(input);
        return Ok(());
    }
    Failure::assertion(!input.vertcnt.contains(&0), "std::count(inmesh.mVertcnt.begin(), inmesh.mVertcnt.end(), 0u) == 0")?;
    let normals = input.polygon_normals(false, 0)?;
    let mut outer_area = 1e-10f32 as f64;
    let mut outer = None;
    for (index, normal) in normals.iter().enumerate() {
        let area = normal.square_length();
        if area > outer_area {
            outer_area = area;
            outer = Some(index);
        }
    }
    let Some(outer) = outer else {
        return Ok(());
    };
    let outer_size = input.vertcnt[outer] as usize;
    let master_normal = normals[outer];
    let mut fake_openings = Vec::with_capacity(input.vertcnt.len() - 1);
    let mut start = 0usize;
    let mut outer_start = 0usize;
    for (index, &count) in input.vertcnt.iter().enumerate() {
        let count = count as usize;
        let polygon_start = start;
        start += count;
        if index == outer {
            outer_start = polygon_start;
            continue;
        }
        if normals[index].square_length() < 1e-5f32 as f64 {
            continue;
        }
        let profile = TempMesh { verts: input.verts[polygon_start..polygon_start + count].to_vec(), vertcnt: vec![count as u32] };
        let shared = Rc::new(RefCell::new(profile));
        fake_openings.push(TempOpening::new(master_normal, Some(shared), None));
    }
    let mut outer_mesh = TempMesh { verts: input.verts[outer_start..outer_start + outer_size].to_vec(), vertcnt: vec![outer_size as u32] };
    generate_openings(&mut fake_openings, &mut outer_mesh, false, false, Vector3::new(0.0, 1.0, 0.0))?;
    result.append(&outer_mesh);
    Ok(())
}

/// `ProcessConnectedFaceSet`: each face's poly loops, as polygons with
/// the inner ones cut out.
pub(super) fn process_connected_face_set(conversion: &Conversion, set: &Object, result: &mut TempMesh) -> Outcome<()> {
    let db = conversion.db;
    for face in set.list(IfcConnectedFaceSet::CFS_FACES).to_vec() {
        let face = db.to(face.entity(), Class::IfcFace)?;
        let mut mesh = TempMesh::default();
        for bound in face.list(IfcFace::BOUNDS).to_vec() {
            let bound = db.to(bound.entity(), Class::IfcFaceBound)?;
            let lop = db.to(bound.entity(IfcFaceBound::BOUND), Class::IfcLoop)?;
            if lop.is_a(Class::IfcPolyLoop) {
                process_polyloop(conversion, &lop, &mut mesh)?;
            }
        }
        process_polygon_boundaries(result, &mesh)?;
    }
    Ok(())
}

/// `ProcessRevolvedAreaSolid`: the profile turned about the axis in
/// segments (at least 2), each profile edge a quad per segment; a
/// profile with area that doesn't turn fully round gets both ends.
fn process_revolved_area_solid(conversion: &mut Conversion, solid: &Object, result: &mut TempMesh) -> Outcome<()> {
    let db = conversion.db;
    let mut profile_mesh = TempMesh::default();
    let profile = db.to(solid.entity(IfcSweptAreaSolid::SWEPT_AREA), Class::IfcProfileDef)?;
    if !process_profile(conversion, &profile, &mut profile_mesh)? || profile_mesh.verts.len() <= 1 {
        return Ok(());
    }
    let axis_placement = db.to(solid.entity(IfcRevolvedAreaSolid::AXIS), Class::IfcAxis1Placement)?;
    let (axis, position) = convert_axis_placement_1d(db, &axis_placement)?;
    let to_axis = Matrix4::translation(position);
    let from_axis = Matrix4::translation(-position);

    let input = &profile_mesh.verts;
    let size = input.len();
    let profile = db.to(solid.entity(IfcSweptAreaSolid::SWEPT_AREA), Class::IfcProfileDef)?;
    let mut has_area = profile.text(IfcProfileDef::PROFILE_TYPE) == b"AREA" && size > 2;
    let max_angle = solid.real(IfcRevolvedAreaSolid::ANGLE) * conversion.angle_scale;
    if max_angle.abs() < 1e-3 {
        if has_area {
            *result = profile_mesh.clone();
        }
        return Ok(());
    }
    let tessellation = conversion.settings.cylindrical_tessellation as f64;
    let segments = to_unsigned(tessellation * max_angle.abs() / HALF_PI_F as f64).max(2);
    let delta = max_angle / segments as f64;
    has_area = has_area && max_angle.abs() < TWO_PI_F as f64 * 0.99;

    let per_vertex = (segments as u64 + 1) * 4 + if has_area { 2 } else { 0 };
    check_allocation((size as u64).saturating_mul(per_vertex), VECTOR_SIZE)?;
    check_allocation((size as u64).saturating_mul(segments as u64).saturating_add(2), 4)?;
    let rotation = Matrix4::rotation(delta, axis);
    let rotation = to_axis.product(&rotation).product(&from_axis);

    let mut out: Vec<Vector3> = Vec::with_capacity(size * per_vertex as usize);
    for &vertex in input.iter() {
        out.extend([vertex; 4]);
    }
    let mut base = 0usize;
    for _ in 0..segments {
        for index in 0..size {
            let next = (index + 1) % size;
            result.vertcnt.push(4);
            let base_0 = out[base + index * 4 + 3];
            let base_1 = out[base + next * 4 + 3];
            out.push(base_0);
            out.push(base_1);
            out.push(rotation.transform(base_1));
            out.push(rotation.transform(base_0));
        }
        base += size * 4;
    }
    out.drain(..size * 4);
    if has_area {
        base -= size * 8;
        for index in (0..size).rev() {
            let vertex = out[base + index * 4 + 3];
            out.push(vertex);
        }
        for index in 0..size {
            let vertex = out[index * 4];
            out.push(vertex);
        }
        result.vertcnt.push(size as u32);
        result.vertcnt.push(size as u32);
    }
    result.verts.extend(out);
    let position = db.to(solid.entity(IfcSweptAreaSolid::POSITION), Class::IfcAxis2Placement3D)?;
    let trafo = convert_axis_placement_3d(db, &position)?;
    result.transform(&trafo);
    Ok(())
}

/// `ProcessSweptDiskSolid`: rings of the radius around the sampled
/// directrix, each joined to the next by quads from its nearest point.
fn process_swept_disk_solid(conversion: &mut Conversion, solid: &Object, result: &mut TempMesh) -> Outcome<()> {
    let db = conversion.db;
    let directrix = db.to(solid.entity(IfcSweptDiskSolid::DIRECTRIX), Class::IfcCurve)?;
    let Some(curve) = Curve::convert(conversion, &directrix)? else {
        return Ok(());
    };
    let segments = conversion.settings.cylindrical_tessellation;
    let delta_angle = std::f64::consts::PI * 2.0 / segments as f64;
    let mut sampled = TempMesh::default();
    let start = solid.real(IfcSweptDiskSolid::START_PARAM);
    let end = solid.real(IfcSweptDiskSolid::END_PARAM);
    curve.sample_discrete(conversion, &mut sampled, start, end)?;
    let curve_points = &sampled.verts;
    let samples = curve_points.len();
    check_allocation((segments as u64).saturating_mul(samples as u64).saturating_mul(4), VECTOR_SIZE)?;
    if curve_points.is_empty() {
        return Ok(());
    }
    let radius = solid.real(IfcSweptDiskSolid::RADIUS);

    let mut points: Vec<Vector3> = Vec::with_capacity(segments as usize * samples);
    let mut current = curve_points[0];
    let mut previous = current;
    let mut next = Vector3::default();
    let mut start_vector = Vector3::new(1.0, 1.0, 1.0);
    let mut last_dir = 0u32;
    for index in 0..samples {
        if index != samples - 1 {
            next = curve_points[index + 1];
        }
        let d = ((current - previous) + (next - previous)).normalize();
        let mut q = Vector3::default();
        let mut take_any = false;
        for _ in 0..2 {
            if (last_dir == 0 || take_any) && d.x.abs() > EPSILON {
                q.y = start_vector.y;
                q.z = start_vector.z;
                q.x = -(d.y * q.y + d.z * q.z) / d.x;
                last_dir = 0;
                break;
            } else if (last_dir == 1 || take_any) && d.y.abs() > EPSILON {
                q.x = start_vector.x;
                q.z = start_vector.z;
                q.y = -(d.x * q.x + d.z * q.z) / d.y;
                last_dir = 1;
                break;
            } else if (last_dir == 2 && d.z.abs() > EPSILON) || take_any {
                q.y = start_vector.y;
                q.x = start_vector.x;
                q.z = -(d.y * q.y + d.x * q.x) / d.z;
                last_dir = 2;
                break;
            }
            take_any = true;
        }
        let factor = radius / q.length();
        q = Vector3::new(q.x * factor, q.y * factor, q.z * factor);
        start_vector = q;
        let rotation = Matrix4::rotation(delta_angle, d);
        for _ in 0..segments {
            points.push(q + current);
            q = rotation.transform(q);
        }
        previous = current;
        current = next;
    }

    let segments = segments as usize;
    for index in 0..samples - 1 {
        let this_start = points[index * segments].to_f32();
        let mut best_pair_offset = 0usize;
        let mut best_distance_squared = 1e10f32;
        for segment in 0..segments {
            let point = points[(index + 1) * segments + segment].to_f32();
            let difference = [point[0] - this_start[0], point[1] - this_start[1], point[2] - this_start[2]];
            let length = difference[0] * difference[0] + difference[1] * difference[1] + difference[2] * difference[2];
            if length < best_distance_squared {
                best_pair_offset = segment;
                best_distance_squared = length;
            }
        }
        for segment in 0..segments {
            let mut v4 = points[index * segments + segment % segments];
            let mut v3 = points[index * segments + (segment + 1) % segments];
            let mut v2 = points[(index + 1) * segments + (segment + 1 + best_pair_offset) % segments];
            let mut v1 = points[(index + 1) * segments + (segment + best_pair_offset) % segments];
            if dot(cross(v4 - v3, v4 - v1), v4 - curve_points[index]) < 0.0 {
                std::mem::swap(&mut v4, &mut v1);
                std::mem::swap(&mut v3, &mut v2);
            }
            result.verts.extend([v4, v3, v2, v1]);
            result.vertcnt.push(4);
        }
    }
    Ok(())
}

/// `DerivePlaneCoordinateSpace`: a basis in the plane of a polygon (the
/// whole mesh read as one): the first vertex pair spanning a normal with
/// the last vertex gives the normal, the first of them the x axis. A
/// single vertex has the search run on forever; none reads before them.
pub(super) fn derive_plane_coordinate_space(mesh: &TempMesh) -> Outcome<(Matrix3, bool, Vector3)> {
    let out = &mesh.verts;
    let size = out.len();
    // The input must be a single polygon.
    Failure::assertion(mesh.vertcnt.len() == 1, "curmesh.mVertcnt.size() == 1")?;
    let last_count = mesh.vertcnt.last().map(|&count| count as usize);
    Failure::assertion(last_count == Some(size), "curmesh.mVertcnt.back() == s")?;
    if size == 0 {
        return Err(Failure::undefined("an IFC plane derived from a polygon of no vertices"));
    }
    if size == 1 {
        return Err(Failure::endless("an IFC plane derived from a polygon of one vertex"));
    }
    let any_point = out[size - 1];
    let mut normal = Vector3::default();
    let mut found = false;
    let mut first_index = 0usize;
    for i in 0..size - 2 {
        first_index = i;
        for j in i + 1..size - 1 {
            normal = -cross(out[i] - any_point, out[j] - any_point);
            if normal.length().abs() > 1e-8f32 as f64 {
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return Ok((Matrix3::default(), false, Vector3::default()));
    }
    let normal = normal.normalize();
    let r = (out[first_index] - any_point).normalize();
    let u = cross(r, normal).normalize();
    let matrix = Matrix3([r.x, r.y, r.z, u.x, u.y, u.z, -normal.x, -normal.y, -normal.z]);
    Ok((matrix, true, normal))
}

/// `areClose` of two vectors: every axis within `ai_epsilon`.
fn are_close(a: Vector3, b: Vector3) -> bool {
    (a.x - b.x).abs() < EPSILON && (a.y - b.y).abs() < EPSILON && (a.z - b.z).abs() < EPSILON
}

/// `ProcessExtrudedArea`: the profile placed and wound to face the
/// extrusion, a side quad per edge (each cut by the element's openings
/// when long enough), the caps for a profile with area; an opening
/// element's result is collected as an opening instead.
fn process_extruded_area(
    conversion: &mut Conversion,
    solid: &Object,
    curve: &TempMesh,
    extrusion: Vector3,
    result: &mut TempMesh,
    collect_openings: bool,
) -> Outcome<()> {
    let db = conversion.db;
    let profile = db.to(solid.entity(IfcSweptAreaSolid::SWEPT_AREA), Class::IfcProfileDef)?;
    let has_area = profile.text(IfcProfileDef::PROFILE_TYPE) == b"AREA" && curve.verts.len() > 2;
    if solid.real(IfcExtrudedAreaSolid::DEPTH) < EPSILON {
        if has_area {
            result.append(curve);
        }
        return Ok(());
    }
    let mut input = curve.verts.clone();
    let position = db.to(solid.entity(IfcSweptAreaSolid::POSITION), Class::IfcAxis2Placement3D)?;
    let trafo = convert_axis_placement_3d(db, &position)?;
    let (mut min, mut max) = Vector3::bounds_start();
    for vertex in &mut input {
        *vertex = trafo.transform(*vertex);
        min = min.min(*vertex);
        max = max.max(*vertex);
    }
    let diagonal = (max - min).length();
    let direction = trafo.matrix3().transform(extrusion);
    let profile_normal = TempMesh::polygon_normal(&input, true);
    if dot(profile_normal, direction) < 0.0 {
        input.reverse();
    }

    let apply = conversion.apply_openings;
    let openings = match apply {
        Some(reference) => !conversion.list(reference)?.is_empty(),
        None => false,
    };
    if openings && let Some(reference) = apply {
        for opening in conversion.list(reference)? {
            let Some(bounds) = &opening.profile_mesh else {
                return Err(Failure::undefined("an IFC opening without a profile mesh read"));
            };
            let bounds = bounds.borrow();
            if bounds.verts.len() <= 2 {
                continue;
            }
            let Some(&first_count) = bounds.vertcnt.first() else {
                return Err(Failure::undefined("an IFC opening's first polygon count read when it has none"));
            };
            let first = first_count as usize;
            let normal = cross(bounds.verts[2] - bounds.verts[0], bounds.verts[1] - bounds.verts[0]).normalize();
            for &count in &bounds.vertcnt {
                if count < 3 {
                    continue;
                }
                let Some(corners) = bounds.verts.get(first..first + 3) else {
                    return Err(Failure::undefined("an IFC opening's polygon read past its vertices"));
                };
                let other = cross(corners[2] - corners[0], corners[1] - corners[0]).normalize();
                let _parallel = are_close(normal, other);
            }
        }
    }

    let mut temp = TempMesh::default();
    let mut sides_with_openings = 0usize;
    for index in 0..input.len() {
        let next = (index + 1) % input.len();
        let target = if openings { &mut temp } else { &mut *result };
        target.vertcnt.push(4);
        target.verts.extend([input[index], input[next], input[next] + direction, input[index] + direction]);
        if openings {
            let long = (input[index] - input[next]).length() > diagonal * 0.1;
            if long && apply_openings(conversion, &mut temp, direction)? {
                sides_with_openings += 1;
            }
            result.append(&temp);
            temp.clear();
        }
    }
    if openings && let Some(reference) = apply {
        for opening in conversion.list_mut(reference)? {
            opening.wall_points.clear();
        }
    }
    if has_area {
        for n in 0..2 {
            let target = if openings { &mut temp } else { &mut *result };
            if n > 0 {
                for &vertex in &input {
                    target.verts.push(vertex + direction);
                }
            } else {
                for &vertex in input.iter().rev() {
                    target.verts.push(vertex);
                }
            }
            target.vertcnt.push(input.len() as u32);
            if openings && input.len() > 2 {
                apply_openings(conversion, &mut temp, direction)?;
                result.append(&temp);
                temp.clear();
            }
        }
    }
    let _ = sides_with_openings;

    if collect_openings && !result.is_empty() {
        let mut profile_mesh = TempMesh::default();
        std::mem::swap(&mut profile_mesh, result);
        let profile_2d = TempMesh { verts: input.clone(), vertcnt: vec![input.len() as u32] };
        let opening = TempOpening::new(direction, Some(Rc::new(RefCell::new(profile_mesh))), Some(Rc::new(RefCell::new(profile_2d))));
        Failure::assertion(conversion.collect_openings.is_some(), "conv.collect_openings")?;
        let Some(collect) = conversion.collect_openings else {
            return Err(Failure::undefined("an IFC opening collected into no list"));
        };
        conversion.list_mut(collect)?.push(opening);
    }
    Ok(())
}

/// `GenerateOpenings(*conv.apply_openings, temp, true, true, dir)`.
fn apply_openings(conversion: &mut Conversion, mesh: &mut TempMesh, direction: Vector3) -> Outcome<bool> {
    let Some(reference) = conversion.apply_openings else {
        return Err(Failure::undefined("IFC openings applied from no list"));
    };
    let (list, mut openings) = conversion.take_list(reference)?;
    let result = generate_openings(&mut openings, mesh, true, true, direction);
    conversion.put_list(list, openings);
    result
}

/// `ProcessExtrudedAreaSolid`: the profile extruded along the direction
/// times the depth. A profile's inner curves would be cut out as
/// openings first, but the schema reader never fills them in.
pub(super) fn process_extruded_area_solid(conversion: &mut Conversion, solid: &Object, result: &mut TempMesh, collect_openings: bool) -> Outcome<()> {
    let db = conversion.db;
    let mut profile_mesh = TempMesh::default();
    let profile = db.to(solid.entity(IfcSweptAreaSolid::SWEPT_AREA), Class::IfcProfileDef)?;
    if !process_profile(conversion, &profile, &mut profile_mesh)? || profile_mesh.verts.len() <= 1 {
        return Ok(());
    }
    let extruded = db.to(solid.entity(IfcExtrudedAreaSolid::EXTRUDED_DIRECTION), Class::IfcDirection)?;
    let depth = solid.real(IfcExtrudedAreaSolid::DEPTH);
    let direction = convert_direction(&extruded);
    let direction = Vector3::new(direction.x * depth, direction.y * depth, direction.z * depth);
    process_extruded_area(conversion, solid, &profile_mesh, direction, result, collect_openings)
}

/// `ProcessSweptAreaSolid`: an extrusion or a revolution.
pub(super) fn process_swept_area_solid(conversion: &mut Conversion, swept: &Object, result: &mut TempMesh) -> Outcome<()> {
    if swept.is_a(Class::IfcExtrudedAreaSolid) {
        let collect = conversion.collect_openings.is_some();
        return process_extruded_area_solid(conversion, swept, result, collect);
    }
    if swept.is_a(Class::IfcRevolvedAreaSolid) {
        return process_revolved_area_solid(conversion, swept, result);
    }
    Ok(())
}

/// `ProcessGeometricItem`: the item's mesh (duplicates and degenerates
/// removed) as a new output mesh, or the mesh as an opening when an
/// opening element is being read.
fn process_geometric_item(conversion: &mut Conversion, item: &Object, material: u32, mesh_indices: &mut BTreeSet<u32>) -> Outcome<bool> {
    let db = conversion.db;
    let mut mesh = TempMesh::default();
    if item.is_a(Class::IfcShellBasedSurfaceModel) {
        for shell in item.list(IfcShellBasedSurfaceModel::SBSM_BOUNDARY).to_vec() {
            let result = (|| {
                let Some(value) = shell.select() else {
                    return Err(Failure::undefined("an unset IFC shell read"));
                };
                let Some(id) = value.as_entity() else {
                    return Err(Failure::BadCast);
                };
                let index = db.must_get_object(id)?;
                let set = db.object(index)?;
                if !set.is_a(Class::IfcConnectedFaceSet) {
                    return Err(Failure::BadCast);
                }
                process_connected_face_set(conversion, &set, &mut mesh)
            })();
            match result {
                Ok(()) | Err(Failure::BadCast) => {}
                Err(failure) => return Err(failure),
            }
        }
    } else if item.is_a(Class::IfcConnectedFaceSet) {
        process_connected_face_set(conversion, item, &mut mesh)?;
    } else if item.is_a(Class::IfcSweptAreaSolid) {
        process_swept_area_solid(conversion, item, &mut mesh)?;
    } else if item.is_a(Class::IfcSweptDiskSolid) {
        process_swept_disk_solid(conversion, item, &mut mesh)?;
    } else if item.is_a(Class::IfcManifoldSolidBrep) {
        let outer = db.to(item.entity(IfcManifoldSolidBrep::OUTER), Class::IfcClosedShell)?;
        process_connected_face_set(conversion, &outer, &mut mesh)?;
    } else if item.is_a(Class::IfcFaceBasedSurfaceModel) {
        for set in item.list(IfcFaceBasedSurfaceModel::FBSM_FACES).to_vec() {
            let set = db.to(set.entity(), Class::IfcConnectedFaceSet)?;
            process_connected_face_set(conversion, &set, &mut mesh)?;
        }
    } else if item.is_a(Class::IfcBooleanResult) {
        boolean::process_boolean(conversion, item, &mut mesh)?;
    } else {
        return Ok(false);
    }

    if let Some(collect) = conversion.collect_openings {
        if !mesh.is_empty() {
            let opening = TempOpening::new(Vector3::default(), Some(Rc::new(RefCell::new(mesh))), None);
            conversion.list_mut(collect)?.push(opening);
        }
        return Ok(true);
    }
    if mesh.is_empty() {
        return Ok(false);
    }
    mesh.remove_adjacent_duplicates()?;
    mesh.remove_degenerates()?;
    let Some(mut output) = mesh.to_mesh()? else {
        return Ok(false);
    };
    output.material_index = material;
    mesh_indices.insert(conversion.meshes.len() as u32);
    conversion.meshes.push(output);
    Ok(true)
}

/// `ProcessRepresentationItem`: the item's material, then its meshes from
/// the cache (by item and material) or made now (cached with every mesh
/// gathered so far).
pub(super) fn process_representation_item(conversion: &mut Conversion, item: &Object, material: u32, mesh_indices: &mut BTreeSet<u32>) -> Outcome<bool> {
    let local_material = material::process_materials(conversion, item.id, material, true)?;
    let key = (item.index, local_material);
    if let Some(cached) = conversion.cached_meshes.get(&key) {
        mesh_indices.extend(cached.iter().copied());
        return Ok(true);
    }
    if !process_geometric_item(conversion, item, local_material, mesh_indices)? {
        return Ok(false);
    }
    if !mesh_indices.is_empty() {
        conversion.cached_meshes.insert(key, mesh_indices.clone());
    }
    Ok(true)
}
