//! Certifies the query declaration's refusals against Core's
//! `laws/query-declaration-laws.json` (family `queryDeclarationLaws`, the
//! reference's D263): the declaration reader, the admission gate, registration,
//! argument validation and the registry's gated dispatch (DECISIONS.md D17).
//!
//! The registry is the file's head (`registry`, each a canonical declaration as a
//! JSON string, read through the declaration reader and asserted to write back
//! the bytes it was read from) under the head's gate table (`gates`). A
//! declaration an `admissionFault` or `register` vector states as DATA is built as
//! the typed value from that data, never read from text (D17.2). Every refusal
//! outside the reader is compared as its query-error document through the seam
//! codec, never as a sentence (D17.4). The family's size is its row in
//! `laws/manifest.json`, and the leg asserts the count of each case it ran.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use fuaran_core::capability::{Grant, PolicyDecision, PolicyGate};
use fuaran_core::column::{Cell, ColumnType};
use fuaran_core::decode::{self, DecodeError};
use fuaran_core::json::{JVal, JsonLimits, PathSeg, render_canonical};
use fuaran_core::query::{
    ColumnPredicate, Query, QueryError, QueryParam, QueryRegistry, SortDirection, SortKey,
};
use fuaran_core::seam::{self, Reading, Slot};

const FILE: &str = "laws/query-declaration-laws.json";
const CASES: [&str; 5] = [
    "declarationRefused",
    "admissionFault",
    "register",
    "validateParams",
    "dispatch",
];

fn doc() -> JVal {
    let d = read(FILE);
    assert_eq!(s(&d, "family"), "queryDeclarationLaws");
    d
}

/// The head's declarations, each read through the declaration reader and
/// asserted to write back the canonical bytes it was read from.
fn queries(d: &JVal) -> Vec<Query> {
    str_items(d, "registry")
        .iter()
        .map(|text| {
            let q = Query::decode(text)
                .unwrap_or_else(|e| panic!("the head's declaration is refused: {e:?}\n{text}"));
            assert_eq!(
                &q.encode(),
                text,
                "{}: the declaration's canonical text",
                q.id
            );
            q
        })
        .collect()
}

/// The head's registry under the head's gate table: each gate answers the
/// decision of its rule for a read's id, and `Allow` for one it has no rule for
/// (D17.3).
fn registry_of(d: &JVal, qs: &[Query]) -> QueryRegistry {
    let mut r = QueryRegistry::new();
    for q in qs {
        r = r
            .register(q.clone())
            .unwrap_or_else(|e| panic!("{}: refused at register: {e:?}", q.id));
    }
    for gate in arr(d, "gates") {
        let rules: Vec<(String, PolicyDecision)> = arr(gate, "rules")
            .iter()
            .map(|rule| (s(rule, "query").to_string(), decision_of(rule)))
            .collect();
        r = r.with_gate(PolicyGate::ignoring_context(
            s(gate, "policy"),
            move |q: &Query, _: &[(String, Cell)]| {
                rules
                    .iter()
                    .find(|(id, _)| *id == q.id)
                    .map_or(PolicyDecision::Allow, |(_, d)| d.clone())
            },
        ));
    }
    r
}

fn named<'a>(qs: &'a [Query], v: &JVal) -> &'a Query {
    let id = s(get(v, "input"), "query");
    qs.iter()
        .find(|q| q.id == id)
        .unwrap_or_else(|| panic!("{}: no query {id} in the head", s(v, "id")))
}

// ─── A declaration stated as data (D17.2) ────────────────────────────────────

fn column_type(tag: &str) -> ColumnType {
    ColumnType::of_tag(tag).unwrap_or_else(|| panic!("unknown column type {tag}"))
}

fn predicate_of(p: &JVal) -> ColumnPredicate {
    let column = s(p, "column").to_string();
    let value = || cell_of(get(p, "cell"));
    match s(p, "predicate") {
        "equalTo" => ColumnPredicate::EqualTo {
            column,
            value: value(),
        },
        "greaterThan" => ColumnPredicate::GreaterThan {
            column,
            value: value(),
        },
        "atLeast" => ColumnPredicate::AtLeast {
            column,
            value: value(),
        },
        "lessThan" => ColumnPredicate::LessThan {
            column,
            value: value(),
        },
        "atMost" => ColumnPredicate::AtMost {
            column,
            value: value(),
        },
        "contains" => ColumnPredicate::Contains {
            column,
            text: s(p, "text").to_string(),
        },
        "isNull" => ColumnPredicate::IsNull { column },
        "isNotNull" => ColumnPredicate::IsNotNull { column },
        other => panic!("unknown predicate {other}"),
    }
}

