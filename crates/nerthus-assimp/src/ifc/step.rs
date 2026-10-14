//! The STEP reader under the IFC importer (`STEPFileReader`, `STEPFile.h`):
//! the header, every data line kept as an object converted when first
//! used, the inverse references of a few relation types, and the
//! conversion of an object's arguments by the schema's `GenericFill`
//! rules.
//!
//! Lines are read by a `LineSplitter` that skips empty lines and trims;
//! a data line has its spaces removed, then `#id=TYPE(...);`, joined with
//! the following lines while the brackets or the final `;` are missing
//! (a join that reached the next entity line steps past it when the `;`
//! is missing too). Lines of types the schema doesn't name are dropped;
//! a later line with the same id replaces the earlier one.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::Failure;
use super::encoding::string_to_utf8;
use super::schema::{self, Class, Converter, FieldType, Slot};
use crate::fast_atof::{fast_atoreal_move_f64, strtoul10_64};
use crate::line_splitter::LineSplitter;
use crate::parsing::{at, is_space, is_space_or_newline, skip_spaces};

const ISO_TOKEN: &[u8] = b"ISO-10303-21;";
const FILE_SCHEMA_TOKEN: &[u8] = b"FILE_SCHEMA";

/// The types whose objects the reader lists (`SetTypesToTrack`).
pub(crate) const TRACKED_TYPES: [&str; 3] = ["ifcsite", "ifcbuilding", "ifcproject"];

/// The types whose references the reader keeps as inverse indices.
const INVERSE_TYPES: [&str; 6] =
    ["ifcrelcontainedinspatialstructure", "ifcrelaggregates", "ifcrelvoidselement", "ifcreldefinesbyproperties", "ifcpropertyset", "ifcstyleditem"];

/// `EXPRESS::DataType`: a parsed argument.
#[derive(Debug)]
pub(crate) enum Value {
    /// `$`.
    Unset,
    /// `*`.
    Derived,
    /// An integer (no conversion reads its value).
    Integer,
    Real(f64),
    /// A string literal, its escapes decoded.
    String(Vec<u8>),
    /// `.NAME.` (an ENUMERATION is a STRING too).
    Enumeration(Vec<u8>),
    Entity(u64),
    List(Vec<Rc<Value>>),
}

impl Value {
    /// `ToPtr<REAL>`.
    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(value) => Some(*value),
            _ => None,
        }
    }

    /// `ToPtr<ENTITY>`.
    pub fn as_entity(&self) -> Option<u64> {
        match self {
            Value::Entity(id) => Some(*id),
            _ => None,
        }
    }
}

/// A converted argument (what `GenericConvert` stores in a member).
#[derive(Clone, Debug)]
pub(crate) enum Field {
    /// Never converted (`*`, `$`, or a class that fills nothing): the
    /// member's default value (and a `Maybe` not valid).
    Default,
    Real(f64),
    /// An integer member (no conversion reads one).
    Integer,
    Text(Rc<[u8]>),
    /// `Lazy<T>`: the object the id named when converted, if any.
    Entity(Option<usize>),
    Select(Rc<Value>),
    List(Rc<[Field]>),
}

impl Field {
    pub fn real(&self) -> f64 {
        match self {
            Field::Real(value) => *value,
            _ => 0.0,
        }
    }

    pub fn text(&self) -> &[u8] {
        match self {
            Field::Text(text) => text,
            _ => &[],
        }
    }

    pub fn entity(&self) -> Option<usize> {
        match self {
            Field::Entity(object) => *object,
            _ => None,
        }
    }

    /// A select's value (none: the null `shared_ptr` of a member never
    /// converted).
    pub fn select(&self) -> Option<Rc<Value>> {
        match self {
            Field::Select(value) => Some(value.clone()),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Field] {
        match self {
            Field::List(items) => items,
            _ => &[],
        }
    }
}

/// A converted object: its class (none for `NotImplemented`, which no
/// cast takes) and its members by argument slot.
#[derive(Debug)]
pub(crate) struct Object {
    /// The `LazyObject` it came from (its identity, as the C++'s pointer).
    pub index: usize,
    pub id: u64,
    pub class: Option<Class>,
    fields: Vec<Field>,
}

impl Object {
    /// `dynamic_cast` to a class.
    pub fn is_a(&self, class: Class) -> bool {
        self.class.is_some_and(|own| own.is_a(class))
    }

