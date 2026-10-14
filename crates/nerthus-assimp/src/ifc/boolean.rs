//! `IFCBoolean`: DIFFERENCE results. The first operand (a swept solid or
//! another boolean) is clipped by a half space (its plane, or its plane
//! within a polygonal boundary) or has an extruded solid cut out of it
//! as an opening.

use std::cell::RefCell;
use std::rc::Rc;

use super::geometry::{process_extruded_area_solid, process_swept_area_solid};
use super::math::{Vector2, Vector3, array_bounds, cross, dot};
use super::openings::generate_openings;
use super::profile::process_curve;
use super::schema::Class;
use super::schema::fields::{IfcBooleanResult, IfcElementarySurface, IfcHalfSpaceSolid, IfcPlacement, IfcPolygonalBoundedHalfSpace, IfcAxis2Placement3D};
use super::step::Object;
use super::util::{EPSILON, TempMesh, TempOpening, convert_axis_placement_3d, convert_cartesian_point, convert_direction, is_true, unique};
use super::{BOOLEAN_FRAME, Conversion, Failure, Outcome};

/// `IntersectSegmentPlane`: where e0–e1 crosses the plane, with the side
/// the start is taken to be on deciding hits that start on it.
fn intersect_segment_plane(p: Vector3, n: Vector3, e0: Vector3, e1: Vector3, start_on_white_side: bool) -> Option<Vector3> {
    let pdelta = e0 - p;
    let segment = e1 - e0;
    let dot_one = dot(n, segment);
    let dot_two = -dot(n, pdelta);
    if (dot_one + dot_two).abs() < EPSILON {
        return None;
    }
    if dot_two.abs() < EPSILON {
        if (start_on_white_side && dot_one + dot_two < EPSILON) || (!start_on_white_side && dot_one + dot_two > -EPSILON) {
            return Some(e0);
        }
        return None;
    }
    if dot_one.abs() < EPSILON {
        return None;
    }
    let t = dot_two / dot_one;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(e0 + t * segment)
}

/// `FilterPolygon`: adjacent near-duplicates (bounding box diagonal
/// squared / 1e6) and a last point equal to the first go; fewer than 3
/// points empty it.
fn filter_polygon(polygon: &mut Vec<Vector3>) {
    if polygon.len() < 3 {
        polygon.clear();
        return;
    }
    let (min, max) = array_bounds(polygon);
    let epsilon = (max - min).square_length() / 1e6f32 as f64;
    let close = |a: Vector3, b: Vector3| (a - b).square_length().abs() < epsilon;
    *polygon = unique(polygon, close);
    if let (Some(&first), Some(&last)) = (polygon.first(), polygon.last())
        && close(first, last)
    {
        polygon.pop();
    }
}

/// `WritePolygon`.
fn write_polygon(polygon: &mut Vec<Vector3>, result: &mut TempMesh) {
    filter_polygon(polygon);
    if polygon.len() > 2 {
        result.verts.extend_from_slice(polygon);
        result.vertcnt.push(polygon.len() as u32);
    }
}

/// The plane of a half space's base surface (an `IfcPlane`), its normal
/// flipped unless the agreement flag is set; none for another surface.
fn half_space_plane(conversion: &Conversion, half_space: &Object) -> Outcome<Option<(Vector3, Vector3)>> {
    let db = conversion.db;
    let surface = db.to(half_space.entity(IfcHalfSpaceSolid::BASE_SURFACE), Class::IfcSurface)?;
    if !surface.is_a(Class::IfcPlane) {
        return Ok(None);
    }
    let position = db.to(surface.entity(IfcElementarySurface::POSITION), Class::IfcAxis2Placement3D)?;
    let mut normal = Vector3::new(0.0, 0.0, 1.0);
    if position.has(IfcAxis2Placement3D::AXIS) {
        let axis = db.to(position.entity(IfcAxis2Placement3D::AXIS), Class::IfcDirection)?;
        normal = convert_direction(&axis);
    }
    let location = db.to(position.entity(IfcPlacement::LOCATION), Class::IfcCartesianPoint)?;
    let point = convert_cartesian_point(&location);
    if !is_true(half_space.text(IfcHalfSpaceSolid::AGREEMENT_FLAG)) {
        normal = -normal;
    }
    Ok(Some((point, normal)))
}

