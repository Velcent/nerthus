//! `IFCCurve`: curves sampled into points. Circles and ellipses (by the
//! angle unit's scale), lines, polylines (parameter = segment index),
//! trimmed curves (by a parameter or a point found on the basis curve)
//! and composite curves (each segment sampled whole, reversed when its
//! sense disagrees).

use std::rc::Rc;

use super::math::{Vector3, std_max, std_min};
use super::schema::Class;
use super::schema::fields::{
    IfcCircle, IfcCompositeCurve, IfcCompositeCurveSegment, IfcConic, IfcEllipse, IfcLine, IfcPolyline, IfcTrimmedCurve,
};
use super::step::Object;
use super::util::{TempMesh, convert_axis_placement_select, convert_cartesian_point, convert_vector, is_true};
use super::{COMPOSITE_CURVE_FRAME, CURVE_FRAME, Conversion, Failure, Outcome, TRIMMED_CURVE_FRAME};
use crate::cast::to_u64;
use crate::scene::check_allocation;

/// `sizeof(IfcVector3)`.
const VECTOR_SIZE: u64 = 24;

/// A circle's or an ellipse's placement: centre and axes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Conic {
    location: Vector3,
    axes: [Vector3; 2],
    angle_scale: f64,
    conic_sampling_angle: f32,
}

/// A converted `IfcCurve`.
#[derive(Debug)]
pub(super) enum Curve {
    Circle { conic: Conic, radius: f64 },
    Ellipse { conic: Conic, semi_axis1: f64, semi_axis2: f64 },
    Line { point: Vector3, vector: Vector3 },
    Composite { curves: Vec<(Rc<Curve>, bool)>, total: f64 },
    Trimmed { base: Rc<Curve>, range: (f64, f64), maxval: f64, agree_sense: bool },
    PolyLine { points: Vec<Vector3> },
}

impl Curve {
    /// `Curve::Convert`: polylines, trimmed and composite curves, circles,
    /// ellipses and lines; none for any other curve.
    pub fn convert(conversion: &Conversion, curve: &Object) -> Outcome<Option<Rc<Curve>>> {
        conversion.enter(CURVE_FRAME, "IFC curves")?;
        let converted = Self::convert_kind(conversion, curve);
        conversion.leave(CURVE_FRAME);
        converted
    }

    /// The part of `Curve::Convert` after its frame.
    fn convert_kind(conversion: &Conversion, curve: &Object) -> Outcome<Option<Rc<Curve>>> {
        if curve.is_a(Class::IfcBoundedCurve) {
            if curve.is_a(Class::IfcPolyline) {
                return Ok(Some(Rc::new(polyline(conversion, curve)?)));
            }
            if curve.is_a(Class::IfcTrimmedCurve) {
                conversion.enter(TRIMMED_CURVE_FRAME, "IFC curves")?;
                let trimmed = trimmed(conversion, curve);
                conversion.leave(TRIMMED_CURVE_FRAME);
                return Ok(Some(Rc::new(trimmed?)));
            }
            if curve.is_a(Class::IfcCompositeCurve) {
                conversion.enter(COMPOSITE_CURVE_FRAME, "IFC curves")?;
                let composite = composite(conversion, curve);
                conversion.leave(COMPOSITE_CURVE_FRAME);
                return Ok(Some(Rc::new(composite?)));
            }
        }
        if curve.is_a(Class::IfcConic) {
            if curve.is_a(Class::IfcCircle) {
                let conic = conic(conversion, curve)?;
                return Ok(Some(Rc::new(Curve::Circle { conic, radius: curve.real(IfcCircle::RADIUS) })));
            }
            if curve.is_a(Class::IfcEllipse) {
                let conic = conic(conversion, curve)?;
                let semi_axis1 = curve.real(IfcEllipse::SEMI_AXIS1);
                let semi_axis2 = curve.real(IfcEllipse::SEMI_AXIS2);
                return Ok(Some(Rc::new(Curve::Ellipse { conic, semi_axis1, semi_axis2 })));
            }
        }
        if curve.is_a(Class::IfcLine) {
            let db = conversion.db;
            let point = db.to(curve.entity(IfcLine::PNT), Class::IfcCartesianPoint)?;
            let vector = db.to(curve.entity(IfcLine::DIR), Class::IfcVector)?;
            let point = convert_cartesian_point(&point);
            let vector = convert_vector(db, &vector)?;
            return Ok(Some(Rc::new(Curve::Line { point, vector })));
        }
        Ok(None)
    }