    pub fn field(&self, slot: Slot) -> &Field {
        const DEFAULT: &Field = &Field::Default;
        self.fields.get(slot.index).unwrap_or(DEFAULT)
    }

    /// Whether a `Maybe` member is valid.
    pub fn has(&self, slot: Slot) -> bool {
        !matches!(self.field(slot), Field::Default)
    }

    pub fn real(&self, slot: Slot) -> f64 {
        self.field(slot).real()
    }

    pub fn text(&self, slot: Slot) -> &[u8] {
        self.field(slot).text()
    }

    pub fn entity(&self, slot: Slot) -> Option<usize> {
        self.field(slot).entity()
    }

    pub fn select(&self, slot: Slot) -> Option<Rc<Value>> {
        self.field(slot).select()
    }

    pub fn list(&self, slot: Slot) -> &[Field] {
        self.field(slot).list()
    }

}

/// Where a `LazyObject` is: its argument text, the object, or neither
/// (the converter threw after the text was freed).
enum State {
    Pending(Rc<[u8]>),
    Ready(Rc<Object>),
    Freed,
}

/// `LazyObject`: a data line, converted when first used.
struct LazyObject {
    id: u64,
    converter: Converter,
    type_name: &'static str,
    state: RefCell<State>,
}

/// `STEP::DB`: the header's schema and the objects by id and by type.
pub(crate) struct Database {
    pub file_schema: Vec<u8>,
    lazies: Vec<LazyObject>,
    /// `objects`: by id (`std::map`, a later line with an id replacing
    /// the earlier).
    ids: BTreeMap<u64, usize>,
    /// `objects_bytype` for the tracked types, in file order (the C++'s
    /// set orders them by address).
    by_type: HashMap<&'static str, Vec<usize>>,
    /// `refs`: for each id, the ids of the objects referencing it, in the
    /// order they were read.
    refs: HashMap<u64, Vec<u64>>,
}

fn syntax_error<T>(message: &str) -> Result<T, Failure> {
    Err(Failure::Std(message.to_string()))
}

/// `LineSplitter::operator++`: the next line, or `logic_error` once no
/// bytes remain.
fn advance(splitter: &mut LineSplitter) -> Result<(), Failure> {
    if !splitter.advance() {
        return Err(Failure::Std("End of file, no more lines to be retrieved.".to_string()));
    }
    Ok(())
}

/// `ReadFileHeader` then `ReadFile`: the database with every data line.
pub(crate) fn read(bytes: &[u8]) -> Result<Database, Failure> {
    let mut splitter = LineSplitter::new(bytes, true, true);
    if !splitter.is_valid() || splitter.current() != ISO_TOKEN {
        return syntax_error("(line 1) expected magic token: ISO-10303-21;");
    }
    let mut database = Database {
        file_schema: Vec::new(),
        lazies: Vec::new(),
        ids: BTreeMap::new(),
        by_type: TRACKED_TYPES.iter().map(|&name| (name, Vec::new())).collect(),
        refs: HashMap::new(),
    };
    read_header(&mut splitter, &mut database)?;
    read_data(&mut splitter, &mut database)?;
    Ok(database)
}

/// The header up to `DATA;`: only `FILE_SCHEMA` is read, its first list's
/// first string.
fn read_header(splitter: &mut LineSplitter, database: &mut Database) -> Result<(), Failure> {
    advance(splitter)?;
    while splitter.is_valid() {
        let line = splitter.current().to_vec();
        if line == b"DATA;" {
            advance(splitter)?;
            return Ok(());
        }
        if line.starts_with(FILE_SCHEMA_TOKEN) {
            read_file_schema(&line, database)?;
        }
        advance(splitter)?;
    }
    Ok(())
}

fn read_file_schema(line: &[u8], database: &mut Database) -> Result<(), Failure> {
    let (start, _) = skip_spaces(line, FILE_SCHEMA_TOKEN.len(), line.len());
    let mut position = start;
    let value = parse_value(line, &mut position, false)?;
    let Value::List(outer) = &*value else {
        return Ok(());
    };
    let Some(first) = outer.first() else {
        return Ok(());
    };
    let Value::List(inner) = &**first else {
        return syntax_error("expected FILE_SCHEMA to be a list");
    };
    // `(*list)[0]` comes before the size check: an empty list's first
    // element is the reserved, unwritten slot.
    let Some(name) = inner.first() else {
        return Err(Failure::undefined("the first entry of an empty FILE_SCHEMA list read"));
    };
    let Some(text) = (match &**name {
        Value::String(text) | Value::Enumeration(text) => Some(text),
        _ => None,
    }) else {
        return syntax_error("expected FILE_SCHEMA to contain a single string literal");
    };
    database.file_schema = text.clone();
    Ok(())
}

/// `IsEntityDef`: `#`, digits or spaces, then `=`.
fn is_entity_definition(line: &[u8]) -> bool {
    if line.first() != Some(&b'#') {
        return false;
    }
    for &byte in &line[1..] {
        if byte == b'=' {
            return true;
        }
        if !byte.is_ascii_digit() && byte != b' ' {
            break;
        }
    }
    false
}

fn find_from(text: &[u8], start: usize, byte: u8) -> Option<usize> {
    text.get(start..)?.iter().position(|&other| other == byte).map(|offset| start + offset)
}

/// Whether the line's last `)` is followed by `;` and comes after the
/// `(` at `open`.
fn closed(text: &[u8], open: usize) -> bool {
    let Some(close) = text.iter().rposition(|&byte| byte == b')') else {
        return false;
    };
    close >= open && close != text.len() - 1 && text[close + 1] == b';'
}

/// The data lines up to `ENDSEC;` (the last line is never looked at).
fn read_data(splitter: &mut LineSplitter, database: &mut Database) -> Result<(), Failure> {
    while splitter.is_valid() {
        let mut has_next = false;
        let mut line = splitter.current().to_vec();
        if line == b"ENDSEC;" {
            break;
        }
        line.retain(|&byte| byte != b' ');
        if line.first() != Some(&b'#') {
            advance(splitter)?;
            continue;
        }
        let Some(equals) = find_from(&line, 0, b'=') else {
            advance(splitter)?;
            continue;
        };
        let mut id_text = line[1..equals].to_vec();
        id_text.push(0);
        let (id, _) = strtoul10_64(&id_text, 0, None)?;
        if id == 0 {
            advance(splitter)?;
            continue;
        }
        let mut open = find_from(&line, equals, b'(');
        if open.is_none() {
            has_next = true;
            let mut found = false;
            advance(splitter)?;
            while splitter.is_valid() {
                let next = splitter.current();
                if !next.is_empty() {
                    if is_entity_definition(next) {
                        break;
                    }
                    line.extend_from_slice(next);
                    open = find_from(&line, equals, b'(');
                    found = open.is_some();
                }
                advance(splitter)?;
            }
            if !found {
                continue;
            }
        }
        let open = open.unwrap_or_default();
        if !closed(&line, open) {
            has_next = true;
            let mut found = false;
            advance(splitter)?;
            while splitter.is_valid() {
                let next = splitter.current();
                if !next.is_empty() {
                    if is_entity_definition(next) {
                        break;
                    }
                    line.extend_from_slice(next);
                    found = closed(&line, open);
                }
                advance(splitter)?;
            }
            if !found {
                continue;
            }
        }
        let close = line.iter().rposition(|&byte| byte == b')').unwrap_or(open);
        let mut type_start = equals + 1;
        while is_space(line[type_start]) {
            type_start += 1;
        }
        let mut type_end = open - 1;
        while is_space(line[type_end]) {
            type_end -= 1;
        }
        let type_name: Vec<u8> = line[type_start..type_end + 1].to_ascii_lowercase();
        if let Ok(index) = schema::SCHEMA.binary_search_by(|(entry, _)| entry.as_bytes().cmp(&type_name[..])) {
            let (name, converter) = schema::SCHEMA[index];
            let mut args = line[open..close + 1].to_vec();
            args.push(0);
            insert(database, id, name, converter, args)?;
        }
        if !has_next {
            advance(splitter)?;
        }
    }
    Ok(())
}

/// `LazyObject`'s constructor and `InternInsert`: the inverse references
/// of a tracked relation type (every `#n` inside the brackets, a `##`
/// skipped; a `#` without digits after it throws), then the object by id
/// and by type.
fn insert(database: &mut Database, id: u64, type_name: &'static str, converter: Converter, args: Vec<u8>) -> Result<(), Failure> {
    if INVERSE_TYPES.contains(&type_name) {
        let mut depth = 0i64;
        let mut position = 0;
        while args[position] != 0 {
            match args[position] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth >= 1 && args[position] == b'#' {
                if args[position + 1] != b'#' {
                    let (referenced, _) = strtoul10_64(&args, position + 1, None)?;
                    database.refs.entry(referenced).or_default().push(id);
                } else {
                    position += 1;
                }
            }
            position += 1;
        }
    }
    let index = database.lazies.len();
    let state = RefCell::new(State::Pending(Rc::from(args)));
    database.lazies.push(LazyObject { id, converter, type_name, state });
    database.ids.insert(id, index);
    if let Some(list) = database.by_type.get_mut(type_name) {
        list.push(index);
    }
    Ok(())
}

/// `EXPRESS::DataType::Parse`: the value at `position` (with the schema,
/// a typed value such as `IFCLABEL('x')` is read as its value, the byte
/// after it skipped).
pub(crate) fn parse_value(buffer: &[u8], position: &mut usize, with_schema: bool) -> Result<Rc<Value>, Failure> {
    let end = buffer.len();
    let (mut cursor, _) = skip_spaces(buffer, *position, end);
    let first = at(buffer, cursor);
    if first == b',' || is_space_or_newline(first) {
        return syntax_error("unexpected token, expected parameter");
    }
    if with_schema {
        let mut ok = false;
        let mut scan = cursor;
        loop {
            let byte = at(buffer, scan);
            if byte == 0 || byte == b')' || byte == b',' {
                break;
            }
            if byte == b'(' {
                if !ok {
                    break;
                }
                let mut name_end = scan - 1;
                while is_space(at(buffer, name_end)) {
                    name_end -= 1;
                }
                let name = buffer[cursor..name_end + 1].to_ascii_lowercase();
                if schema::converter(&name).is_some() {
                    let mut inner = name_end + 1;
                    loop {
                        let byte = at(buffer, inner);
                        inner += 1;
                        if byte == b'(' {
                            break;
                        }
                    }
                    let value = parse_value(buffer, &mut inner, false)?;
                    *position = if at(buffer, inner) != 0 { inner + 1 } else { inner };
                    return Ok(value);
                }
                break;
            } else if !is_space(byte) {
                ok = true;
            }
            scan += 1;
        }
    }
    match first {
        b'*' => {
            *position = cursor + 1;
            Ok(Rc::new(Value::Derived))
        }
        b'$' => {
            *position = cursor + 1;
            Ok(Rc::new(Value::Unset))
        }
        b'(' => {
            *position = cursor;
            parse_list(buffer, position, with_schema)
        }
        b'.' => {
            cursor += 1;
            let start = cursor;
            while at(buffer, cursor) != b'.' {
                if at(buffer, cursor) == 0 {
                    return syntax_error("enum not closed");
                }
                cursor += 1;
            }
            *position = cursor + 1;
            Ok(Rc::new(Value::Enumeration(buffer[start..cursor].to_vec())))
        }
        b'#' => {
            let (id, after) = strtoul10_64(buffer, cursor + 1, None)?;
            if let Some(after) = after {
                *position = after;
            }
            Ok(Rc::new(Value::Entity(id)))
        }
        b'\'' => {
            cursor += 1;
            let start = cursor;
            while at(buffer, cursor) != b'\'' {
                if at(buffer, cursor) == 0 {
                    return syntax_error("string literal not closed");
                }
                cursor += 1;
            }
            if at(buffer, cursor + 1) == b'\'' {
                loop {
                    cursor += 2;
                    while at(buffer, cursor) != b'\'' {
                        if at(buffer, cursor) == 0 {
                            return syntax_error("string literal not closed");
                        }
                        cursor += 1;
                    }
                    if at(buffer, cursor + 1) != b'\'' {
                        break;
                    }
                }
            }
            *position = cursor + 1;
            let mut text = buffer[start..cursor].to_vec();
            string_to_utf8(&mut text)?;
            Ok(Rc::new(Value::String(text)))
        }
        b'"' => syntax_error("binary data not supported yet"),
        _ => parse_number(buffer, position, cursor),
    }
}

/// A number: real if a `.` comes before the next `,`, `)` or blank.
fn parse_number(buffer: &[u8], position: &mut usize, start: usize) -> Result<Rc<Value>, Failure> {
    let mut cursor = start;
    loop {
        let byte = at(buffer, cursor);
        if byte == 0 || byte == b',' || byte == b')' || is_space(byte) {
            break;
        }
        if byte == b'.' {
            let (value, after) = fast_atoreal_move_f64(buffer, start, true)?;
            *position = after;
            return Ok(Rc::new(Value::Real(value)));
        }
        cursor += 1;
    }
    let mut digits = start;
    if at(buffer, digits) == b'-' || at(buffer, digits) == b'+' {
        digits += 1;
    }
    let (_, after) = strtoul10_64(buffer, digits, None)?;
    if let Some(after) = after {
        *position = after;
    }
    Ok(Rc::new(Value::Integer))
}

/// `EXPRESS::LIST::Parse`.
fn parse_list(buffer: &[u8], position: &mut usize, with_schema: bool) -> Result<Rc<Value>, Failure> {
    let end = buffer.len();
    let mut cursor = *position;
    if at(buffer, cursor) != b'(' {
        return syntax_error("unexpected token, expected '(' token at beginning of list");
    }
    cursor += 1;
    let mut members = Vec::new();
    loop {
        if at(buffer, cursor) == 0 {
            return syntax_error("unexpected end of line while reading list");
        }
        cursor = skip_spaces(buffer, cursor, end).0;
        if at(buffer, cursor) == b')' {
            break;
        }
        members.push(parse_value(buffer, &mut cursor, with_schema)?);
        cursor = skip_spaces(buffer, cursor, end).0;
        if at(buffer, cursor) != b',' {
            if at(buffer, cursor) == b')' {
                break;
            }
            return syntax_error("unexpected token, expected ',' or ')' token after list element");
        }
        cursor += 1;
    }
    *position = cursor + 1;
    Ok(Rc::new(Value::List(members)))
}

impl Database {
    /// `GetObject(id)`.
    pub fn get_object(&self, id: u64) -> Option<usize> {
        self.ids.get(&id).copied()
    }