/// `ProcessBooleanHalfSpaceDifference`: each polygon clipped to the white
/// side of the plane.
fn process_half_space_difference(conversion: &Conversion, half_space: &Object, result: &mut TempMesh, first: &TempMesh) -> Outcome<()> {
    let Some((p, n)) = half_space_plane(conversion, half_space)? else {
        return Ok(());
    };
    let input = &first.verts;
    let mut start = 0usize;
    for &count in &first.vertcnt {
        let count_usize = count as usize;
        let polygon_start = start;
        start += count_usize;
        // The first vertex is read even for an empty polygon.
        let Some(&first_vertex) = input.get(polygon_start) else {
            return Err(Failure::undefined("an IFC boolean operand's vertex read past its vertices"));
        };
        let Some(polygon) = input.get(polygon_start..polygon_start + count_usize) else {
            return Err(Failure::undefined("an IFC boolean operand's polygon read past its vertices"));
        };
        let mut new_count = 0usize;
        let mut white = dot(first_vertex - p, n) > -EPSILON;
        for index in 0..count_usize {
            let e0 = polygon[index];
            let e1 = polygon[(index + 1) % count_usize];
            if let Some(hit) = intersect_segment_plane(p, n, e0, e1, white) {
                if white {
                    result.verts.push(e0);
                    result.verts.push(hit);
                    new_count += 2;
                } else {
                    result.verts.push(hit);
                    new_count += 1;
                }
                white = !white;
            } else if white {
                result.verts.push(e0);
                new_count += 1;
            }
        }
        if new_count == 0 {
            continue;
        }
        let tail_start = result.verts.len() - new_count;
        let (min, max) = array_bounds(&result.verts[tail_start..]);
        let epsilon = (max - min).square_length() / 1e6f32 as f64;
        let close = |a: Vector3, b: Vector3| (a - b).square_length().abs() < epsilon;
        let kept = unique(&result.verts[tail_start..], close);
        new_count = kept.len();
        result.verts.truncate(tail_start);
        result.verts.extend(kept);
        let tail_start = result.verts.len() - new_count;
        if close(result.verts[tail_start], *result.verts.last().expect("a kept vertex")) {
            result.verts.pop();
            new_count -= 1;
        }
        if new_count > 2 {
            result.vertcnt.push(new_count as u32);
        } else {
            for _ in 0..new_count {
                result.verts.pop();
            }
        }
    }
    Ok(())
}

/// `IntersectsBoundaryProfile`: where e0–e1 crosses the boundary's edges
/// (in xy), each with its edge, given the side the start is taken to be
/// on; a ray when half open.
fn intersects_boundary_profile(e0: Vector3, e1: Vector3, boundary: &[Vector3], start_inside: bool, half_open: bool) -> Outcome<Vec<(usize, Vector3)>> {
    let count = boundary.len();
    let mut results: Vec<(usize, Vector3)> = Vec::new();
    let mut winding = 0.0f64;
    for i in 0..count {
        let b01 = boundary[(i + 1) % count] - boundary[i];
        let b12 = boundary[(i + 2) % count] - boundary[(i + 1) % count];
        let side = Vector3::new(b01.y, -b01.x, 0.0);
        winding += side.x * b12.x + side.y * b12.y;
    }
    let winding = if winding > 0.0 { 1.0 } else { -1.0 };
    let e = e1 - e0;
    for i in 0..count {
        let b0 = boundary[i];
        let b1 = boundary[(i + 1) % count];
        let b = b1 - b0;
        let det = -b.x * e.y + e.x * b.y;
        if det.abs() < EPSILON {
            continue;
        }
        let b_inverse_square = 1.0 / b.square_length();
        let x = b0.x - e0.x;
        let y = b0.y - e0.y;
        let s = (x * e.y - e.x * y) / det;
        let t = (x * b.y - b.x * y) / det;
        let p = e0 + e * t;
        // The debug build's check, which Gaea's has.
        let check = b0 + b * s - p;
        Failure::assertion(Vector2::new(check.x, check.y).square_length() < 1e-5, "(IfcVector2(check.x, check.y)).SquareLength() < 1e-5")?;

        let along0 = (b.x * (e0.x - b0.x) + b.y * (e0.y - b0.y)) * b_inverse_square;
        let closest0 = b0 + along0.clamp_like_std() * b;
        let starts_at_segment = (closest0 - Vector3::new(e0.x, e0.y, 0.0)).square_length() < 1e-12;
        let along1 = (b.x * (e1.x - b0.x) + b.y * (e1.y - b0.y)) * b_inverse_square;
        let closest1 = b0 + along1.clamp_like_std() * b;
        let ends_at_segment = (closest1 - Vector3::new(e1.x, e1.y, 0.0)).square_length() < 1e-12;

        if ends_at_segment && !half_open {
            continue;
        }
        let previous_close = |results: &Vec<(usize, Vector3)>, point: Vector3| {
            if let Some(&(edge, last)) = results.last()
                && edge == i.wrapping_sub(1)
            {
                let diff = last - point;
                return Vector2::new(diff.x, diff.y).square_length() < 1e-10;
            }
            false
        };
        if starts_at_segment {
            let inside_direction = Vector3::new(b.y, -b.x, 0.0) * winding;
            let going_inside = dot(inside_direction, e) > 0.0;
            if going_inside == start_inside {
                continue;
            }
            if previous_close(&results, e0) {
                continue;
            }
            results.push((i, e0));
            continue;
        }
        let slack = EPSILON * b_inverse_square;
        if s >= -slack && s <= 1.0 + slack && t >= 0.0 && (t <= 1.0 || half_open) {
            if previous_close(&results, p) {
                continue;
            }
            results.push((i, p));
        }
    }
    Ok(results)
}

