//! `IFCOpenings`: openings cut into a polygon of a wall. The polygon is
//! projected onto its plane and scaled to [0,1]²; each opening's faces
//! facing the plane (or its 2D profile) give a contour whose bounding
//! box becomes a hole; boxes are merged with Clipper, the square is
//! split into quads around them, the window shapes put back in, the
//! result clipped to the polygon's own contour and projected back, and
//! the two sides of a wall joined across each window. When merging
//! leaves several pieces, the polygon is triangulated around the holes
//! with poly2tri instead.

use std::collections::VecDeque;

use super::clipper::{ClipType, Clipper, IntPoint, Path, PolyType, orientation};
use super::geometry::derive_plane_coordinate_space;
use super::math::{Matrix3, Matrix4, Vector2, Vector3, cross, dot, std_max, std_min};
use super::poly2tri::{Cdt, Point as P2tPoint};
use super::util::{EPSILON, TempMesh, TempOpening};
use super::{Failure, Outcome};
use crate::cast::to_u64;
use crate::msvc::UniqueSet;

/// `max_ulong64`: [0,1] is scaled to this in Clipper's integers.
const MAX_ULONG64: f64 = 1_518_500_249.0;

/// `std::numeric_limits<float>::epsilon()`.
const FLOAT_EPSILON: f64 = f32::EPSILON as f64;

/// Quadrify's recursion: 224-byte frames, past the stack budget the
/// native stack is gone (what the conversion has used above it left out).
const MAX_QUADRIFY_DEPTH: usize = (crate::scene::STACK_BUDGET / 224) as usize;

/// `to_int64`: `(ulong64)(p * max_ulong64)`, kept as Clipper's `cInt`.
fn to_int64(p: f64) -> i64 {
    to_u64(p * MAX_ULONG64) as i64
}

/// `from_int64`: the coordinate as an unsigned value, over the scale.
fn from_int64(p: i64) -> f64 {
    (p as u64) as f64 / MAX_ULONG64
}

fn one_vec() -> Vector2 {
    Vector2::new(1.0, 1.0)
}

/// `std::max(v, IfcVector2())` then `std::min(v, one_vec)`.
fn clamp_unit(v: Vector2) -> Vector2 {
    let low = Vector2::new(std_max(v.x, 0.0), std_max(v.y, 0.0));
    Vector2::new(std_min(low.x, 1.0), std_min(low.y, 1.0))
}

fn min2(a: Vector2, b: Vector2) -> Vector2 {
    Vector2::new(std_min(a.x, b.x), std_min(a.y, b.y))
}

fn max2(a: Vector2, b: Vector2) -> Vector2 {
    Vector2::new(std_max(a.x, b.x), std_max(a.y, b.y))
}

/// `MinMaxChooser<IfcVector2>`'s start.
fn bounds_start_2d() -> (Vector2, Vector2) {
    let far = 1e10f32 as f64;
    (Vector2::new(far, far), Vector2::new(-far, -far))
}

/// `XYSorter`.
fn xy_less(a: &Vector2, b: &Vector2) -> bool {
    if a.x == b.x {
        return a.y < b.y;
    }
    a.x < b.x
}

/// `0 != (a - b)` as a condition (NaN counts as non-zero).
fn nonzero(value: f64) -> bool {
    value != 0.0
}

type BoundingBox = (Vector2, Vector2);

fn fill_rectangle(min: Vector2, max: Vector2, out: &mut Vec<Vector2>) {
    out.extend([Vector2::new(min.x, min.y), Vector2::new(min.x, max.y), Vector2::new(max.x, max.y), Vector2::new(max.x, min.y)]);
}

/// `QuadrifyPart`: the rectangle split into quads around the boxes, the
/// boxes ordered by their minimum corner (`XYSortedField`).
fn quadrify_part(min: Vector2, max: Vector2, field: &[(Vector2, usize)], boxes: &[BoundingBox], out: &mut Vec<Vector2>, depth: usize) -> Outcome<()> {
    if depth >= MAX_QUADRIFY_DEPTH {
        return Err(Failure::undefined("IFC window quads split past the stack"));
    }
    if !nonzero(min.x - max.x) || !nonzero(min.y - max.y) {
        return Ok(());
    }
    let mut xs = 1e10f64;
    let mut xe = 1e10f64;
    let mut found = false;
    let mut start = 0usize;
    while start < field.len() {
        let bb = boxes[field[start].1];
        if bb.0.x >= max.x {
            break;
        }
        if bb.1.x > min.x && bb.1.y > min.y && bb.0.y < max.y {
            xs = bb.0.x;
            xe = bb.1.x;
            found = true;
            break;
        }
        start += 1;
    }
    if !found {
        fill_rectangle(min, max, out);
        return Ok(());
    }
    xs = std_max(min.x, xs);
    xe = std_min(max.x, xe);
    if nonzero(xs - min.x) {
        out.extend([min, Vector2::new(min.x, max.y), Vector2::new(xs, max.y), Vector2::new(xs, min.y)]);
    }
    let mut ylast = min.y;
    found = false;
    while start < field.len() {
        let bb = boxes[field[start].1];
        if bb.0.x > xs || bb.0.y >= max.y {
            break;
        }
        if bb.1.y > ylast {
            found = true;
            let ys = std_max(bb.0.y, min.y);
            let ye = std_min(bb.1.y, max.y);
            if ys - ylast > 0.0 {
                quadrify_part(Vector2::new(xs, ylast), Vector2::new(xe, ys), field, boxes, out, depth + 1)?;
            }
            ylast = ye;
        }
        start += 1;
    }
    if !found {
        out.extend([Vector2::new(xs, min.y), Vector2::new(xs, max.y), Vector2::new(xe, max.y), Vector2::new(xe, min.y)]);
        return Ok(());
    }
    if ylast < max.y {
        quadrify_part(Vector2::new(xs, ylast), Vector2::new(xe, max.y), field, boxes, out, depth + 1)?;
    }
    if nonzero(max.x - xe) {
        quadrify_part(Vector2::new(xe, min.y), max, field, boxes, out, depth + 1)?;
    }
    Ok(())
}