    /// `MustGetObject(id)`.
    pub fn must_get_object(&self, id: u64) -> Result<usize, Failure> {
        match self.get_object(id) {
            Some(index) => Ok(index),
            None => Err(Failure::Std(format!("(entity #{id}) requested entity is not present"))),
        }
    }

    /// The objects of a tracked type, in file order.
    pub fn tracked(&self, type_name: &str) -> &[usize] {
        self.by_type.get(type_name).map_or(&[], Vec::as_slice)
    }

    /// `refs.equal_range(id)`: the referencing objects' ids.
    pub fn refs(&self, id: u64) -> &[u64] {
        self.refs.get(&id).map_or(&[], Vec::as_slice)
    }

    /// `LazyObject::operator*`: the object, converted on first use.
    pub fn object(&self, index: usize) -> Result<Rc<Object>, Failure> {
        let lazy = &self.lazies[index];
        let args = match &*lazy.state.borrow() {
            State::Ready(object) => return Ok(object.clone()),
            State::Freed => return Err(Failure::undefined("an IFC object whose conversion failed read again (strlen of its freed arguments)")),
            State::Pending(args) => args.clone(),
        };
        let class = match lazy.converter {
            Converter::Missing => return Err(Failure::Std(format!("(entity #{}) unknown object type: {}", lazy.id, lazy.type_name))),
            Converter::NotImplemented => None,
            Converter::Class(class) => Some(class),
        };
        let mut position = 0;
        let length = args.iter().position(|&byte| byte == 0).unwrap_or(args.len());
        let list = parse_list(&args[..length], &mut position, true)?;
        *lazy.state.borrow_mut() = State::Freed;
        let Value::List(params) = &*list else {
            return Err(Failure::undefined("an IFC argument list that isn't one"));
        };
        let mut fields = vec![Field::Default; params.len()];
        if let Some(class) = class
            && let Err(failure) = self.fill(class, params, &mut fields)
        {
            return Err(failure.with_entity(lazy.id));
        }
        let object = Rc::new(Object { index, id: lazy.id, class, fields });
        *lazy.state.borrow_mut() = State::Ready(object.clone());
        Ok(object)
    }

