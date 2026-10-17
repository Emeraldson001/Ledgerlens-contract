//! Spike prototype (issue #1438): a minimal, dependency-free C header
//! describing the contract ABI — function signatures, user-defined types, and
//! error codes — generated from the same Soroban contract-spec (XDR) source
//! the JSON Schema / TypeScript / Python bindings are generated from.
//!
//! The header is a *reference* artifact, not a linkable client: it gives an
//! integrator with no generated client (embedded tooling, a language without
//! a Soroban generator) the exact function names, argument order and types,
//! struct field order, and error discriminants, plus the `ScVal` wire tags
//! needed to hand-encode an invocation. It only includes `<stdint.h>`,
//! `<stdbool.h>`, and `<stddef.h>`.
//!
//! This module depends only on the XDR spec types, never on the contract
//! crate, so it can run against any built or deployed contract WASM.

use soroban_sdk::xdr::{
    ScSpecEntry, ScSpecFunctionV0, ScSpecTypeDef, ScSpecUdtErrorEnumV0, ScSpecUdtStructV0,
    ScSpecUdtUnionCaseV0, ScValType,
};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Filename of the generated C header artifact.
pub const C_HEADER_FILE: &str = "ledgerlens_score.h";

const C_KEYWORDS: [&str; 34] = [
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while",
];