/// `ProjectedWindowContour`.
#[derive(Clone, Debug)]
struct WindowContour {
    contour: Vec<Vector2>,
    bb: BoundingBox,
    skiplist: Vec<bool>,
    is_rectangular: bool,
}

impl WindowContour {
    fn is_invalid(&self) -> bool {
        self.contour.is_empty()
    }
}

/// `BoundingBoxesOverlapping`.
fn boxes_overlapping(ibb: &BoundingBox, bb: &BoundingBox) -> bool {
    ibb.0.x < bb.1.x && ibb.1.x > bb.0.x && ibb.0.y < bb.1.y && ibb.1.y > bb.0.y
}

/// `IsDuplicateVertex`.
fn is_duplicate_vertex(vertex: Vector2, contour: &[Vector2]) -> bool {
    contour.iter().any(|&point| (point - vertex).square_length() < 1e-5f32 as f64)
}

/// `ExtractVerticesFromClipper`.
fn extract_vertices(path: &Path, contour: &mut Vec<Vector2>, filter_duplicates: bool) {
    contour.clear();
    for point in path {
        let vertex = clamp_unit(Vector2::new(from_int64(point.x), from_int64(point.y)));
        if !filter_duplicates || !is_duplicate_vertex(vertex, contour) {
            contour.push(vertex);
        }
    }
}

/// `GetBoundingBox` of a Clipper path.
fn path_bounding_box(path: &Path) -> BoundingBox {
    let (mut min, mut max) = bounds_start_2d();
    for point in path {
        let vertex = clamp_unit(Vector2::new(from_int64(point.x), from_int64(point.y)));
        min = min2(min, vertex);
        max = max2(max, vertex);
    }
    (min, max)
}

fn to_path(contour: &[Vector2]) -> Path {
    contour.iter().map(|point| IntPoint { x: to_int64(point.x), y: to_int64(point.y) }).collect()
}

/// A path wound clockwise (reversed when Clipper finds it positive).
fn negative_path(contour: &[Vector2]) -> Path {
    let mut path = to_path(contour);
    if orientation(&path) {
        path.reverse();
    }
    path
}

/// `MergeWindowContours`: the union of two contours.
fn merge_window_contours(a: &[Vector2], b: &[Vector2]) -> Outcome<Vec<Path>> {
    let mut clipper = Clipper::new();
    clipper.add_path(&negative_path(a), PolyType::Subject)?;
    clipper.add_path(&negative_path(b), PolyType::Subject)?;
    clipper.execute(ClipType::Union)
}

/// `MakeDisjunctWindowContours`: b minus a.
fn make_disjunct_window_contours(a: &[Vector2], b: &[Vector2]) -> Outcome<Vec<Path>> {
    let mut clipper = Clipper::new();
    clipper.add_path(&negative_path(a), PolyType::Clip)?;
    clipper.add_path(&negative_path(b), PolyType::Subject)?;
    clipper.execute(ClipType::Difference)
}

/// `CleanupWindowContours`: a window whose contour unites to nothing is
/// dropped (what it unites to is otherwise unused).
fn cleanup_window_contours(contours: &mut [WindowContour]) -> Outcome<()> {
    for window in contours.iter_mut() {
        let mut clipper = Clipper::new();
        clipper.add_path(&to_path(&window.contour), PolyType::Subject)?;
        let clipped = clipper.execute(ClipType::Union)?;
        if clipped.is_empty() {
            window.contour.clear();
        }
    }
    Ok(())
}

/// `CleanupOuterContour`: every polygon intersected with the outer
/// contour (a zero count drops the vertex it was reached at).
fn cleanup_outer_contour(contour_flat: &[Vector2], mesh: &mut TempMesh) -> Outcome<()> {
    let mut clip = to_path(contour_flat);
    if !orientation(&clip) {
        clip.reverse();
    }
    let mut vertices = Vec::with_capacity(mesh.verts.len());
    let mut counts = Vec::with_capacity(mesh.vertcnt.len());
    let mut subject: Path = Vec::with_capacity(4);
    let mut index = 0usize;
    let mut countdown = 0u32;
    let mut clipper = Clipper::new();
    for vertex in &mesh.verts {
        if countdown == 0 {
            let Some(&count) = mesh.vertcnt.get(index) else {
                return Err(Failure::undefined("an IFC polygon count read past the counts"));
            };
            index += 1;
            countdown = count;
            if countdown == 0 {
                continue;
            }
        }
        subject.push(IntPoint { x: to_int64(vertex.x), y: to_int64(vertex.y) });
        countdown -= 1;
        if countdown == 0 {
            if !orientation(&subject) {
                subject.reverse();
            }
            clipper.add_path(&subject, PolyType::Subject)?;
            clipper.add_path(&clip, PolyType::Clip)?;
            let clipped = clipper.execute(ClipType::Intersection)?;
            for path in &clipped {
                counts.push(path.len() as u32);
                for point in path {
                    vertices.push(Vector3::new(from_int64(point.x), from_int64(point.y), 0.0));
                }
            }
            subject.clear();
            clipper.clear();
        }
    }
    mesh.verts = vertices;
    mesh.vertcnt = counts;
    Ok(())
}