/// `std::max(0, std::min(1, x))` as the C++ writes it.
trait ClampLikeStd {
    fn clamp_like_std(self) -> f64;
}

impl ClampLikeStd for f64 {
    fn clamp_like_std(self) -> f64 {
        let upper = if self < 1.0 { self } else { 1.0 };
        if 0.0 < upper { upper } else { 0.0 }
    }
}

/// `PointInPoly`: inside when at least two of three rays cross the
/// boundary an odd number of times.
fn point_in_poly(p: Vector3, boundary: &[Vector3]) -> Outcome<bool> {
    let rays = [Vector3::new(1.0, 0.0, 0.0), Vector3::new(0.0, 1.0, 0.0), Vector3::new(0.6, -0.6, 0.0)];
    let mut votes = 0usize;
    for ray in rays {
        let hits = intersects_boundary_profile(p, p + ray, boundary, true, true)?;
        votes += hits.len() % 2;
    }
    Ok(votes > 1)
}

/// `ProcessPolygonalBoundedBooleanHalfSpaceDifference`: each polygon cut
/// by the plane; the white part kept, the black part kept only outside
/// the boundary (walked along the boundary between the crossings).
fn process_bounded_half_space_difference(conversion: &Conversion, half_space: &Object, result: &mut TempMesh, first: &TempMesh) -> Outcome<()> {
    let db = conversion.db;
    let Some((p, n)) = half_space_plane(conversion, half_space)? else {
        return Ok(());
    };
    let n = n.normalize();
    let boundary_curve = db.to(half_space.entity(IfcPolygonalBoundedHalfSpace::POLYGONAL_BOUNDARY), Class::IfcBoundedCurve)?;
    let mut profile = TempMesh::default();
    if !process_curve(conversion, &boundary_curve, &mut profile)? {
        return Ok(());
    }
    let profile_normal = TempMesh::polygon_normal(&profile.verts, true);
    let position = db.to(half_space.entity(IfcPolygonalBoundedHalfSpace::POSITION), Class::IfcAxis2Placement3D)?;
    let projection_inverse = convert_axis_placement_3d(db, &position)?;
    let projection = projection_inverse.inverse();
    let boundary = &profile.verts;

    let input = &first.verts;
    let mut start = 0usize;
    for &count in &first.vertcnt {
        let count = count as usize;
        let polygon_start = start;
        start += count;
        if count == 0 {
            continue;
        }
        let Some(source) = input.get(polygon_start..polygon_start + count) else {
            return Err(Failure::undefined("an IFC boolean operand's polygon read past its vertices"));
        };
        let mut whiteside = Vec::new();
        let mut blackside = Vec::new();
        let polygon_normal = TempMesh::polygon_normal(source, true);
        if dot(polygon_normal, n).abs() > 0.9999 {
            let white = dot(source[0] - p, n) > -EPSILON;
            let target = if white { &mut whiteside } else { &mut blackside };
            target.extend_from_slice(source);
        } else {
            let mut white = dot(source[0] - p, n) > -EPSILON;
            for a in 0..count {
                let e0 = source[a];
                let e1 = source[(a + 1) % count];
                let target = if white { &mut whiteside } else { &mut blackside };
                target.push(source[a]);
                if let Some(hit) = intersect_segment_plane(p, n, e0, e1, white) {
                    if target.last().is_none_or(|&last| (last - hit).square_length() > 1e-12) {
                        target.push(hit);
                    }
                    white = !white;
                    let other = if white { &mut whiteside } else { &mut blackside };
                    other.push(hit);
                }
            }
        }
        write_polygon(&mut whiteside, result);
        filter_polygon(&mut blackside);
        if blackside.is_empty() {
            continue;
        }

        let started_inside = point_in_poly(projection.transform(blackside[0]), boundary)?;
        let mut inside = started_inside;
        let mut intersections: Vec<(usize, Vector3, usize)> = Vec::new();
        for a in 0..blackside.len() {
            let e0 = projection.transform(blackside[a]);
            let e1 = projection.transform(blackside[(a + 1) % blackside.len()]);
            let mut hits = intersects_boundary_profile(e0, e1, boundary, inside, false)?;
            if hits.len() > 1 {
                let mut sorting = true;
                while sorting {
                    sorting = false;
                    for b in 0..hits.len() - 1 {
                        if (hits[b + 1].1 - e0).square_length() < (hits[b].1 - e0).square_length() {
                            sorting = true;
                            hits.swap(b + 1, b);
                        }
                    }
                }
            }
            for &(edge, point) in &hits {
                intersections.push((a, projection_inverse.transform(point), edge));
            }
            if hits.len() & 1 == 1 {
                inside = !inside;
            }
        }
        if intersections.len() & 1 != 0 {
            continue;
        }
        if intersections.len() > 1 {
            if !started_inside {
                let length = intersections.len();
                for b in 0..length - 1 {
                    intersections.swap(b, (b + length - 1) % length);
                }
            }
            let mut a = 0usize;
            while !intersections.is_empty() && a < intersections.len() - 1 {
                let next = intersections[(a + 1) % intersections.len()].1;
                if (intersections[a].1 - next).square_length() < 1e-10 {
                    intersections.drain(a..a + 2);
                } else {
                    a += 1;
                }
            }
            if intersections.len() > 1 {
                let front = intersections[0].1;
                let back = intersections[intersections.len() - 1].1;
                if (back - front).square_length() < 1e-10 {
                    intersections.pop();
                    intersections.remove(0);
                }
            }
        }
        if intersections.is_empty() {
            if !started_inside {
                result.vertcnt.push(blackside.len() as u32);
                result.verts.extend_from_slice(&blackside);
            }
            continue;
        }

        let black_normal = TempMesh::polygon_normal(&blackside, true);
        let polygon_normal = projection.matrix3().transform(black_normal);
        let march_backwards = dot(profile_normal, polygon_normal) >= 0.0;
        if boundary.is_empty() {
            return Err(Failure::undefined("an IFC boundary of no points stepped through (a division by zero)"));
        }
        while !intersections.is_empty() {
            let mut polygon: Vec<Vector3> = Vec::new();
            let mut current_index = 0usize;
            loop {
                let pair_left = intersections.len() > current_index.wrapping_add(1);
                Failure::assertion(pair_left, "intersections.size() > currentIntersecIdx + 1")?;
                let (Some(&current), Some(&next)) = (intersections.get(current_index), intersections.get(current_index.wrapping_add(1))) else {
                    return Err(Failure::undefined("an IFC boolean intersection read past its list"));
                };
                intersections.drain(current_index..current_index + 2);
                polygon.push(current.1);
                let wrap = if current.0 > next.0 { blackside.len() } else { 0 };
                let points = wrap + next.0 - current.0;
                for a in 1..=points {
                    polygon.push(blackside[(current.0 + a) % blackside.len()]);
                }
                polygon.push(next.1);

                let mut starting_point = projection.transform(next.1);
                let mut edge = (next.2 + if march_backwards { 1 } else { 0 }) % boundary.len();
                let mut next_intersection = usize::MAX;
                while next_intersection == usize::MAX {
                    let mut t = 1e10f64;
                    let mut next_edge = if march_backwards { edge + boundary.len() - 1 } else { edge + 1 };
                    next_edge %= boundary.len();
                    let mut current_point = boundary[edge];
                    let mut next_point = boundary[next_edge];
                    if polygon_normal.z.abs() > 1e-5 {
                        current_point.z = starting_point.z
                            + (current_point.x - starting_point.x) * polygon_normal.x / polygon_normal.z
                            + (current_point.y - starting_point.y) * polygon_normal.y / polygon_normal.z;
                        next_point.z = starting_point.z
                            + (next_point.x - starting_point.x) * polygon_normal.x / polygon_normal.z
                            + (next_point.y - starting_point.y) * polygon_normal.y / polygon_normal.z;
                    }
                    let boundary_plane_normal = cross(next_point - current_point, profile_normal).normalize();
                    let along = cross(boundary_plane_normal, polygon_normal).normalize() * if march_backwards { -1.0 } else { 1.0 };
                    if polygon_normal.z.abs() > 1e-5 {
                        let length = (next_point - starting_point).length();
                        t = if length < t { length } else { t };
                    }
                    let to_start = projection.transform(polygon[0]) - starting_point;
                    let mut tpt = dot(to_start, along);
                    if tpt > -1e-6 && tpt <= t && (to_start - tpt * along).square_length() < 1e-10 {
                        next_intersection = intersections.len();
                        t = tpt;
                    }
                    let mut a = 0usize;
                    while a < intersections.len() {
                        let to_point = projection.transform(intersections[a].1) - starting_point;
                        tpt = dot(to_point, along);
                        if tpt > -1e-6 && tpt <= t && (to_point - tpt * along).square_length() < 1e-10 {
                            next_intersection = a;
                            t = tpt;
                        }
                        a += 2;
                    }
                    if next_intersection == usize::MAX {
                        polygon.push(projection_inverse.transform(next_point));
                        edge = next_edge;
                        starting_point = next_point;
                    }
                    if polygon.len() > blackside.len() + boundary.len() {
                        break;
                    }
                }
                if next_intersection >= intersections.len() {
                    break;
                }
                current_index = next_intersection;
            }
            write_polygon(&mut polygon, result);
        }
    }
    Ok(())
}

