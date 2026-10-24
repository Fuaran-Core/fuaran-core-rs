//! The schema field and the schema: a column's name, its type and the metadata
//! that says what the column MEANS — an optional unit of measure, a label, a
//! description, and extension members every host preserves verbatim — and the
//! schema-level operations over a list of them: the structural `diff`, its replay
//! `patch`, the depended-on-column compatibility verdict and the cross-host
//! fingerprint. The twin of the reference's `Field`, `Schema` and
//! `SchemaDeltaCodec` (`Fuaran.Core.Column`), and of the schema-entry half of
//! `ColumnCodec`; certified against Core's `laws/field-laws.json`.
//!
//! **Metadata is part of a field's identity**: two fields differing only in a unit
//! are different fields, a delta reports the change, and the fingerprint moves
//! with it. A field that states no metadata is exactly the `{name, type}` entry,
//! and encodes and fingerprints as one.

use crate::column::{self, ColumnError, ColumnType};
use crate::decode::Value;
use crate::hash::{canonical_fields, fnv1a};
use crate::json::{self, JVal, ordinal_cmp};
use crate::prelude::*;
use crate::unit::{self, UnitOfMeasure};

// ─── The field ───────────────────────────────────────────────────────────────

/// A schema entry — twin of `Field`. Built with [`Field::create`] and refined
/// with the `with_…` builders; equality is structural over every member.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    name: String,
    ty: ColumnType,
    unit: Option<UnitOfMeasure>,
    label: Option<String>,
    description: Option<String>,
    /// Sorted by key, ordinally; one entry per key.
    ext: Vec<(String, String)>,
}

