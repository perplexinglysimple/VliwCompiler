use vliw_asm::opcode::Opcode;
use vliw_backend::mir::{Reg, Terminator, Value};
use vliw_backend::regs::RETVAL_REG;
use vliw_backend::{OptimizationLevel, compile_to_mir};

/// `int main() { return 7; }` must produce exactly one `movi r1, 7` syllable
/// followed by a `ret` terminator.
#[test]
fn return_const_emits_movi_retval_reg() {
    let func = compile_to_mir(
        include_str!("fixtures/ret/return_7.ll"),
        OptimizationLevel::None,
    )
    .expect("return_7.ll should compile");

    let block = &func.blocks[0];

    // Exactly one syllable.
    assert_eq!(
        block.syllables.len(),
        1,
        "expected exactly one syllable, got {:?}",
        block.syllables
    );

    let syl = &block.syllables[0];
    assert_eq!(syl.opcode, Opcode::MovImm, "opcode should be movi");
    assert_eq!(
        syl.dst,
        Some(Reg::PReg(RETVAL_REG)),
        "destination should be RETVAL_REG (r{RETVAL_REG})"
    );
    assert_eq!(syl.srcs, [Value::Imm(7)], "immediate should be 7");

    assert_eq!(block.terminator, Terminator::Return, "terminator should be ret");
}

/// `ret void` produces no syllables, just a `ret` terminator.
#[test]
fn return_void_emits_no_syllables() {
    let func = compile_to_mir(
        include_str!("fixtures/ret/return_void.ll"),
        OptimizationLevel::None,
    )
    .expect("return_void.ll should compile");

    let block = &func.blocks[0];
    assert!(block.syllables.is_empty(), "ret void should emit no syllables");
    assert_eq!(block.terminator, Terminator::Return);
}

/// `ret i32 %x` (returning a parameter) emits `mov r1, <vreg>`.
#[test]
fn return_param_emits_mov_retval_reg() {
    let func = compile_to_mir(
        include_str!("fixtures/ret/return_param.ll"),
        OptimizationLevel::None,
    )
    .expect("return_param.ll should compile");

    let block = &func.blocks[0];
    assert_eq!(block.syllables.len(), 1, "expected exactly one syllable");

    let syl = &block.syllables[0];
    assert_eq!(syl.opcode, Opcode::Mov, "opcode should be mov");
    assert_eq!(syl.dst, Some(Reg::PReg(RETVAL_REG)), "destination should be RETVAL_REG");

    // Source is the first (and only) parameter vreg.
    assert_eq!(syl.srcs, [Value::Reg(Reg::VReg(0))], "source should be v0 (first param)");

    assert_eq!(block.terminator, Terminator::Return);
}
