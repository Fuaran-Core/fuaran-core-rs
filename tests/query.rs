//! Port-side tests of the query declaration (DECISIONS.md D14): what the
//! reference's vectors do not reach. No Core family reads a query declaration,
//! so every refusal of the declaration reader and of the admission gate is
//! pinned here to the reference's text at the pin (`a9592b7`, `0.43.0`), each
//! test citing the file and line it is the twin of under
//! `src/Fuaran.Core.Query/` (`Query.fs`, `QueryRegistry.fs`, `QueryCodec.fs`) or
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
    ColumnPredicate, Query, QueryError, QueryParam, QueryRegistry, SortDirection, SortKey,
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

fn path(segs: &[&str]) -> Vec<PathSeg> {
    segs.iter()
        .map(|s| match s.parse::<usize>() {
            Ok(i) => PathSeg::Index(i),
            Err(_) => PathSeg::Key(s.to_string()),
        })
        .collect()
}

/// The admission refusal, asserted the same three ways: the gate with its path,
/// `register`, and the declaration reader over the declaration's own text (when
/// its text can carry it), which refuses `OutOfRange` at the path with the
/// registry's sentence.
fn refused(q: &Query, at: &[&str], expected: &QueryError, sentence: &str) {
    assert_eq!(expected.describe(), sentence);
    let (p, e) = q.admission_fault_at().expect("the gate refuses");
    assert_eq!((&p, &e), (&path(at), expected));
    assert_eq!(
        QueryRegistry::new().register(q.clone()).unwrap_err(),
        *expected
    );
}

fn refused_on_read(q: &Query, at: &[&str], sentence: &str) {
    let e = Query::decode(&q.encode()).expect_err("the reader refuses");
    assert_eq!((e.code, e.path.clone()), (DecodeCode::OutOfRange, path(at)));
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
    let e = QueryError::DuplicateParam {
        name: "rate".to_string(),
    };
    refused(&q, &["params"], &e, sentence);
    refused_on_read(&q, &["params"], sentence);
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
    let e = QueryError::UnknownColumn {
        name: "nope".to_string(),
        declared: ["n", "region", "at", "amount", "day", "score"]
            .map(String::from)
            .to_vec(),
    };
    refused(&q, &["where", "1"], &e, &sentence);
    refused_on_read(&q, &["where", "1"], &sentence);
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
    let e = QueryError::PredicateNotApplicable {
        predicate: "contains".to_string(),
        column: "n".to_string(),
        column_type: ColumnType::Int,
    };
    refused(&q, &["where", "0"], &e, sentence);
    refused_on_read(&q, &["where", "0"], sentence);
    // On a string column it is admitted.
    assert_eq!(
        with_filter(ColumnPredicate::Contains {
            column: "region".to_string(),
            text: "eu".to_string(),
        })
        .admission_fault(),
        None
    );
}

/// Query.fs:83-84, 259-264. A declaration's text cannot carry a `null` literal:
/// the strict reader reads no `null` (D13.3), as the reference's does, so this
/// refusal is reached by a declaration built by hand.
#[test]
fn a_null_literal_is_ill_formed() {
    let q = with_filter(eq("n", Cell::Null));
    let e = QueryError::IllFormedLiteral {
        column: "n".to_string(),
        reason: "a null literal compares with nothing; test for absence with isNull".to_string(),
    };
    refused(
        &q,
        &["where", "0"],
        &e,
        "Refused: the filter on column 'n' compares with a value the column cannot carry: a null literal compares with nothing; test for absence with isNull.",
    );
}

