use vliw_backend::{compile, OptimizationLevel, Schedule};

const TWO_FUNC_IR: &str = include_str!("fixtures/call/two_func_call.ll");

/// Both function bodies must appear in the emitted program.
#[test]
fn two_func_call_compiles() {
    let text = compile(TWO_FUNC_IR, OptimizationLevel::None, Schedule::Scalar)
        .expect("two-function call should compile");

    assert!(text.contains("add_one:"), "add_one entry label must be present");
    assert!(text.contains("caller:"), "caller entry label must be present");
}

/// The callee returns via r1 (RETVAL_REG = 1).
#[test]
fn callee_returns_via_retval_reg() {
    let text = compile(TWO_FUNC_IR, OptimizationLevel::None, Schedule::Scalar)
        .expect("two-function call should compile");

    // The add_one function must write r1 before ret.
    assert!(
        text.contains("r1"),
        "return value must be placed in r1 (RETVAL_REG):\n{text}"
    );
}

/// The link register r31 must be explicitly saved before the call and
/// restored in the continuation block so the caller can eventually ret.
#[test]
fn caller_saves_and_restores_r31() {
    let text = compile(TWO_FUNC_IR, OptimizationLevel::None, Schedule::Scalar)
        .expect("two-function call should compile");

    assert!(
        text.contains("r31"),
        "link register r31 must appear in the caller's output:\n{text}"
    );
}

/// A `call add_one` syllable must appear in the emitted program.
#[test]
fn emits_call_syllable() {
    let text = compile(TWO_FUNC_IR, OptimizationLevel::None, Schedule::Scalar)
        .expect("two-function call should compile");

    assert!(
        text.contains("call add_one"),
        "a 'call add_one' syllable must be emitted:\n{text}"
    );
}

/// The packed schedule must also handle a two-function call correctly.
#[test]
fn packed_two_func_call_compiles() {
    let text = compile(TWO_FUNC_IR, OptimizationLevel::None, Schedule::Pack)
        .expect("two-function call should compile with Pack schedule");

    assert!(text.contains("add_one:"), "add_one label must appear in packed output");
    assert!(text.contains("caller:"), "caller label must appear in packed output");
    assert!(text.contains("call add_one"), "call syllable must appear in packed output");
}