impl Field {
    /// A field of `name` and `ty` with no metadata (`Field.create`).
    pub fn create(name: &str, ty: ColumnType) -> Field {
        Field {
            name: name.to_string(),
            ty,
            unit: None,
            label: None,
            description: None,
            ext: Vec::new(),
        }
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The column type.
    pub fn column_type(&self) -> ColumnType {
        self.ty
    }

    /// The unit the values are measured in, when stated.
    pub fn unit(&self) -> Option<&UnitOfMeasure> {
        self.unit.as_ref()
    }

    /// The label, when stated.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The description, when stated.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The extension members, in ordinal key order.
    pub fn ext(&self) -> &[(String, String)] {
        &self.ext
    }

    /// The extension member `key`, when carried.
    pub fn ext_value(&self, key: &str) -> Option<&str> {
        self.ext
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The field with its type replaced (`Field.withType`).
    pub fn with_type(mut self, ty: ColumnType) -> Field {
        self.ty = ty;
        self
    }

    /// The field measured in `unit` (`Field.withUnit`), or with none (`withoutUnit`).
    pub fn with_unit(mut self, unit: Option<UnitOfMeasure>) -> Field {
        self.unit = unit;
        self
    }

    /// The field labelled `label` (`Field.withLabel`), or with none (`withoutLabel`).
    pub fn with_label(mut self, label: Option<&str>) -> Field {
        self.label = label.map(str::to_string);
        self
    }

    /// The field described by `description` (`Field.withDescription`), or with none.
    pub fn with_description(mut self, description: Option<&str>) -> Field {
        self.description = description.map(str::to_string);
        self
    }

    /// The field with the extension member `key` set to `value`, replacing one of
    /// that key (`Field.withExt`).
    pub fn with_ext(mut self, key: &str, value: &str) -> Field {
        match self.ext.binary_search_by(|(k, _)| ordinal_cmp(k, key)) {
            Ok(i) => self.ext[i].1 = value.to_string(),
            Err(i) => self.ext.insert(i, (key.to_string(), value.to_string())),
        }
        self
    }

    /// The field without the extension member `key` (`Field.withoutExt`).
    pub fn without_ext(mut self, key: &str) -> Field {
        self.ext.retain(|(k, _)| k != key);
        self
    }

    /// Whether the field states any metadata (`Field.hasMetadata`).
    pub fn has_metadata(&self) -> bool {
        self.unit.is_some()
            || self.label.is_some()
            || self.description.is_some()
            || !self.ext.is_empty()
    }
}

/// An ordered schema: one field per column.
pub type Schema = Vec<Field>;

/// One schema entry as the wire carries it (`ColumnCodec.fieldJson`): `name` and
/// `type`, then exactly the metadata members the field states — `unit` as its
/// canonical text, `label`, `description`, and `ext` as an object of strings.
pub fn field_json(f: &Field) -> JVal {
    let mut members = vec![
        ("name".to_string(), JVal::Str(f.name.clone())),
        ("type".to_string(), JVal::Str(f.ty.tag().to_string())),
    ];
    if let Some(u) = &f.unit {
        members.push(("unit".to_string(), JVal::Str(unit::render(u))));
    }
    if let Some(l) = &f.label {
        members.push(("label".to_string(), JVal::Str(l.clone())));
    }
    if let Some(d) = &f.description {
        members.push(("description".to_string(), JVal::Str(d.clone())));
    }
    if !f.ext.is_empty() {
        let ext = f
            .ext
            .iter()
            .map(|(k, v)| (k.clone(), JVal::Str(v.clone())))
            .collect();
        members.push(("ext".to_string(), JVal::Obj(ext)));
    }
    JVal::Obj(members)
}

/// A schema as its canonical wire array.
pub fn schema_json(s: &[Field]) -> JVal {
    JVal::Arr(s.iter().map(field_json).collect())
}

/// An optional string member of a schema entry: absent is `None`; present and
/// not a string is refused naming the column and the member.
fn optional_text(column: &str, name: &str, el: &Value) -> Result<Option<String>, ColumnError> {
    match column::try_member(name, el) {
        None => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ColumnError::MalformedShape(format!(
            "{column}: schema entry member '{name}' must be a string, got {}",
            other.kind_name()
        ))),
    }
}

/// A schema entry from the wire (`ColumnCodec.decodeField`): `name` and `type`,
/// then the metadata members when present — `unit` parsed and REFUSED where it is
/// not a unit the algebra parses (a parseable spelling is read to the unit, so it
/// re-encodes canonical), `label` and `description` strings, `ext` an object of
/// strings. A member it does not know is read past.
pub fn decode_field(el: &Value) -> Result<Field, ColumnError> {
    let name = column::as_str("schema.name", column::get_field("name", el)?)?;
    let tag = column::as_str("schema.type", column::get_field("type", el)?)?;
    let ty = ColumnType::of_tag(tag).ok_or_else(|| ColumnError::UnknownType {
        got: tag.to_string(),
        expected: ColumnType::ALL.to_vec(),
    })?;
    let mut f = Field::create(name, ty);
    if let Some(text) = optional_text(name, "unit", el)? {
        match unit::parse(&text) {
            Ok(u) => f = f.with_unit(Some(u)),
            Err(r) => {
                return Err(ColumnError::MalformedShape(format!(
                    "{name}: unit '{text}' is not a unit the kit parses: {}",
                    r.text()
                )));
            }
        }
    }
    if let Some(l) = optional_text(name, "label", el)? {
        f = f.with_label(Some(&l));
    }
    if let Some(d) = optional_text(name, "description", el)? {
        f = f.with_description(Some(&d));
    }
    match column::try_member("ext", el) {
        None => {}
        Some(Value::Obj(members)) => {
            for (k, v) in members {
                match v {
                    Value::Str(s) => f = f.with_ext(k, s),
                    other => {
                        return Err(ColumnError::MalformedShape(format!(
                            "{name}: schema entry extension member '{k}' must be a string, got {}",
                            other.kind_name()
                        )));
                    }
                }
            }
        }
        Some(other) => {
            return Err(ColumnError::MalformedShape(format!(
                "{name}: schema entry member 'ext' must be an object, got {}",
                other.kind_name()
            )));
        }
    }
    Ok(f)
}

/// A schema array from the wire, each entry through [`decode_field`].
pub fn decode_schema(el: &Value) -> Result<Schema, ColumnError> {
    column::as_arr("schema", el)?
        .iter()
        .map(decode_field)
        .collect()
}

// ─── The schema delta ────────────────────────────────────────────────────────

/// One change to the metadata of a field common to both schemas — twin of
/// `FieldChange`: the member's value before and after (`None` where unstated).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldChange {
    Unit(Option<UnitOfMeasure>, Option<UnitOfMeasure>),
    Label(Option<String>, Option<String>),
    Description(Option<String>, Option<String>),
    Ext(String, Option<String>, Option<String>),
}