/// Query.fs:94, 146-158, 242-250.
#[test]
fn a_literal_of_another_type_is_a_type_mismatch_saying_how_to_write_one() {
    let q = with_filter(eq("n", Cell::Str("5".to_string())));
    let sentence = "Refused: the filter on column 'n' compares with a string value; the column is int, and a filter compares with a value of the column's own type.";
    let e = QueryError::PredicateTypeMismatch {
        column: "n".to_string(),
        expected: ColumnType::Int,
        got: ColumnType::String,
    };
    refused(&q, &["where", "0"], &e, sentence);
    refused_on_read(&q, &["where", "0"], sentence);
    // No widening: an integer literal on a decimal column is refused, with how
    // to write a decimal.
    let q = with_filter(eq("amount", Cell::Int(5)));
    let sentence = "Refused: the filter on column 'amount' compares with a int value; the column is decimal, and a filter compares with a value of the column's own type. Write a decimal as a JSON string of decimal text, such as \"12.50\": an optional '-', digits, and an optional '.' followed by digits; never as a JSON number.";
    let e = QueryError::PredicateTypeMismatch {
        column: "amount".to_string(),
        expected: ColumnType::Decimal,
        got: ColumnType::Int,
    };
    refused(&q, &["where", "0"], &e, sentence);
    refused_on_read(&q, &["where", "0"], sentence);
    // Nor on a float column.
    assert!(matches!(
        with_filter(eq("score", Cell::Int(1))).admission_fault(),
        Some(QueryError::PredicateTypeMismatch { .. })
    ));
}

/// Query.fs:86-93 (Phase 422), 146-158.
#[test]
fn a_timestamp_literal_fits_a_column_at_least_as_fine_and_no_finer() {
    // A whole second is a `timestamp` cell, and fits a millisecond column.
    let coarse = with_filter(eq(
        "at",
        Cell::Timestamp("2026-11-15T09:30:00Z".to_string()),
    ));
    assert_eq!(coarse.admission_fault(), None);
    assert_eq!(Query::decode(&coarse.encode()).unwrap(), coarse);
    // Microseconds do not.
    let fine = with_filter(eq(
        "at",
        Cell::Timestamp("2026-11-15T09:30:00.123456Z".to_string()),
    ));
    let sentence = "Refused: the filter on column 'at' compares with a timestamp_us value; the column is timestamp_ms, and a filter compares with a value of the column's own type. Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ, or YYYY-MM-DDThh:mm:ss.FZ with up to 3 fraction digits and no trailing zero.";
    let e = QueryError::PredicateTypeMismatch {
        column: "at".to_string(),
        expected: ColumnType::Timestamp(TimeUnit::Milliseconds),
        got: ColumnType::Timestamp(TimeUnit::Microseconds),
    };
    refused(&fine, &["where", "0"], &e, sentence);
    refused_on_read(&fine, &["where", "0"], sentence);
}