    /// Whether it is a `BoundedCurve`.
    pub fn is_bounded(&self) -> bool {
        matches!(self, Curve::Composite { .. } | Curve::Trimmed { .. } | Curve::PolyLine { .. })
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Curve::Circle { .. } | Curve::Ellipse { .. })
    }

    /// `GetParametricRange`.
    pub fn parametric_range(&self) -> (f64, f64) {
        match self {
            Curve::Circle { conic, .. } | Curve::Ellipse { conic, .. } => (0.0, std::f64::consts::PI * 2.0 / conic.angle_scale),
            Curve::Line { .. } => (f64::NEG_INFINITY, f64::INFINITY),
            Curve::Composite { total, .. } => (0.0, *total),
            Curve::Trimmed { maxval, .. } => (0.0, *maxval),
            Curve::PolyLine { points } => (0.0, points.len().wrapping_sub(1) as f64),
        }
    }

    /// `GetParametricRangeDelta`.
    pub fn parametric_range_delta(&self) -> f64 {
        let (first, second) = self.parametric_range();
        (second - first).abs()
    }

    /// `InRange` (a debug build's check, which Gaea's has): within the
    /// parametric range give or take a float epsilon; anything on a closed
    /// curve.
    fn in_range(&self, u: f64) -> bool {
        if self.is_closed() {
            return true;
        }
        let (first, second) = self.parametric_range();
        let epsilon = f32::EPSILON as f64;
        u - first > -epsilon && second - u > -epsilon
    }

    /// The `ai_assert`s every sampling starts with.
    fn assert_in_range(&self, a: f64, b: f64) -> Outcome<()> {
        Failure::assertion(self.in_range(a), "InRange( a )")?;
        Failure::assertion(self.in_range(b), "InRange( b )")
    }

    /// `TrimmedCurve::TrimParam`.
    fn trim_param(range: (f64, f64), agree_sense: bool, f: f64) -> f64 {
        if agree_sense { f + range.0 } else { range.1 - f }
    }

    /// `Eval`.
    pub fn eval(&self, u: f64) -> Outcome<Vector3> {
        match self {
            Curve::Circle { conic, radius } => {
                let u = -conic.angle_scale * u;
                let cosine = u.cos() * conic.axes[0];
                let sine = u.sin() * conic.axes[1];
                Ok(conic.location + *radius * (cosine + sine))
            }
            Curve::Ellipse { conic, semi_axis1, semi_axis2 } => {
                let u = -conic.angle_scale * u;
                let first = (semi_axis1 * u.cos()) * conic.axes[0];
                let second = (semi_axis2 * u.sin()) * conic.axes[1];
                Ok(conic.location + first + second)
            }
            Curve::Line { point, vector } => Ok(*point + u * *vector),
            Curve::Composite { curves, .. } => {
                if curves.is_empty() {
                    return Ok(Vector3::default());
                }
                let mut accumulated = 0.0;
                for (curve, sense) in curves {
                    let range = curve.parametric_range();
                    let delta = (range.1 - range.0).abs();
                    if u < accumulated + delta {
                        let parameter = if *sense { (u - accumulated) + range.0 } else { range.1 - (u - accumulated) };
                        return curve.eval(parameter);
                    }
                    accumulated += delta;
                }
                let (last, _) = curves.last().expect("checked not empty");
                last.eval(last.parametric_range().1)
            }
            Curve::Trimmed { base, range, agree_sense, .. } => {
                Failure::assertion(self.in_range(u), "InRange(p)")?;
                base.eval(Self::trim_param(*range, *agree_sense, u))
            }
            Curve::PolyLine { points } => {
                Failure::assertion(self.in_range(u), "InRange(p)")?;
                let segment = to_u64(u.floor()) as usize;
                if segment == points.len().wrapping_sub(1) {
                    return match points.last() {
                        Some(&last) => Ok(last),
                        None => Err(Failure::undefined("the last point of an empty IFC polyline read")),
                    };
                }
                let (Some(&start), Some(&end)) = (points.get(segment), points.get(segment.wrapping_add(1))) else {
                    return Err(Failure::undefined("an IFC polyline evaluated past its points"));
                };
                let d = u - segment as f64;
                Ok(end * d + start * (1.0 - d))
            }
        }
    }

    /// `EstimateSampleCount`.
    pub fn estimate_sample_count(&self, a: f64, b: f64) -> Outcome<u64> {
        self.assert_in_range(a, b)?;
        let count = match self {
            Curve::Circle { conic, .. } | Curve::Ellipse { conic, .. } => {
                let a = (a * conic.angle_scale) % (std::f64::consts::PI * 2.0);
                let b = (b * conic.angle_scale) % (std::f64::consts::PI * 2.0);
                let setting = std::f64::consts::PI * conic.conic_sampling_angle as f64 / 180.0;
                to_u64((b - a).abs().ceil() / setting)
            }
            Curve::Line { .. } => {
                if a == b {
                    1
                } else {
                    2
                }
            }
            Curve::Composite { curves, .. } => {
                let mut count = 0u64;
                let mut accumulated = 0.0;
                for (curve, sense) in curves {
                    let range = curve.parametric_range();
                    let delta = (range.1 - range.0).abs();
                    if a <= accumulated + delta && b >= accumulated {
                        let at = std_max(0.0, a - accumulated);
                        let bt = std_min(delta, b - accumulated);
                        let (start, end) = if *sense { (at + range.0, bt + range.0) } else { (range.1 - bt, range.1 - at) };
                        count = count.wrapping_add(curve.estimate_sample_count(start, end)?);
                    }
                    accumulated += delta;
                }
                count
            }
            Curve::Trimmed { base, range, agree_sense, .. } => {
                base.estimate_sample_count(Self::trim_param(*range, *agree_sense, a), Self::trim_param(*range, *agree_sense, b))?
            }
            Curve::PolyLine { .. } => to_u64(b.ceil() - a.floor()),
        };
        Ok(count)
    }

    /// `SampleDiscrete(out, a, b)`: points of the curve between the two
    /// parameters (a composite curve samples each segment whole).
    pub fn sample_discrete(&self, conversion: &Conversion, out: &mut TempMesh, a: f64, b: f64) -> Outcome<()> {
        self.assert_in_range(a, b)?;
        match self {
            Curve::Line { .. } => {
                out.verts.push(self.eval(a)?);
                if a != b {
                    out.verts.push(self.eval(b)?);
                }
                Ok(())
            }
            Curve::Composite { curves, .. } => {
                let count = self.estimate_sample_count(a, b)?;
                check_allocation((out.verts.len() as u64).saturating_add(count), VECTOR_SIZE)?;
                for (curve, sense) in curves {
                    let start = out.verts.len();
                    curve.sample_bounded(conversion, out)?;
                    if !sense && start != out.verts.len() {
                        out.verts[start..].reverse();
                    }
                }
                Ok(())
            }
            Curve::Trimmed { base, range, agree_sense, .. } => {
                let start = Self::trim_param(*range, *agree_sense, a);
                let end = Self::trim_param(*range, *agree_sense, b);
                base.sample_discrete(conversion, out, start, end)
            }
            _ => {
                let count = self.estimate_sample_count(a, b)?;
                let total = (out.verts.len() as u64).saturating_add(count).saturating_add(1);
                check_allocation(total, VECTOR_SIZE)?;
                let delta = (b - a) / count as f64;
                let mut p = a;
                for _ in 0..=count {
                    out.verts.push(self.eval(p)?);
                    p += delta;
                }
                Ok(())
            }
        }
    }

    /// `BoundedCurve::SampleDiscrete(out)`: over the whole range.
    pub fn sample_bounded(&self, conversion: &Conversion, out: &mut TempMesh) -> Outcome<()> {
        let (first, second) = self.parametric_range();
        Failure::assertion(first != f64::INFINITY, "range.first != std::numeric_limits<IfcFloat>::infinity()")?;
        Failure::assertion(second != f64::INFINITY, "range.second != std::numeric_limits<IfcFloat>::infinity()")?;
        self.sample_discrete(conversion, out, first, second)
    }

    /// `Curve::ReverseEval`: the parameter of the point nearest the value
    /// (16 samples, refined 15 times or to 1e-4).
    fn reverse_eval(&self, value: Vector3) -> Outcome<f64> {
        let (first, second) = self.parametric_range();
        recursive_search(self, value, first, second, 16, 1e-4f32 as f64, 0, 15)
    }
}

