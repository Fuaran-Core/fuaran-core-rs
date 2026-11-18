//! Port-side tests of the query declaration (DECISIONS.md D14, D17, D18): what
//! the reference's vectors do not reach. Since Core `bd1a339f` (`0.43.1`, draft)
//! the family `queryDeclarationLaws` (tests/query_declaration.rs) pins the code,
//! the path and the error value of every refusal of the declaration reader, the
//! admission gate, registration and validation, so those assertions retired from
//! here (the table is D17.9); since Core `889c869d` (`0.44.0`, draft) it pins the
//! reader's refusals D17.10 held and `-Infinity`'s reason too, and those retired
//! as well (D18.8). What stays is the port's own: each refusal's
//! sentence (`QueryError::describe`, and the reader's message carrying it), which
//! no vector pins, and the few codes, paths and values no vector reaches, each
//! test citing the reference's file and line under `src/Fuaran.Core.Query/`
//! (`Query.fs`, `QueryRegistry.fs`, `QueryCodec.fs`) or
//! `src/Fuaran.Core.Column/Column.fs`. The predicate and order-key documents are
//! also read inside a declaration against the `seamRefusals` vectors that read
//! them alone, so the typed reader answers what the seams' reader answers.

mod common;

use std::sync::Arc;

use common::*;
use fuaran_core::capability::{
    DeterminismFactor, DeterminismSet, EffectClass, Grant, HostEffect, PolicyDecision, PolicyGate,
};
use fuaran_core::column::{Cell, ColumnType, DataSource, TimeUnit};
use fuaran_core::decode::DecodeCode;
use fuaran_core::field::Field;
use fuaran_core::json::{JVal, PathSeg, render_canonical};
use fuaran_core::query::{
    AdmittedQuery, ColumnPredicate, PositiveInt, Query, QueryError, QueryParam, QueryRegistry,
    SortDirection, SortKey,
};

// ─── Fixtures ────────────────────────────────────────────────────────────────

fn param(name: &str, ty: ColumnType, required: bool) -> QueryParam {
    QueryParam {
        name: name.to_string(),
        ty,
        required,
    }
}

/// A read over six result columns, one of each kind a literal is held to.
fn sales() -> Query {
    Query {
        id: "sales".to_string(),
        params: vec![
            param("n", ColumnType::Int, true),
            param("rate", ColumnType::Float, false),
            param("tag", ColumnType::String, false),
        ],
        result_schema: vec![
            Field::create("n", ColumnType::Int),
            Field::create("region", ColumnType::String),
            Field::create("at", ColumnType::Timestamp(TimeUnit::Milliseconds)),
            Field::create("amount", ColumnType::Decimal),
            Field::create("day", ColumnType::Date),
            Field::create("score", ColumnType::Float),
        ],
        effect: EffectClass {
            host: HostEffect::ReadsHost,
            determinism: DeterminismSet::of(&[DeterminismFactor::Network]),
        },
        source: DataSource::Ref("sales".to_string()),
        timeout_ms: None,
        page_size: None,
        filter: Vec::new(),
        order_by: Vec::new(),
    }
}

const COLUMNS: &str = "'n', 'region', 'at', 'amount', 'day', 'score'";

fn with_filter(p: ColumnPredicate) -> Query {
    Query {
        filter: vec![p],
        ..sales()
    }
}

fn eq(column: &str, value: Cell) -> ColumnPredicate {
    ColumnPredicate::EqualTo {
        column: column.to_string(),
        value,
    }
}

fn key(column: &str, direction: SortDirection) -> SortKey {
    SortKey {
        column: column.to_string(),
        direction,
    }
}

/// The sentence of the gate's refusal of `q`: what the port says of it, which no
/// vector pins (D17.9). The refusal's code, path and value are the family's.
fn says(q: &Query, sentence: &str) {
    let e = q.admission_fault().expect("the gate refuses");
    assert_eq!(e.describe(), sentence);
}

/// The declaration the gate admitted: what the writer, the identity and the
/// grant take (D18.2).
fn admitted(q: Query) -> AdmittedQuery {
    q.admit().expect("the gate admits the declaration")
}

/// The declaration reader over `text` refuses with the registry's sentence; its
/// code and path are the family's (`declarationRefused`). No writer takes a
/// declaration the gate refuses (D18.2), so the text of a refused declaration
/// is an admitted declaration's text with members replaced ([`edited_to`]).
fn refused_on_read(text: &str, sentence: &str) {
    let e = AdmittedQuery::decode(text).expect_err("the reader refuses");
    assert_eq!(
        e.message,
        format!("a declaration the registry admits: {sentence}")
    );
}

