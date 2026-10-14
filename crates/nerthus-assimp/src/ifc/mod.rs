//! `IFCImporter`: IFC 2x3 in STEP files (`.ifc`, `.ifczip`, `.step`,
//! `.stp`). The project's site (or building) is walked through its
//! aggregation, containment and void relations; each product's preferred
//! representation becomes meshes (extrusions, revolutions, swept disks,
//! face sets, booleans), and openings found on the way are cut into the
//! walls they void.
//!
//! Every C++ exception ends the import except where a `catch` takes it:
//! `std::bad_cast` from a reference cast in a few places, `CurveError`
//! around sampling a profile curve, and any `std::exception` around the
//! triangulator.

mod boolean;
mod clipper;
mod curve;
mod encoding;
mod geometry;
mod material;
mod math;
mod openings;
mod poly2tri;
mod profile;
mod schema;
mod step;
mod util;

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use math::Matrix4;
use schema::Class;
use schema::fields::{
    IfcComplexProperty, IfcConversionBasedUnit, IfcGeometricRepresentationContext, IfcLocalPlacement, IfcMappedItem, IfcMeasureWithUnit, IfcNamedUnit,
    IfcProduct, IfcProductRepresentation, IfcProject, IfcProperty, IfcPropertyListValue, IfcPropertySet, IfcPropertySingleValue, IfcRelDecomposes,
    IfcRelContainedInSpatialStructure, IfcRelDefinesByProperties, IfcRelVoidsElement, IfcRepresentation, IfcRepresentationContext, IfcRepresentationMap,
    IfcSIUnit, IfcUnitAssignment,
};
use step::{Database, Object, Value};
use util::TempOpening;

use crate::matrix::Matrix4 as Matrix4F;
use crate::msvc::sort;
use crate::registry::FileInput;
use crate::scene::{self, ImportError, STACK_BUDGET, Scene};

/// Stack frames of the C++'s recursive functions as MSVC lays them out on
/// x64, in bytes per level: past the stack budget the recursion has
/// overflowed the native stack.
const SPATIAL_STRUCTURE_FRAME: u64 = 2384;
const PLACEMENT_FRAME: u64 = 1184;
/// `ConvertUnit` of a select and then of the named unit it gives.
const UNIT_FRAME: u64 = 64 + 608;
const RATE_FRAME: u64 = 144;
pub(super) const BOOLEAN_FRAME: u64 = 960;
/// `Curve::Convert`, and the trimmed and composite curve constructors it
/// calls.
pub(super) const CURVE_FRAME: u64 = 176;
pub(super) const TRIMMED_CURVE_FRAME: u64 = 256;
pub(super) const COMPOSITE_CURVE_FRAME: u64 = 1472;

/// How a C++ step ends when it doesn't return.
#[derive(Debug)]
pub(crate) enum Failure {
    /// `std::bad_cast` from a reference `dynamic_cast`.
    BadCast,
    /// `CurveError` (its message is only logged).
    Curve,
    /// Any other `std::exception` (`DeadlyImportError`, STEP's
    /// `TypeError` and `SyntaxError`, `logic_error`, poly2tri's
    /// `runtime_error`, Clipper's exception …).
    Std(String),
    /// What the process does that no `catch` sees: undefined behaviour,
    /// an endless loop.
    Process(ImportError),
}

pub(crate) type Outcome<T> = Result<T, Failure>;

impl Failure {
    pub fn undefined(what: &str) -> Failure {
        Failure::Process(ImportError(format!("undefined behaviour: {what}")))
    }

    pub fn endless(what: &str) -> Failure {
        Failure::Process(ImportError(format!("endless loop: {what}")))
    }

    /// An `ai_assert` (or `assert`), whose failure ends the process.
    pub fn assertion(condition: bool, expression: &str) -> Outcome<()> {
        scene::ai_assert(condition, expression).map_err(Failure::Process)
    }

    /// `TypeError(t.what(), id)`: a conversion's error for its entity.
    pub fn with_entity(self, id: u64) -> Failure {
        match self {
            Failure::Std(message) => Failure::Std(format!("(entity #{id}) {message}")),
            other => other,
        }
    }

    /// Whether a `catch (const std::exception&)` takes it.
    pub fn is_std_exception(&self) -> bool {
        matches!(self, Failure::BadCast | Failure::Std(_))
    }