    /// `GenericFill<T>`: the parent's arguments first, then the class's own
    /// (after its argument-count check).
    fn fill(&self, class: Class, params: &[Rc<Value>], fields: &mut [Field]) -> Result<usize, Failure> {
        let mut base = match class.parent() {
            Some(parent) => self.fill(parent, params, fields)?,
            None => 0,
        };
        let class_fill = class.fill();
        if let Some(required) = class_fill.required
            && params.len() < required
        {
            return Err(Failure::Std(format!("expected {required} arguments to {}", class.name())));
        }
        for field in class_fill.fields {
            let Some(argument) = params.get(base) else {
                return Err(Failure::undefined("an IFC argument read past its list"));
            };
            base += 1;
            if field.derived_check && matches!(**argument, Value::Derived) {
                continue;
            }
            if field.unset_check && matches!(**argument, Value::Unset) {
                continue;
            }
            fields[field.slot] = self.convert(field.field_type, argument)?;
        }
        Ok(base)
    }

    /// `GenericConvert`: the argument as the member's type.
    fn convert(&self, field_type: FieldType, value: &Rc<Value>) -> Result<Field, Failure> {
        let mismatch = |what: &str| Err(Failure::Std(format!("type error reading {what}")));
        match field_type {
            FieldType::Real => match **value {
                Value::Real(real) => Ok(Field::Real(real)),
                _ => mismatch("literal field"),
            },
            FieldType::Integer => match **value {
                Value::Integer => Ok(Field::Integer),
                _ => mismatch("literal field"),
            },
            FieldType::Text => match &**value {
                Value::String(text) | Value::Enumeration(text) => Ok(Field::Text(Rc::from(text.as_slice()))),
                _ => mismatch("literal field"),
            },
            FieldType::Entity => match **value {
                Value::Entity(id) => Ok(Field::Entity(self.get_object(id))),
                _ => mismatch("entity"),
            },
            FieldType::Select => Ok(Field::Select(value.clone())),
            FieldType::List(element, _, _) => {
                let Value::List(items) = &**value else {
                    return mismatch("aggregate");
                };
                let mut converted = Vec::with_capacity(items.len());
                for item in items {
                    converted.push(self.convert(*element, item)?);
                }
                Ok(Field::List(Rc::from(converted)))
            }
        }
    }

