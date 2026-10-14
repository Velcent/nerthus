//! `IFCUtil`: the polygon soup meshes are built in (`TempMesh`), openings
//! waiting to be cut (`TempOpening`), and the conversions of points,
//! directions, placements and transformation operators.

use std::cell::RefCell;
use std::rc::Rc;

use super::math::{Matrix4, Vector3, array_bounds, cross, dot};
use super::schema::Class;
use super::schema::fields::{
    IfcAxis1Placement, IfcAxis2Placement2D, IfcAxis2Placement3D, IfcCartesianPoint, IfcCartesianTransformationOperator,
    IfcCartesianTransformationOperator3D, IfcCartesianTransformationOperator3DnonUniform, IfcDirection, IfcPlacement, IfcVector,
};
use super::step::{Database, Object, Value};
use super::{Failure, Outcome};
use crate::scene;

/// `ai_epsilon` (a float) as the doubles here compare it.
pub(crate) const EPSILON: f64 = 1e-6f32 as f64;

/// A polygon soup: vertices and each polygon's vertex count.
#[derive(Clone, Debug, Default)]
pub(crate) struct TempMesh {
    pub verts: Vec<Vector3>,
    pub vertcnt: Vec<u32>,
}

/// A mesh openings share (copies of a `TempOpening` share theirs).
pub(crate) type SharedMesh = Rc<RefCell<TempMesh>>;

impl TempMesh {
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.vertcnt.is_empty()
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.vertcnt.clear();
    }

    pub fn append(&mut self, other: &TempMesh) {
        self.verts.extend_from_slice(&other.verts);
        self.vertcnt.extend_from_slice(&other.vertcnt);
    }

    pub fn transform(&mut self, matrix: &Matrix4) {
        for vertex in &mut self.verts {
            *vertex = matrix.transform(*vertex);
        }
    }

    /// `ComputePolygonNormal`: Newell's normal of a polygon (normalized if
    /// asked).
    pub fn polygon_normal(vertices: &[Vector3], normalize: bool) -> Vector3 {
        let normal = newell_normal(vertices);
        if normalize { normal.normalize() } else { normal }
    }

    /// `ComputePolygonNormals`: every polygon's Newell normal from `offset`
    /// on (an empty polygon gets a zero one); the counts must stay inside
    /// the vertices.
    pub fn polygon_normals(&self, normalize: bool, offset: usize) -> Outcome<Vec<Vector3>> {
        let mut normals = Vec::with_capacity(self.vertcnt.len().saturating_sub(offset));
        let mut start: usize = self.vertcnt[..offset.min(self.vertcnt.len())].iter().map(|&count| count as usize).sum();
        for &count in self.vertcnt.iter().skip(offset) {
            let count = count as usize;
            if count == 0 {
                normals.push(Vector3::default());
                continue;
            }
            let Some(polygon) = self.verts.get(start..start + count) else {
                return Err(Failure::undefined("an IFC polygon's normal read past its vertices"));
            };
            normals.push(newell_normal(polygon));
            start += count;
        }
        if normalize {
            for normal in &mut normals {
                *normal = normal.normalize();
            }
        }
        Ok(normals)
    }

    /// `RemoveDegenerates`: polygons whose Newell normal (twice the area)
    /// is under 1e-10 squared go.
    pub fn remove_degenerates(&mut self) -> Outcome<()> {
        let normals = self.polygon_normals(false, 0)?;
        let mut kept_verts = Vec::with_capacity(self.verts.len());
        let mut kept_counts = Vec::with_capacity(self.vertcnt.len());
        let mut start = 0usize;
        for (index, &count) in self.vertcnt.iter().enumerate() {
            let end = start + count as usize;
            // Dropped below the limit: a NaN normal keeps its polygon.
            let area = normals[index].square_length();
            if area.is_nan() || area >= 1e-10f32 as f64 {
                kept_verts.extend_from_slice(&self.verts[start..end]);
                kept_counts.push(count);
            }
            start = end;
        }
        kept_verts.extend_from_slice(&self.verts[start.min(self.verts.len())..]);
        self.verts = kept_verts;
        self.vertcnt = kept_counts;
        Ok(())
    }

    /// `RemoveAdjacentDuplicates`: in every polygon of 2 or more vertices,
    /// repeats closer than its bounding box diagonal squared / 1e9 go
    /// (`std::unique`), then a last vertex equal to the first.
    pub fn remove_adjacent_duplicates(&mut self) -> Outcome<()> {
        let mut base = 0usize;
        for index in 0..self.vertcnt.len() {
            let count = self.vertcnt[index] as usize;
            if count < 2 {
                base += count;
                continue;
            }
            let Some(polygon) = self.verts.get(base..base + count) else {
                return Err(Failure::undefined("an IFC polygon's vertices read past the mesh"));
            };
            let (min, max) = array_bounds(polygon);
            let epsilon = (max - min).square_length() / 1e9;
            let close = |a: Vector3, b: Vector3| ((a - b).square_length()).abs() < epsilon;
            let mut kept = unique(polygon, close);
            let mut new_count = kept.len();
            if new_count > 1 && close(kept[0], kept[new_count - 1]) {
                kept.pop();
                new_count -= 1;
            }
            self.verts.splice(base..base + count, kept);
            self.vertcnt[index] = new_count as u32;
            base += new_count;
        }
        Ok(())
    }

    /// `ToMesh`: the vertices in single precision and a face per non-empty
    /// polygon, numbered on (none without vertices).
    pub fn to_mesh(&self) -> Outcome<Option<scene::Mesh>> {
        let counted: u64 = self.vertcnt.iter().map(|&count| count as u64).sum();
        Failure::assertion(self.verts.len() as u64 == counted, "mVerts.size() == std::accumulate(mVertcnt.begin(),mVertcnt.end(),size_t(0))")?;
        if self.verts.is_empty() {
            return Ok(None);
        }
        let vertices: Vec<[f32; 3]> = self.verts.iter().map(|vertex| vertex.to_f32()).collect();
        let mut faces = Vec::with_capacity(self.vertcnt.len());
        let mut next = 0u32;
        for &count in &self.vertcnt {
            if count == 0 {
                continue;
            }
            let face: Vec<u32> = (next..next + count).collect();
            next += count;
            faces.push(face);
        }
        let mesh = scene::Mesh {
            vertex_count: vertices.len(),
            vertices: Some(vertices),
            faces: Some(faces),
            ..scene::Mesh::default()
        };
        Ok(Some(mesh))
    }
}

