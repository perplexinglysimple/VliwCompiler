use vliw_asm::opcode::Opcode;
use vliw_backend::mir::Terminator;
use vliw_backend::{OptimizationLevel, compile_to_mir};

const IF_LT: &str = include_str!("fixtures/cmp/if_lt.ll");

/// The fixture `if (x < y) return 1; else return 0;` must produce exactly
/// one cmp syllable, one conditional branch terminator, and two return paths.
#[test]
fn if_lt_structure() {
    let func = compile_to_mir(IF_LT, OptimizationLevel::None)
        .expect("if_lt should compile");

    // Exactly one cmp syllable across all blocks.
    let cmp_count = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .filter(|s| matches!(s.opcode, Opcode::CmpEq | Opcode::CmpLt | Opcode::CmpUlt))
        .count();
    assert_eq!(cmp_count, 1, "expected exactly one cmp syllable, got {cmp_count}");

    // Exactly one conditional branch terminator.
    let branch_count = func
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, Terminator::Branch { .. }))
        .count();
    assert_eq!(branch_count, 1, "expected exactly one Branch terminator, got {branch_count}");

    // Exactly two return paths.
    let return_count = func
        .blocks
        .iter()
        .filter(|b| b.terminator == Terminator::Return)
        .count();
    assert_eq!(return_count, 2, "expected exactly two Return terminators, got {return_count}");
}

/// The slt comparison in the fixture must lower to CmpLt (not CmpEq or CmpUlt).
#[test]
fn if_lt_uses_cmplt() {
    let func = compile_to_mir(IF_LT, OptimizationLevel::None).unwrap();
    let has_cmplt = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .any(|s| s.opcode == Opcode::CmpLt);
    assert!(has_cmplt, "icmp slt should lower to CmpLt");
}

/// The conditional branch must reference the vreg produced by the cmp.
#[test]
fn branch_cond_is_cmp_result() {
    let func = compile_to_mir(IF_LT, OptimizationLevel::None).unwrap();

    // Find the cmp syllable's destination vreg.
    let cmp_dst = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .find(|s| s.opcode == Opcode::CmpLt)
        .and_then(|s| s.dst)
        .expect("CmpLt should have a destination");

    // Find the Branch terminator and check its condition register.
    let branch_cond = func
        .blocks
        .iter()
        .find_map(|b| match &b.terminator {
            Terminator::Branch { cond, .. } => Some(*cond),
            _ => None,
        })
        .expect("should have a Branch terminator");

    assert_eq!(
        branch_cond, cmp_dst,
        "branch condition should be the cmp result vreg"
    );
}

// --- icmp predicate mapping tests ---

fn single_cmp_opcode(ir: &str) -> Opcode {
    let func = compile_to_mir(ir, OptimizationLevel::None)
        .expect("compile_to_mir should succeed");
    func.blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .find(|s| matches!(s.opcode, Opcode::CmpEq | Opcode::CmpLt | Opcode::CmpUlt))
        .expect("should have exactly one cmp syllable")
        .opcode
}

#[test]
fn icmp_eq_lowers_to_cmpeq() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp eq i32 %a, %b
  ret i1 %r
}"#;
    assert_eq!(single_cmp_opcode(ir), Opcode::CmpEq);
}

#[test]
fn icmp_slt_lowers_to_cmplt() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp slt i32 %a, %b
  ret i1 %r
}"#;
    assert_eq!(single_cmp_opcode(ir), Opcode::CmpLt);
}

#[test]
fn icmp_sgt_lowers_to_cmplt_swapped() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp sgt i32 %a, %b
  ret i1 %r
}"#;
    // sgt(a, b) → cmplt(b, a)
    assert_eq!(single_cmp_opcode(ir), Opcode::CmpLt);
}

#[test]
fn icmp_ult_lowers_to_cmpult() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp ult i32 %a, %b
  ret i1 %r
}"#;
    assert_eq!(single_cmp_opcode(ir), Opcode::CmpUlt);
}

#[test]
fn icmp_ugt_lowers_to_cmpult_swapped() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp ugt i32 %a, %b
  ret i1 %r
}"#;
    // ugt(a, b) → cmpult(b, a)
    assert_eq!(single_cmp_opcode(ir), Opcode::CmpUlt);
}

#[test]
fn icmp_ne_lowers_to_cmpeq_plus_pnot() {
    let ir = r#"define i1 @f(i32 %a, i32 %b) {
entry:
  %r = icmp ne i32 %a, %b
  ret i1 %r
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let opcodes: Vec<_> = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter().map(|s| s.opcode))
        .collect();
    assert!(
        opcodes.contains(&Opcode::CmpEq),
        "ne should emit CmpEq"
    );
    assert!(
        opcodes.contains(&Opcode::PNot),
        "ne should emit PNot to invert"
    );
}