/// Query.fs:45-67 (`literalReason`, `literalFault`); Column.fs:531, 850. A
/// literal of its column's type that the column cannot carry; the reader
/// reads a literal through the column codec, which refuses each of these first,
/// so they are reached by declarations built by hand.
#[test]
fn a_literal_its_column_cannot_carry_is_ill_formed_with_the_column_layers_reason() {
    let reason = |q: Query| match q.admission_fault() {
        Some(QueryError::IllFormedLiteral { column, reason }) => (column, reason),
        other => panic!("not an ill-formed literal: {other:?}"),
    };
    assert_eq!(
        reason(with_filter(eq("score", Cell::Float(f64::NAN)))),
        (
            "score".to_string(),
            "the float NaN is not finite, and the wire has no non-finite float".to_string()
        )
    );
    assert_eq!(
        reason(with_filter(eq("score", Cell::Float(f64::NEG_INFINITY)))).1,
        "the float -Infinity is not finite, and the wire has no non-finite float"
    );
    assert_eq!(
        reason(with_filter(eq(
            "amount",
            Cell::Decimal("12.50".to_string())
        )))
        .1,
        "amount: a decimal cell must carry canonical decimal text — an optional '-', integer digits with no leading zero, and a '.' with fraction digits only where the fraction is non-zero, with no trailing zero (build the cell with Cell.decimal)"
    );
    assert_eq!(
        reason(with_filter(eq("day", Cell::Date("2026-2-1".to_string())))).1,
        "day: a date cell must carry a canonical ISO-8601 date, YYYY-MM-DD, naming a day that exists"
    );
    let e = with_filter(eq("day", Cell::Date("2026-02-30".to_string())))
        .admission_fault()
        .unwrap();
    assert_eq!(
        e.describe(),
        "Refused: the filter on column 'day' compares with a value the column cannot carry: day: a date cell must carry a canonical ISO-8601 date, YYYY-MM-DD, naming a day that exists."
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
    let declared: Vec<String> = ["n", "region", "at", "amount", "day", "score"]
        .map(String::from)
        .to_vec();
    let e = QueryError::UnknownColumn {
        name: "nope".to_string(),
        declared,
    };
    refused(&unknown, &["orderBy", "0"], &e, &sentence);
    refused_on_read(&unknown, &["orderBy", "0"], &sentence);
    let twice = Query {
        order_by: vec![
            key("n", SortDirection::Descending),
            key("region", SortDirection::Ascending),
            key("n", SortDirection::Ascending),
        ],
        ..sales()
    };
    let sentence = "Refused: the order names column 'n' more than once. Name each column once.";
    let e = QueryError::DuplicateSortColumn {
        column: "n".to_string(),
    };
    refused(&twice, &["orderBy", "2"], &e, sentence);
    refused_on_read(&twice, &["orderBy", "2"], sentence);
}

/// QueryRegistry.fs:44-52: the parameters, then the filter, then the order.
#[test]
fn the_filter_is_checked_before_the_order() {
    let q = Query {
        filter: vec![eq("n", Cell::Str("x".to_string()))],
        order_by: vec![key("nope", SortDirection::Ascending)],
        ..sales()
    };
    assert_eq!(q.admission_fault_at().unwrap().0, path(&["where", "0"]));
}

/// Query.fs:171; QueryRegistry.fs:82-84 (`Seam.register`: the id first).
#[test]
fn a_held_id_is_refused_before_the_declaration_is_admitted() {
    let r = QueryRegistry::new().register(sales()).unwrap();
    let mut again = sales();
    again.params.push(param("n", ColumnType::Int, false));
    let e = r.register(again).unwrap_err();
    assert_eq!(
        e,
        QueryError::DuplicateQuery {
            id: "sales".to_string()
        }
    );
    assert_eq!(
        e.describe(),
        "Refused: the query 'sales' is registered twice."
    );
}

// ─── The declaration reader (QueryCodec.fs:144-192) ─────────────────────────

/// A declaration's text with `member` set to `value` (or removed, for `None`).
fn edited(member: &str, value: Option<&str>) -> String {
    let mut doc = sales().encode_json();
    let JVal::Obj(fields) = &mut doc else {
        unreachable!()
    };
    fields.retain(|(k, _)| k != member);
    if let Some(v) = value {
        fields.push((member.to_string(), fuaran_core::json::parse(v).unwrap()));
    }
    render_canonical(&doc)
}

fn refusal(text: &str) -> (DecodeCode, Vec<PathSeg>) {
    let e = Query::decode(text).expect_err("the reader refuses");
    (e.code, e.path)
}

/// QueryCodec.fs:147-176: the reader's refusals, member by member, in its order,
/// on code and path.
#[test]
fn the_reader_refuses_each_member_where_it_fails() {
    use DecodeCode::*;
    let cases: &[(&str, Option<&str>, DecodeCode, &[&str])] = &[
        ("$type", None, MissingField, &["$type"]),
        ("$type", Some("\"capability\""), UnknownTag, &["$type"]),
        ("id", None, MissingField, &["id"]),
        ("id", Some("1"), WrongKind, &["id"]),
        ("params", None, MissingField, &["params"]),
        ("params", Some("{}"), WrongKind, &["params"]),
        (
            "params",
            Some("[{\"name\":\"n\",\"type\":\"integer\",\"required\":true}]"),
            UnknownTag,
            &["params", "0", "type"],
        ),
        (
            "params",
            Some("[{\"name\":\"n\",\"type\":\"int\",\"required\":\"yes\"}]"),
            WrongKind,
            &["params", "0", "required"],
        ),
        (
            "params",
            Some("[{\"type\":\"int\",\"required\":true}]"),
            MissingField,
            &["params", "0", "name"],
        ),
        (
            "resultSchema",
            Some("[{\"name\":\"n\",\"type\":\"integer\"}]"),
            OutOfRange,
            &["resultSchema", "0"],
        ),
        ("resultSchema", Some("1"), WrongKind, &["resultSchema"]),
        ("effect", None, MissingField, &["effect"]),
        (
            "effect",
            Some("{\"determinism\":\"network\",\"host\":\"everywhere\"}"),
            UnknownTag,
            &["effect", "host"],
        ),
        ("timeoutMs", Some("\"5\""), WrongKind, &["timeoutMs"]),
        ("pageSize", Some("1.5"), WrongKind, &["pageSize"]),
        // The strict reader reads no `null` (D13.3), as the reference's does.
        ("pageSize", Some("null"), InvalidJson, &[]),
        ("where", Some("{}"), WrongKind, &["where"]),
        (
            "where",
            Some("[{\"$type\":\"like\",\"column\":\"region\"}]"),
            UnknownTag,
            &["where", "0", "$type"],
        ),
        (
            "where",
            Some("[{\"$type\":\"equalTo\",\"column\":\"n\",\"type\":\"int\",\"value\":1.5}]"),
            OutOfRange,
            &["where", "0", "value"],
        ),
        (
            "orderBy",
            Some("[{\"column\":\"n\",\"direction\":\"up\"}]"),
            UnknownTag,
            &["orderBy", "0", "direction"],
        ),
        ("source", None, MissingField, &["source"]),
        ("source", Some("5"), OutOfRange, &["source"]),
    ];
    for (member, value, code, at) in cases {
        assert_eq!(
            refusal(&edited(member, *value)),
            (*code, path(at)),
            "{member} = {value:?}"
        );
    }
    // A document that is not an object at all.
    assert_eq!(refusal("[]").1, Vec::<PathSeg>::new());
    // The order the reader reads in: `id` before `params`, `where` before
    // `source`.
    let text = edited("id", Some("1"));
    let text = text.replace("\"params\":[", "\"params\":7,\"_\":[");
    assert_eq!(refusal(&text).1, path(&["id"]));
}

/// QueryCodec.fs:144 (`Decoder.field` / `optField`, which read past every other
/// member): the lenient reading.
#[test]
fn an_unknown_member_is_read_past() {
    let text = edited("note", Some("\"anything\""));
    assert_eq!(Query::decode(&text).unwrap(), sales());
}

/// QueryCodec.fs:69-91 (`queryJson`): every member, written canonically, read
/// back to the same declaration.
#[test]
fn a_declaration_writes_its_canonical_text_and_reads_back_to_itself() {
    let q = Query {
        timeout_ms: Some(2500),
        page_size: Some(50),
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
    assert_eq!(Query::decode(&text).unwrap(), q);
    // Absent and empty members are not written.
    assert!(!sales().encode().contains("where"));
    assert!(!sales().encode().contains("timeoutMs"));
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
    let q = Query::decode(&text).unwrap();
    assert_eq!(
        q.filter,
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
                let q = Query::decode(&text).unwrap_or_else(|e| panic!("{}: {e:?}", s(v, "name")));
                assert_eq!(
                    render_canonical(&q.order_by[0].to_json()),
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

/// Query.fs:424-470, 172-211: each refusal in the order the reference checks.
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
    assert_eq!(
        e,
        QueryError::RequiredParamsUnbound {
            names: vec!["n".to_string()]
        }
    );
    assert_eq!(e.describe(), "Refused: required parameters missing: 'n'.");
    let e = refuse(&[("n", Cell::Null)]);
    assert_eq!(
        e.describe(),
        "Refused: required parameters bound to no value: 'n'."
    );
    // An integer fills a float parameter; a null binds any optional one.
    assert_eq!(
        q.validate_params(&args(&[
            ("n", Cell::Int(1)),
            ("rate", Cell::Int(2)),
            ("tag", Cell::Null)
        ])),
        Ok(())
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
    let q = sales();
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
    let plain = sales();
    let filtered = with_filter(eq("n", Cell::Int(2)));
    let ordered = Query {
        order_by: vec![key("n", SortDirection::Ascending)],
        ..sales()
    };
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