/// A structured schema delta — twin of `SchemaDelta`.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDelta {
    /// Columns only the target holds, in target order.
    pub added: Vec<Field>,
    /// Columns only the old schema holds, in old order, as the old schema spelled them.
    pub removed: Vec<Field>,
    /// Columns in both whose type changed, in old order, as `(name, from, to)`.
    pub retyped: Vec<(String, ColumnType, ColumnType)>,
    /// Columns in both whose metadata changed, in old order, one entry per member.
    pub amended: Vec<(String, FieldChange)>,
    /// Whether the common columns appear in a different relative order.
    pub reordered: bool,
    /// The target's column order, or empty when it is the order `patch` derives.
    pub order: Vec<String>,
}

/// Why [`patch`] refused a delta — twin of `SchemaError`.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaError {
    DuplicateColumn(String),
    AbsentColumn(String),
    TypeDisagrees(String, ColumnType, ColumnType),
    MetadataDisagrees(String, FieldChange),
    AlreadyPresent(String),
    OrderMismatch(Vec<String>, Vec<String>),
}

/// A compatibility verdict relative to the depended-on columns — twin of `SchemaCompat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaCompat {
    Compatible,
    Breaking(Vec<String>),
    Unknown(Vec<String>),
}

impl SchemaCompat {
    /// `compatible`, `breaking` or `unknown`.
    pub fn tag(&self) -> &'static str {
        match self {
            SchemaCompat::Compatible => "compatible",
            SchemaCompat::Breaking(_) => "breaking",
            SchemaCompat::Unknown(_) => "unknown",
        }
    }
}

/// The metadata changes between two fields of one name, in member order: unit,
/// label, description, then each extension key in ordinal order.
fn changes(before: &Field, after: &Field) -> Vec<FieldChange> {
    let mut out = Vec::new();
    if before.unit != after.unit {
        out.push(FieldChange::Unit(before.unit.clone(), after.unit.clone()));
    }
    if before.label != after.label {
        out.push(FieldChange::Label(
            before.label.clone(),
            after.label.clone(),
        ));
    }
    if before.description != after.description {
        out.push(FieldChange::Description(
            before.description.clone(),
            after.description.clone(),
        ));
    }
    let mut keys: Vec<&str> = before
        .ext
        .iter()
        .chain(after.ext.iter())
        .map(|(k, _)| k.as_str())
        .collect();
    keys.sort_by(|a, b| ordinal_cmp(a, b));
    keys.dedup();
    for k in keys {
        let (b, a) = (before.ext_value(k), after.ext_value(k));
        if b != a {
            out.push(FieldChange::Ext(
                k.to_string(),
                b.map(str::to_string),
                a.map(str::to_string),
            ));
        }
    }
    out
}

fn find<'a>(s: &'a [Field], name: &str) -> Option<&'a Field> {
    s.iter().find(|f| f.name == name)
}