/// `InsertWindowContours`: the real window shapes put into their square
/// holes, a polygon for each stretch between points on the box's sides.
fn insert_window_contours(contours: &[WindowContour], mesh: &mut TempMesh) {
    for window in contours {
        let bb = window.bb;
        let contour = &window.contour;
        if contour.is_empty() {
            continue;
        }
        if contour.len() == 4 {
            let mut corners = UniqueSet::new();
            let mut less = |a: &Vector2, b: &Vector2| Ok::<bool, ()>(xy_less(a, b));
            for &point in contour.iter().take(4) {
                let _ = corners.insert(point, &mut less);
            }
            let wanted = [bb.0, bb.1, Vector2::new(bb.0.x, bb.1.y), Vector2::new(bb.1.x, bb.0.y)];
            let all = wanted.iter().all(|corner| matches!(corners.find(corner, &mut less), Ok(Some(_))));
            if all {
                continue;
            }
        }
        let diagonal = (bb.0 - bb.1).length();
        let epsilon = diagonal / 1000.0f32 as f64;
        let size = contour.len();
        let mut last_hit = usize::MAX;
        let mut very_first_hit = usize::MAX;
        let mut edge = Vector2::default();
        let mut n = 0usize;
        let mut steps = 0usize;
        loop {
            if steps == size * 2 {
                break;
            }
            let v = contour[n];
            let mut hit = false;
            if (v.x - bb.0.x).abs() < epsilon {
                edge.x = bb.0.x;
                hit = true;
            } else if (v.x - bb.1.x).abs() < epsilon {
                edge.x = bb.1.x;
                hit = true;
            }
            if (v.y - bb.0.y).abs() < epsilon {
                edge.y = bb.0.y;
                hit = true;
            } else if (v.y - bb.1.y).abs() < epsilon {
                edge.y = bb.1.y;
                hit = true;
            }
            if hit {
                if last_hit != usize::MAX {
                    let old = mesh.verts.len();
                    let count = if last_hit > n { size - (last_hit - n) } else { n - last_hit };
                    let mut a = last_hit;
                    for _ in 0..=count {
                        let far = (contour[a] - edge).square_length() > diagonal * diagonal * 0.7;
                        if !far {
                            mesh.verts.push(Vector3::new(contour[a].x, contour[a].y, 0.0));
                        }
                        a = (a + 1) % size;
                    }
                    if edge != contour[last_hit] {
                        let mut corner = edge;
                        let last = contour[last_hit];
                        if (last.x - bb.0.x).abs() < epsilon {
                            corner.x = bb.0.x;
                        } else if (last.x - bb.1.x).abs() < epsilon {
                            corner.x = bb.1.x;
                        }
                        if (last.y - bb.0.y).abs() < epsilon {
                            corner.y = bb.0.y;
                        } else if (last.y - bb.1.y).abs() < epsilon {
                            corner.y = bb.1.y;
                        }
                        mesh.verts.push(Vector3::new(corner.x, corner.y, 0.0));
                    } else if count == 1 {
                        mesh.verts.truncate(old);
                    }
                    let added = mesh.verts.len() - old;
                    if added > 0 {
                        mesh.vertcnt.push(added as u32);
                        let length = mesh.verts.len();
                        mesh.verts[length - added..].reverse();
                    }
                    if n == very_first_hit {
                        break;
                    }
                } else {
                    very_first_hit = n;
                }
                last_hit = n;
            }
            n = (n + 1) % size;
            steps += 1;
        }
    }
}

/// `BoundingBoxesAdjacent`.
fn boxes_adjacent(bb: &BoundingBox, ibb: &BoundingBox) -> bool {
    let epsilon = FLOAT_EPSILON;
    ((bb.1.x - ibb.0.x).abs() < epsilon && bb.0.y <= ibb.1.y && bb.1.y >= ibb.0.y)
        || ((bb.0.x - ibb.1.x).abs() < epsilon && ibb.0.y <= bb.1.y && ibb.1.y >= bb.0.y)
        || ((bb.1.y - ibb.0.y).abs() < epsilon && bb.0.x <= ibb.1.x && bb.1.x >= ibb.0.x)
        || ((bb.0.y - ibb.1.y).abs() < epsilon && ibb.0.x <= bb.1.x && ibb.1.x >= bb.0.x)
}

/// `IntersectingLineSegments`: where m0–m1 overlaps n0–n1 when the two
/// are collinear, as points on n0–n1.
fn intersecting_line_segments(n0: Vector2, n1: Vector2, m0: Vector2, m1: Vector2) -> Option<(Vector2, Vector2)> {
    let n0_to_n1 = n1 - n0;
    let n0_to_m0 = m0 - n0;
    let n1_to_m1 = m1 - n1;
    let n0_to_m1 = m1 - n0;
    let e = 1e-5f32 as f64;
    let small = 1e-9f32 as f64;
    let dot2 = |a: Vector2, b: Vector2| a.x * b.x + a.y * b.y;
    if !(n0_to_m0.square_length() < e * e || dot2(n0_to_m0, n0_to_n1).abs() / (n0_to_m0.length() * n0_to_n1.length()) > 1.0 - 1e-5) {
        return None;
    }
    if !(n1_to_m1.square_length() < e * e || dot2(n1_to_m1, n0_to_n1).abs() / (n1_to_m1.length() * n0_to_n1.length()) > 1.0 - 1e-5) {
        return None;
    }
    let (mut s0, mut s1);
    if n0_to_n1.x.abs() > n0_to_n1.y.abs() {
        s0 = n0_to_m0.x / n0_to_n1.x;
        s1 = n0_to_m1.x / n0_to_n1.x;
        if s0.abs() == f64::INFINITY && n0_to_m0.x.abs() < small {
            s0 = 0.0;
        }
        if s1.abs() == f64::INFINITY && n0_to_m1.x.abs() < small {
            s1 = 0.0;
        }
    } else {
        s0 = n0_to_m0.y / n0_to_n1.y;
        s1 = n0_to_m1.y / n0_to_n1.y;
        if s0.abs() == f64::INFINITY && n0_to_m0.y.abs() < small {
            s0 = 0.0;
        }
        if s1.abs() == f64::INFINITY && n0_to_m1.y.abs() < small {
            s1 = 0.0;
        }
    }
    if s1 < s0 {
        std::mem::swap(&mut s0, &mut s1);
    }
    s0 = std_max(0.0, s0);
    s1 = std_max(0.0, s1);
    s0 = std_min(1.0, s0);
    s1 = std_min(1.0, s1);
    if (s1 - s0).abs() < e {
        return None;
    }
    Some((n0 + s0 * n0_to_n1, n0 + s1 * n0_to_n1))
}

