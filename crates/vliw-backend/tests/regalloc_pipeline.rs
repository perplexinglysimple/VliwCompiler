use vliw_backend::{compile, OptimizationLevel, Schedule};

const LIVE_29: &str = include_str!("fixtures/regalloc/live_29.ll");
const LIVE_40: &str = include_str!("fixtures/regalloc/live_40.ll");
const LIVENESS_REUSE: &str = include_str!("fixtures/regalloc/liveness_reuse.ll");

#[test]
fn fixture_with_29_simultaneously_live_values_compiles() {
    let text = compile(LIVE_29, OptimizationLevel::None, Schedule::Scalar)
        .expect("live_29 fixture should fit in the allocatable register set");

    assert!(text.contains("ret"));
    assert!(
        !text.contains("r31"),
        "allocator must not use the link register"
    );
}

#[test]
fn fixture_with_40_simultaneously_live_values_spills() {
    let text = compile(LIVE_40, OptimizationLevel::None, Schedule::Scalar)
        .expect("live_40 fixture should compile by spilling excess live values");

    assert!(text.contains("ret"));
    assert!(text.contains("std [r0 + 0xf000]"), "expected spill stores in output");
    assert!(text.contains("ldd "), "expected spill reloads in output");
    assert!(
        !text.contains("r31"),
        "allocator must not use the link register"
    );
}

/// REGS-3: liveness-aware reuse.
///
/// The fixture defines 31 virtual registers (1 parameter + 30 results) in a
/// straight-line chain where each value is used exactly once in the next
/// definition and is then dead.  Only the final value (%v29) is live-out of
/// `entry` and therefore kept across the block boundary.
///
/// The liveness-aware allocator recycles the same physical register (r2) for
/// every definition: each value's register is freed as soon as its single use
/// is consumed, so the next definition can reuse it.  The entire computation
/// requires just one allocatable GPR — `r3` must not appear in the output.
#[test]
fn fixture_with_chain_of_values_reuses_single_register() {
    let text = compile(LIVENESS_REUSE, OptimizationLevel::None, Schedule::Scalar)
        .expect("liveness_reuse fixture should compile: each definition is freed at its single use, recycling r2 for all 31 virtual registers");

    assert!(text.contains("ret"), "output should contain ret");
    assert!(
        !text.contains("r3"),
        "with sequential register reuse only r1 (return) and r2 (computation) should appear;\
         r3 appearing means the allocator failed to recycle registers"
    );
}