fn sort_key_of(k: &JVal) -> SortKey {
    SortKey {
        column: s(k, "column").to_string(),
        direction: match s(k, "direction") {
            "ascending" => SortDirection::Ascending,
            "descending" => SortDirection::Descending,
            other => panic!("unknown direction {other}"),
        },
    }
}

/// The named declaration of the head with each member the vector states
/// replaced by the typed value built from its data.
fn stated(qs: &[Query], v: &JVal) -> Query {
    let input = get(v, "input");
    let mut q = named(qs, v).clone();
    if input.field("id").is_some() {
        q.id = s(input, "id").to_string();
    }
    if input.field("params").is_some() {
        q.params = arr(input, "params")
            .iter()
            .map(|p| QueryParam {
                name: s(p, "name").to_string(),
                ty: column_type(s(p, "type")),
                required: match get(p, "required") {
                    JVal::Bool(b) => *b,
                    other => panic!("`required` is not a boolean: {other:?}"),
                },
            })
            .collect();
    }
    if input.field("where").is_some() {
        q.filter = arr(input, "where").iter().map(predicate_of).collect();
    }
    if input.field("orderBy").is_some() {
        q.order_by = arr(input, "orderBy").iter().map(sort_key_of).collect();
    }
    q
}

// ─── A refusal as its query-error document (D17.4) ───────────────────────────

fn strs(xs: &[String]) -> JVal {
    JVal::Arr(xs.iter().map(|x| JVal::Str(x.clone())).collect())
}

fn shared(xs: &[Arc<str>]) -> JVal {
    JVal::Arr(xs.iter().map(|x| JVal::Str(x.to_string())).collect())
}

fn ty(t: ColumnType) -> JVal {
    JVal::Str(t.tag().to_string())
}

/// The typed refusal projected into the members the seam's `QUERY_ERROR`
/// declaration names, before the codec reads it.
fn projected(e: &QueryError) -> JVal {
    let str = |x: &str| JVal::Str(x.to_string());
    let (tag, members): (&str, Vec<(&str, JVal)>) = match e {
        QueryError::NoSuchQuery { id, known } => {
            ("noSuchQuery", vec![("id", str(id)), ("known", strs(known))])
        }
        QueryError::DuplicateQuery { id } => ("duplicateQuery", vec![("id", str(id))]),
        QueryError::UnknownParam { name, declared } => (
            "unknownParam",
            vec![("name", str(name)), ("declared", strs(declared))],
        ),
        QueryError::ParamTypeMismatch {
            name,
            expected,
            got,
        } => (
            "paramTypeMismatch",
            vec![
                ("name", str(name)),
                ("expected", ty(*expected)),
                ("got", ty(*got)),
            ],
        ),
        QueryError::RequiredParamsUnbound { names } => {
            ("requiredParamsUnbound", vec![("names", strs(names))])
        }
        QueryError::SourceNotResolved { reference } => {
            ("sourceNotResolved", vec![("ref", str(reference))])
        }
        QueryError::ExecutionFailed {
            detail,
            recoverable,
        } => (
            "executionFailed",
            vec![("detail", str(detail)), ("recoverable", strs(recoverable))],
        ),
        QueryError::Timeout => ("timeout", vec![]),
        QueryError::RequiredParamsNull { names } => {
            ("requiredParamsNull", vec![("names", strs(names))])
        }
        QueryError::DuplicateParam { name } => ("duplicateParam", vec![("name", str(name))]),
        QueryError::QueryPolicyRefused {
            policy,
            reason,
            allowed,
        } => (
            "queryPolicyRefused",
            vec![
                ("policy", str(policy)),
                ("reason", str(reason)),
                ("allowed", shared(allowed)),
            ],
        ),
        QueryError::QueryApprovalRequired { policy } => {
            ("queryApprovalRequired", vec![("policy", str(policy))])
        }
        QueryError::UnreadableArgs { error } => panic!(
            "unreadableArgs is raised by the argument document's reader, which the crate does \
             not hold (D14.3) and no vector reaches: {error:?}"
        ),
        QueryError::UnknownColumn { name, declared } => (
            "unknownColumn",
            vec![("name", str(name)), ("declared", strs(declared))],
        ),
        QueryError::PredicateTypeMismatch {
            column,
            expected,
            got,
        } => (
            "predicateTypeMismatch",
            vec![
                ("column", str(column)),
                ("expected", ty(*expected)),
                ("got", ty(*got)),
            ],
        ),
        QueryError::PredicateNotApplicable {
            predicate,
            column,
            column_type,
        } => (
            "predicateNotApplicable",
            vec![
                ("predicate", str(predicate)),
                ("column", str(column)),
                ("columnType", ty(*column_type)),
            ],
        ),
        QueryError::IllFormedLiteral { column, reason } => (
            "illFormedLiteral",
            vec![("column", str(column)), ("reason", str(reason))],
        ),
        QueryError::DuplicateSortColumn { column } => {
            ("duplicateSortColumn", vec![("column", str(column))])
        }
        QueryError::PredicateNotHonoured { predicate } => (
            "predicateNotHonoured",
            vec![("predicate", predicate.to_json())],
        ),
        QueryError::OrderNotHonoured { column } => {
            ("orderNotHonoured", vec![("column", str(column))])
        }
    };
    let mut fields = vec![("$type".to_string(), str(tag))];
    fields.extend(members.into_iter().map(|(k, v)| (k.to_string(), v)));
    JVal::Obj(fields)
}