/// `FindAdjacentContours`: for the current window, the edges it shares
/// with adjacent windows (and itself) are marked to skip, splitting its
/// contour where they overlap.
fn find_adjacent_contours(current: usize, contours: &mut [WindowContour]) -> Outcome<()> {
    let sqlen_epsilon = FLOAT_EPSILON;
    let bb = contours[current].bb;
    for other in 0..contours.len() {
        if contours[other].is_invalid() {
            continue;
        }
        let is_me = other == current;
        let ibb = contours[other].bb;
        // The boxes are taken to be disjoint or identical.
        Failure::assertion(is_me || !boxes_overlapping(&bb, &ibb), "is_me || !BoundingBoxesOverlapping(bb, ibb)")?;
        if !(is_me || boxes_adjacent(&bb, &ibb)) {
            continue;
        }
        let mut n = 0usize;
        while n < contours[current].contour.len() {
            let size = contours[current].contour.len();
            let n0 = contours[current].contour[n];
            let n1 = contours[current].contour[(n + 1) % size];
            let end = if is_me { n } else { contours[other].contour.len() };
            for m in 0..end {
                let other_contour = &contours[other].contour;
                let m0 = other_contour[m];
                let m1 = other_contour[(m + 1) % other_contour.len()];
                let Some((isect0, isect1)) = intersecting_line_segments(n0, n1, m0, m1) else {
                    continue;
                };
                let window = &mut contours[current];
                if (isect0 - n0).square_length() > sqlen_epsilon {
                    n += 1;
                    window.contour.insert(n, isect0);
                    window.skiplist.insert(n, true);
                } else {
                    window.skiplist[n] = true;
                }
                if (isect1 - n1).square_length() > sqlen_epsilon {
                    n += 1;
                    window.contour.insert(n, isect1);
                    window.skiplist.insert(n, false);
                }
            }
            n += 1;
        }
    }
    Ok(())
}

/// `LikelyBorder`.
fn likely_border(delta: Vector2) -> bool {
    (delta.x * delta.y).abs() < FLOAT_EPSILON
}

/// `FindBorderContours`: edges along the projection's outer border are
/// skipped (door openings have nothing to close there).
fn find_border_contours(window: &mut WindowContour) {
    let upper = 1.0 - 1e-4;
    let lower = 1e-4;
    let mut outer_border = false;
    let mut start_on_outer_border = false;
    let mut last = Vector2::default();
    for index in 0..window.contour.len() {
        let point = window.contour[index];
        if point.x <= lower || point.x >= upper || point.y <= lower || point.y >= upper {
            if outer_border {
                if likely_border(point - last) {
                    window.skiplist[index - 1] = true;
                }
            } else if index == 0 {
                start_on_outer_border = true;
            }
            outer_border = true;
        } else {
            outer_border = false;
        }
        last = point;
    }
    if outer_border && start_on_outer_border && likely_border(window.contour[0] - last) {
        let last_index = window.skiplist.len() - 1;
        window.skiplist[last_index] = true;
    }
}

/// `LikelyDiagonal`.
fn likely_diagonal(delta: Vector2) -> bool {
    let x = delta.x.abs();
    let y = delta.y.abs();
    (x - y).abs() < 0.8 * std_max(x, y)
}

/// `FindLikelyCrossingLines`: diagonal edges of a rectangular window are
/// skipped.
fn find_likely_crossing_lines(window: &mut WindowContour) {
    let mut last = Vector2::default();
    for index in 0..window.contour.len() {
        let point = window.contour[index];
        if index != 0 && likely_diagonal(point - last) {
            window.skiplist[index - 1] = true;
        }
        last = point;
    }
    if likely_diagonal(window.contour[0] - last) {
        let last_index = window.skiplist.len() - 1;
        window.skiplist[last_index] = true;
    }
}