/// The structured delta from `old` to `target` (`Schema.diff`; total). A field
/// name is looked up as the LAST field of that name, as the reference's map is.
pub fn diff(old: &[Field], target: &[Field]) -> SchemaDelta {
    let last = |s: &[Field], name: &str| s.iter().rev().find(|f| f.name == name).cloned();
    let in_old = |n: &str| old.iter().any(|f| f.name == n);
    let in_new = |n: &str| target.iter().any(|f| f.name == n);
    let added: Vec<Field> = target
        .iter()
        .filter(|f| !in_old(&f.name))
        .cloned()
        .collect();
    let removed: Vec<Field> = old.iter().filter(|f| !in_new(&f.name)).cloned().collect();
    let retyped = old
        .iter()
        .filter_map(|f| match last(target, &f.name) {
            Some(nf) if nf.ty != f.ty => Some((f.name.clone(), f.ty, nf.ty)),
            _ => None,
        })
        .collect();
    let amended = old
        .iter()
        .flat_map(|f| match last(target, &f.name) {
            Some(nf) => changes(f, &nf)
                .into_iter()
                .map(|c| (f.name.clone(), c))
                .collect(),
            None => Vec::new(),
        })
        .collect();
    let common_old: Vec<String> = old
        .iter()
        .filter(|f| in_new(&f.name))
        .map(|f| f.name.clone())
        .collect();
    let common_new: Vec<String> = target
        .iter()
        .filter(|f| in_old(&f.name))
        .map(|f| f.name.clone())
        .collect();
    let mut derived = common_old.clone();
    derived.extend(added.iter().map(|f| f.name.clone()));
    let target_order: Vec<String> = target.iter().map(|f| f.name.clone()).collect();
    SchemaDelta {
        added,
        removed,
        retyped,
        amended,
        reordered: common_old != common_new,
        order: if target_order == derived {
            Vec::new()
        } else {
            target_order
        },
    }
}

/// The delta that changes nothing (`Schema.identityDelta`).
pub fn identity_delta() -> SchemaDelta {
    SchemaDelta {
        added: Vec::new(),
        removed: Vec::new(),
        retyped: Vec::new(),
        amended: Vec::new(),
        reordered: false,
        order: Vec::new(),
    }
}

/// The field `change` leaves when applied to `f`, or `None` where `f` does not
/// carry the change's `before`.
fn apply_change(change: &FieldChange, f: &Field) -> Option<Field> {
    let f = f.clone();
    match change {
        FieldChange::Unit(before, after) if f.unit == *before => Some(f.with_unit(after.clone())),
        FieldChange::Label(before, after) if f.label == *before => {
            Some(f.with_label(after.as_deref()))
        }
        FieldChange::Description(before, after) if f.description == *before => {
            Some(f.with_description(after.as_deref()))
        }
        FieldChange::Ext(key, before, after) if f.ext_value(key) == before.as_deref() => {
            Some(match after {
                Some(v) => f.with_ext(key, v),
                None => f.without_ext(key),
            })
        }
        _ => None,
    }
}

