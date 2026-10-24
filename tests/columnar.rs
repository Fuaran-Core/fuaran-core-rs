//! Certifies the columnar strand against the vectors the reference emits: the
//! `column` vectors of `refusals/codec-refusals.json`, the codec and aggregate
//! cases of `laws/decimal-laws.json`, and the whole of `laws/field-laws.json`.
//! Each leg counts the vectors it ran and asserts that count exactly.

mod common;

use common::*;
use fuaran_core::column::{self, AggFn, Cell, Column, ColumnError, ColumnType, DataSource, Table};
use fuaran_core::field::{self, Field, Schema};
use fuaran_core::json::{JVal, render_canonical};

/// A decode's outcome as the vectors spell it: the canonical re-encoding, or the
/// refusal's class (`NotJson/<kind>` for a parse failure where `kinded`).
fn decode_outcome(text: &str, kinded: bool) -> Result<String, String> {
    match column::decode(text) {
        Ok(src) => Ok(column::encode(&src)),
        Err(ColumnError::NotJson(e)) if kinded => Err(format!("NotJson/{}", e.kind.as_str())),
        Err(e) => Err(e.class().to_string()),
    }
}

/// A `{kind, text}` cell of the decimal documents.
fn cell_of(v: &JVal) -> Cell {
    match s(v, "kind") {
        "null" => Cell::Null,
        "int" => Cell::Int(s(v, "text").parse().expect("an int cell's text")),
        "decimal" => Cell::Decimal(s(v, "text").to_string()),
        other => panic!("the decimal documents write no {other} cell"),
    }
}

/// A schema value as the wire spells it, read through the codec's entry reader.
fn schema_of(v: &JVal) -> Schema {
    let value = column::read_value(&render_canonical(v)).expect("a schema is JSON");
    field::decode_schema(&value).expect("a schema decodes")
}

/// The accept/reject verdict of a decimal or field vector's `expected`.
fn expect_outcome(expected: &JVal, member: &str) -> Result<String, String> {
    match s(expected, "verdict") {
        "accept" => Ok(s(expected, member).to_string()),
        "reject" => Err(s(expected, "error").to_string()),
        other => panic!("unknown verdict {other}"),
    }
}

#[test]
fn codec_refusals_column() {
    let doc = read("refusals/codec-refusals.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        if s(v, "codec") != "column" {
            continue;
        }
        let name = s(v, "name");
        let expected = match v.field("refusal") {
            Some(JVal::Str(r)) => Err(r.clone()),
            _ => Ok(s(v, "canonical").to_string()),
        };
        assert_eq!(decode_outcome(s(v, "input"), true), expected, "{name}");
        ran += 1;
    }
    assert_eq!(ran, 26, "the column vectors run");
}

#[test]
fn decimal_codec_and_aggregates() {
    let doc = read("laws/decimal-laws.json");
    let (mut decoded, mut encoded, mut aggregated) = (0, 0, 0);
    for v in vectors(&doc) {
        let id = s(v, "id");
        let input = get(v, "input");
        let expected = get(v, "expected");
        match s(v, "case") {
            "codecDecode" => {
                let got = decode_outcome(s(input, "document"), false);
                assert_eq!(got, expect_outcome(expected, "canonical"), "{id}");
                decoded += 1;
            }
            "codecEncode" => {
                let cells: Vec<Cell> = arr(input, "cells").iter().map(cell_of).collect();
                let got = Column::of_cells("c", ColumnType::Decimal, &cells)
                    .and_then(|col| {
                        column::try_encode(&DataSource::Embedded(Table {
                            schema: vec![Field::create("c", ColumnType::Decimal)],
                            columns: vec![col],
                        }))
                    })
                    .map_err(|e| e.class().to_string());
                assert_eq!(got, expect_outcome(expected, "canonical"), "{id}");
                encoded += 1;
            }
            "aggregate" => {
                let agg = AggFn::of_tag(s(input, "fn")).expect("a known aggregate");
                let cells: Vec<Cell> = arr(input, "cells").iter().map(cell_of).collect();
                // A cell of another type is refused at construction, which the
                // exporter reports as the aggregate's `CellOutsideType`.
                let got = match Column::of_cells("c", ColumnType::Decimal, &cells) {
                    Ok(col) => col
                        .aggregate(agg)
                        .map(|c| c.token())
                        .map_err(|e| e.class().to_string()),
                    Err(ColumnError::TypeMismatch { .. }) => Err("CellOutsideType".to_string()),
                    Err(e) => panic!("{id}: decimal column c did not build: {e:?}"),
                };
                assert_eq!(got, expect_outcome(expected, "token"), "{id}");
                aggregated += 1;
            }
            _ => {}
        }
    }
    assert_eq!((decoded, encoded, aggregated), (10, 2, 10));
    assert_eq!(decoded + encoded + aggregated, 22);
}