/// `CloseWindows`: a window whose openings already have the other side's
/// points gets quads from each contour point to the nearest of them;
/// otherwise its points are left for the other side.
fn close_windows(contours: &mut [WindowContour], minv: &Matrix4, contours_to_openings: &[Vec<usize>], openings: &mut [TempOpening], mesh: &mut TempMesh) -> Outcome<usize> {
    let mut closed = 0usize;
    for index in 0..contours.len() {
        if contours[index].is_invalid() {
            continue;
        }
        let Some(refs) = contours_to_openings.get(index) else {
            return Err(Failure::undefined("an IFC window's openings read past the list"));
        };
        let has_other_side = refs.iter().any(|&opening| !openings[opening].wall_points.is_empty());
        if !has_other_side {
            for &opening in refs {
                Failure::assertion(openings[opening].wall_points.is_empty(), "opening->wallPoints.empty()")?;
                for point in &contours[index].contour {
                    let world = minv.transform(Vector3::new(point.x, point.y, 0.0));
                    openings[opening].wall_points.push(world);
                }
            }
            continue;
        }
        let size = contours[index].contour.len();
        contours[index].skiplist = vec![false; size];
        find_adjacent_contours(index, contours)?;
        find_border_contours(&mut contours[index]);
        if contours[index].is_rectangular {
            find_likely_crossing_lines(&mut contours[index]);
        }
        let lengths_match = contours[index].skiplist.len() == contours[index].contour.len();
        Failure::assertion(lengths_match, "(*it).skiplist.size() == (*it).contour.size()")?;
        let window = contours[index].clone();
        let mut reverse_faces = false;
        if let Some(&first_count) = mesh.vertcnt.first() {
            let Some(base_polygon) = mesh.verts.get(..first_count as usize) else {
                return Err(Failure::undefined("an IFC wall polygon read past its vertices"));
            };
            let base_normal = TempMesh::polygon_normal(base_polygon, true);
            let world: Vec<Vector3> = window.contour.iter().map(|point| minv.transform(Vector3::new(point.x, point.y, 0.0))).collect();
            let contour_normal = TempMesh::polygon_normal(&world, true);
            reverse_faces = dot(contour_normal, base_normal) > 0.0;
        }
        let mut start0 = Vector3::default();
        let mut start1 = Vector3::default();
        let mut drop_this_edge = false;
        let count = window.contour.len();
        for position in 0..count {
            let point = window.contour[position];
            let mut best = 1e10f64;
            let mut best_point = Vector3::default();
            let world = minv.transform(Vector3::new(point.x, point.y, 0.0));
            for &opening in refs {
                for &other in &openings[opening].wall_points {
                    let distance = (world - other).square_length();
                    if distance < best {
                        if distance < 1e-5 {
                            continue;
                        }
                        best_point = other;
                        best = distance;
                    }
                }
            }
            let first = position == 0;
            if drop_this_edge {
                mesh.verts.pop();
                mesh.verts.pop();
            } else {
                let world_first = first != reverse_faces;
                mesh.verts.push(if world_first { world } else { best_point });
                mesh.verts.push(if world_first { best_point } else { world });
                mesh.vertcnt.push(4);
                closed += 1;
            }
            if first {
                start0 = world;
                start1 = best_point;
                drop_this_edge = window.skiplist[position];
                continue;
            }
            mesh.verts.push(if reverse_faces { best_point } else { world });
            mesh.verts.push(if reverse_faces { world } else { best_point });
            if position == count - 1 {
                if window.skiplist[position] {
                    closed = closed.wrapping_sub(1);
                    mesh.vertcnt.pop();
                    mesh.verts.pop();
                    mesh.verts.pop();
                } else {
                    mesh.verts.push(if reverse_faces { start0 } else { start1 });
                    mesh.verts.push(if reverse_faces { start1 } else { start0 });
                }
            }
            drop_this_edge = window.skiplist[position];
        }
    }
    Ok(closed)
}

/// `Quadrify`: [0,1]² split into quads around the boxes (by minimum
/// corner; a repeated one keeps the later box).
fn quadrify(boxes: &[BoundingBox], mesh: &mut TempMesh) -> Outcome<()> {
    let mut field = UniqueSet::new();
    let mut less = |a: &(Vector2, usize), b: &(Vector2, usize)| Ok::<bool, ()>(xy_less(&a.0, &b.0));
    for (index, bb) in boxes.iter().enumerate() {
        let key = (bb.0, index);
        match field.find(&key, &mut less) {
            Ok(Some(node)) => {
                let existing = field.get(node).0;
                field.replace(node, (existing, index));
            }
            _ => {
                let _ = field.insert(key, &mut less);
            }
        }
    }
    let ordered = field.in_order();
    let mut quads = Vec::with_capacity(boxes.len() * 4);
    quadrify_part(Vector2::default(), one_vec(), &ordered, boxes, &mut quads, 0)?;
    mesh.vertcnt.resize(quads.len() / 4, 4);
    mesh.verts.extend(quads.iter().map(|point| Vector3::new(point.x, point.y, 0.0)));
    Ok(())
}

/// `ProjectOntoPlane`: the mesh's points in its plane's basis, scaled to
/// [0,1]², and the matrix doing that (z moved to the plane).
fn project_onto_plane(mesh: &TempMesh) -> Outcome<Option<(Matrix4, Vec<Vector2>, Vector3)>> {
    let input = &mesh.verts;
    if input.is_empty() {
        return Ok(None);
    }
    let (basis, ok, normal) = derive_plane_coordinate_space(mesh)?;
    if !ok {
        return Ok(None);
    }
    let m = Matrix4::from_matrix3(&basis);
    // The debug build's checks, which Gaea's has.
    let determinant = m.determinant();
    Failure::assertion((determinant - 1.0).abs() < 1e-5, "std::fabs(det-1) < 1e-5")?;
    let mut z = 0.0f64;
    let (mut min, mut max) = Vector3::bounds_start();
    let mut contour = Vec::with_capacity(input.len());
    for &vertex in input {
        let projected = m.transform(vertex);
        z += projected.z;
        min = projected.min(min);
        max = projected.max(max);
        contour.push(Vector2::new(projected.x, projected.y));
    }
    z /= input.len() as f64;
    let extent = max - min;
    for point in &mut contour {
        let scaled = Vector2::new((point.x - min.x) / extent.x, (point.y - min.y) / extent.y);
        *point = clamp_unit(scaled);
    }
    let mut multiplier = Matrix4::IDENTITY;
    multiplier.0[0] = 1.0 / extent.x;
    multiplier.0[5] = 1.0 / extent.y;
    multiplier.0[3] = -min.x * multiplier.0[0];
    multiplier.0[7] = -min.y * multiplier.0[5];
    multiplier.0[11] = -z;
    let m = multiplier.product(&m);
    let mut contour_2 = Vec::with_capacity(input.len());
    for &vertex in input {
        let projected = m.transform(vertex);
        contour_2.push(Vector2::new(projected.x, projected.y));
        Failure::assertion(projected.z.abs() < extent.z + 1e-8, "std::fabs(vv.z) < vmax.z + 1e-8")?;
    }
    for (point, point_2) in contour.iter().zip(&contour_2) {
        let difference = *point - *point_2;
        Failure::assertion(difference.square_length() < EPSILON, "(out_contour[i] - out_contour2[i]).SquareLength() < ai_epsilon")?;
    }
    Ok(Some((m, contour, normal)))
}