/// Apply a delta to a schema (`Schema.patch`): removals, retypes, amendments,
/// additions, then the order — each checked against what the delta recorded, and
/// anything else a named [`SchemaError`], never a repair. `patch old (diff old
/// target) = Ok target` for schemas that name no column twice.
pub fn patch(old: &[Field], delta: &SchemaDelta) -> Result<Schema, SchemaError> {
    if let Some(n) = column::first_duplicate(old.iter().map(|f| f.name.as_str())) {
        return Err(SchemaError::DuplicateColumn(n.to_string()));
    }
    let mut s: Schema = old.to_vec();
    let replace = |s: &mut Schema, name: &str, f: Field| {
        for g in s.iter_mut() {
            if g.name == name {
                *g = f.clone();
            }
        }
    };
    for r in &delta.removed {
        match find(&s, &r.name) {
            None => return Err(SchemaError::AbsentColumn(r.name.clone())),
            Some(f) if f.ty != r.ty => {
                return Err(SchemaError::TypeDisagrees(r.name.clone(), r.ty, f.ty));
            }
            Some(_) => s.retain(|f| f.name != r.name),
        }
    }
    for (name, from, to) in &delta.retyped {
        match find(&s, name) {
            None => return Err(SchemaError::AbsentColumn(name.clone())),
            Some(f) if f.ty != *from => {
                return Err(SchemaError::TypeDisagrees(name.clone(), *from, f.ty));
            }
            Some(f) => {
                let g = f.clone().with_type(*to);
                replace(&mut s, name, g);
            }
        }
    }
    for (name, change) in &delta.amended {
        match find(&s, name) {
            None => return Err(SchemaError::AbsentColumn(name.clone())),
            Some(f) => match apply_change(change, f) {
                None => {
                    return Err(SchemaError::MetadataDisagrees(name.clone(), change.clone()));
                }
                Some(g) => replace(&mut s, name, g),
            },
        }
    }
    for a in &delta.added {
        if find(&s, &a.name).is_some() {
            return Err(SchemaError::AlreadyPresent(a.name.clone()));
        }
        s.push(a.clone());
    }
    if delta.order.is_empty() {
        return Ok(s);
    }
    let names: Vec<String> = s.iter().map(|f| f.name.clone()).collect();
    let key = |xs: &[String]| {
        let mut v = xs.to_vec();
        v.sort_by(|a, b| ordinal_cmp(a, b));
        v
    };
    if delta.order.len() != names.len() || key(&delta.order) != key(&names) {
        return Err(SchemaError::OrderMismatch(names, delta.order.clone()));
    }
    Ok(delta
        .order
        .iter()
        .filter_map(|n| find(&s, n).cloned())
        .collect())
}

fn unit_text(u: &Option<UnitOfMeasure>) -> String {
    u.as_ref().map_or_else(|| "none".to_string(), unit::render)
}

/// Classify a delta against the columns a consumer depends on
/// (`Schema.classify`): a removed depended-on column, a depended-on retype that is
/// not a widening, or a depended-on column whose stated unit was replaced or
/// withdrawn is `Breaking`; a depended-on column's extension change is `Unknown`;
/// everything else is compatible. `Breaking` outranks `Unknown`.
pub fn classify(depends_on: &[&str], delta: &SchemaDelta) -> SchemaCompat {
    let dep = |n: &str| depends_on.contains(&n);
    let mut breaking: Vec<String> = delta
        .removed
        .iter()
        .filter(|f| dep(&f.name))
        .map(|f| format!("depended-on column removed: {} ({})", f.name, f.ty.tag()))
        .collect();
    breaking.extend(
        delta
            .retyped
            .iter()
            .filter(|(n, o, t)| dep(n) && !o.widens(*t))
            .map(|(n, o, t)| {
                format!(
                    "depended-on column {n} retyped {} → {} (not a safe widening)",
                    o.tag(),
                    t.tag()
                )
            }),
    );
    breaking.extend(delta.amended.iter().filter_map(|(n, c)| match c {
        FieldChange::Unit(Some(before), after) if dep(n) => Some(format!(
            "depended-on column {n} unit changed {} → {} (its values no longer mean what was read)",
            unit::render(before),
            unit_text(after)
        )),
        _ => None,
    }));
    let unknown: Vec<String> = delta
        .amended
        .iter()
        .filter_map(|(n, c)| match c {
            FieldChange::Ext(key, _, _) if dep(n) => Some(format!(
                "depended-on column {n} extension member {key} changed (not interpreted by the kit)"
            )),
            _ => None,
        })
        .collect();
    match (breaking.is_empty(), unknown.is_empty()) {
        (true, true) => SchemaCompat::Compatible,
        (true, false) => SchemaCompat::Unknown(unknown),
        (false, _) => SchemaCompat::Breaking(breaking),
    }
}

/// The pre-image element a field's metadata contributes: the unit's text, the
/// label and the description, each tagged so an absent member and an empty one
/// differ, then each extension key and value, through [`canonical_fields`].
fn metadata_preimage(f: &Field) -> String {
    let stated = |tag: &str, v: Option<String>| v.map_or_else(String::new, |s| format!("{tag}{s}"));
    let mut parts = vec![
        stated("u", f.unit.as_ref().map(unit::render)),
        stated("l", f.label.clone()),
        stated("d", f.description.clone()),
    ];
    for (k, v) in &f.ext {
        parts.push(k.clone());
        parts.push(v.clone());
    }
    canonical_fields(&parts)
}