#[test]
fn field_laws() {
    let doc = read("laws/field-laws.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        let id = s(v, "id");
        let input = get(v, "input");
        let expected = get(v, "expected");
        match s(v, "case") {
            "fieldEncode" => {
                // The input is the entry's members; a field is built from them by
                // its builders, its unit through the parser.
                let ty = ColumnType::of_tag(s(input, "type")).expect("a known type");
                let mut f = Field::create(s(input, "name"), ty);
                if let Some(JVal::Str(u)) = input.field("unit") {
                    f = f.with_unit(Some(fuaran_core::unit::parse(u).expect("a unit")));
                }
                if let Some(JVal::Str(l)) = input.field("label") {
                    f = f.with_label(Some(l));
                }
                if let Some(JVal::Str(d)) = input.field("description") {
                    f = f.with_description(Some(d));
                }
                if let Some(JVal::Obj(members)) = input.field("ext") {
                    for (k, x) in members {
                        match x {
                            JVal::Str(x) => f = f.with_ext(k, x),
                            other => panic!("{id}: an ext member is a string: {other:?}"),
                        }
                    }
                }
                let got = render_canonical(&field::field_json(&f));
                assert_eq!(Ok(got), expect_outcome(expected, "canonical"), "{id}");
            }
            "codecDecode" => {
                let got = decode_outcome(s(input, "document"), false);
                assert_eq!(got, expect_outcome(expected, "canonical"), "{id}");
            }
            "fingerprint" => {
                let got = field::fingerprint(&schema_of(get(input, "schema")));
                assert_eq!(Ok(got), expect_outcome(expected, "fingerprint"), "{id}");
            }
            "delta" => {
                let old = schema_of(get(input, "old"));
                let target = schema_of(get(input, "target"));
                let delta = field::diff(&old, &target);
                assert_eq!(
                    Ok(field::encode_delta(&delta)),
                    expect_outcome(expected, "canonical"),
                    "{id}"
                );
                let patched = field::patch(&old, &delta).expect("the delta replays");
                assert_eq!(
                    render_canonical(&field::schema_json(&patched)),
                    render_canonical(get(expected, "patched")),
                    "{id}: patched"
                );
                assert_eq!(patched, target, "{id}: the replay is the target");
            }
            "classify" => {
                let old = schema_of(get(input, "old"));
                let target = schema_of(get(input, "target"));
                let deps = str_items(input, "dependsOn");
                let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
                let got = field::classify(&deps, &field::diff(&old, &target));
                assert_eq!(
                    Ok(got.tag().to_string()),
                    expect_outcome(expected, "compat"),
                    "{id}"
                );
            }
            other => panic!("{id}: unknown field case {other}"),
        }
        ran += 1;
    }
    assert_eq!(ran, vectors(&doc).len(), "every field vector runs");
    assert_eq!(ran, declared("laws", "fieldLaws"), "the field vectors run");
}