/// `GenerateOpenings`: the openings cut into the (single-polygon) mesh;
/// whether any were.
pub(super) fn generate_openings(
    openings: &mut [TempOpening],
    mesh: &mut TempMesh,
    check_intersection: bool,
    generate_connection_geometry: bool,
    _wall_extrusion_axis: Vector3,
) -> Outcome<bool> {
    let Some((m, contour_flat, normal)) = project_onto_plane(mesh)? else {
        return Ok(false);
    };
    let minv = m.inverse();
    let mut contours: Vec<WindowContour> = Vec::new();
    let mut contours_to_openings: Vec<Vec<usize>> = Vec::new();

    for opening_index in 0..openings.len() {
        let opening = &openings[opening_index];
        let mut extrusion = opening.extrusion_dir;
        if extrusion.square_length() > 1e-10 {
            extrusion = extrusion.normalize();
        } else {
            extrusion = Vector3::default();
        }
        let Some(profile_mesh) = opening.profile_mesh.clone() else {
            return Err(Failure::undefined("an IFC opening without a profile mesh read"));
        };
        let mut profile = profile_mesh;
        let mut is_2d_source = false;
        if let Some(profile_2d) = &opening.profile_mesh_2d
            && extrusion.square_length() > 0.0
            && dot(extrusion, normal).abs() > 0.9
        {
            profile = profile_2d.clone();
            is_2d_source = true;
        }
        let (profile_verts, profile_counts) = {
            let profile = profile.borrow();
            (profile.verts.clone(), profile.vertcnt.clone())
        };
        if profile_verts.len() <= 2 {
            continue;
        }

        let (mut dmin, mut dmax) = (1e10f64, -1e10f64);
        let mut contour: Vec<Vector2> = Vec::new();
        let mut contour2: Vec<Vector2> = Vec::new();
        let (mut vpmin, mut vpmax) = bounds_start_2d();
        let (mut vpmin2, mut vpmax2) = bounds_start_2d();
        let mut vertex_index = 0usize;
        for &count in &profile_counts {
            let mut side = true;
            if !is_2d_source {
                let (Some(&a), Some(&b), Some(&c)) =
                    (profile_verts.get(vertex_index), profile_verts.get(vertex_index + 1), profile_verts.get(vertex_index + 2))
                else {
                    return Err(Failure::undefined("an IFC opening face read past its vertices"));
                };
                let face_normal = cross(c - a, b - a).normalize();
                if dot(normal, face_normal).abs() < 0.9 {
                    vertex_index += count as usize;
                    continue;
                }
                side = dot(normal, face_normal) > 0.0;
            }
            for _ in 0..count {
                let Some(&point) = profile_verts.get(vertex_index) else {
                    return Err(Failure::undefined("an IFC opening face read past its vertices"));
                };
                vertex_index += 1;
                let projected = m.transform(point);
                dmin = std_min(dmin, projected.z);
                dmax = std_max(dmax, projected.z);
                let flat = clamp_unit(Vector2::new(projected.x, projected.y));
                if side {
                    vpmin = min2(vpmin, flat);
                    vpmax = max2(vpmax, flat);
                } else {
                    vpmin2 = min2(vpmin2, flat);
                    vpmax2 = max2(vpmax2, flat);
                }
                let store = if side { &mut contour } else { &mut contour2 };
                if !is_duplicate_vertex(flat, store) {
                    store.push(flat);
                }
            }
        }
        if contour2.len() > 2 {
            let area = vpmax - vpmin;
            let area2 = vpmax2 - vpmin2;
            if contour.len() <= 2 || (area2.x * area2.y).abs() > (area.x * area.y).abs() {
                std::mem::swap(&mut contour, &mut contour2);
                vpmax = vpmax2;
                vpmin = vpmin2;
            }
        }
        if contour.len() <= 2 {
            continue;
        }
        let epsilon = (dmax - dmin).abs() * 0.0001;
        if !is_2d_source && check_intersection && (0.0 < dmin - epsilon || 0.0 > dmax + epsilon) {
            continue;
        }
        let mut bb: BoundingBox = (vpmin, vpmax);
        if (vpmax.x - vpmin.x).abs() * (vpmax.y - vpmin.y).abs() < 1e-10 {
            continue;
        }
        let mut joined = vec![opening_index];
        let mut is_rectangle = contour.len() == 4;

        let mut it = 0usize;
        while it < contours.len() {
            let ibb = contours[it].bb;
            if boxes_overlapping(&ibb, &bb) {
                if !contours[it].is_rectangular {
                    is_rectangle = false;
                }
                let other = contours[it].contour.clone();
                let disjunct = make_disjunct_window_contours(&other, &contour)?;
                if disjunct.len() == 1 {
                    let new_bb = path_bounding_box(&disjunct[0]);
                    if !boxes_overlapping(&ibb, &new_bb) {
                        bb = new_bb;
                        extract_vertices(&disjunct[0], &mut contour, false);
                        continue;
                    }
                }
                let merged = merge_window_contours(&contour, &other)?;
                if merged.len() > 1 {
                    return try_add_openings_poly2tri(openings, mesh);
                }
                if merged.is_empty() {
                    contour.clear();
                    break;
                }
                extract_vertices(&merged[0], &mut contour, false);
                bb.0 = min2(bb.0, ibb.0);
                bb.1 = max2(bb.1, ibb.1);
                if generate_connection_geometry {
                    let Some(earlier) = contours_to_openings.get(it).cloned() else {
                        return Err(Failure::undefined("an IFC window's openings read past the list"));
                    };
                    joined.extend(earlier);
                    contours_to_openings.remove(it);
                }
                contours.remove(it);
                it = 0;
                continue;
            }
            it += 1;
        }
        if !contour.is_empty() {
            if generate_connection_geometry {
                contours_to_openings.push(joined);
            }
            contours.push(WindowContour { contour, bb, skiplist: Vec::new(), is_rectangular: is_rectangle });
        }
    }

    if contours.is_empty() {
        return Ok(false);
    }
    mesh.clear();
    let boxes: Vec<BoundingBox> = contours.iter().map(|contour| contour.bb).collect();
    quadrify(&boxes, mesh)?;
    cleanup_window_contours(&mut contours)?;
    insert_window_contours(&contours, mesh);
    cleanup_outer_contour(&contour_flat, mesh)?;
    for vertex in &mut mesh.verts {
        *vertex = minv.transform(*vertex);
    }
    if generate_connection_geometry {
        close_windows(&mut contours, &minv, &contours_to_openings, openings, mesh)?;
    }
    Ok(true)
}

