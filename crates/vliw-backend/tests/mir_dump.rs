use vliw_backend::{compile_to_mir, emit_mir, OptimizationLevel};

#[test]
fn add_two_mir_matches_golden() {
    let golden = include_str!("fixtures/mir_dump/add_two.mir");
    let ir = include_str!("fixtures/mir_dump/add_two.ll");
    let func =
        compile_to_mir(ir, OptimizationLevel::None).expect("add_two should compile to MIR");
    let got = emit_mir(&func);
    assert_eq!(got, golden, "MIR dump differs from golden file");
}