    /// `Lazy<T>::operator const T&` (and `*`, `->`): the object, which must
    /// be a `class` (`std::bad_cast` otherwise); a null reference throws.
    pub fn to(&self, lazy: Option<usize>, class: Class) -> Result<Rc<Object>, Failure> {
        let object = self.to_any(lazy)?;
        if !object.is_a(class) {
            return Err(Failure::BadCast);
        }
        Ok(object)
    }

    /// `Lazy<T>::operator const T*`: the object if it is a `class`.
    pub fn to_ptr(&self, lazy: Option<usize>, class: Class) -> Result<Option<Rc<Object>>, Failure> {
        let object = self.to_any(lazy)?;
        Ok(object.is_a(class).then_some(object))
    }

    /// The object a `Lazy` names, converted (a null one throws).
    pub fn to_any(&self, lazy: Option<usize>) -> Result<Rc<Object>, Failure> {
        let Some(index) = lazy else {
            return Err(Failure::Std("Obj type is nullptr.".to_string()));
        };
        self.object(index)
    }

    /// `ResolveSelect<T>`: the select must be an ENTITY (`bad_cast`), whose
    /// object must exist and be a `class`.
    pub fn resolve_select(&self, value: &Option<Rc<Value>>, class: Class) -> Result<Rc<Object>, Failure> {
        let Some(value) = value else {
            return Err(Failure::undefined("an unset IFC select resolved"));
        };
        let Some(id) = value.as_entity() else {
            return Err(Failure::BadCast);
        };
        let index = self.must_get_object(id)?;
        let object = self.object(index)?;
        if !object.is_a(class) {
            return Err(Failure::BadCast);
        }
        Ok(object)
    }

    /// `ResolveSelectPtr<T>`: the object if the select is an ENTITY whose
    /// object exists (else it throws) and is a `class`.
    pub fn resolve_select_ptr(&self, value: &Option<Rc<Value>>, class: Class) -> Result<Option<Rc<Object>>, Failure> {
        let Some(id) = value.as_ref().and_then(|value| value.as_entity()) else {
            return Ok(None);
        };
        let index = self.must_get_object(id)?;
        let object = self.object(index)?;
        Ok(object.is_a(class).then_some(object))
    }
}