/// `1.f - ai_epsilon`, in floats.
fn alignment_limit() -> f64 {
    (1.0f32 - 1e-6f32) as f64
}

/// `GetContourInPlane2D`: a single-polygon opening's points in the
/// plane, each from the side of the extrusion nearer the plane.
#[allow(clippy::too_many_arguments)]
fn contour_in_plane_2d(
    mesh: &TempMesh,
    plane_space: &Matrix3,
    plane_normal: Vector3,
    plane_offset: f64,
    extrusion: Vector3,
    wall_extrusion: &mut Vector3,
    first: &mut bool,
) -> Outcome<Option<Vec<Vector2>>> {
    let (Some(&a), Some(&b), Some(&c)) = (mesh.verts.first(), mesh.verts.get(1), mesh.verts.get(2)) else {
        return Err(Failure::undefined("an IFC opening polygon's first vertices read past its vertices"));
    };
    let outer_normal = cross(c - a, b - a).normalize();
    let alignment = dot(plane_normal, outer_normal);
    if alignment.abs() < alignment_limit() {
        return Ok(None);
    }
    let mut contour = Vec::with_capacity(mesh.verts.len());
    for &vertex in &mesh.verts {
        let near = plane_space.transform(vertex);
        let far = plane_space.transform(vertex + extrusion);
        let is_extruded_side = (near.z - plane_offset).abs() > (far.z - plane_offset).abs();
        if *first {
            *first = false;
            if alignment > 0.0 {
                *wall_extrusion = extrusion;
                if is_extruded_side {
                    *wall_extrusion = -*wall_extrusion;
                }
            }
        }
        let chosen = if is_extruded_side { far } else { near };
        contour.push(Vector2::new(chosen.x, chosen.y));
    }
    Ok(Some(contour))
}

/// `isClose` (within `ai_epsilon` on both axes).
fn is_close(first: Vector2, second: Vector2) -> bool {
    let difference = second - first;
    difference.x.abs() < EPSILON && difference.y.abs() < EPSILON
}

/// `GetContoursInPlane3D`: the segments where the opening's faces cross
/// the plane, joined into contours.
fn contours_in_plane_3d(mesh: &TempMesh, plane_space: &Matrix3, plane_offset: f64) -> Outcome<Vec<Vec<Vector2>>> {
    let mut segments: VecDeque<(Vector2, Vector2)> = VecDeque::new();
    let mut start = 0usize;
    for &count in &mesh.vertcnt {
        let count = count as usize;
        let Some(face) = mesh.verts.get(start..start + count) else {
            return Err(Failure::undefined("an IFC opening face read past its vertices"));
        };
        if count <= 2 {
            start += count;
            continue;
        }
        let v0 = plane_space.transform(face[0]);
        let mut first_point = Vector2::default();
        let mut got_first = false;
        if (v0.z - plane_offset).abs() < EPSILON {
            first_point = Vector2::new(v0.x, v0.y);
            got_first = true;
        }
        let mut vn = v0;
        for &vertex in &face[1..] {
            let vp = vn;
            vn = plane_space.transform(vertex);
            let intersection = if (vn.z - plane_offset).abs() < EPSILON {
                vn
            } else if (vn.z > plane_offset) != (vp.z > plane_offset) {
                let direction = vn - vp;
                let scale = (plane_offset - vp.z) / direction.z;
                vp + scale * direction
            } else {
                continue;
            };
            if !got_first {
                if (vp.z - plane_offset).abs() < EPSILON {
                    first_point = Vector2::new(vp.x, vp.y);
                    segments.push_back((first_point, Vector2::new(intersection.x, intersection.y)));
                } else {
                    first_point = Vector2::new(intersection.x, intersection.y);
                    got_first = true;
                }
            } else {
                segments.push_back((first_point, Vector2::new(intersection.x, intersection.y)));
                got_first = false;
            }
        }
        start += count;
    }

    let mut contours = Vec::new();
    while let Some(first) = segments.pop_front() {
        let mut contour: VecDeque<Vector2> = VecDeque::from([first.0, first.1]);
        let mut found = true;
        while found {
            found = false;
            let mut index = 0usize;
            while index < segments.len() {
                let (a, b) = segments[index];
                let front = *contour.front().expect("a contour has points");
                let back = *contour.back().expect("a contour has points");
                if (is_close(front, a) && is_close(back, b)) || (is_close(back, a) && is_close(front, b)) {
                    segments.remove(index);
                    break;
                }
                found = true;
                if is_close(front, a) {
                    contour.push_front(b);
                } else if is_close(front, b) {
                    contour.push_front(a);
                } else if is_close(back, a) {
                    contour.push_back(b);
                } else if is_close(back, b) {
                    contour.push_back(a);
                } else {
                    found = false;
                }
                if found {
                    segments.remove(index);
                    break;
                }
                index += 1;
            }
        }
        if contour.len() <= 2 {
            continue;
        }
        contours.push(contour.into_iter().collect());
    }
    Ok(contours)
}

