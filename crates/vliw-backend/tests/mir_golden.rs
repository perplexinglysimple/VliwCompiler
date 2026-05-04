use vliw_backend::{demo_mir, emit_mir};

#[test]
fn demo_mir_matches_golden() {
    let golden = include_str!("fixtures/demo.mir");
    let got = emit_mir(&demo_mir());
    assert_eq!(got, golden, "MIR dump differs from golden file");
}