    fn into_import_error(self) -> ImportError {
        match self {
            Failure::BadCast => ImportError("Bad dynamic_cast!".to_string()),
            Failure::Curve => ImportError("Unknown exception".to_string()),
            Failure::Std(message) => ImportError(message),
            Failure::Process(error) => error,
        }
    }
}

impl From<ImportError> for Failure {
    fn from(error: ImportError) -> Self {
        if error.ends_process() {
            return Failure::Process(error);
        }
        Failure::Std(error.0)
    }
}

/// The importer's settings as the editor leaves them (defaults).
pub(crate) struct Settings {
    pub skip_space_representations: bool,
    pub conic_sampling_angle: f32,
    pub cylindrical_tessellation: u32,
}

const SETTINGS: Settings = Settings { skip_space_representations: true, conic_sampling_angle: 10.0, cylindrical_tessellation: 32 };

/// A `ProcessSpatialStructure` call's opening lists: its `openings` and,
/// inside its void loop, `openings_local`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FrameSlot {
    Openings,
    Local,
}

/// Where `apply_openings`/`collect_openings` point. A frame's lists are
/// locals at fixed offsets in its stack frame: a pointer left behind
/// after the call returned names whatever list the next call at that
/// depth has there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ListRef {
    depth: usize,
    slot: FrameSlot,
}

/// The opening lists of a live `ProcessSpatialStructure` call.
struct FrameLists {
    openings: usize,
    local: Option<usize>,
}

/// `ConversionData`.
pub(crate) struct Conversion<'a> {
    pub db: &'a Database,
    pub settings: &'static Settings,
    pub len_scale: f64,
    pub angle_scale: f64,
    pub meshes: Vec<scene::Mesh>,
    /// The materials' names (`AI_MATKEY_NAME`), for the default lookup.
    pub materials: Vec<Vec<u8>>,
    /// `cached_meshes`: by representation item and material.
    pub cached_meshes: HashMap<(usize, u32), BTreeSet<u32>>,
    /// `cached_materials`: by surface style.
    pub cached_materials: HashMap<usize, u32>,
    pub already_processed: BTreeSet<u64>,
    pub apply_openings: Option<ListRef>,
    pub collect_openings: Option<ListRef>,
    lists: Vec<Option<Vec<TempOpening>>>,
    frames: Vec<FrameLists>,
    /// The native stack the recursion has used.
    stack: Cell<u64>,
}

impl<'a> Conversion<'a> {
    /// One level deeper in a C++ recursion, `frame` bytes of stack: past
    /// the budget the native stack has overflowed.
    pub fn enter(&self, frame: u64, what: &str) -> Outcome<()> {
        let used = self.stack.get() + frame;
        if used > STACK_BUDGET {
            return Err(Failure::undefined(&format!("{what} nested past the stack")));
        }
        self.stack.set(used);
        Ok(())
    }

    pub fn leave(&self, frame: u64) {
        self.stack.set(self.stack.get() - frame);
    }

    /// A new list owned by the caller until `drop_list`.
    pub fn new_list(&mut self) -> usize {
        self.lists.push(Some(Vec::new()));
        self.lists.len() - 1
    }

    pub fn drop_list(&mut self, list: usize) {
        self.lists[list] = None;
    }

    /// The arena list a pointer names now.
    fn resolve(&self, reference: ListRef) -> Outcome<usize> {
        let list = self.frames.get(reference.depth).and_then(|frame| match reference.slot {
            FrameSlot::Openings => Some(frame.openings),
            FrameSlot::Local => frame.local,
        });
        match list {
            Some(list) if self.lists[list].is_some() => Ok(list),
            _ => Err(Failure::undefined("an IFC opening list used after its call returned")),
        }
    }

    /// The list a pointer names.
    pub fn list(&self, reference: ListRef) -> Outcome<&Vec<TempOpening>> {
        let list = self.resolve(reference)?;
        Ok(self.lists[list].as_ref().expect("resolved lists are alive"))
    }

    pub fn list_mut(&mut self, reference: ListRef) -> Outcome<&mut Vec<TempOpening>> {
        let list = self.resolve(reference)?;
        Ok(self.lists[list].as_mut().expect("resolved lists are alive"))
    }

    /// Takes a list out to work on it with the conversion at hand; `put`
    /// returns it.
    pub fn take_list(&mut self, reference: ListRef) -> Outcome<(usize, Vec<TempOpening>)> {
        let list = self.resolve(reference)?;
        let openings = self.lists[list].take().expect("resolved lists are alive");
        Ok((list, openings))
    }