/// `GetContoursInPlane`.
#[allow(clippy::too_many_arguments)]
fn contours_in_plane(
    mesh: &TempMesh,
    plane_space: &Matrix3,
    plane_normal: Vector3,
    plane_offset: f64,
    extrusion: Vector3,
    wall_extrusion: &mut Vector3,
    first: &mut bool,
) -> Outcome<Vec<Vec<Vector2>>> {
    if mesh.vertcnt.len() == 1 {
        let contour = contour_in_plane_2d(mesh, plane_space, plane_normal, plane_offset, extrusion, wall_extrusion, first)?;
        return Ok(contour.into_iter().collect());
    }
    contours_in_plane_3d(mesh, plane_space, plane_offset)
}

/// `TryAddOpenings_Poly2Tri`: the polygon minus the union of all the
/// openings' contours, each piece triangulated with every hole (a piece
/// poly2tri throws on is skipped); the polygon kept if none worked.
fn try_add_openings_poly2tri(openings: &[TempOpening], mesh: &mut TempMesh) -> Outcome<bool> {
    let (basis, ok, normal) = derive_plane_coordinate_space(mesh)?;
    if !ok {
        return Ok(false);
    }
    let inverse = basis.inverse();
    let mut coordinate = -1.0f64;
    let (mut vmin, mut vmax) = bounds_start_2d();
    let mut contour_flat = Vec::with_capacity(mesh.verts.len());
    for &vertex in &mesh.verts {
        let projected = basis.transform(vertex);
        coordinate = projected.z;
        let flat = Vector2::new(projected.x, projected.y);
        vmin = min2(flat, vmin);
        vmax = max2(flat, vmax);
        contour_flat.push(flat);
    }
    let vmax = vmax - vmin;
    // A zero area means the projection went wrong (a NaN passes).
    Failure::assertion(vmax.length() != 0.0, "vmax.Length()")?;

    let mut wall_extrusion = Vector3::default();
    let mut first = true;
    let mut holes = Clipper::new();
    for opening in openings {
        let Some(profile) = &opening.profile_mesh else {
            return Err(Failure::undefined("an IFC opening without a profile mesh read"));
        };
        let profile = profile.borrow().clone();
        let contours = contours_in_plane(&profile, &basis, normal, coordinate, opening.extrusion_dir, &mut wall_extrusion, &mut first)?;
        for contour in contours {
            let mut hole: Path = Vec::with_capacity(contour.len());
            for point in contour {
                let x = (point.x - vmin.x) / vmax.x;
                let y = (point.y - vmin.y) / vmax.y;
                hole.push(IntPoint { x: to_int64(x), y: to_int64(y) });
            }
            if !orientation(&hole) {
                hole.reverse();
            }
            holes.add_path(&hole, PolyType::Subject)?;
        }
    }
    let holes_union = holes.execute(ClipType::Union)?;
    if holes_union.is_empty() {
        return Ok(false);
    }
    let mut polygon: Path = Vec::with_capacity(contour_flat.len());
    for point in &contour_flat {
        let x = (point.x - vmin.x) / vmax.x;
        let y = (point.y - vmin.y) / vmax.y;
        polygon.push(IntPoint { x: to_int64(x), y: to_int64(y) });
    }
    if orientation(&polygon) {
        polygon.reverse();
    }
    holes.clear();
    holes.add_path(&polygon, PolyType::Subject)?;
    holes.add_paths(&holes_union, PolyType::Clip)?;
    let clipped = holes.execute(ClipType::Difference)?;

    let old_verts = std::mem::take(&mut mesh.verts);
    let old_counts = std::mem::take(&mut mesh.vertcnt);
    let mut result = false;
    for clip in &clipped {
        let outline: Vec<P2tPoint> = clip.iter().map(|point| P2tPoint::new(from_int64(point.x), from_int64(point.y))).collect();
        let mut cdt = match Cdt::new(outline) {
            Ok(cdt) => cdt,
            Err(failure) if failure.is_std_exception() => continue,
            Err(failure) => return Err(failure),
        };
        for hole in &holes_union {
            let points: Vec<P2tPoint> = hole.iter().map(|point| P2tPoint::new(from_int64(point.x), from_int64(point.y))).collect();
            cdt.add_hole(points)?;
        }
        match cdt.triangulate() {
            Ok(()) => {}
            Err(failure) if failure.is_std_exception() => continue,
            Err(failure) => return Err(failure),
        }
        for triangle in cdt.triangles() {
            for point in triangle {
                let inside = point.x <= 1.0 && point.x >= 0.0 && point.y <= 1.0 && point.y >= 0.0;
                Failure::assertion(inside, "v.x <= 1.0 && v.x >= 0.0 && v.y <= 1.0 && v.y >= 0.0")?;
                let v = Vector3::new(vmin.x + point.x * vmax.x, vmin.y + point.y * vmax.y, coordinate);
                mesh.verts.push(inverse.transform(v));
            }
            mesh.vertcnt.push(3);
        }
        result = true;
    }
    if !result {
        mesh.verts.extend(old_verts);
        mesh.vertcnt.extend(old_counts);
    }
    Ok(result)
}