fn spec_str(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `C` identifier for a spec field or argument name.
fn ident(name: &str) -> String {
    if C_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

/// `CamelCase` / `snake_case` → `SCREAMING_SNAKE` for macro names.
fn screaming(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        out.push(ch.to_ascii_uppercase());
    }
    out
}

/// Comment-safe text (no premature `*/`).
fn comment(text: &str) -> String {
    text.replace("*/", "* /")
}

/// The spec type as written in `tests/fixtures/historical/*.abi` goldens.
pub fn spec_type_name(type_: &ScSpecTypeDef) -> String {
    match type_ {
        ScSpecTypeDef::Val => "val".into(),
        ScSpecTypeDef::Bool => "bool".into(),
        ScSpecTypeDef::Void => "void".into(),
        ScSpecTypeDef::Error => "error".into(),
        ScSpecTypeDef::U32 => "u32".into(),
        ScSpecTypeDef::I32 => "i32".into(),
        ScSpecTypeDef::U64 => "u64".into(),
        ScSpecTypeDef::I64 => "i64".into(),
        ScSpecTypeDef::Timepoint => "timepoint".into(),
        ScSpecTypeDef::Duration => "duration".into(),
        ScSpecTypeDef::U128 => "u128".into(),
        ScSpecTypeDef::I128 => "i128".into(),
        ScSpecTypeDef::U256 => "u256".into(),
        ScSpecTypeDef::I256 => "i256".into(),
        ScSpecTypeDef::Bytes => "bytes".into(),
        ScSpecTypeDef::String => "string".into(),
        ScSpecTypeDef::Symbol => "symbol".into(),
        ScSpecTypeDef::Address => "address".into(),
        ScSpecTypeDef::Option(o) => format!("option<{}>", spec_type_name(&o.value_type)),
        ScSpecTypeDef::Result(r) => {
            format!("result<{},{}>", spec_type_name(&r.ok_type), spec_type_name(&r.error_type))
        }
        ScSpecTypeDef::Vec(v) => format!("vec<{}>", spec_type_name(&v.element_type)),
        ScSpecTypeDef::Map(m) => {
            format!("map<{},{}>", spec_type_name(&m.key_type), spec_type_name(&m.value_type))
        }
        ScSpecTypeDef::Tuple(t) => format!(
            "tuple<{}>",
            t.value_types.iter().map(spec_type_name).collect::<Vec<_>>().join(",")
        ),
        ScSpecTypeDef::BytesN(b) => format!("bytesn<{}>", b.n),
        ScSpecTypeDef::Udt(u) => spec_str(&u.name),
    }
}

fn is_unit(type_: &ScSpecTypeDef) -> bool {
    match type_ {
        ScSpecTypeDef::Void => true,
        ScSpecTypeDef::Tuple(t) => t.value_types.is_empty(),
        _ => false,
    }
}

/// Collects the `BytesN<N>` sizes used anywhere so a `ll_bytesN_t` typedef is
/// emitted once for each.
fn collect_bytes_n(type_: &ScSpecTypeDef, out: &mut BTreeSet<u32>) {
    match type_ {
        ScSpecTypeDef::BytesN(b) => {
            out.insert(b.n);
        }
        ScSpecTypeDef::Option(o) => collect_bytes_n(&o.value_type, out),
        ScSpecTypeDef::Result(r) => {
            collect_bytes_n(&r.ok_type, out);
            collect_bytes_n(&r.error_type, out);
        }
        ScSpecTypeDef::Vec(v) => collect_bytes_n(&v.element_type, out),
        ScSpecTypeDef::Map(m) => {
            collect_bytes_n(&m.key_type, out);
            collect_bytes_n(&m.value_type, out);
        }
        ScSpecTypeDef::Tuple(t) => t.value_types.iter().for_each(|v| collect_bytes_n(v, out)),
        _ => {}
    }
}

/// UDT names a type embeds *by value* (and therefore must be declared first).
fn by_value_udts(type_: &ScSpecTypeDef, out: &mut BTreeSet<String>) {
    match type_ {
        ScSpecTypeDef::Udt(u) => {
            out.insert(spec_str(&u.name));
        }
        ScSpecTypeDef::Option(o) => by_value_udts(&o.value_type, out),
        ScSpecTypeDef::Result(r) => by_value_udts(&r.ok_type, out),
        ScSpecTypeDef::Tuple(t) => t.value_types.iter().for_each(|v| by_value_udts(v, out)),
        _ => {}
    }
}

/// C spelling of a spec type. Composite wrappers (`Option`, `Result`,
/// `Tuple`) become anonymous structs, which are valid wherever the type is
/// used (a struct field or a `typedef`).
pub fn c_type(type_: &ScSpecTypeDef) -> String {
    match type_ {
        ScSpecTypeDef::Val => "ll_val_t".into(),
        ScSpecTypeDef::Bool => "bool".into(),
        ScSpecTypeDef::Void => "ll_unit_t".into(),
        ScSpecTypeDef::Error => "uint32_t".into(),
        ScSpecTypeDef::U32 => "uint32_t".into(),
        ScSpecTypeDef::I32 => "int32_t".into(),
        ScSpecTypeDef::U64 | ScSpecTypeDef::Timepoint | ScSpecTypeDef::Duration => {
            "uint64_t".into()
        }
        ScSpecTypeDef::I64 => "int64_t".into(),
        ScSpecTypeDef::U128 => "ll_u128_t".into(),
        ScSpecTypeDef::I128 => "ll_i128_t".into(),
        ScSpecTypeDef::U256 => "ll_u256_t".into(),
        ScSpecTypeDef::I256 => "ll_i256_t".into(),
        ScSpecTypeDef::Bytes => "ll_bytes_t".into(),
        ScSpecTypeDef::String => "ll_string_t".into(),
        ScSpecTypeDef::Symbol => "ll_symbol_t".into(),
        ScSpecTypeDef::Address => "ll_address_t".into(),
        ScSpecTypeDef::BytesN(b) => format!("ll_bytes{}_t", b.n),
        ScSpecTypeDef::Vec(_) => "ll_vec_t".into(),
        ScSpecTypeDef::Map(_) => "ll_map_t".into(),
        ScSpecTypeDef::Udt(u) => format!("ll_{}_t", spec_str(&u.name)),
        ScSpecTypeDef::Option(o) => {
            format!("struct {{ bool is_some; {} value; }}", c_type(&o.value_type))
        }
        ScSpecTypeDef::Result(r) if is_unit(&r.ok_type) => {
            "struct { bool is_ok; uint32_t err; }".into()
        }
        ScSpecTypeDef::Result(r) => {
            format!("struct {{ bool is_ok; {} ok; uint32_t err; }}", c_type(&r.ok_type))
        }
        ScSpecTypeDef::Tuple(t) if t.value_types.is_empty() => "ll_unit_t".into(),
        ScSpecTypeDef::Tuple(t) => {
            let fields = t
                .value_types
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{} _{i};", c_type(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("struct {{ {fields} }}")
        }
    }
}

const PRELUDE: &str = r#"#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* ── Primitive wire types ──────────────────────────────────────────────────
 * Every argument and return value travels as an XDR `ScVal`. The C types
 * below are the decoded shapes; `LL_SCV_*` are the `ScValType` tags that
 * prefix each value on the wire (big-endian int32), so a hand-written
 * encoder can be built from this header alone. */
typedef struct { uint8_t _unit; } ll_unit_t;                   /* SCV_VOID */
typedef struct { uint64_t hi; uint64_t lo; } ll_u128_t;        /* SCV_U128 */
typedef struct { int64_t hi; uint64_t lo; } ll_i128_t;         /* SCV_I128 */
typedef struct { uint64_t hi_hi, hi_lo, lo_hi, lo_lo; } ll_u256_t; /* SCV_U256 */
typedef struct { int64_t hi_hi; uint64_t hi_lo, lo_hi, lo_lo; } ll_i256_t; /* SCV_I256 */
typedef struct { const uint8_t *ptr; size_t len; } ll_bytes_t; /* SCV_BYTES */
typedef struct { const char *ptr; size_t len; } ll_string_t;   /* SCV_STRING */
typedef struct { const char *ptr; size_t len; } ll_symbol_t;   /* SCV_SYMBOL, [a-zA-Z0-9_], <= 32 */
/* SCV_ADDRESS: StrKey text ("G..." account or "C..." contract), NUL-terminated. */
typedef struct { char strkey[57]; } ll_address_t;
/* SCV_VEC / SCV_MAP: element type is given in the comment at each use site. */
typedef struct { const void *items; uint32_t len; } ll_vec_t;
typedef struct { const void *entries; uint32_t len; } ll_map_t;
/* Any `ScVal`, carried as base64 XDR text. */
typedef struct { const char *xdr_base64; } ll_val_t;
"#;

/// Render the full C header for a contract spec. `source` names where the
/// spec was read from and is embedded in the header banner.
pub fn spec_to_c_header(entries: &[ScSpecEntry], source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/* Generated by tools/schema-gen from the Soroban contract-spec of");
    let _ = writeln!(out, " * {}. Do not edit by hand.", comment(source));
    let _ =
        writeln!(out, " * Prototype ABI reference (issue #1438): see docs/c-abi-header-spike.md.");
    let _ = writeln!(out, " * Struct fields are in contract-spec (XDR) order. */");
    out.push_str("#ifndef LEDGERLENS_SCORE_H\n#define LEDGERLENS_SCORE_H\n\n");
    out.push_str(PRELUDE);

    out.push('\n');
    for kind in ScValType::VARIANTS {
        let _ = writeln!(out, "#define LL_SCV_{} {}", screaming(kind.name()), kind as i32);
    }

    let mut sizes = BTreeSet::new();
    for entry in entries {
        match entry {
            ScSpecEntry::FunctionV0(f) => {
                f.inputs.iter().for_each(|i| collect_bytes_n(&i.type_, &mut sizes));
                f.outputs.iter().for_each(|o| collect_bytes_n(o, &mut sizes));
            }
            ScSpecEntry::UdtStructV0(s) => {
                s.fields.iter().for_each(|f| collect_bytes_n(&f.type_, &mut sizes))
            }
            _ => {}
        }
    }
    if !sizes.is_empty() {
        out.push('\n');
        for n in sizes {
            let _ = writeln!(
                out,
                "typedef struct {{ uint8_t bytes[{n}]; }} ll_bytes{n}_t; /* SCV_BYTES, fixed */"
            );
        }
    }

    out.push_str(
        "\n/* ── Error codes (contracterror discriminants) ─────────────────────────── */\n",
    );
    for entry in entries {
        if let ScSpecEntry::UdtErrorEnumV0(e) = entry {
            write_error_enum(&mut out, e);
        }
    }

    out.push_str(
        "\n/* ── Enums and unions ──────────────────────────────────────────────────── */\n",
    );
    for entry in entries {
        match entry {
            ScSpecEntry::UdtEnumV0(e) => {
                let name = spec_str(&e.name);
                let _ = writeln!(out, "\ntypedef uint32_t ll_{name}_t; /* SCV_U32 */");
                for case in e.cases.iter() {
                    let _ = writeln!(
                        out,
                        "#define LL_{}_{} {}",
                        screaming(&name),
                        screaming(&spec_str(&case.name)),
                        case.value
                    );
                }
            }
            ScSpecEntry::UdtUnionV0(u) => {
                let name = spec_str(&u.name);
                let _ = writeln!(
                    out,
                    "\n/* SCV_VEC [SCV_SYMBOL case_name, ...payload]. `tag` indexes the cases below. */"
                );
                let _ = writeln!(
                    out,
                    "typedef struct {{ uint32_t tag; ll_val_t payload; }} ll_{name}_t;"
                );
                for (i, case) in u.cases.iter().enumerate() {
                    let (case_name, payload) = match case {
                        ScSpecUdtUnionCaseV0::VoidV0(v) => (spec_str(&v.name), String::new()),
                        ScSpecUdtUnionCaseV0::TupleV0(t) => (
                            spec_str(&t.name),
                            format!(
                                " /* ({}) */",
                                t.type_.iter().map(spec_type_name).collect::<Vec<_>>().join(", ")
                            ),
                        ),
                    };
                    let _ = writeln!(
                        out,
                        "#define LL_{}_TAG_{} {i}{payload}",
                        screaming(&name),
                        screaming(&case_name)
                    );
                }
            }
            _ => {}
        }
    }

    out.push_str(
        "\n/* ── Structs (SCV_MAP keyed by SCV_SYMBOL field name) ──────────────────── */\n",
    );
    let structs: Vec<&ScSpecUdtStructV0> = entries
        .iter()
        .filter_map(|e| match e {
            ScSpecEntry::UdtStructV0(s) => Some(s),
            _ => None,
        })
        .collect();
    for struct_ in order_structs(&structs) {
        write_struct(&mut out, struct_);
    }

    out.push_str(
        "\n/* ── Functions ─────────────────────────────────────────────────────────── */\n",
    );
    for entry in entries {
        if let ScSpecEntry::FunctionV0(f) = entry {
            write_function(&mut out, f);
        }
    }

    out.push_str("\n#endif /* LEDGERLENS_SCORE_H */\n");
    out
}

fn write_error_enum(out: &mut String, e: &ScSpecUdtErrorEnumV0) {
    let name = spec_str(&e.name);
    let _ = writeln!(out, "\ntypedef uint32_t ll_{name}_t; /* SCV_ERROR (contract, code) */");
    for case in e.cases.iter() {
        let _ = writeln!(
            out,
            "#define LL_{}_{} {}",
            screaming(&name),
            screaming(&spec_str(&case.name)),
            case.value
        );
    }
}

/// Order structs so every struct is defined after the structs it embeds by
/// value. Any leftover (recursive) structs are appended in spec order.
fn order_structs<'a>(structs: &[&'a ScSpecUdtStructV0]) -> Vec<&'a ScSpecUdtStructV0> {
    let names: BTreeSet<String> = structs.iter().map(|s| spec_str(&s.name)).collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    let mut remaining: Vec<&ScSpecUdtStructV0> = structs.to_vec();
    loop {
        let before = remaining.len();
        remaining.retain(|s| {
            let mut deps = BTreeSet::new();
            s.fields.iter().for_each(|f| by_value_udts(&f.type_, &mut deps));
            let ready = deps.iter().all(|d| !names.contains(d) || emitted.contains(d));
            if ready {
                emitted.insert(spec_str(&s.name));
                ordered.push(*s);
            }
            !ready
        });
        if remaining.is_empty() || remaining.len() == before {
            break;
        }
    }
    ordered.extend(remaining);
    ordered
}