/// The stable cross-host fingerprint of a schema (`Schema.fingerprint`): FNV-1a
/// over the canonical pre-image of each field's `name:type`, followed, for a
/// field that states metadata, by its metadata element. Order-sensitive.
pub fn fingerprint(s: &[Field]) -> String {
    let mut parts = Vec::new();
    for f in s {
        parts.push(format!("{}:{}", f.name, f.ty.tag()));
        if f.has_metadata() {
            parts.push(metadata_preimage(f));
        }
    }
    fnv1a(&canonical_fields(&parts))
}

// ─── The delta codec ─────────────────────────────────────────────────────────

fn change_json(name: &str, change: &FieldChange) -> JVal {
    let text = |v: &Option<String>| v.clone();
    let (member, key, from, to) = match change {
        FieldChange::Unit(b, a) => (
            "unit",
            None,
            b.as_ref().map(unit::render),
            a.as_ref().map(unit::render),
        ),
        FieldChange::Label(b, a) => ("label", None, text(b), text(a)),
        FieldChange::Description(b, a) => ("description", None, text(b), text(a)),
        FieldChange::Ext(k, b, a) => ("ext", Some(k.clone()), text(b), text(a)),
    };
    let mut members = vec![
        ("name".to_string(), JVal::Str(name.to_string())),
        ("member".to_string(), JVal::Str(member.to_string())),
    ];
    if let Some(k) = key {
        members.push(("key".to_string(), JVal::Str(k)));
    }
    if let Some(f) = from {
        members.push(("from".to_string(), JVal::Str(f)));
    }
    if let Some(t) = to {
        members.push(("to".to_string(), JVal::Str(t)));
    }
    JVal::Obj(members)
}

/// A delta as a value (`SchemaDeltaCodec.encodeJson`): `added`, `removed`,
/// `retyped`, `reordered` and `order` always, `amended` only when non-empty.
pub fn delta_json(d: &SchemaDelta) -> JVal {
    let mut members = vec![
        (
            "added".to_string(),
            JVal::Arr(d.added.iter().map(field_json).collect()),
        ),
        (
            "removed".to_string(),
            JVal::Arr(d.removed.iter().map(field_json).collect()),
        ),
        (
            "retyped".to_string(),
            JVal::Arr(
                d.retyped
                    .iter()
                    .map(|(n, from, to)| {
                        JVal::Obj(vec![
                            ("name".to_string(), JVal::Str(n.clone())),
                            ("from".to_string(), JVal::Str(from.tag().to_string())),
                            ("to".to_string(), JVal::Str(to.tag().to_string())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("reordered".to_string(), JVal::Bool(d.reordered)),
        (
            "order".to_string(),
            JVal::Arr(d.order.iter().map(|n| JVal::Str(n.clone())).collect()),
        ),
    ];
    if !d.amended.is_empty() {
        members.push((
            "amended".to_string(),
            JVal::Arr(d.amended.iter().map(|(n, c)| change_json(n, c)).collect()),
        ));
    }
    JVal::Obj(members)
}

/// The canonical wire string for a delta (`SchemaDeltaCodec.encode`; total).
pub fn encode_delta(d: &SchemaDelta) -> String {
    json::render_canonical(&delta_json(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delta_replays() {
        let kg = unit::parse("kg").unwrap();
        let old = vec![Field::create("m", ColumnType::Float).with_unit(Some(kg))];
        let target = vec![
            Field::create("m", ColumnType::Decimal).with_ext("v.k", "1"),
            Field::create("n", ColumnType::Int),
        ];
        let d = diff(&old, &target);
        assert_eq!(patch(&old, &d), Ok(target.clone()));
        assert_eq!(diff(&target, &target), identity_delta());
    }
}