/// A query-error document read by the seam codec (the strict reading of
/// `queryError`), as its canonical re-encoding; a document the declaration does
/// not admit fails the leg.
fn through_the_codec(document: &JVal, what: &str) -> JVal {
    let (root, strict) = seam::reader("queryError/strict").expect("the queryError reader");
    let reading = Reading {
        strict,
        param: Slot::Str,
    };
    let parsed = decode::parse(&render_canonical(document), &JsonLimits::STRICT)
        .unwrap_or_else(|e| panic!("{what}: unreadable: {e:?}"));
    seam::read(&root, &parsed, &reading).unwrap_or_else(|e| {
        panic!(
            "{what}: the codec refuses {}: {e:?}",
            render_canonical(document)
        )
    })
}

/// A refusal's members as the file writes them: `error`, the document.
fn error_members(e: &QueryError, what: &str) -> (String, JVal) {
    ("error".into(), through_the_codec(&projected(e), what))
}

/// A path as this family writes it: each segment a string, an index its
/// decimal digits (D17.4).
fn path_text(path: &[PathSeg]) -> JVal {
    JVal::Arr(
        path.iter()
            .map(|seg| match seg {
                PathSeg::Key(k) => JVal::Str(k.clone()),
                PathSeg::Index(i) => JVal::Str(i.to_string()),
            })
            .collect(),
    )
}

fn verdict(v: &str) -> (String, JVal) {
    ("verdict".into(), JVal::Str(v.into()))
}

// ─── The cases (D17.5) ───────────────────────────────────────────────────────

fn reader_refusal(e: &DecodeError) -> Vec<(String, JVal)> {
    vec![
        verdict("reject"),
        ("code".into(), JVal::Str(e.code.as_str().into())),
        ("path".into(), path_text(&e.path)),
    ]
}

fn answer(qs: &[Query], r: &QueryRegistry, v: &JVal) -> JVal {
    let id = s(v, "id");
    let input = get(v, "input");
    let members = match s(v, "case") {
        "declarationRefused" => match Query::decode(s(input, "declaration")) {
            Ok(q) => {
                assert_eq!(q.admission_fault(), None, "{id}: read, so admitted");
                vec![verdict("accept")]
            }
            Err(e) => reader_refusal(&e),
        },
        "admissionFault" => {
            let q = stated(qs, v);
            let fault = q.admission_fault_at();
            assert_eq!(
                fault.as_ref().map(|(_, e)| e.clone()),
                q.admission_fault(),
                "{id}: the gate with and without its path"
            );
            match fault {
                None => vec![verdict("admitted")],
                Some((at, e)) => vec![
                    verdict("refused"),
                    ("at".into(), path_text(&at)),
                    error_members(&e, id),
                ],
            }
        }
        "register" => match r.register(stated(qs, v)) {
            Ok(_) => vec![verdict("registered")],
            Err(e) => vec![verdict("refused"), error_members(&e, id)],
        },
        "validateParams" => match named(qs, v).validate_params(&args_of(input)) {
            Ok(()) => vec![verdict("accept")],
            Err(e) => vec![verdict("reject"), error_members(&e, id)],
        },
        "dispatch" => {
            let query = s(input, "query");
            let args = args_of(input);
            let grant = input.field("grant").map(|g| match g {
                JVal::Str(identity) => Grant::of_invocation(identity.clone()),
                other => panic!("{id}: `grant` is not a string: {other:?}"),
            });
            let mut runs = 0;
            let dispatched = r.dispatch(&(), grant.as_ref(), query, &args, |_| {
                runs += 1;
                Ok(())
            });
            let decided = r.decide_named(&(), grant.as_ref(), query, &args);
            match dispatched {
                Ok(()) => {
                    assert_eq!(runs, 1, "{id}: performed, so the resolver ran once");
                    assert!(
                        matches!(decided, Ok((_, PolicyDecision::Allow))),
                        "{id}: performed where the decision is not Allow: {decided:?}"
                    );
                    vec![verdict("performed")]
                }
                Err(e) => {
                    assert_eq!(runs, 0, "{id}: refused, so the resolver never ran");
                    match &decided {
                        Ok((_, d)) => assert_ne!(*d, PolicyDecision::Allow, "{id}"),
                        Err(before) => assert_eq!(*before, e, "{id}: refused before the gate"),
                    }
                    vec![verdict("refused"), error_members(&e, id)]
                }
            }
        }
        other => panic!("{id}: unknown case {other}"),
    };
    JVal::Obj(members)
}