// ─── The admission gate (QueryRegistry.fs:44-52, Query.fs:70-121) ───────────

/// QueryRegistry.fs:46; Query.fs:208-211.
#[test]
fn a_parameter_declared_twice_is_refused_at_params() {
    let mut q = sales();
    q.params.push(param("rate", ColumnType::String, false));
    // It is checked before the filter: an unknown column is not reached.
    q.filter = vec![ColumnPredicate::IsNull {
        column: "nope".to_string(),
    }];
    let sentence = "Refused: parameter 'rate' is given more than once. Give each parameter once.";
    says(&q, sentence);
    refused_on_read(
        &edited_to(&[
            (
                "params",
                r#"[{"name":"n","required":true,"type":"int"},{"name":"rate","required":false,"type":"float"},{"name":"tag","required":false,"type":"string"},{"name":"rate","required":false,"type":"string"}]"#,
            ),
            ("where", r#"[{"$type":"isNull","column":"nope"}]"#),
        ]),
        sentence,
    );
}

/// Query.fs:76-77, 232-241.
#[test]
fn a_predicate_on_an_undeclared_column_is_refused_naming_the_columns() {
    let q = Query {
        filter: vec![
            ColumnPredicate::IsNotNull {
                column: "n".to_string(),
            },
            ColumnPredicate::IsNull {
                column: "nope".to_string(),
            },
        ],
        ..sales()
    };
    let sentence = format!(
        "Refused: 'nope' is not a column of this query's result. Its columns are {COLUMNS}."
    );
    says(&q, &sentence);
    refused_on_read(
        &edited_to(&[(
            "where",
            r#"[{"$type":"isNotNull","column":"n"},{"$type":"isNull","column":"nope"}]"#,
        )]),
        &sentence,
    );
    // A result with no columns says so.
    let bare = Query {
        result_schema: Vec::new(),
        ..q
    };
    assert_eq!(
        bare.admission_fault().unwrap().describe(),
        "Refused: 'n' is not a column of this query's result. It returns no columns."
    );
}

/// Query.fs:82, 251-258.
#[test]
fn contains_on_a_column_that_is_not_a_string_is_not_applicable() {
    let q = with_filter(ColumnPredicate::Contains {
        column: "n".to_string(),
        text: "1".to_string(),
    });
    let sentence = "Refused: the filter 'contains' does not apply to column 'n', which is int. It applies to string columns only.";
    says(&q, sentence);
    refused_on_read(
        &edited_to(&[("where", r#"[{"$type":"contains","column":"n","text":"1"}]"#)]),
        sentence,
    );
}

/// Query.fs:83-84, 259-264. A declaration's text cannot carry a `null` literal:
/// the strict reader reads no `null` (D13.3), as the reference's does, so this
/// refusal is reached by a declaration built by hand.
#[test]
fn a_null_literal_is_ill_formed() {
    says(
        &with_filter(eq("n", Cell::Null)),
        "Refused: the filter on column 'n' compares with a value the column cannot carry: a null literal compares with nothing; test for absence with isNull.",
    );
}

/// Query.fs:94, 146-158, 242-250.
#[test]
fn a_literal_of_another_type_is_a_type_mismatch_saying_how_to_write_one() {
    let q = with_filter(eq("n", Cell::Str("5".to_string())));
    let sentence = "Refused: the filter on column 'n' compares with a string value; the column is int, and a filter compares with a value of the column's own type.";
    says(&q, sentence);
    refused_on_read(
        &edited_to(&[(
            "where",
            r#"[{"$type":"equalTo","column":"n","type":"string","value":"5"}]"#,
        )]),
        sentence,
    );
    // An integer literal on a decimal column says how to write a decimal.
    let q = with_filter(eq("amount", Cell::Int(5)));
    let sentence = "Refused: the filter on column 'amount' compares with a int value; the column is decimal, and a filter compares with a value of the column's own type. Write a decimal as a JSON string of decimal text, such as \"12.50\": an optional '-', digits, and an optional '.' followed by digits; never as a JSON number.";
    says(&q, sentence);
    refused_on_read(
        &edited_to(&[(
            "where",
            r#"[{"$type":"equalTo","column":"amount","type":"int","value":5}]"#,
        )]),
        sentence,
    );
}

/// Query.fs:86-93 (Phase 422), 146-158.
#[test]
fn a_timestamp_literal_fits_a_column_at_least_as_fine_and_no_finer() {
    // A whole second is a `timestamp` cell, fits a millisecond column, and its
    // declaration reads back to itself.
    let coarse = admitted(with_filter(eq(
        "at",
        Cell::Timestamp("2026-11-15T09:30:00Z".to_string()),
    )));
    assert_eq!(AdmittedQuery::decode(&coarse.encode()).unwrap(), coarse);
    // Microseconds do not.
    let fine = with_filter(eq(
        "at",
        Cell::Timestamp("2026-11-15T09:30:00.123456Z".to_string()),
    ));
    let sentence = "Refused: the filter on column 'at' compares with a timestamp_us value; the column is timestamp_ms, and a filter compares with a value of the column's own type. Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ, or YYYY-MM-DDThh:mm:ss.FZ with up to 3 fraction digits and no trailing zero.";
    says(&fine, sentence);
    refused_on_read(
        &edited_to(&[(
            "where",
            r#"[{"$type":"equalTo","column":"at","type":"timestamp_us","value":"2026-11-15T09:30:00.123456Z"}]"#,
        )]),
        sentence,
    );
}

/// Query.fs:45-67 (`literalReason`, `literalFault`); Column.fs:531, 850. A
/// literal of its column's type that the column cannot carry: the sentence. The
/// refusal and its reason are the family's, `-Infinity`'s among them since Core
/// `889c869d` (`admission-a-float-negative-infinity`, D18.8).
#[test]
fn a_literal_its_column_cannot_carry_is_ill_formed_with_the_column_layers_reason() {
    says(
        &with_filter(eq("day", Cell::Date("2026-02-30".to_string()))),
        "Refused: the filter on column 'day' compares with a value the column cannot carry: day: a date cell must carry a canonical ISO-8601 date, YYYY-MM-DD, naming a day that exists.",
    );
}

/// Query.fs:107-121, 236-241, 265-267.
#[test]
fn an_order_names_declared_columns_each_once() {
    let unknown = Query {
        order_by: vec![key("nope", SortDirection::Ascending)],
        ..sales()
    };
    let sentence = format!(
        "Refused: 'nope' is not a column of this query's result. Its columns are {COLUMNS}."
    );
    says(&unknown, &sentence);
    refused_on_read(
        &edited_to(&[("orderBy", r#"[{"column":"nope","direction":"ascending"}]"#)]),
        &sentence,
    );
    let twice = Query {
        order_by: vec![
            key("n", SortDirection::Descending),
            key("region", SortDirection::Ascending),
            key("n", SortDirection::Ascending),
        ],
        ..sales()
    };
    let sentence = "Refused: the order names column 'n' more than once. Name each column once.";
    says(&twice, sentence);
    refused_on_read(
        &edited_to(&[(
            "orderBy",
            r#"[{"column":"n","direction":"descending"},{"column":"region","direction":"ascending"},{"column":"n","direction":"ascending"}]"#,
        )]),
        sentence,
    );
}

/// Query.fs:171; QueryRegistry.fs:82-84 (`Seam.register`: the id first).
#[test]
fn a_held_id_is_refused_before_the_declaration_is_admitted() {
    let r = QueryRegistry::new().register(sales()).unwrap();
    let mut again = sales();
    again.params.push(param("n", ColumnType::Int, false));
    assert_eq!(
        r.register(again).unwrap_err().describe(),
        "Refused: the query 'sales' is registered twice."
    );
}

// ─── The admitted declaration (D18.2, D18.3) ────────────────────────────────

/// QueryRegistry.fs:58-70 (`admitAt`, `admit`), 105-110 (`tryFind`,
/// `enumerate`): the gate answers the admitted declaration or the refusal
/// `admission_fault` names, and only an admitted declaration is written, keyed
/// or granted. A `Null` literal, which the writer once wrote as the empty
/// string a string column reads back as another filter, is refused before
/// anything can write or key it; the empty string itself is admitted.
#[test]
fn only_the_gate_admits_a_declaration_and_only_an_admitted_one_is_written() {
    let null = with_filter(eq("region", Cell::Null));
    let empty = with_filter(eq("region", Cell::Str(String::new())));
    assert_eq!(
        null.clone().admit().unwrap_err(),
        null.admission_fault().unwrap()
    );
    assert_eq!(
        QueryRegistry::new().register(null.clone()).unwrap_err(),
        null.admission_fault().unwrap()
    );
    let a = admitted(empty.clone());
    assert_eq!(a.declaration(), &empty);
    assert_eq!(a.clone().into_declaration(), empty);
    // Two admissions of one declaration are equal, and its text reads back as it.
    assert_eq!(a, admitted(empty.clone()));
    assert_eq!(AdmittedQuery::decode(&a.encode()).unwrap(), a);
    // The registry answers what its gate admitted.
    let r = QueryRegistry::new().register(empty).unwrap();
    assert_eq!(r.try_find("sales"), Some(&a));
    assert_eq!(r.enumerate(), std::slice::from_ref(&a));
    assert_eq!(r.try_find("nope"), None);
}

// ─── The declaration reader (QueryCodec.fs:144-192) ─────────────────────────

/// The admitted `sales` declaration's text with `member` set to `value` (or
/// removed, for `None`).
fn edited(member: &str, value: Option<&str>) -> String {
    let mut doc = admitted(sales()).encode_json();
    let JVal::Obj(fields) = &mut doc else {
        unreachable!()
    };
    fields.retain(|(k, _)| k != member);
    if let Some(v) = value {
        fields.push((member.to_string(), fuaran_core::json::parse(v).unwrap()));
    }
    render_canonical(&doc)
}

/// The admitted `sales` declaration's text with each member set to its value:
/// how a test writes a declaration the gate refuses, which no writer takes.
fn edited_to(members: &[(&str, &str)]) -> String {
    members
        .iter()
        .fold(admitted(sales()).encode(), |text, (member, value)| {
            let mut doc = fuaran_core::json::parse(&text).unwrap();
            let JVal::Obj(fields) = &mut doc else {
                unreachable!()
            };
            fields.retain(|(k, _)| k != member);
            fields.push((member.to_string(), fuaran_core::json::parse(value).unwrap()));
            render_canonical(&doc)
        })
}

fn refusal(text: &str) -> (DecodeCode, Vec<PathSeg>) {
    let e = AdmittedQuery::decode(text).expect_err("the reader refuses");
    (e.code, e.path)
}

/// QueryTypes.fs:118-138 (`PositiveInt`), QueryCodec.fs:75-83 (`budgetJson`,
/// `budgetOf`): a budget is a positive integer, built only by its constructor,
/// and the writer writes its value. That the reader refuses zero and below as
/// `OutOfRange` at the member, and admits one, is the family's
/// (`declaration-a-timeout-of-zero`, `declaration-budgets-of-one` and their
/// siblings); the sentence is the port's own.
#[test]
fn a_budget_is_a_positive_integer() {
    assert_eq!(PositiveInt::new(0), None);
    assert_eq!(PositiveInt::new(-1), None);
    assert_eq!(PositiveInt::new(i32::MIN), None);
    assert_eq!(PositiveInt::new(1).map(PositiveInt::get), Some(1));
    assert_eq!(
        PositiveInt::new(i32::MAX).map(PositiveInt::get),
        Some(i32::MAX)
    );
    let budgets = admitted(Query {
        timeout_ms: PositiveInt::new(1),
        page_size: PositiveInt::new(2147483647),
        ..sales()
    });
    let text = budgets.encode();
    assert!(text.contains(r#""pageSize":2147483647"#), "{text}");
    assert!(text.ends_with(r#""timeoutMs":1}"#), "{text}");
    assert_eq!(AdmittedQuery::decode(&text).unwrap(), budgets);
    let e = AdmittedQuery::decode(&edited("timeoutMs", Some("0"))).unwrap_err();
    assert_eq!(e.message, "0 is outside [1, 2147483647]");
}

/// QueryCodec.fs:144 (`Decoder.field` / `optField`, which read past every other
/// member): the lenient reading.
#[test]
fn an_unknown_member_is_read_past() {
    let text = edited("note", Some("\"anything\""));
    assert_eq!(AdmittedQuery::decode(&text).unwrap(), admitted(sales()));
}

/// QueryCodec.fs:69-91 (`queryJson`): every member, written canonically, read
/// back to the same declaration.
#[test]
fn a_declaration_writes_its_canonical_text_and_reads_back_to_itself() {
    let q = Query {
        timeout_ms: PositiveInt::new(2500),
        page_size: PositiveInt::new(50),
        filter: vec![
            eq("n", Cell::Int(-3)),
            ColumnPredicate::GreaterThan {
                column: "score".to_string(),
                value: Cell::Float(1.5),
            },
            ColumnPredicate::AtLeast {
                column: "amount".to_string(),
                value: Cell::Decimal("12.5".to_string()),
            },
            ColumnPredicate::LessThan {
                column: "day".to_string(),
                value: Cell::Date("2026-11-15".to_string()),
            },
            ColumnPredicate::AtMost {
                column: "at".to_string(),
                value: Cell::Timestamp("2026-11-15T09:30:00.25Z".to_string()),
            },
            ColumnPredicate::Contains {
                column: "region".to_string(),
                text: "eu".to_string(),
            },
            ColumnPredicate::IsNull {
                column: "day".to_string(),
            },
            ColumnPredicate::IsNotNull {
                column: "n".to_string(),
            },
        ],
        order_by: vec![
            key("region", SortDirection::Ascending),
            key("n", SortDirection::Descending),
        ],
        ..sales()
    };
    let q = admitted(q);
    let text = q.encode();
    assert_eq!(
        text,
        concat!(
            r#"{"$type":"query","effect":{"determinism":"network","host":"readsHost"},"id":"sales","#,
            r#""orderBy":[{"column":"region","direction":"ascending"},{"column":"n","direction":"descending"}],"#,
            r#""pageSize":50,"params":[{"name":"n","required":true,"type":"int"},{"name":"rate","required":false,"type":"float"},{"name":"tag","required":false,"type":"string"}],"#,
            r#""resultSchema":[{"name":"n","type":"int"},{"name":"region","type":"string"},{"name":"at","type":"timestamp_ms"},{"name":"amount","type":"decimal"},{"name":"day","type":"date"},{"name":"score","type":"float"}],"#,
            r#""source":{"ref":"sales","schema":[]},"timeoutMs":2500,"#,
            r#""where":[{"$type":"equalTo","column":"n","type":"int","value":-3},{"$type":"greaterThan","column":"score","type":"float","value":1.5},"#,
            r#"{"$type":"atLeast","column":"amount","type":"decimal","value":"12.5"},{"$type":"lessThan","column":"day","type":"date","value":"2026-11-15"},"#,
            r#"{"$type":"atMost","column":"at","type":"timestamp_ms","value":"2026-11-15T09:30:00.25Z"},{"$type":"contains","column":"region","text":"eu"},"#,
            r#"{"$type":"isNull","column":"day"},{"$type":"isNotNull","column":"n"}]}"#
        )
    );
    assert_eq!(AdmittedQuery::decode(&text).unwrap(), q);
    // Absent and empty members are not written.
    assert!(!admitted(sales()).encode().contains("where"));
    assert!(!admitted(sales()).encode().contains("timeoutMs"));
}

/// The reader reads a literal through the column codec at the type its document
/// states and holds the cell it reads (QueryTypes.fs:323-350): an integer token
/// at a float column is a float, decimal text is canonicalised.
#[test]
fn a_literal_is_read_as_the_cell_its_stated_type_reads() {
    let text = edited(
        "where",
        Some(
            "[{\"$type\":\"equalTo\",\"column\":\"score\",\"type\":\"float\",\"value\":2},\
              {\"$type\":\"equalTo\",\"column\":\"amount\",\"type\":\"decimal\",\"value\":\"12.50\"}]",
        ),
    );
    let q = AdmittedQuery::decode(&text).unwrap();
    assert_eq!(
        q.declaration().filter,
        vec![
            eq("score", Cell::Float(2.0)),
            eq("amount", Cell::Decimal("12.5".to_string())),
        ]
    );
}

/// The `seamRefusals` vectors that read a predicate or an order key alone, read
/// again as a declaration's `where` and `orderBy` item: the typed reader is the
/// seams' reader, so a refusal is the same code at the same path below the
/// item, and an order key reads back to the same canonical text.
#[test]
fn the_seam_vectors_for_a_predicate_and_an_order_key_hold_inside_a_declaration() {
    let doc = read("refusals/seam-refusals.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (member, schema) = match s(v, "reader") {
            "columnPredicate" => (
                "where",
                "[{\"name\":\"revenue\",\"type\":\"float\"},{\"name\":\"region\",\"type\":\"string\"}]",
            ),
            "sortKey" => (
                "orderBy",
                "[{\"name\":\"revenue\",\"type\":\"float\"},{\"name\":\"region\",\"type\":\"string\"}]",
            ),
            _ => continue,
        };
        let text = edited(member, Some(&format!("[{}]", s(v, "document"))));
        let text = render_canonical(&{
            let mut d = fuaran_core::json::parse(&text).unwrap();
            let JVal::Obj(fields) = &mut d else {
                unreachable!()
            };
            for (k, val) in fields.iter_mut() {
                if k == "resultSchema" {
                    *val = fuaran_core::json::parse(schema).unwrap();
                }
            }
            d
        });
        match v.field("canonical") {
            Some(JVal::Str(canonical)) => {
                let q = AdmittedQuery::decode(&text)
                    .unwrap_or_else(|e| panic!("{}: {e:?}", s(v, "name")));
                assert_eq!(
                    render_canonical(&q.declaration().order_by[0].to_json()),
                    *canonical,
                    "{}",
                    s(v, "name")
                );
            }
            _ => {
                let (code, at) = refusal(&text);
                let mut expected = vec![PathSeg::Key(member.to_string()), PathSeg::Index(0)];
                expected.extend(arr(v, "path").iter().map(|p| match p {
                    JVal::Str(k) => PathSeg::Key(k.clone()),
                    JVal::Num(i) => PathSeg::Index(*i as usize),
                    other => panic!("{other:?}"),
                }));
                assert_eq!(
                    (code.as_str(), at),
                    (s(v, "code"), expected),
                    "{}",
                    s(v, "name")
                );
            }
        }
        ran += 1;
    }
    assert_eq!(ran, 6, "the lenient columnPredicate and sortKey vectors");
}

// ─── Validation (Query.fs:424-470) ──────────────────────────────────────────

fn args(xs: &[(&str, Cell)]) -> Vec<(String, Cell)> {
    xs.iter().map(|(n, c)| (n.to_string(), c.clone())).collect()
}

/// Query.fs:424-470, 172-211: each refusal's sentence, in the order the reference
/// checks. Which refusal comes first, and its value, are the family's
/// (`validateParams`, D17.9).
#[test]
fn the_arguments_are_held_to_the_parameters_first_refusal_first() {
    let q = sales();
    let refuse = |xs: &[(&str, Cell)]| q.validate_params(&args(xs)).unwrap_err();
    let e = refuse(&[
        ("n", Cell::Int(1)),
        ("nope", Cell::Int(1)),
        ("n", Cell::Int(2)),
    ]);
    assert_eq!(
        e.describe(),
        "Refused: parameter 'n' is given more than once. Give each parameter once."
    );
    let e = refuse(&[("n", Cell::Int(1)), ("nope", Cell::Int(1))]);
    assert_eq!(
        e.describe(),
        "Refused: 'nope' is not a parameter of this query. Its parameters are 'n', 'rate', 'tag'."
    );
    let e = refuse(&[("n", Cell::Str("1".to_string()))]);
    assert_eq!(
        e.describe(),
        "Refused: parameter 'n' must be int; you sent string."
    );
    let e = refuse(&[("rate", Cell::Float(1.0))]);
    assert_eq!(e.describe(), "Refused: required parameters missing: 'n'.");
    let e = refuse(&[("n", Cell::Null)]);
    assert_eq!(
        e.describe(),
        "Refused: required parameters bound to no value: 'n'."
    );
    let none = Query {
        params: Vec::new(),
        ..sales()
    };
    assert_eq!(
        none.validate_params(&args(&[("x", Cell::Int(1))]))
            .unwrap_err()
            .describe(),
        "Refused: 'x' is not a parameter of this query. It takes no parameters."
    );
}

/// Query.fs:146-158: how a model is told to write a value no JSON scalar carries.
#[test]
fn a_type_mismatch_says_how_to_write_a_decimal_a_date_or_a_timestamp() {
    let mismatch = |expected| QueryError::ParamTypeMismatch {
        name: "p".to_string(),
        expected,
        got: ColumnType::Float,
    };
    assert_eq!(
        mismatch(ColumnType::Date).describe(),
        "Refused: parameter 'p' must be date; you sent float. Write a date as a JSON string, YYYY-MM-DD."
    );
    assert_eq!(
        mismatch(ColumnType::Timestamp(TimeUnit::Seconds)).describe(),
        "Refused: parameter 'p' must be timestamp; you sent float. Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ."
    );
    assert_eq!(
        mismatch(ColumnType::Timestamp(TimeUnit::Nanoseconds)).describe(),
        "Refused: parameter 'p' must be timestamp_ns; you sent float. Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ, or YYYY-MM-DDThh:mm:ss.FZ with up to 9 fraction digits and no trailing zero."
    );
}

// ─── The identity (Query.fs:340-411) ────────────────────────────────────────

/// Query.fs:340-355 (`cellFields`, through `FloatLayout.canonical`): a float's
/// payload collapses a negative zero and spells a non-finite float as its
/// quoted token; equal names keep their list order.
#[test]
fn a_floats_payload_is_its_canonical_layout() {
    let q = admitted(sales());
    let id = |c: Cell| q.invocation_identity(&args(&[("rate", c)]));
    assert_eq!(id(Cell::Float(-0.0)), "sales\u{1}rate\u{1}f\u{1}0\u{1}");
    assert_eq!(
        id(Cell::Float(1e300)),
        "sales\u{1}rate\u{1}f\u{1}1E+300\u{1}"
    );
    assert_eq!(
        id(Cell::Float(f64::NAN)),
        "sales\u{1}rate\u{1}f\u{1}\"NaN\"\u{1}"
    );
    assert_eq!(
        id(Cell::Float(f64::INFINITY)),
        "sales\u{1}rate\u{1}f\u{1}\"Infinity\"\u{1}"
    );
    assert_eq!(id(Cell::Bool(false)), "sales\u{1}rate\u{1}b\u{1}0\u{1}");
    let twice = q.invocation_identity(&args(&[
        ("n", Cell::Int(2)),
        ("a", Cell::Null),
        ("n", Cell::Int(1)),
    ]));
    assert_eq!(
        twice,
        "sales\u{1}a\u{1}n\u{1}\u{1}n\u{1}i\u{1}2\u{1}n\u{1}i\u{1}1\u{1}"
    );
}

/// D261.3: an approval of a read is for its filter and order too, so two
/// declarations under one id that filter differently mint two identities.
#[test]
fn the_filter_and_the_order_are_in_the_identity() {
    let plain = admitted(sales());
    let filtered = admitted(with_filter(eq("n", Cell::Int(2))));
    let ordered = admitted(Query {
        order_by: vec![key("n", SortDirection::Ascending)],
        ..sales()
    });
    let a = args(&[("n", Cell::Int(5))]);
    let ids = [
        plain.invocation_identity(&a),
        filtered.invocation_identity(&a),
        ordered.invocation_identity(&a),
    ];
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert_ne!(ids[1], ids[2]);
    assert!(!plain.grant(&a).covers_read(&filtered, &a));
    assert!(filtered.grant(&a).covers_read(&filtered, &a));
}

// ─── The registry (QueryRegistry.fs:74-235) ─────────────────────────────────

fn registry() -> QueryRegistry<u8> {
    let r: QueryRegistry<u8> = QueryRegistry::empty();
    let r = r.register(with_filter(eq("n", Cell::Int(2)))).unwrap();
    let r = r
        .register(Query {
            id: "ledger".to_string(),
            ..sales()
        })
        .unwrap();
    // Context 0 may read anything; 1 must have each read approved; 2 is denied
    // every read of `sales`.
    r.with_gate(PolicyGate::new(
        "role",
        |ctx: &u8, q: &Query, _: &[(String, Cell)]| match (ctx, q.id.as_str()) {
            (0, _) => PolicyDecision::Allow,
            (2, "sales") => {
                PolicyDecision::deny_with("not for this role", vec![Arc::<str>::from("ledger")])
            }
            _ => PolicyDecision::NeedsApproval,
        },
    ))
}

/// Query.fs:599-618 at Core `889c869d` (`invokePageWithArgs`, Phase 685, the
/// reference's D264.8 point 3): a resolver's `PredicateNotHonoured` reaches the
/// caller only for a predicate of the declaration's own filter; any other is
/// the resolver's fault, `ExecutionFailed` naming its tag and column, in the
/// reference's words (Query.fs:610-614), with no recoverable argument. No
/// vector reaches either arm (`unreached`), so both are pinned here (D18.7).
#[test]
fn the_dispatch_holds_a_resolvers_unhonoured_predicate_to_the_declarations_own() {
    let r = registry();
    let a = args(&[("n", Cell::Int(5))]);
    let refuse = |p: ColumnPredicate| {
        r.dispatch(&0, None, "sales", &a, |_| {
            Err::<(), _>(QueryError::PredicateNotHonoured { predicate: p })
        })
    };
    // The declaration's own predicate passes through.
    assert_eq!(
        refuse(eq("n", Cell::Int(2))),
        Err(QueryError::PredicateNotHonoured {
            predicate: eq("n", Cell::Int(2))
        })
    );
    // Another literal on the same column, or another column, is the resolver's.
    assert_eq!(
        refuse(eq("n", Cell::Int(3))),
        Err(QueryError::ExecutionFailed {
            detail: "the resolver refused filter 'equalTo' on column 'n', which the declaration does not hold".to_string(),
            recoverable: Vec::new(),
        })
    );
    assert_eq!(
        refuse(ColumnPredicate::IsNull {
            column: "region".to_string()
        }),
        Err(QueryError::ExecutionFailed {
            detail: "the resolver refused filter 'isNull' on column 'region', which the declaration does not hold".to_string(),
            recoverable: Vec::new(),
        })
    );
    // A read whose declaration declares no filter holds no predicate.
    assert!(matches!(
        r.dispatch(&0, None, "ledger", &a, |_| Err::<(), _>(
            QueryError::PredicateNotHonoured {
                predicate: eq("n", Cell::Int(2))
            }
        )),
        Err(QueryError::ExecutionFailed { .. })
    ));
}

/// QueryRegistry.fs:74-235: enumerate, the named decision under a context and a
/// grant, dispatch from the same decision, `under`, `unregister`.
#[test]
fn the_registry_decides_reads_as_the_capability_registry_decides_invocations() {
    let r = registry();
    assert_eq!(r.ids(), vec!["ledger".to_string(), "sales".to_string()]);
    assert_eq!(r.gates(), vec!["role"]);
    let a = args(&[("n", Cell::Int(5))]);
    let q = r.try_find("sales").unwrap().clone();
    let grant = q.grant(&a);
    assert_eq!(
        r.decide_named(&0, None, "sales", &a),
        Ok(("", PolicyDecision::Allow))
    );
    assert_eq!(
        r.decide_named(&1, None, "sales", &a),
        Ok(("role", PolicyDecision::NeedsApproval))
    );
    assert_eq!(
        r.decide_named(&1, Some(&grant), "sales", &a),
        Ok(("role", PolicyDecision::Allow))
    );
    // A grant for another read, or the capture-key-shaped text, lifts nothing.
    let other = Grant::of_invocation(q.invocation_identity(&args(&[("n", Cell::Int(6))])));
    assert_eq!(
        r.decide_named(&1, Some(&other), "sales", &a),
        Ok(("role", PolicyDecision::NeedsApproval))
    );
    // No grant lifts a denial.
    let denied = r
        .dispatch(&2, Some(&grant), "sales", &a, |_| Ok(()))
        .unwrap_err();
    assert_eq!(
        denied.describe(),
        "Refused: the policy 'role' does not allow this: not for this role. What it allows instead: 'ledger'."
    );
    // Dispatch runs the resolver exactly once, and only when allowed.
    let mut runs = 0;
    assert_eq!(
        r.dispatch(&1, Some(&grant), "sales", &a, |q| {
            runs += 1;
            Ok(q.id.clone())
        }),
        Ok("sales".to_string())
    );
    let parked = r
        .dispatch(&1, None, "sales", &a, |_| {
            runs += 1;
            Ok(String::new())
        })
        .unwrap_err();
    assert_eq!(runs, 1);
    assert_eq!(
        parked.describe(),
        "Refused: the policy 'role' requires an approval before this query runs."
    );
    // Resolve and validate come first, whatever is presented.
    assert_eq!(
        r.decide_named(&0, Some(&grant), "nope", &a)
            .unwrap_err()
            .describe(),
        "Refused: there is no query 'nope' you may run. The queries you may run are 'ledger', 'sales'."
    );
    assert!(matches!(
        r.decide_named(&0, None, "sales", &[]),
        Err(QueryError::RequiredParamsUnbound { .. })
    ));
    // A fixed context's view decides as the registry does under it.
    let fixed = r.under(1);
    assert_eq!(
        fixed.decide_named(&(), Some(&grant), "sales", &a),
        Ok(("role", PolicyDecision::Allow))
    );
    assert_eq!(
        fixed.decide_named(&(), None, "ledger", &a),
        Ok(("role", PolicyDecision::NeedsApproval))
    );
    // A refusal from the resolver is the dispatch's.
    assert_eq!(
        r.dispatch(&0, None, "ledger", &a, |_| Err::<(), _>(
            QueryError::Timeout
        )),
        Err(QueryError::Timeout)
    );
    // Unregistering removes the query and its rendered shape together.
    let fewer = r.unregister("sales").unwrap();
    assert_eq!(fewer.ids(), vec!["ledger".to_string()]);
    assert_eq!(
        fewer.decide_named(&1, Some(&grant), "ledger", &a),
        Ok(("role", PolicyDecision::NeedsApproval))
    );
    assert_eq!(
        QueryRegistry::new().unregister("x").unwrap_err().describe(),
        "Refused: there is no query 'x' you may run. There are no queries you may run."
    );
}
