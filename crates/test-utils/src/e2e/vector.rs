//! Parser for the raw calldata/returndata vectors used by upstream Solcore.
//!
//! These vectors deliberately bypass the compiler's ABI model. That makes
//! them suitable for bringing up dynamic ABI features before the typed E2E
//! directive resolver knows how to describe those types.

use std::{fs, path::Path};

use serde_json::{Map, Value};

use super::{
    E2eFailure, FailureKind, ResolvedE2eCall, ResolvedExpectedOutcome, Word256,
    configured_anvil_hardfork, decode_hex_data, encode_hex,
};

/// EVM target used to compile and execute every upstream raw vector.
///
/// The fixture's optional `evmVersion` is retained as upstream metadata, but
/// both backend runners intentionally use one Osaka runtime for the complete
/// raw-vector corpus.
pub const RAW_E2E_TARGET_EVM_VERSION: &str = "osaka";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum E2eExecution {
    Directives(Vec<ResolvedE2eCall>),
    Raw(RawE2eVector),
}

impl E2eExecution {
    pub fn effective_evm_version(&self) -> String {
        match self {
            Self::Directives(_) => configured_anvil_hardfork(),
            Self::Raw(vector) => vector.effective_evm_version().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawE2eVector {
    pub name: String,
    pub contract: String,
    pub evm_version: Option<String>,
    pub constructor: RawE2eConstructor,
    pub calls: Vec<RawE2eCall>,
}

impl RawE2eVector {
    pub fn effective_evm_version(&self) -> &str {
        RAW_E2E_TARGET_EVM_VERSION
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawE2eConstructor {
    /// Constructor arguments appended to creation bytecode.
    pub calldata: Vec<u8>,
    pub value: Word256,
    pub expected_success: bool,
    /// Optional constructor output to compare for either success or failure.
    pub expected_output: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawE2eCall {
    pub label: String,
    /// Complete call data, including the selector.
    pub calldata: String,
    pub value: Word256,
    pub expected: ResolvedExpectedOutcome,
}

impl RawE2eCall {
    pub fn value_rpc_quantity(&self) -> String {
        word_rpc_quantity(self.value)
    }
}

impl RawE2eConstructor {
    pub fn value_rpc_quantity(&self) -> String {
        word_rpc_quantity(self.value)
    }
}

/// Loads `main.json` next to a `main.solc` fixture when it exists.
pub fn load_raw_e2e_vector(source_path: &Path) -> Result<Option<RawE2eVector>, E2eFailure> {
    let vector_path = source_path.with_extension("json");
    let source = match fs::read_to_string(&vector_path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(vector_failure(format!(
                "failed to read raw E2E vector {}: {error}",
                vector_path.display()
            )));
        }
    };
    parse_raw_e2e_vector(&source).map(Some).map_err(|failure| {
        E2eFailure::new(
            failure.kind,
            format!("{}: {}", vector_path.display(), failure.message),
        )
    })
}

/// Parses one upstream-compatible dispatch test vector.
///
/// The top-level object must contain exactly one named suite. Constructor
/// calldata is retained separately so a backend's creation bytecode can be
/// substituted for the upstream `_CODE` placeholder.
pub fn parse_raw_e2e_vector(source: &str) -> Result<RawE2eVector, E2eFailure> {
    let root: Value = serde_json::from_str(source)
        .map_err(|error| vector_failure(format!("invalid JSON: {error}")))?;
    let suites = object(&root, "top-level value")?;
    if suites.len() != 1 {
        return Err(vector_failure(format!(
            "expected exactly one test suite, found {}",
            suites.len()
        )));
    }
    let (name, suite) = suites.iter().next().expect("suite count checked");
    parse_suite(name, object(suite, &format!("suite `{name}`"))?)
}

fn parse_suite(name: &str, suite: &Map<String, Value>) -> Result<RawE2eVector, E2eFailure> {
    let contract = required_string(suite, "contract", name)?.to_owned();
    if contract.is_empty() {
        return Err(vector_failure(format!(
            "suite `{name}` has an empty contract name"
        )));
    }
    let evm_version = optional_string(suite, "evmVersion", name)?.map(str::to_owned);
    let tests = suite
        .get("tests")
        .ok_or_else(|| vector_failure(format!("suite `{name}` has no `tests` array")))?
        .as_array()
        .ok_or_else(|| vector_failure(format!("suite `{name}`: `tests` must be an array")))?;

    let mut constructor = None;
    let mut calls = Vec::new();
    for (index, test) in tests.iter().enumerate() {
        let context = format!("suite `{name}` test #{}", index + 1);
        let test = object(test, &context)?;
        match required_string(test, "kind", &context)? {
            "constructor" => {
                if constructor.is_some() {
                    return Err(vector_failure(format!(
                        "{context}: duplicate constructor vector"
                    )));
                }
                if !calls.is_empty() {
                    return Err(vector_failure(format!(
                        "{context}: constructor must precede every call"
                    )));
                }
                constructor = Some(parse_constructor(test, &context)?);
            }
            "call" => calls.push(parse_call(test, &context)?),
            kind => {
                return Err(vector_failure(format!(
                    "{context}: unsupported test kind `{kind}`"
                )));
            }
        }
    }

    let has_explicit_constructor = constructor.is_some();
    let constructor = constructor.unwrap_or(RawE2eConstructor {
        calldata: Vec::new(),
        value: Word256::ZERO,
        expected_success: true,
        expected_output: None,
    });
    if !constructor.expected_success && !calls.is_empty() {
        return Err(vector_failure(format!(
            "suite `{name}` cannot contain calls after a failing constructor"
        )));
    }
    if calls.is_empty() && !has_explicit_constructor {
        return Err(vector_failure(format!(
            "suite `{name}` contains neither a constructor nor an executable call"
        )));
    }

    Ok(RawE2eVector {
        name: name.to_owned(),
        contract,
        evm_version,
        constructor,
        calls,
    })
}

fn parse_constructor(
    test: &Map<String, Value>,
    context: &str,
) -> Result<RawE2eConstructor, E2eFailure> {
    let input = input(test, context)?;
    let calldata = parse_hex_field(input, "calldata", context)?;
    let value = parse_value(input, context)?;
    let (expected_success, expected_output) = match test.get("output") {
        None => (true, None),
        Some(output) => {
            let output = object(output, &format!("{context} output"))?;
            let status = required_string(output, "status", context)?;
            let data = output
                .get("returndata")
                .map(|_| parse_hex_field(output, "returndata", context))
                .transpose()?;
            match status {
                "success" => (true, data),
                "failure" => (false, data),
                status => {
                    return Err(vector_failure(format!(
                        "{context}: unsupported constructor status `{status}`"
                    )));
                }
            }
        }
    };
    Ok(RawE2eConstructor {
        calldata,
        value,
        expected_success,
        expected_output,
    })
}

fn parse_call(test: &Map<String, Value>, context: &str) -> Result<RawE2eCall, E2eFailure> {
    let input = input(test, context)?;
    let calldata = parse_hex_field(input, "calldata", context)?;
    let value = parse_value(input, context)?;
    let label = input
        .get("text-calldata")
        .or_else(|| input.get("comment"))
        .and_then(Value::as_str)
        .unwrap_or(context)
        .to_owned();
    let output = object(
        test.get("output")
            .ok_or_else(|| vector_failure(format!("{context}: call has no output")))?,
        &format!("{context} output"),
    )?;
    let returndata = parse_hex_field(output, "returndata", context)?;
    let expected = match required_string(output, "status", context)? {
        "success" => ResolvedExpectedOutcome::Return(returndata),
        "failure" => ResolvedExpectedOutcome::Revert(Some(returndata)),
        status => {
            return Err(vector_failure(format!(
                "{context}: unsupported call status `{status}`"
            )));
        }
    };
    Ok(RawE2eCall {
        label,
        calldata: format!("0x{}", encode_hex(&calldata)),
        value,
        expected,
    })
}

fn input<'a>(
    test: &'a Map<String, Value>,
    context: &str,
) -> Result<&'a Map<String, Value>, E2eFailure> {
    object(
        test.get("input")
            .ok_or_else(|| vector_failure(format!("{context}: test has no input")))?,
        &format!("{context} input"),
    )
}

fn parse_value(input: &Map<String, Value>, context: &str) -> Result<Word256, E2eFailure> {
    let literal = required_string(input, "value", context)?;
    literal
        .parse::<Word256>()
        .map_err(|error| vector_failure(format!("{context}: invalid value `{literal}`: {error}")))
}

fn parse_hex_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Vec<u8>, E2eFailure> {
    let data = required_string(object, field, context)?;
    decode_hex_data(data)
        .map_err(|error| vector_failure(format!("{context}: invalid `{field}`: {error}")))
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, E2eFailure> {
    value
        .as_object()
        .ok_or_else(|| vector_failure(format!("{context} must be an object")))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, E2eFailure> {
    object
        .get(field)
        .ok_or_else(|| vector_failure(format!("{context}: missing `{field}`")))?
        .as_str()
        .ok_or_else(|| vector_failure(format!("{context}: `{field}` must be a string")))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Option<&'a str>, E2eFailure> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| vector_failure(format!("{context}: `{field}` must be a string")))
        })
        .transpose()
}

fn word_rpc_quantity(word: Word256) -> String {
    let bytes = word.as_be_bytes();
    let first = bytes.iter().position(|byte| *byte != 0);
    let Some(first) = first else {
        return "0x0".to_owned();
    };
    let encoded = encode_hex(&bytes[first..]);
    let encoded = encoded.strip_prefix('0').unwrap_or(&encoded);
    format!("0x{encoded}")
}

fn vector_failure(message: impl Into<String>) -> E2eFailure {
    E2eFailure::new(FailureKind::Directive, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTOR: &str = r#"
    {
      "arrays": {
        "bytecode": "_CODE",
        "contract": "ArrayOps",
        "evmVersion": "osaka",
        "tests": [
          {
            "input": { "comment": "constructor()", "calldata": "", "value": "0" },
            "kind": "constructor"
          },
          {
            "input": {
              "text-calldata": "roundtrip(uint256[])",
              "calldata": "0x123456780000",
              "value": "42"
            },
            "kind": "call",
            "output": { "returndata": "0020aabb", "status": "success" }
          },
          {
            "input": { "comment": "bad bool", "calldata": "abcdef01", "value": "0" },
            "kind": "call",
            "output": { "returndata": "4fbfae63", "status": "failure" }
          }
        ]
      }
    }
    "#;

    #[test]
    fn parses_upstream_raw_vectors_without_static_abi_shapes() {
        let vector = parse_raw_e2e_vector(VECTOR).expect("valid vector");
        assert_eq!(vector.name, "arrays");
        assert_eq!(vector.contract, "ArrayOps");
        assert_eq!(vector.evm_version.as_deref(), Some("osaka"));
        assert_eq!(vector.effective_evm_version(), "osaka");
        assert_eq!(vector.constructor.value, Word256::ZERO);
        assert!(vector.constructor.expected_success);
        assert_eq!(vector.constructor.expected_output, None);
        assert_eq!(vector.calls.len(), 2);
        assert_eq!(vector.calls[0].calldata, "0x123456780000");
        assert_eq!(vector.calls[0].value_rpc_quantity(), "0x2a");
        assert_eq!(
            vector.calls[0].expected,
            ResolvedExpectedOutcome::Return(vec![0x00, 0x20, 0xaa, 0xbb])
        );
        assert_eq!(
            vector.calls[1].expected,
            ResolvedExpectedOutcome::Revert(Some(vec![0x4f, 0xbf, 0xae, 0x63]))
        );
    }

    #[test]
    fn retains_constructor_status_and_output() {
        let vector = parse_raw_e2e_vector(
            r#"{
              "constructor-output": {
                "contract": "FailsToDeploy",
                "tests": [{
                  "input": { "calldata": "1234", "value": "0" },
                  "kind": "constructor",
                  "output": { "returndata": "4fbfae63", "status": "failure" }
                }]
              }
            }"#,
        )
        .expect("valid failing constructor vector");
        assert!(!vector.constructor.expected_success);
        assert_eq!(
            vector.constructor.expected_output,
            Some(vec![0x4f, 0xbf, 0xae, 0x63])
        );

        let successful = parse_raw_e2e_vector(
            r#"{
              "constructor-output": {
                "contract": "Deploys",
                "tests": [{
                  "input": { "calldata": "", "value": "0" },
                  "kind": "constructor",
                  "output": { "returndata": "6000", "status": "success" }
                }]
              }
            }"#,
        )
        .expect("valid successful constructor-only vector");
        assert!(successful.constructor.expected_success);
        assert_eq!(
            successful.constructor.expected_output,
            Some(vec![0x60, 0x00])
        );
        assert!(successful.calls.is_empty());
        assert_eq!(successful.evm_version, None);
        assert_eq!(successful.effective_evm_version(), "osaka");
    }

    #[test]
    fn raw_vectors_always_target_osaka_without_rewriting_metadata() {
        for (declared, expected_metadata) in [
            (None, None),
            (Some("prague"), Some("prague")),
            (Some("osaka"), Some("osaka")),
        ] {
            let evm_version = declared
                .map(|version| format!(r#", "evmVersion": "{version}""#))
                .unwrap_or_default();
            let vector = parse_raw_e2e_vector(&format!(
                r#"{{
                  "target": {{
                    "contract": "C"{evm_version},
                    "tests": [{{
                      "input": {{ "calldata": "", "value": "0" }},
                      "kind": "constructor"
                    }}]
                  }}
                }}"#
            ))
            .expect("valid target vector");

            assert_eq!(vector.evm_version.as_deref(), expected_metadata);
            assert_eq!(vector.effective_evm_version(), RAW_E2E_TARGET_EVM_VERSION);
            assert_eq!(
                E2eExecution::Raw(vector).effective_evm_version(),
                RAW_E2E_TARGET_EVM_VERSION
            );
        }
    }

    #[test]
    fn rejects_ambiguous_or_malformed_vectors() {
        let ambiguous = parse_raw_e2e_vector(r#"{"a": {}, "b": {}}"#).unwrap_err();
        assert!(ambiguous.message.contains("exactly one test suite"));

        let odd_hex = VECTOR.replace("abcdef01", "abc");
        let error = parse_raw_e2e_vector(&odd_hex).unwrap_err();
        assert!(error.message.contains("even number of hex"), "{error}");
    }

    #[test]
    fn renders_full_width_rpc_quantities() {
        assert_eq!(word_rpc_quantity(Word256::ZERO), "0x0");
        assert_eq!(word_rpc_quantity(Word256::from_u128(0x0102)), "0x102");
        assert_eq!(
            word_rpc_quantity(
                "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .parse()
                    .unwrap()
            ),
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        );
    }
}
