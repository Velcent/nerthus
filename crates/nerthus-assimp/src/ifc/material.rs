//! `IFCMaterial`: a representation item's (or product's) surface style
//! as a material. Only which material an item gets matters for the
//! meshes (it keys the mesh cache), and what reading the style throws.

use super::schema::Class;
use super::schema::fields::{
    IfcPresentationStyle, IfcPresentationStyleAssignment, IfcSurfaceStyle, IfcSurfaceStyleRendering, IfcSurfaceStyleShading, IfcStyledItem,
};
use super::step::Value;
use super::{Conversion, Failure, Outcome};

/// `aiString` holds this many bytes with its NUL: `Set(std::string)`
/// leaves a longer name unset.
const MAX_NAME: usize = 1024;

const DEFAULT_NAME: &[u8] = b"<IFCDefault>";

/// `ProcessMaterials`: the material of the first surface style an
/// `IfcStyledItem` referencing the object gives (made once per style),
/// else the inherited one, else (when forced) the default material.
pub(super) fn process_materials(conversion: &mut Conversion, id: u64, previous: u32, force_default: bool) -> Outcome<u32> {
    let db = conversion.db;
    for &relation in db.refs(id).to_vec().iter() {
        let Some(styled) = db.get_object(relation) else {
            return Err(Failure::undefined("a missing IFC styled item read"));
        };
        let styled = db.object(styled)?;
        if !styled.is_a(Class::IfcStyledItem) {
            continue;
        }
        for assignment in styled.list(IfcStyledItem::STYLES).to_vec() {
            let assignment = db.to(assignment.entity(), Class::IfcPresentationStyleAssignment)?;
            for select in assignment.list(IfcPresentationStyleAssignment::STYLES).to_vec() {
                let Some(surface) = db.resolve_select_ptr(&select.select(), Class::IfcSurfaceStyle)? else {
                    continue;
                };
                if let Some(&cached) = conversion.cached_materials.get(&surface.index) {
                    return Ok(cached);
                }
                let _side = surface.text(IfcSurfaceStyle::SIDE);
                fill_material(conversion, &surface)?;
                let index = (conversion.materials.len() - 1) as u32;
                conversion.cached_materials.insert(surface.index, index);
                return Ok(index);
            }
        }
    }
    if previous != u32::MAX {
        return Ok(previous);
    }
    if !force_default {
        return Ok(u32::MAX);
    }
    if let Some(index) = conversion.materials.iter().position(|name| name == DEFAULT_NAME) {
        return Ok(index as u32);
    }
    conversion.materials.push(DEFAULT_NAME.to_vec());
    Ok((conversion.materials.len() - 1) as u32)
}

/// `FillMaterial`: the style's name and the shading styles' colours, read
/// for what they throw.
fn fill_material(conversion: &mut Conversion, surface: &super::step::Object) -> Outcome<()> {
    let db = conversion.db;
    let mut name = b"IfcSurfaceStyle_Unnamed".to_vec();
    if surface.has(IfcPresentationStyle::NAME) {
        name = surface.text(IfcPresentationStyle::NAME).to_vec();
    }
    if name.len() >= MAX_NAME {
        name.clear();
    }
    conversion.materials.push(name);
    for style in surface.list(IfcSurfaceStyle::STYLES).to_vec() {
        let Some(shading) = db.resolve_select_ptr(&style.select(), Class::IfcSurfaceStyleShading)? else {
            continue;
        };
        db.to(shading.entity(IfcSurfaceStyleShading::SURFACE_COLOUR), Class::IfcColourRgb)?;
        if !shading.is_a(Class::IfcSurfaceStyleRendering) {
            continue;
        }
        let colours = [
            IfcSurfaceStyleRendering::DIFFUSE_COLOUR,
            IfcSurfaceStyleRendering::SPECULAR_COLOUR,
            IfcSurfaceStyleRendering::TRANSMISSION_COLOUR,
            IfcSurfaceStyleRendering::REFLECTION_COLOUR,
        ];
        for slot in colours {
            if shading.has(slot) {
                convert_colour(conversion, shading.select(slot))?;
            }
        }
    }
    Ok(())
}

/// `ConvertColor(IfcColourOrFactor)`: a REAL factor, or an `IfcColourRgb`
/// the select names (a missing entity throws).
fn convert_colour(conversion: &Conversion, select: Option<std::rc::Rc<Value>>) -> Outcome<()> {
    let Some(value) = &select else {
        return Err(Failure::undefined("an unset IFC colour read"));
    };
    if value.as_real().is_some() {
        return Ok(());
    }
    conversion.db.resolve_select_ptr(&select, Class::IfcColourRgb)?;
    Ok(())
}