/// `RecursiveSearch`: the two samples nearest the value bracket the next
/// search (across the seam on a closed curve).
#[allow(clippy::too_many_arguments)]
fn recursive_search(curve: &Curve, value: Vector3, a: f64, b: f64, samples: u32, threshold: f64, recurse: u32, max_recurse: u32) -> Outcome<f64> {
    let delta = (b - a) / samples as f64;
    let mut min_point = [a, b];
    let mut min_difference = [f64::INFINITY, f64::INFINITY];
    let mut runner = a;
    for _ in 0..samples {
        let difference = (curve.eval(runner)? - value).square_length();
        if difference < min_difference[0] {
            min_difference[1] = min_difference[0];
            min_point[1] = min_point[0];
            min_difference[0] = difference;
            min_point[0] = runner;
        } else if difference < min_difference[1] {
            min_difference[1] = difference;
            min_point[1] = runner;
        }
        runner += delta;
    }
    Failure::assertion(min_difference[0] != f64::INFINITY, "min_diff[ 0 ] != inf")?;
    Failure::assertion(min_difference[1] != f64::INFINITY, "min_diff[ 1 ] != inf")?;
    if (a - min_point[0]).abs() < threshold || recurse >= max_recurse {
        return Ok(min_point[0]);
    }
    if curve.is_closed() && (min_point[0] - min_point[1]).abs() > curve.parametric_range_delta() * 0.5 {
        let range = curve.parametric_range();
        let wrap_difference = (curve.eval(range.0)? - value).square_length();
        if wrap_difference < min_difference[0] {
            let t = min_point[0];
            min_point[0] = if min_point[1] > min_point[0] { range.0 } else { range.1 };
            min_point[1] = t;
        }
    }
    recursive_search(curve, value, min_point[0], min_point[1], samples, threshold, recurse + 1, max_recurse)
}