/// `std::unique` with a predicate: each element compared with the last
/// one kept.
pub(crate) fn unique(items: &[Vector3], same: impl Fn(Vector3, Vector3) -> bool) -> Vec<Vector3> {
    let mut kept: Vec<Vector3> = Vec::with_capacity(items.len());
    for &item in items {
        if let Some(&last) = kept.last()
            && same(last, item)
        {
            continue;
        }
        kept.push(item);
    }
    kept
}

/// `NewellNormal` (`PolyTools`): the polygon's normal by Newell's method,
/// the first two vertices repeated after the last; the sums start at
/// vertex 1 (`x[i+1]·(y[i+2] - y[i])`).
pub(crate) fn newell_normal(vertices: &[Vector3]) -> Vector3 {
    let count = vertices.len();
    if count == 0 {
        return Vector3::default();
    }
    let get = |index: usize| vertices[index % count];
    let mut sum_xy = 0.0f64;
    let mut sum_yz = 0.0f64;
    let mut sum_zx = 0.0f64;
    for index in 0..count {
        let low = get(index);
        let middle = get(index + 1);
        let high = get(index + 2);
        sum_xy += middle.x * (high.y - low.y);
        sum_yz += middle.y * (high.z - low.z);
        sum_zx += middle.z * (high.x - low.x);
    }
    Vector3::new(sum_yz, sum_zx, sum_xy)
}

/// One opening to cut into the walls of the element it belongs to.
#[derive(Clone, Debug)]
pub(crate) struct TempOpening {
    pub extrusion_dir: Vector3,
    pub profile_mesh: Option<SharedMesh>,
    pub profile_mesh_2d: Option<SharedMesh>,
    /// Points made for the opening on one side of the wall, for the other
    /// side to join up to.
    pub wall_points: Vec<Vector3>,
}

impl TempOpening {
    pub fn new(extrusion_dir: Vector3, profile_mesh: Option<SharedMesh>, profile_mesh_2d: Option<SharedMesh>) -> Self {
        TempOpening { extrusion_dir, profile_mesh, profile_mesh_2d, wall_points: Vec::new() }
    }

    /// `Transform`: both meshes (shared with the copies) and the
    /// direction by the matrix's 3x3 part.
    pub fn transform(&mut self, matrix: &Matrix4) {
        if let Some(mesh) = &self.profile_mesh {
            mesh.borrow_mut().transform(matrix);
        }
        if let Some(mesh) = &self.profile_mesh_2d {
            mesh.borrow_mut().transform(matrix);
        }
        self.extrusion_dir = matrix.matrix3().transform(self.extrusion_dir);
    }
}

