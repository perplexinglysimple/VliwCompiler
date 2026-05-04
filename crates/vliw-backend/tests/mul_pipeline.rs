use vliw_asm::{parse, Item};
use vliw_backend::{compile, OptimizationLevel, Schedule};

const MUL_DEPENDENT_STORE: &str = include_str!("fixtures/mul_dependent_store.ll");

#[test]
fn mul_dependent_store_emits_latency_padding() {
    let text = compile(
        MUL_DEPENDENT_STORE,
        OptimizationLevel::None,
        Schedule::Scalar,
    )
    .expect("mul_dependent_store.ll should compile through VLIW emission");
    let program = parse(&text).expect("emitted VLIW should parse");

    let bundles: Vec<_> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Bundle(bundle) => Some(bundle),
            Item::Label(_) => None,
        })
        .collect();

    let mul_idx = bundles
        .iter()
        .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "mul"))
        .expect("compiler fixture should emit mul");
    // Find the dependent store: first stw *after* the mul instruction.
    let store_idx = bundles[mul_idx + 1..]
        .iter()
        .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "stw"))
        .map(|rel| mul_idx + 1 + rel)
        .expect("compiler fixture should emit dependent stw after mul");

    assert!(
        store_idx >= mul_idx + 3,
        "dependent store should wait for mul latency; output:\n{text}"
    );
    assert!(
        bundles[mul_idx + 1..store_idx]
            .iter()
            .all(|bundle| bundle.slots.iter().all(Option::is_none)),
        "mul latency padding should be empty nop bundles; output:\n{text}"
    );
}