    pub fn put_list(&mut self, list: usize, openings: Vec<TempOpening>) {
        self.lists[list] = Some(openings);
    }
}

/// An output node as far as the openings need it: its transformation.
#[derive(Clone, Copy, Debug)]
struct Node {
    transformation: Matrix4F,
}

fn to_single(matrix: &Matrix4) -> Matrix4F {
    Matrix4F(matrix.0.map(|value| value as f32))
}

fn to_double(matrix: &Matrix4F) -> Matrix4 {
    Matrix4(matrix.0.map(|value| value as f64))
}

/// `BaseImporter::GetExtension`.
fn extension(path: &str) -> String {
    match path.rfind('.') {
        Some(dot) => path[dot + 1..].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// `IFCImporter::InternReadFile`: on a thread of its own, as the
/// conversion recurses as deep as the C++'s.
pub(crate) fn read(input: &FileInput) -> Result<Scene, ImportError> {
    let bytes = if extension(input.path) == "ifczip" { geometry::read_ifczip(input.bytes).map_err(Failure::into_import_error)? } else { input.bytes.to_vec() };
    let worker = std::thread::Builder::new().stack_size(CONVERSION_STACK);
    std::thread::scope(|scope| {
        let Ok(conversion) = worker.spawn_scoped(scope, || convert(&bytes)) else {
            return Err(ImportError("could not start a thread for the IFC conversion".to_string()));
        };
        match conversion.join() {
            Ok(result) => result.map_err(Failure::into_import_error),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

/// The conversion thread's stack (reserved, not committed).
const CONVERSION_STACK: usize = 64 << 20;

fn convert(bytes: &[u8]) -> Outcome<Scene> {
    let db = step::read(bytes)?;
    if !db.file_schema.starts_with(b"IFC2") {
        let schema = String::from_utf8_lossy(&db.file_schema);
        return Err(Failure::Std(format!("IFC: Unrecognized file schema: {schema}")));
    }
    let Some(&project_index) = db.tracked("ifcproject").first() else {
        return Err(Failure::Std("IFC: missing IfcProject entity".to_string()));
    };
    let project = db.object(project_index)?;
    if !project.is_a(Class::IfcProject) {
        return Err(Failure::BadCast);
    }
    let mut conversion = Conversion {
        db: &db,
        settings: &SETTINGS,
        len_scale: 1.0,
        angle_scale: -1.0,
        meshes: Vec::new(),
        materials: Vec::new(),
        cached_meshes: HashMap::new(),
        cached_materials: HashMap::new(),
        already_processed: BTreeSet::new(),
        apply_openings: None,
        collect_openings: None,
        lists: Vec::new(),
        frames: Vec::new(),
        stack: Cell::new(0),
    };
    set_units(&mut conversion, &project)?;
    set_coordinate_space(&conversion, &project)?;
    process_spatial_structures(&mut conversion, &project)?;
    Ok(Scene { meshes: conversion.meshes })
}

/// `SetUnits`: every unit of the project's assignment (none when it
/// isn't one; a missing one throws).
fn set_units(conversion: &mut Conversion, project: &Object) -> Outcome<()> {
    let db = conversion.db;
    let Some(assignment) = db.to_ptr(project.entity(IfcProject::UNITS_IN_CONTEXT), Class::IfcUnitAssignment)? else {
        return Ok(());
    };
    for unit in assignment.list(IfcUnitAssignment::UNITS) {
        convert_unit_select(conversion, unit.select())?;
    }
    Ok(())
}

/// `ConvertUnit(DataType)`: an entity naming a length or plane angle unit
/// (a `bad_cast` anywhere in it is caught and skips the unit).
fn convert_unit_select(conversion: &mut Conversion, value: Option<Rc<Value>>) -> Outcome<()> {
    conversion.enter(UNIT_FRAME, "IFC units")?;
    let result = (|| {
        let Some(value) = &value else {
            return Err(Failure::undefined("an unset IFC unit read"));
        };
        if value.as_entity().is_none() {
            return Err(Failure::BadCast);
        }
        let unit = conversion.db.resolve_select(&Some(value.clone()), Class::IfcNamedUnit)?;
        let unit_type = unit.text(IfcNamedUnit::UNIT_TYPE);
        if unit_type != b"LENGTHUNIT" && unit_type != b"PLANEANGLEUNIT" {
            return Ok(());
        }
        convert_named_unit(conversion, &unit)
    })();
    conversion.leave(UNIT_FRAME);
    match result {
        Err(Failure::BadCast) => Ok(()),
        other => other,
    }
}

/// `ConvertUnit(IfcNamedUnit)`: an SI length unit's prefix, or a
/// conversion-based plane angle unit's factor (and its own unit).
fn convert_named_unit(conversion: &mut Conversion, unit: &Object) -> Outcome<()> {
    if unit.is_a(Class::IfcSIUnit) {
        if unit.text(IfcNamedUnit::UNIT_TYPE) == b"LENGTHUNIT" {
            conversion.len_scale = if unit.has(IfcSIUnit::PREFIX) { util::si_prefix(unit.text(IfcSIUnit::PREFIX)) } else { 1.0 };
        }
        return Ok(());
    }
    if !unit.is_a(Class::IfcConversionBasedUnit) || unit.text(IfcNamedUnit::UNIT_TYPE) != b"PLANEANGLEUNIT" {
        return Ok(());
    }
    let result = (|| {
        let db = conversion.db;
        let factor = db.to(unit.entity(IfcConversionBasedUnit::CONVERSION_FACTOR), Class::IfcMeasureWithUnit)?;
        let Some(value) = factor.select(IfcMeasureWithUnit::VALUE_COMPONENT) else {
            return Err(Failure::undefined("an unset IFC unit factor read"));
        };
        let Some(scale) = value.as_real() else {
            return Err(Failure::BadCast);
        };
        conversion.angle_scale = scale;
        let unit_component = factor.select(IfcMeasureWithUnit::UNIT_COMPONENT);
        convert_unit_select(conversion, unit_component)
    })();
    match result {
        Err(Failure::BadCast) => Ok(()),
        other => other,
    }
}

/// `SetCoordinateSpace`: the "Model" context (else the last one) as the
/// world system. Only what it throws matters here.
fn set_coordinate_space(conversion: &Conversion, project: &Object) -> Outcome<()> {
    let db = conversion.db;
    let mut favourite = None;
    for context in project.list(IfcProject::REPRESENTATION_CONTEXTS) {
        let context = db.to(context.entity(), Class::IfcRepresentationContext)?;
        let is_model = context.has(IfcRepresentationContext::CONTEXT_TYPE) && context.text(IfcRepresentationContext::CONTEXT_TYPE) == b"Model";
        favourite = Some(context);
        if is_model {
            break;
        }
    }
    if let Some(context) = favourite
        && context.is_a(Class::IfcGeometricRepresentationContext)
    {
        let system = context.select(IfcGeometricRepresentationContext::WORLD_COORDINATE_SYSTEM);
        let mut wcs = Matrix4::IDENTITY;
        util::convert_axis_placement_select(db, &system, &mut wcs)?;
    }
    Ok(())
}

/// `ProcessSpatialStructures`: the sites (else the buildings) the project
/// aggregates, else all of them.
fn process_spatial_structures(conversion: &mut Conversion, project: &Object) -> Outcome<()> {
    let db = conversion.db;
    let mut range = db.tracked("ifcsite");
    if range.is_empty() {
        range = db.tracked("ifcbuilding");
        if range.is_empty() {
            return Err(Failure::Std("IFC: no root element found (expected IfcBuilding or preferably IfcSite)".to_string()));
        }
    }
    let range = range.to_vec();
    let mut nodes = 0usize;
    for &lazy in &range {
        let element = db.object(lazy)?;
        if !element.is_a(Class::IfcSpatialStructureElement) {
            continue;
        }
        for &relation in db.refs(project.id) {
            let Some(relation) = db.get_object(relation) else {
                return Err(Failure::undefined("a missing IFC relation read"));
            };
            let relation = db.object(relation)?;
            if !relation.is_a(Class::IfcRelAggregates) {
                continue;
            }
            for definition in relation.list(IfcRelDecomposes::RELATED_OBJECTS).to_vec() {
                let definition = db.to(definition.entity(), Class::IfcObjectDefinition)?;
                if definition.id == element.id {
                    process_spatial_structure(conversion, &element, None)?;
                    nodes += 1;
                }
            }
        }
    }
    if nodes == 0 {
        for &lazy in &range {
            let element = db.object(lazy)?;
            if !element.is_a(Class::IfcSpatialStructureElement) {
                continue;
            }
            process_spatial_structure(conversion, &element, None)?;
            nodes += 1;
        }
    }
    if nodes == 0 {
        return Err(Failure::Std("IFC: failed to determine primary site element".to_string()));
    }
    Ok(())
}

/// `ResolveObjectPlacement`: a local placement's matrix after its parent
/// placement's, in single precision.
fn resolve_object_placement(conversion: &Conversion, placement: &Object) -> Outcome<Matrix4F> {
    conversion.enter(PLACEMENT_FRAME, "IFC placements")?;
    let matrix = placement_matrix(conversion, placement);
    conversion.leave(PLACEMENT_FRAME);
    matrix
}

/// The part of `ResolveObjectPlacement` after its frame.
fn placement_matrix(conversion: &Conversion, placement: &Object) -> Outcome<Matrix4F> {
    let db = conversion.db;
    if !placement.is_a(Class::IfcLocalPlacement) {
        return Ok(Matrix4F::IDENTITY);
    }
    // An unset select resolves to no placement (`dynamic_cast` of a null
    // `this` is null), leaving the identity.
    let mut relative = Matrix4::IDENTITY;
    let select = placement.select(IfcLocalPlacement::RELATIVE_PLACEMENT);
    util::convert_axis_placement_select(db, &select, &mut relative)?;
    let mut matrix = to_single(&relative);
    if placement.has(IfcLocalPlacement::PLACEMENT_REL_TO) {
        let parent = db.to(placement.entity(IfcLocalPlacement::PLACEMENT_REL_TO), Class::IfcObjectPlacement)?;
        let parent_matrix = resolve_object_placement(conversion, &parent)?;
        matrix = parent_matrix.product(&matrix);
    }
    Ok(matrix)
}

/// `ProcessMetadata` of one relation: its property set's properties, read
/// only for what they throw.
fn process_metadata(db: &Database, relation_id: u64) -> Outcome<()> {
    let Some(relation) = db.get_object(relation_id) else {
        return Err(Failure::undefined("a missing IFC relation read"));
    };
    let relation = db.object(relation)?;
    if !relation.is_a(Class::IfcRelDefinesByProperties) {
        return Ok(());
    }
    let definition = db.to(relation.entity(IfcRelDefinesByProperties::RELATING_PROPERTY_DEFINITION), Class::IfcPropertySetDefinition)?;
    let Some(set) = db.get_object(definition.id) else {
        return Err(Failure::undefined("a missing IFC property set read"));
    };
    let set = db.object(set)?;
    if !set.is_a(Class::IfcPropertySet) {
        return Ok(());
    }
    process_properties(db, set.list(IfcPropertySet::HAS_PROPERTIES), 0)
}

/// `ProcessMetadata` of a property list: complex properties down to the
/// third level.
fn process_properties(db: &Database, properties: &[step::Field], nest: u32) -> Outcome<()> {
    for property in properties {
        let property = db.to(property.entity(), Class::IfcProperty)?;
        let _name = property.text(IfcProperty::NAME);
        if property.is_a(Class::IfcPropertySingleValue) {
            if property.has(IfcPropertySingleValue::NOMINAL_VALUE) && property.select(IfcPropertySingleValue::NOMINAL_VALUE).is_none() {
                return Err(Failure::undefined("an unset IFC property value read"));
            }
        } else if property.is_a(Class::IfcPropertyListValue) {
            let _values = property.list(IfcPropertyListValue::LIST_VALUES);
        } else if property.is_a(Class::IfcComplexProperty) && nest <= 2 {
            process_properties(db, property.list(IfcComplexProperty::HAS_PROPERTIES), nest + 1)?;
        }
    }
    Ok(())
}

/// `ProcessSpatialStructure`: the element's metadata and placement, then
/// what it contains (openings set aside for it), its void openings, what
/// it aggregates, and its own representation with its openings cut in
/// (or collected for the element it voids). Spaces have no geometry,
/// annotations nothing.
fn process_spatial_structure(conversion: &mut Conversion, element: &Object, collect: Option<ListRef>) -> Outcome<Option<Node>> {
    conversion.enter(SPATIAL_STRUCTURE_FRAME, "IFC spatial structures")?;
    let node = spatial_structure(conversion, element, collect);
    conversion.leave(SPATIAL_STRUCTURE_FRAME);
    node
}

/// The part of `ProcessSpatialStructure` after its frame.
fn spatial_structure(conversion: &mut Conversion, element: &Object, collect: Option<ListRef>) -> Outcome<Option<Node>> {
    let db = conversion.db;
    let skip_geometry = conversion.settings.skip_space_representations && element.is_a(Class::IfcSpace);
    if element.is_a(Class::IfcAnnotation) {
        return Ok(None);
    }
    conversion.already_processed.insert(element.id);

    let relations = db.refs(element.id).to_vec();
    for &relation in &relations {
        process_metadata(db, relation)?;
    }

    let mut node = Node { transformation: Matrix4F::IDENTITY };
    if element.has(IfcProduct::OBJECT_PLACEMENT) {
        let placement = db.to(element.entity(IfcProduct::OBJECT_PLACEMENT), Class::IfcObjectPlacement)?;
        node.transformation = resolve_object_placement(conversion, &placement)?;
    }

    let openings = conversion.new_list();
    conversion.frames.push(FrameLists { openings, local: None });
    let result = process_structure_contents(conversion, element, &node, &relations, collect, skip_geometry);
    let frame = conversion.frames.pop().expect("the frame pushed above");
    conversion.drop_list(frame.openings);
    result?;

    let processed = conversion.already_processed.contains(&element.id);
    Failure::assertion(processed, "conv.already_processed.find(el.GetID()) != conv.already_processed.end()")?;
    conversion.already_processed.remove(&element.id);
    Ok(Some(node))
}

/// The part of `ProcessSpatialStructure` inside its `try`.
fn process_structure_contents(
    conversion: &mut Conversion,
    element: &Object,
    node: &Node,
    relations: &[u64],
    collect: Option<ListRef>,
    skip_geometry: bool,
) -> Outcome<()> {
    let db = conversion.db;
    let depth = conversion.frames.len() - 1;
    let openings_ref = ListRef { depth, slot: FrameSlot::Openings };
    let mut my_inverse: Option<Matrix4> = None;

    for &relation_id in relations {
        if conversion.already_processed.contains(&relation_id) {
            continue;
        }
        let relation = db.must_get_object(relation_id)?;
        let relation = db.object(relation)?;
        if relation.is_a(Class::IfcRelContainedInSpatialStructure) {
            let structure = db.to(relation.entity(IfcRelContainedInSpatialStructure::RELATING_STRUCTURE), Class::IfcSpatialStructureElement)?;
            if structure.id != element.id {
                continue;
            }
            for product in relation.list(IfcRelContainedInSpatialStructure::RELATED_ELEMENTS).to_vec() {
                let product = db.to(product.entity(), Class::IfcProduct)?;
                if product.is_a(Class::IfcOpeningElement) {
                    continue;
                }
                process_spatial_structure(conversion, &product, None)?;
            }
        } else if relation.is_a(Class::IfcRelVoidsElement) {
            let building_element = db.to(relation.entity(IfcRelVoidsElement::RELATING_BUILDING_ELEMENT), Class::IfcElement)?;
            if building_element.id != element.id {
                continue;
            }
            let opening = db.to(relation.entity(IfcRelVoidsElement::RELATED_OPENING_ELEMENT), Class::IfcFeatureElementSubtraction)?;
            let local = conversion.new_list();
            conversion.frames[depth].local = Some(local);
            let local_ref = ListRef { depth, slot: FrameSlot::Local };
            let child = process_spatial_structure(conversion, &opening, Some(local_ref));
            let child = match child {
                Ok(child) => child,
                Err(failure) => {
                    conversion.frames[depth].local = None;
                    conversion.drop_list(local);
                    return Err(failure);
                }
            };
            if let Some(child) = child {
                let collected = conversion.lists[local].take().unwrap_or_default();
                if !collected.is_empty() {
                    let inverse = *my_inverse.get_or_insert_with(|| to_double(&node.transformation.inverse()));
                    let child_transformation = to_double(&child.transformation);
                    let transformation = inverse.product(&child_transformation);
                    for mut collected_opening in collected {
                        collected_opening.transform(&transformation);
                        conversion.list_mut(openings_ref)?.push(collected_opening);
                    }
                }
            }
            conversion.frames[depth].local = None;
            conversion.drop_list(local);
        }
    }

    for &relation_id in relations {
        if conversion.already_processed.contains(&relation_id) {
            continue;
        }
        let Some(relation) = db.get_object(relation_id) else {
            return Err(Failure::undefined("a missing IFC relation read"));
        };
        let relation = db.object(relation)?;
        if !relation.is_a(Class::IfcRelAggregates) {
            continue;
        }
        let relating = db.to(relation.entity(IfcRelDecomposes::RELATING_OBJECT), Class::IfcObjectDefinition)?;
        if relating.id != element.id {
            continue;
        }
        for definition in relation.list(IfcRelDecomposes::RELATED_OBJECTS).to_vec() {
            let definition = db.to(definition.entity(), Class::IfcObjectDefinition)?;
            if definition.is_a(Class::IfcProduct) {
                process_spatial_structure(conversion, &definition, None)?;
            }
        }
    }

    conversion.collect_openings = collect;
    if conversion.collect_openings.is_none() {
        conversion.apply_openings = Some(openings_ref);
    }
    if !skip_geometry {
        process_product_representation(conversion, element, node)?;
        conversion.apply_openings = None;
        conversion.collect_openings = None;
    }
    Ok(())
}

/// `RateRepresentationPredicate::Rate(name)`.
fn rate_name(name: &[u8]) -> i32 {
    match name {
        b"SolidModel" => -3,
        b"SweptSolid" => -10,
        b"Clipping" => -5,
        b"Brep" => -2,
        b"BoundingBox" | b"Curve2D" => 100,
        _ => 0,
    }
}

/// `RateRepresentationPredicate::Rate(representation)`: by its identifier;
/// a mapped representation by its first item's mapped one.
fn rate(conversion: &Conversion, representation: Option<&Rc<Object>>) -> Outcome<i32> {
    conversion.enter(RATE_FRAME, "IFC mapped representations")?;
    let rating = rate_representation(conversion, representation);
    conversion.leave(RATE_FRAME);
    rating
}

/// The part of `Rate` after its frame.
fn rate_representation(conversion: &Conversion, representation: Option<&Rc<Object>>) -> Outcome<i32> {
    let db = conversion.db;
    let Some(representation) = representation else {
        return Err(Failure::undefined("a null IFC representation rated"));
    };
    if !representation.has(IfcRepresentation::REPRESENTATION_IDENTIFIER) {
        return Ok(0);
    }
    let name = representation.text(IfcRepresentation::REPRESENTATION_IDENTIFIER);
    if name != b"MappedRepresentation" {
        return Ok(rate_name(name));
    }
    let items = representation.list(IfcRepresentation::ITEMS);
    let Some(first) = items.first() else {
        return Ok(100);
    };
    let item = db.to(first.entity(), Class::IfcRepresentationItem)?;
    if !item.is_a(Class::IfcMappedItem) {
        return Ok(100);
    }
    let source = db.to(item.entity(IfcMappedItem::MAPPING_SOURCE), Class::IfcRepresentationMap)?;
    let mapped = db.to_ptr(source.entity(IfcRepresentationMap::MAPPED_REPRESENTATION), Class::IfcRepresentation)?;
    rate(conversion, mapped.as_ref())
}

/// `ProcessProductRepresentation`: the representations best rated first
/// (MSVC's sort), until one gives geometry.
fn process_product_representation(conversion: &mut Conversion, element: &Object, node: &Node) -> Outcome<()> {
    let db = conversion.db;
    if !element.has(IfcProduct::REPRESENTATION) {
        return Ok(());
    }
    let element_material = material::process_materials(conversion, element.id, u32::MAX, false)?;
    let mut meshes = BTreeSet::new();
    let product_representation = db.to(element.entity(IfcProduct::REPRESENTATION), Class::IfcProductRepresentation)?;
    let sources = product_representation.list(IfcProductRepresentation::REPRESENTATIONS);
    let mut ordered = Vec::with_capacity(sources.len());
    for source in sources {
        ordered.push(db.to_ptr(source.entity(), Class::IfcRepresentation)?);
    }
    if ordered.len() >= 2 {
        let mut rated = Vec::with_capacity(ordered.len());
        for (index, representation) in ordered.iter().enumerate() {
            rated.push((rate(conversion, representation.as_ref())?, index));
        }
        sort(&mut rated, |a, b| a.0 < b.0);
        ordered = rated.iter().map(|&(_, index)| ordered[index].clone()).collect();
    }
    for representation in ordered {
        let Some(representation) = representation else {
            return Err(Failure::undefined("a null IFC representation's items read"));
        };
        let mut got = false;
        for item in representation.list(IfcRepresentation::ITEMS).to_vec() {
            let item = db.to(item.entity(), Class::IfcRepresentationItem)?;
            if item.is_a(Class::IfcMappedItem) {
                got = process_mapped_item(conversion, &item, node, element_material)? || got;
            } else {
                got = geometry::process_representation_item(conversion, &item, element_material, &mut meshes)? || got;
            }
        }
        if got {
            break;
        }
    }
    Ok(())
}

/// `ProcessMappedItem`: the mapped representation's items under the
/// mapping's transformation; the element's openings move into the
/// mapping's space for good, openings collected here move out of it.
fn process_mapped_item(conversion: &mut Conversion, mapped: &Object, _node: &Node, material: u32) -> Outcome<bool> {
    let db = conversion.db;
    let target = db.to(mapped.entity(IfcMappedItem::MAPPING_TARGET), Class::IfcCartesianTransformationOperator)?;
    let operator = util::convert_transform_operator(db, &target)?;
    let source = db.to(mapped.entity(IfcMappedItem::MAPPING_SOURCE), Class::IfcRepresentationMap)?;
    let origin = source.select(IfcRepresentationMap::MAPPING_ORIGIN);
    let mut source_matrix = Matrix4::IDENTITY;
    util::convert_axis_placement_select(db, &origin, &mut source_matrix)?;
    let source_matrix = operator.product(&source_matrix);

    let mut meshes = BTreeSet::new();
    let old_openings = match conversion.collect_openings {
        Some(collect) => conversion.list(collect)?.len(),
        None => 0,
    };
    if let Some(apply) = conversion.apply_openings {
        let inverse = source_matrix.inverse();
        for opening in conversion.list_mut(apply)? {
            opening.transform(&inverse);
        }
    }

    let local_material = material::process_materials(conversion, mapped.id, material, false)?;
    let source = db.to(mapped.entity(IfcMappedItem::MAPPING_SOURCE), Class::IfcRepresentationMap)?;
    let representation = db.to(source.entity(IfcRepresentationMap::MAPPED_REPRESENTATION), Class::IfcRepresentation)?;
    let mut got = false;
    for item in representation.list(IfcRepresentation::ITEMS).to_vec() {
        let item = db.to(item.entity(), Class::IfcRepresentationItem)?;
        if geometry::process_representation_item(conversion, &item, local_material, &mut meshes)? {
            got = true;
        }
    }
    if !got {
        return Ok(false);
    }
    if let Some(collect) = conversion.collect_openings {
        let list = conversion.list_mut(collect)?;
        let added = list.len().wrapping_sub(old_openings);
        for index in 0..added {
            let Some(opening) = list.get_mut(old_openings + index) else {
                return Err(Failure::undefined("an IFC opening transformed past its list"));
            };
            opening.transform(&source_matrix);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project with one site whose placement is relative to a chain of
    /// `levels` placements in all.
    fn placement_chain(levels: usize) -> Vec<u8> {
        let mut text = String::from(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\n\
             FILE_SCHEMA(('IFC2X3'));\nENDSEC;\nDATA;\n\
             #1=IFCOWNERHISTORY($,$,$,.ADDED.,$,$,$,0);\n#2=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #3=IFCAXIS2PLACEMENT3D(#2,$,$);\n#4=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-5,#3,$);\n\
             #5=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n#6=IFCUNITASSIGNMENT((#5));\n\
             #7=IFCPROJECT('p',#1,$,$,$,$,$,(#4),#6);\n#10=IFCLOCALPLACEMENT($,#3);\n",
        );
        for level in 1..levels {
            text.push_str(&format!("#{}=IFCLOCALPLACEMENT(#{},#3);\n", 10 + level, 9 + level));
        }
        let site = 10 + levels;
        text.push_str(&format!("#{site}=IFCSITE('s',#1,$,$,$,#{},$,$,.ELEMENT.,$,$,$,$,$);\n", site - 1));
        text.push_str(&format!("#{}=IFCRELAGGREGATES('a',#1,$,$,#7,(#{site}));\n", site + 1));
        text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        text.into_bytes()
    }

    fn convert_on_a_large_stack(bytes: Vec<u8>) -> Outcome<Scene> {
        let worker = std::thread::Builder::new().stack_size(CONVERSION_STACK);
        worker.spawn(move || convert(&bytes)).unwrap().join().unwrap()
    }

    #[test]
    fn placement_chains_overflow_where_the_native_stack_does() {
        // The site's frame and 1,271 placement frames fit in the budget.
        assert!(convert_on_a_large_stack(placement_chain(1271)).is_ok());
        let Err(Failure::Process(error)) = convert_on_a_large_stack(placement_chain(1272)) else {
            panic!("a chain one placement longer should overflow");
        };
        assert_eq!(error.0, "undefined behaviour: IFC placements nested past the stack");
    }
}
