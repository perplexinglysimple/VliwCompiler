use vliw_backend::{compile, CompileError, OptimizationLevel, Schedule};

fn assert_unsupported(ir: &str) {
    match compile(ir, OptimizationLevel::None, Schedule::Scalar) {
        Err(CompileError::UnsupportedFeature(msg)) => {
            assert!(
                !msg.is_empty(),
                "UnsupportedFeature message must not be empty"
            );
        }
        other => panic!("expected UnsupportedFeature, got {:?}", other),
    }
}

// Multiple function definitions are now supported (LATER-2).
#[test]
fn multi_func_compiles() {
    let ir = include_str!("fixtures/multi_func.ll");
    compile(ir, OptimizationLevel::None, Schedule::Scalar)
        .expect("multi-function module should compile after LATER-2");
}

// Calls to external declarations (no body) are still rejected by ISel.
#[test]
fn call_to_external_rejected() {
    let ir = include_str!("fixtures/call.ll");
    assert_unsupported(ir);
}

#[test]
fn bad_int_width_rejected() {
    let ir = include_str!("fixtures/bad_int_width.ll");
    assert_unsupported(ir);
}

#[test]
fn atomic_rejected() {
    let ir = include_str!("fixtures/atomic.ll");
    assert_unsupported(ir);
}

// phi nodes are now supported by the phi-lowering pass (LLVM-7); see phi_isel.rs.