/// `ProcessBooleanExtrudedAreaSolidDifference`: the solid extruded (with
/// the element's openings) and cut out of each polygon of the operand as
/// an opening; the result starts as the operand. A degenerate polygon is
/// skipped without moving on through the vertices.
fn process_extruded_difference(conversion: &mut Conversion, solid: &Object, result: &mut TempMesh, first: &TempMesh) -> Outcome<()> {
    let mut extruded = TempMesh::default();
    process_extruded_area_solid(conversion, solid, &mut extruded, false)?;
    let shared = Rc::new(RefCell::new(extruded));
    let mut openings = vec![TempOpening::new(Vector3::default(), Some(shared), None)];
    *result = first.clone();
    let mut start = 0usize;
    for &count in &first.vertcnt {
        let count_usize = count as usize;
        let Some(polygon) = first.verts.get(start..start + count_usize) else {
            return Err(Failure::undefined("an IFC boolean operand's polygon read past its vertices"));
        };
        let mut temp = TempMesh { verts: polygon.to_vec(), vertcnt: vec![count] };
        let normal = TempMesh::polygon_normal(&temp.verts, false);
        if normal.square_length() < 1e-5f64 {
            continue;
        }
        generate_openings(&mut openings, &mut temp, false, true, Vector3::new(0.0, 1.0, 0.0))?;
        result.append(&temp);
        start += count_usize;
    }
    Ok(())
}

