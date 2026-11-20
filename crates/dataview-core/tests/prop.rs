//! Property test: the canonical form of a query parses back to the same query.
use dataview_core::{parse, Cmp, Column, Date, Dir, Expr, Field, Lit, Query, SortKey};
use proptest::prelude::*;
use proptest::string::string_regex;

/// Identifiers start with `k`, so a generated name never spells a keyword.
fn name() -> impl Strategy<Value = String> {
    string_regex("[a-z0-9_]{0,7}").unwrap().prop_map(|s| format!("k{s}"))
}

fn field() -> impl Strategy<Value = Field> {
    prop_oneof![
        name().prop_map(Field),
        (name(), name()).prop_map(|(a, b)| Field(format!("{a}.{b}"))),
        Just(Field("file.name".into())),
        Just(Field("file.mtime".into())),
    ]
}

fn lit() -> impl Strategy<Value = Lit> {
    prop_oneof![
        Just(Lit::Null),
        any::<String>().prop_map(Lit::Str),
        any::<i32>().prop_map(|n| Lit::Num(f64::from(n))),
        prop_oneof![Just(0.5), Just(-1.25), Just(1e-7), Just(1e20), Just(3e-20)].prop_map(Lit::Num),
        (1900i32..2100, 1u32..13, 1u32..32).prop_map(|(year, month, day)| Lit::Date(Date {
            year,
            month,
            day
        })),
    ]
}

fn comparison() -> impl Strategy<Value = Expr> {
    (field(), 0usize..6, lit()).prop_map(|(f, op, l)| {
        let op = [Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge][op];
        Expr::Cmp(f, op, l)
    })
}

fn filter() -> impl Strategy<Value = Expr> {
    prop::collection::vec(comparison(), 1..4).prop_map(Expr::And)
}

fn column() -> impl Strategy<Value = Column> {
    (field(), prop::option::of(any::<String>())).prop_map(|(field, alias)| Column { field, alias })
}

fn sort_key() -> impl Strategy<Value = SortKey> {
    (field(), any::<bool>())
        .prop_map(|(field, desc)| SortKey { field, dir: if desc { Dir::Desc } else { Dir::Asc } })
}

fn query() -> impl Strategy<Value = Query> {
    (
        any::<bool>(),
        prop::collection::vec(column(), 1..4),
        prop::option::of(any::<String>()),
        prop::option::of(filter()),
        prop::collection::vec(sort_key(), 0..3),
    )
        .prop_map(|(without_id, columns, from, filter, sort)| Query {
            without_id,
            columns,
            from,
            filter,
            sort,
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn canonical_form_round_trips(q in query()) {
        let text = q.to_string();
        let back = parse(&text).map_err(|e| TestCaseError::fail(format!("{e} in\n{text}")))?;
        prop_assert_eq!(back, q);
    }
}
