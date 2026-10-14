//! `IFCProfile`: profiles as polygons. Arbitrary profiles are their
//! (bounded) curves sampled; rectangles, circles (in tessellation
//! segments) and I-shapes are made and placed.

use super::curve::Curve;
use super::geometry::TWO_PI_F;
use super::math::Vector3;
use super::schema::Class;
use super::schema::fields::{
    IfcArbitraryClosedProfileDef, IfcArbitraryOpenProfileDef, IfcCircleProfileDef, IfcIShapeProfileDef, IfcParameterizedProfileDef,
    IfcRectangleProfileDef,
};
use super::step::Object;
use super::util::{TempMesh, convert_axis_placement_2d};
use super::{Conversion, Failure, Outcome};

/// `ProcessCurve`: a bounded curve sampled whole as one polygon. A
/// `CurveError` while sampling is caught (the points so far stay);
/// converting the curve isn't guarded.
pub(super) fn process_curve(conversion: &Conversion, curve: &Object, mesh: &mut TempMesh) -> Outcome<bool> {
    let Some(converted) = Curve::convert(conversion, curve)? else {
        return Ok(false);
    };
    if !converted.is_bounded() {
        return Ok(false);
    }
    match converted.sample_bounded(conversion, mesh) {
        Ok(()) => {}
        Err(Failure::Curve) => return Ok(false),
        Err(failure) => return Err(failure),
    }
    mesh.vertcnt.push(mesh.verts.len() as u32);
    Ok(true)
}

/// `ProcessParametrizedProfile`: the shape, then its placement (an
/// unknown shape is left as it is, unplaced).
fn process_parametrized_profile(conversion: &Conversion, profile: &Object, mesh: &mut TempMesh) -> Outcome<()> {
    if profile.is_a(Class::IfcRectangleProfileDef) {
        let x = profile.real(IfcRectangleProfileDef::X_DIM) * 0.5;
        let y = profile.real(IfcRectangleProfileDef::Y_DIM) * 0.5;
        mesh.verts.extend([Vector3::new(x, y, 0.0), Vector3::new(-x, y, 0.0), Vector3::new(-x, -y, 0.0), Vector3::new(x, -y, 0.0)]);
        mesh.vertcnt.push(4);
    } else if profile.is_a(Class::IfcCircleProfileDef) {
        let segments = conversion.settings.cylindrical_tessellation as usize;
        let delta = (TWO_PI_F / segments as f32) as f64;
        let radius = profile.real(IfcCircleProfileDef::RADIUS);
        let mut angle = 0.0f64;
        for _ in 0..segments {
            mesh.verts.push(Vector3::new(angle.cos() * radius, angle.sin() * radius, 0.0));
            angle += delta;
        }
        mesh.vertcnt.push(segments as u32);
    } else if profile.is_a(Class::IfcIShapeProfileDef) {
        let width = profile.real(IfcIShapeProfileDef::OVERALL_WIDTH);
        let depth = profile.real(IfcIShapeProfileDef::OVERALL_DEPTH);
        let web = profile.real(IfcIShapeProfileDef::WEB_THICKNESS);
        let flange = profile.real(IfcIShapeProfileDef::FLANGE_THICKNESS);
        let offset = (width - web) / 2.0;
        let inner_height = depth - flange * 2.0;
        mesh.verts.extend([
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, flange, 0.0),
            Vector3::new(offset, flange, 0.0),
            Vector3::new(offset, flange + inner_height, 0.0),
            Vector3::new(0.0, flange + inner_height, 0.0),
            Vector3::new(0.0, depth, 0.0),
            Vector3::new(width, depth, 0.0),
            Vector3::new(width, flange + inner_height, 0.0),
            Vector3::new(offset + web, flange + inner_height, 0.0),
            Vector3::new(offset + web, flange, 0.0),
            Vector3::new(width, flange, 0.0),
            Vector3::new(width, 0.0, 0.0),
        ]);
        mesh.vertcnt.push(12);
    } else {
        return Ok(());
    }
    let position = conversion.db.to(profile.entity(IfcParameterizedProfileDef::POSITION), Class::IfcAxis2Placement2D)?;
    let trafo = convert_axis_placement_2d(conversion.db, &position)?;
    mesh.transform(&trafo);
    Ok(())
}

/// `ProcessProfile`: the profile's polygon, adjacent duplicates removed;
/// whether it has one of 2 or more points.
pub(super) fn process_profile(conversion: &Conversion, profile: &Object, mesh: &mut TempMesh) -> Outcome<bool> {
    let db = conversion.db;
    if profile.is_a(Class::IfcArbitraryClosedProfileDef) {
        let curve = db.to(profile.entity(IfcArbitraryClosedProfileDef::OUTER_CURVE), Class::IfcCurve)?;
        process_curve(conversion, &curve, mesh)?;
    } else if profile.is_a(Class::IfcArbitraryOpenProfileDef) {
        let curve = db.to(profile.entity(IfcArbitraryOpenProfileDef::CURVE), Class::IfcBoundedCurve)?;
        process_curve(conversion, &curve, mesh)?;
    } else if profile.is_a(Class::IfcParameterizedProfileDef) {
        process_parametrized_profile(conversion, profile, mesh)?;
    } else {
        return Ok(false);
    }
    mesh.remove_adjacent_duplicates()?;
    Ok(mesh.vertcnt.first().is_some_and(|&count| count > 1))
}