/// `ProcessBoolean`: DIFFERENCE of a swept solid or boolean by a half
/// space (bounded or not) or an extruded solid.
pub(super) fn process_boolean(conversion: &mut Conversion, boolean: &Object, result: &mut TempMesh) -> Outcome<()> {
    conversion.enter(BOOLEAN_FRAME, "IFC booleans")?;
    let processed = boolean_difference(conversion, boolean, result);
    conversion.leave(BOOLEAN_FRAME);
    processed
}

/// The part of `ProcessBoolean` after its frame.
fn boolean_difference(conversion: &mut Conversion, boolean: &Object, result: &mut TempMesh) -> Outcome<()> {
    let db = conversion.db;
    if !boolean.is_a(Class::IfcBooleanResult) || boolean.text(IfcBooleanResult::OPERATOR) != b"DIFFERENCE" {
        return Ok(());
    }
    let second = boolean.select(IfcBooleanResult::SECOND_OPERAND);
    let half_space = db.resolve_select_ptr(&second, Class::IfcHalfSpaceSolid)?;
    let extruded = db.resolve_select_ptr(&second, Class::IfcExtrudedAreaSolid)?;
    if half_space.is_none() && extruded.is_none() {
        return Ok(());
    }
    let first_select = boolean.select(IfcBooleanResult::FIRST_OPERAND);
    let mut first = TempMesh::default();
    if let Some(nested) = db.resolve_select_ptr(&first_select, Class::IfcBooleanResult)? {
        process_boolean(conversion, &nested, &mut first)?;
    } else if let Some(swept) = db.resolve_select_ptr(&first_select, Class::IfcSweptAreaSolid)? {
        process_swept_area_solid(conversion, &swept, &mut first)?;
    } else {
        return Ok(());
    }
    if let Some(half_space) = half_space {
        if let Some(bounded) = db.resolve_select_ptr(&second, Class::IfcPolygonalBoundedHalfSpace)? {
            return process_bounded_half_space_difference(conversion, &bounded, result, &first);
        }
        return process_half_space_difference(conversion, &half_space, result, &first);
    }
    let extruded = extruded.expect("one operand was found");
    process_extruded_difference(conversion, &extruded, result, &first)
}