fn write_struct(out: &mut String, s: &ScSpecUdtStructV0) {
    let name = spec_str(&s.name);
    let doc = spec_str(&s.doc);
    out.push('\n');
    if !doc.is_empty() {
        let _ = writeln!(out, "/* {} */", comment(doc.lines().next().unwrap_or_default()));
    }
    let _ = writeln!(out, "typedef struct {{");
    for field in s.fields.iter() {
        let _ = writeln!(
            out,
            "    {} {}; /* {} */",
            c_type(&field.type_),
            ident(&spec_str(&field.name)),
            spec_type_name(&field.type_)
        );
    }
    let _ = writeln!(out, "}} ll_{name}_t;");
}

fn write_function(out: &mut String, f: &ScSpecFunctionV0) {
    let name = spec_str(&f.name.0);
    let upper = screaming(&name);
    let signature = format!(
        "{name}({}){}",
        f.inputs
            .iter()
            .map(|i| format!("{}:{}", spec_str(&i.name), spec_type_name(&i.type_)))
            .collect::<Vec<_>>()
            .join(","),
        f.outputs.iter().map(|o| format!("->{}", spec_type_name(o))).collect::<String>()
    );
    out.push('\n');
    let _ = writeln!(out, "/* fn {}", comment(&signature));
    for line in spec_str(&f.doc).lines().take(3) {
        let _ = writeln!(out, " * {}", comment(line.trim()));
    }
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "#define LL_FN_{upper} \"{name}\"");
    let _ = writeln!(out, "#define LL_FN_{upper}_ARGC {}", f.inputs.len());
    if !f.inputs.is_empty() {
        let _ = writeln!(out, "typedef struct {{");
        for input in f.inputs.iter() {
            let _ = writeln!(
                out,
                "    {} {}; /* {} */",
                c_type(&input.type_),
                ident(&spec_str(&input.name)),
                spec_type_name(&input.type_)
            );
        }
        let _ = writeln!(out, "}} ll_{name}_args_t;");
    }
    if let Some(output) = f.outputs.first().filter(|o| !is_unit(o)) {
        let _ = writeln!(out, "typedef {} ll_{name}_ret_t;", c_type(output));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::xdr::{
        ScSpecFunctionInputV0, ScSpecTypeOption, ScSpecTypeResult, ScSpecUdtErrorEnumCaseV0,
        ScSpecUdtStructFieldV0, ScSymbol,
    };

    fn field(name: &str, type_: ScSpecTypeDef) -> ScSpecUdtStructFieldV0 {
        ScSpecUdtStructFieldV0 { doc: Default::default(), name: name.try_into().unwrap(), type_ }
    }

    fn sample() -> Vec<ScSpecEntry> {
        let risk = ScSpecEntry::UdtStructV0(ScSpecUdtStructV0 {
            doc: "Canonical risk score.".try_into().unwrap(),
            lib: Default::default(),
            name: "RiskScore".try_into().unwrap(),
            fields: vec![
                field("benford_flag", ScSpecTypeDef::Bool),
                field(
                    "commitment",
                    ScSpecTypeDef::Option(Box::new(ScSpecTypeOption {
                        value_type: Box::new(ScSpecTypeDef::Bytes),
                    })),
                ),
                field("score", ScSpecTypeDef::U32),
                field("timestamp", ScSpecTypeDef::U64),
            ]
            .try_into()
            .unwrap(),
        });
        let error = ScSpecEntry::UdtErrorEnumV0(ScSpecUdtErrorEnumV0 {
            doc: Default::default(),
            lib: Default::default(),
            name: "Error".try_into().unwrap(),
            cases: vec![ScSpecUdtErrorEnumCaseV0 {
                doc: Default::default(),
                name: "ScoreNotFound".try_into().unwrap(),
                value: 4,
            }]
            .try_into()
            .unwrap(),
        });
        let get_score = ScSpecEntry::FunctionV0(ScSpecFunctionV0 {
            doc: Default::default(),
            name: ScSymbol("get_score".try_into().unwrap()),
            inputs: vec![
                ScSpecFunctionInputV0 {
                    doc: Default::default(),
                    name: "wallet".try_into().unwrap(),
                    type_: ScSpecTypeDef::Address,
                },
                ScSpecFunctionInputV0 {
                    doc: Default::default(),
                    name: "asset_pair".try_into().unwrap(),
                    type_: ScSpecTypeDef::Symbol,
                },
            ]
            .try_into()
            .unwrap(),
            outputs: vec![ScSpecTypeDef::Result(Box::new(ScSpecTypeResult {
                ok_type: Box::new(ScSpecTypeDef::Udt(soroban_sdk::xdr::ScSpecTypeUdt {
                    name: "RiskScore".try_into().unwrap(),
                })),
                error_type: Box::new(ScSpecTypeDef::Error),
            }))]
            .try_into()
            .unwrap(),
        });
        vec![get_score, error, risk]
    }

    #[test]
    fn header_contains_signature_types_and_error_codes() {
        let header = spec_to_c_header(&sample(), "unit-test spec");
        assert!(header.contains("#define LL_FN_GET_SCORE \"get_score\""));
        assert!(header.contains("#define LL_FN_GET_SCORE_ARGC 2"));
        assert!(header.contains("ll_address_t wallet; /* address */"));
        assert!(header.contains("#define LL_ERROR_SCORE_NOT_FOUND 4"));
        assert!(header.contains("struct { bool is_some; ll_bytes_t value; } commitment;"));
        assert!(header.contains(
            "typedef struct { bool is_ok; ll_RiskScore_t ok; uint32_t err; } ll_get_score_ret_t;"
        ));
        assert!(header.contains("#define LL_SCV_U32 3"));
        assert!(header.contains("#define LL_SCV_ADDRESS 18"));
    }

    #[test]
    fn struct_fields_keep_spec_order() {
        let header = spec_to_c_header(&sample(), "unit-test spec");
        let pos = |needle: &str| header.find(needle).unwrap();
        assert!(pos(" benford_flag;") < pos(" commitment;"));
        assert!(pos(" commitment;") < pos(" score;"));
        assert!(pos(" score;") < pos(" timestamp;"));
        // A struct used by value is defined before the function that returns it.
        assert!(pos("} ll_RiskScore_t;") < pos("ll_get_score_ret_t;"));
    }

    #[test]
    fn keywords_and_names_are_c_safe() {
        assert_eq!(ident("default"), "default_");
        assert_eq!(ident("score"), "score");
        assert_eq!(screaming("ScoreNotFound"), "SCORE_NOT_FOUND");
        assert_eq!(screaming("get_score"), "GET_SCORE");
        assert_eq!(comment("a */ b"), "a * / b");
    }
}