/// `IsTrue`: "TRUE" or "T".
pub(crate) fn is_true(value: &[u8]) -> bool {
    value == b"TRUE" || value == b"T"
}

/// `ConvertSIPrefix`: the prefix's factor (float constants), 1 for an
/// unknown one ("DECA" is 1 too).
pub(crate) fn si_prefix(prefix: &[u8]) -> f64 {
    let factor: f32 = match prefix {
        b"EXA" => 1e18,
        b"PETA" => 1e15,
        b"TERA" => 1e12,
        b"GIGA" => 1e9,
        b"MEGA" => 1e6,
        b"KILO" => 1e3,
        b"HECTO" => 1e2,
        b"DECA" => 1e-0,
        b"DECI" => 1e-1,
        b"CENTI" => 1e-2,
        b"MILLI" => 1e-3,
        b"MICRO" => 1e-6,
        b"NANO" => 1e-9,
        b"PICO" => 1e-12,
        b"FEMTO" => 1e-15,
        b"ATTO" => 1e-18,
        _ => 1.0,
    };
    factor as f64
}

/// `ConvertCartesianPoint`: the coordinates into x, y, z (a fourth and
/// later one lands in x again).
pub(crate) fn convert_cartesian_point(point: &Object) -> Vector3 {
    let mut out = Vector3::default();
    for (index, coordinate) in point.list(IfcCartesianPoint::COORDINATES).iter().enumerate() {
        out.set(index, coordinate.real());
    }
    out
}

/// `ConvertDirection`: the ratios, normalized unless shorter than
/// `ai_epsilon`.
pub(crate) fn convert_direction(direction: &Object) -> Vector3 {
    let mut out = Vector3::default();
    for (index, ratio) in direction.list(IfcDirection::DIRECTION_RATIOS).iter().enumerate() {
        out.set(index, ratio.real());
    }
    let length = out.length();
    if length < EPSILON {
        return out;
    }
    out.divided(length)
}

/// `ConvertVector`: the orientation times the magnitude.
pub(crate) fn convert_vector(db: &Database, vector: &Object) -> Outcome<Vector3> {
    let orientation = db.to(vector.entity(IfcVector::ORIENTATION), Class::IfcDirection)?;
    let direction = convert_direction(&orientation);
    let magnitude = vector.real(IfcVector::MAGNITUDE);
    Ok(Vector3::new(direction.x * magnitude, direction.y * magnitude, direction.z * magnitude))
}

/// `AssignMatrixAxes`: the axes as the first three columns.
fn assign_axes(out: &mut Matrix4, x: Vector3, y: Vector3, z: Vector3) {
    out.0[0] = x.x;
    out.0[4] = x.y;
    out.0[8] = x.z;
    out.0[1] = y.x;
    out.0[5] = y.y;
    out.0[9] = y.z;
    out.0[2] = z.x;
    out.0[6] = z.y;
    out.0[10] = z.z;
}

/// The point an `IfcPlacement`'s location names.
fn placement_location(db: &Database, placement: &Object) -> Outcome<Vector3> {
    let location = db.to(placement.entity(IfcPlacement::LOCATION), Class::IfcCartesianPoint)?;
    Ok(convert_cartesian_point(&location))
}

/// `ConvertAxisPlacement(IfcAxis2Placement3D)`: z from the axis, x from
/// the reference direction made orthogonal to it.
pub(crate) fn convert_axis_placement_3d(db: &Database, placement: &Object) -> Outcome<Matrix4> {
    let location = placement_location(db, placement)?;
    let mut z = Vector3::new(0.0, 0.0, 1.0);
    let mut r = Vector3::new(1.0, 0.0, 0.0);
    if placement.has(IfcAxis2Placement3D::AXIS) {
        let axis = db.to(placement.entity(IfcAxis2Placement3D::AXIS), Class::IfcDirection)?;
        z = convert_direction(&axis);
    }
    if placement.has(IfcAxis2Placement3D::REF_DIRECTION) {
        let reference = db.to(placement.entity(IfcAxis2Placement3D::REF_DIRECTION), Class::IfcDirection)?;
        r = convert_direction(&reference);
    }
    let v = r.normalize();
    let along = z * dot(v, z);
    let x = (v - along).normalize();
    let y = cross(z, x);
    let mut out = Matrix4::translation(location);
    assign_axes(&mut out, x, y, z);
    Ok(out)
}