/// The expected answer, its `error` read by the same codec as the port's.
fn expected(v: &JVal) -> JVal {
    let JVal::Obj(members) = get(v, "expected") else {
        panic!("{}: `expected` is not an object", s(v, "id"))
    };
    JVal::Obj(
        members
            .iter()
            .map(|(k, x)| {
                if k == "error" {
                    (k.clone(), through_the_codec(x, s(v, "id")))
                } else {
                    (k.clone(), x.clone())
                }
            })
            .collect(),
    )
}

#[test]
fn every_vector_of_the_family() {
    let d = doc();
    let qs = queries(&d);
    let r = registry_of(&d, &qs);
    let mut counts = [0usize; CASES.len()];
    // Every vector is answered before the leg judges, so a red names each vector
    // answered differently, not only the first.
    let mut wrong = Vec::new();
    for v in vectors(&d) {
        let case = s(v, "case");
        let i = CASES
            .iter()
            .position(|c| *c == case)
            .unwrap_or_else(|| panic!("{}: unknown case {case}", s(v, "id")));
        let (got, want) = (
            render_canonical(&answer(&qs, &r, v)),
            render_canonical(&expected(v)),
        );
        if got != want {
            wrong.push(format!(
                "{} ({case}): answered {got}, expected {want}",
                s(v, "id")
            ));
        }
        counts[i] += 1;
    }
    let total: usize = counts.iter().sum();
    assert_eq!(total, declared("laws", "queryDeclarationLaws"));
    assert_eq!(
        counts,
        [36, 21, 4, 14, 9],
        "the vectors of each case, in {CASES:?}"
    );
    assert!(
        wrong.is_empty(),
        "{} of {total} vectors answered differently:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    eprintln!("queryDeclarationLaws: {total} of {total} vectors ({CASES:?} = {counts:?})");
}

/// `referenceOnly` names which MODEL answers, not whether a host must: the
/// port answers those vectors like every other (D17.6). It names vectors the
/// file holds, 13 of them.
#[test]
fn reference_only_names_vectors_the_file_holds() {
    let d = doc();
    let all = vectors(&d);
    let mut named = BTreeSet::new();
    for entry in arr(&d, "referenceOnly") {
        let case = s(entry, "case");
        assert!(CASES.contains(&case), "referenceOnly names a case {case}");
        assert!(!s(entry, "reason").is_empty());
        match entry.field("vectors") {
            Some(_) => {
                for id in str_items(entry, "vectors") {
                    let v = all
                        .iter()
                        .find(|v| s(v, "id") == id)
                        .unwrap_or_else(|| panic!("referenceOnly names no vector {id}"));
                    assert_eq!(s(v, "case"), case, "{id}");
                    assert!(named.insert(id));
                }
            }
            None => {
                for v in all.iter().filter(|v| s(v, "case") == case) {
                    assert!(named.insert(s(v, "id").to_string()));
                }
            }
        }
    }
    assert_eq!(named.len(), 13, "{named:?}");
}

/// `unreached` names cases of the seam's `QueryError` that no vector raises
/// (D17.6).
#[test]
fn unreached_names_cases_no_vector_raises() {
    let d = doc();
    let raised: BTreeSet<String> = vectors(&d)
        .iter()
        .filter_map(|v| {
            get(v, "expected")
                .field("error")
                .map(|e| s(e, "$type").to_string())
        })
        .collect();
    let unreached: Vec<String> = arr(&d, "unreached")
        .iter()
        .map(|u| s(u, "error").to_string())
        .collect();
    assert_eq!(unreached.len(), 6, "{unreached:?}");
    for tag in &unreached {
        assert!(
            seam::QUERY_ERROR.cases.iter().any(|c| c.tag == tag),
            "{tag} is not a case of the seam's QueryError"
        );
        assert!(!raised.contains(tag), "{tag} is named unreached and raised");
    }
}