/// The `Conic` constructor: the placement's translation and axes.
fn conic(conversion: &Conversion, curve: &Object) -> Outcome<Conic> {
    let mut trafo = super::math::Matrix4::IDENTITY;
    convert_axis_placement_select(conversion.db, &curve.select(IfcConic::POSITION), &mut trafo)?;
    let m = trafo.0;
    Ok(Conic {
        location: Vector3::new(m[3], m[7], m[11]),
        axes: [Vector3::new(m[0], m[4], m[8]), Vector3::new(m[1], m[5], m[9])],
        angle_scale: conversion.angle_scale,
        conic_sampling_angle: conversion.settings.conic_sampling_angle,
    })
}

fn polyline(conversion: &Conversion, curve: &Object) -> Outcome<Curve> {
    let mut points = Vec::new();
    for point in curve.list(IfcPolyline::POINTS) {
        let point = conversion.db.to(point.entity(), Class::IfcCartesianPoint)?;
        points.push(convert_cartesian_point(&point));
    }
    Ok(Curve::PolyLine { points })
}

/// The `CompositeCurve` constructor: its bounded segments (others are
/// left out); none is a `CurveError`.
fn composite(conversion: &Conversion, curve: &Object) -> Outcome<Curve> {
    let db = conversion.db;
    let mut curves = Vec::new();
    let mut total = 0.0f64;
    for segment in curve.list(IfcCompositeCurve::SEGMENTS).to_vec() {
        let segment = db.to(segment.entity(), Class::IfcCompositeCurveSegment)?;
        let parent = db.to(segment.entity(IfcCompositeCurveSegment::PARENT_CURVE), Class::IfcCurve)?;
        let Some(converted) = Curve::convert(conversion, &parent)? else {
            continue;
        };
        if !converted.is_bounded() {
            continue;
        }
        let sense = is_true(segment.text(IfcCompositeCurveSegment::SAME_SENSE));
        total += converted.parametric_range_delta();
        curves.push((converted, sense));
    }
    if curves.is_empty() {
        return Err(Failure::Curve);
    }
    Ok(Curve::Composite { curves, total })
}

/// A trim's parameter: the first REAL, else the last point found on the
/// basis curve; neither is a `CurveError`.
fn trim(conversion: &Conversion, trims: &[super::step::Field], base: Option<&Rc<Curve>>) -> Outcome<f64> {
    let db = conversion.db;
    let mut point = None;
    for selected in trims {
        let select = selected.select();
        if let Some(value) = select.as_ref().and_then(|value| value.as_real()) {
            return Ok(value);
        }
        if let Some(cartesian) = db.resolve_select_ptr(&select, Class::IfcCartesianPoint)? {
            point = Some(convert_cartesian_point(&cartesian));
        }
    }
    let Some(point) = point else {
        return Err(Failure::Curve);
    };
    let Some(base) = base else {
        return Err(Failure::undefined("a point found on an IFC trimmed curve's missing basis curve"));
    };
    base.reverse_eval(point)
}

/// The `TrimmedCurve` constructor: both trims, swapped when the sense
/// disagrees, the second moved a turn on when a closed curve wraps.
fn trimmed(conversion: &Conversion, curve: &Object) -> Outcome<Curve> {
    let db = conversion.db;
    let basis = db.to(curve.entity(IfcTrimmedCurve::BASIS_CURVE), Class::IfcCurve)?;
    let base = Curve::convert(conversion, &basis)?;
    let first = trim(conversion, curve.list(IfcTrimmedCurve::TRIM1), base.as_ref())?;
    let second = trim(conversion, curve.list(IfcTrimmedCurve::TRIM2), base.as_ref())?;
    let mut range = (first, second);
    let agree_sense = is_true(curve.text(IfcTrimmedCurve::SENSE_AGREEMENT));
    if !agree_sense {
        range = (range.1, range.0);
    }
    let Some(base) = base else {
        return Err(Failure::undefined("an IFC trimmed curve's missing basis curve asked whether it is closed"));
    };
    if base.is_closed() && range.0 > range.1 {
        range.1 += base.parametric_range_delta();
    }
    let maxval = range.1 - range.0;
    Failure::assertion(maxval >= 0.0, "maxval >= 0")?;
    Ok(Curve::Trimmed { base, range, maxval, agree_sense })
}