/// `ConvertAxisPlacement(IfcAxis2Placement2D)`.
pub(crate) fn convert_axis_placement_2d(db: &Database, placement: &Object) -> Outcome<Matrix4> {
    let location = placement_location(db, placement)?;
    let mut x = Vector3::new(1.0, 0.0, 0.0);
    if placement.has(IfcAxis2Placement2D::REF_DIRECTION) {
        let reference = db.to(placement.entity(IfcAxis2Placement2D::REF_DIRECTION), Class::IfcDirection)?;
        x = convert_direction(&reference);
    }
    let y = Vector3::new(x.y, -x.x, 0.0);
    let mut out = Matrix4::translation(location);
    assign_axes(&mut out, x, y, Vector3::new(0.0, 0.0, 1.0));
    Ok(out)
}

/// `ConvertAxisPlacement(IfcAxis1Placement)`: the axis (z if none) and
/// the position.
pub(crate) fn convert_axis_placement_1d(db: &Database, placement: &Object) -> Outcome<(Vector3, Vector3)> {
    let position = placement_location(db, placement)?;
    let mut axis = Vector3::new(0.0, 0.0, 1.0);
    if placement.has(IfcAxis1Placement::AXIS) {
        let direction = db.to(placement.entity(IfcAxis1Placement::AXIS), Class::IfcDirection)?;
        axis = convert_direction(&direction);
    }
    Ok((axis, position))
}

/// `ConvertAxisPlacement(IfcAxis2Placement)`: a select of a 3D or a 2D
/// placement; anything else leaves `out` as it was.
pub(crate) fn convert_axis_placement_select(db: &Database, select: &Option<Rc<Value>>, out: &mut Matrix4) -> Outcome<()> {
    if let Some(placement) = db.resolve_select_ptr(select, Class::IfcAxis2Placement3D)? {
        *out = convert_axis_placement_3d(db, &placement)?;
    } else if let Some(placement) = db.resolve_select_ptr(select, Class::IfcAxis2Placement2D)? {
        *out = convert_axis_placement_2d(db, &placement)?;
    }
    Ok(())
}

/// `ConvertTransformOperator`: translation, the axes (3D: the third
/// too), then the scale (non-uniform: per axis).
pub(crate) fn convert_transform_operator(db: &Database, operator: &Object) -> Outcome<Matrix4> {
    let origin = db.to(operator.entity(IfcCartesianTransformationOperator::LOCAL_ORIGIN), Class::IfcCartesianPoint)?;
    let location = convert_cartesian_point(&origin);
    let mut x = Vector3::new(1.0, 0.0, 0.0);
    let mut y = Vector3::new(0.0, 1.0, 0.0);
    let mut z = Vector3::new(0.0, 0.0, 1.0);
    if operator.has(IfcCartesianTransformationOperator::AXIS1) {
        let axis = db.to(operator.entity(IfcCartesianTransformationOperator::AXIS1), Class::IfcDirection)?;
        x = convert_direction(&axis);
    }
    if operator.has(IfcCartesianTransformationOperator::AXIS2) {
        let axis = db.to(operator.entity(IfcCartesianTransformationOperator::AXIS2), Class::IfcDirection)?;
        y = convert_direction(&axis);
    }
    if operator.is_a(Class::IfcCartesianTransformationOperator3D) && operator.has(IfcCartesianTransformationOperator3D::AXIS3) {
        let axis = db.to(operator.entity(IfcCartesianTransformationOperator3D::AXIS3), Class::IfcDirection)?;
        z = convert_direction(&axis);
    }
    let translation = Matrix4::translation(location);
    let mut axes = Matrix4::IDENTITY;
    assign_axes(&mut axes, x, y, z);
    let scale = operator.real(IfcCartesianTransformationOperator::SCALE);
    let has_scale = operator.has(IfcCartesianTransformationOperator::SCALE);
    let vscale = if operator.is_a(Class::IfcCartesianTransformationOperator3DnonUniform) {
        let scale2 = IfcCartesianTransformationOperator3DnonUniform::SCALE2;
        let scale3 = IfcCartesianTransformationOperator3DnonUniform::SCALE3;
        Vector3::new(
            if has_scale { scale } else { 1.0 },
            if operator.has(scale2) { operator.real(scale2) } else { 1.0 },
            if operator.has(scale3) { operator.real(scale3) } else { 1.0 },
        )
    } else {
        let uniform = if has_scale { scale } else { 1.0 };
        Vector3::new(uniform, uniform, uniform)
    };
    let scaling = Matrix4::scaling(vscale);
    let combined = translation.product(&axes);
    Ok(combined.product(&scaling))
}
