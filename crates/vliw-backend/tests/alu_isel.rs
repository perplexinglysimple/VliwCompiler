use vliw_asm::opcode::Opcode;
use vliw_backend::{OptimizationLevel, compile_to_mir};

/// Compile the given LLVM IR and collect every syllable opcode in order.
fn syllable_opcodes(ir: &str) -> Vec<Opcode> {
    let func = compile_to_mir(ir, OptimizationLevel::None)
        .expect("ISel should succeed for integer ALU fixtures");
    func.blocks
        .iter()
        .flat_map(|b| b.syllables.iter().map(|s| s.opcode))
        .collect()
}

// --- reg-reg tests: one ALU syllable, no movi ---

#[test]
fn add_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/add.ll")), [Opcode::Add]);
}

#[test]
fn sub_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/sub.ll")), [Opcode::Sub]);
}

#[test]
fn mul_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/mul.ll")), [Opcode::Mul]);
}

#[test]
fn and_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/and.ll")), [Opcode::And]);
}

#[test]
fn or_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/or.ll")), [Opcode::Or]);
}

#[test]
fn xor_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/xor.ll")), [Opcode::Xor]);
}

#[test]
fn shl_reg_reg() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/shl.ll")), [Opcode::Shl]);
}

#[test]
fn lshr_maps_to_srl() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/lshr.ll")), [Opcode::Srl]);
}

#[test]
fn ashr_maps_to_sra() {
    assert_eq!(syllable_opcodes(include_str!("fixtures/alu/ashr.ll")), [Opcode::Sra]);
}

// --- constant materialization: movi precedes the ALU op ---

#[test]
fn add_const_emits_movi_then_add() {
    assert_eq!(
        syllable_opcodes(include_str!("fixtures/alu/add_const.ll")),
        [Opcode::MovImm, Opcode::Add],
    );
}

// --- structural checks: VReg wiring ---

#[test]
fn add_result_vreg_fed_as_src() {
    let func = compile_to_mir(
        include_str!("fixtures/alu/add.ll"),
        OptimizationLevel::None,
    )
    .unwrap();
    let block = &func.blocks[0];
    assert_eq!(block.syllables.len(), 1);
    let add_syl = &block.syllables[0];
    assert_eq!(add_syl.opcode, Opcode::Add);
    // Two parameter VRegs (v0, v1) feed the add.
    assert!(add_syl.dst.is_some());
    assert_eq!(add_syl.srcs.len(), 2);
}

#[test]
fn add_const_movi_value_fed_into_add() {
    use vliw_backend::mir::Value;

    let func = compile_to_mir(
        include_str!("fixtures/alu/add_const.ll"),
        OptimizationLevel::None,
    )
    .unwrap();
    let block = &func.blocks[0];
    assert_eq!(block.syllables.len(), 2);

    let movi = &block.syllables[0];
    assert_eq!(movi.opcode, Opcode::MovImm);
    assert_eq!(movi.srcs, [Value::Imm(42)]);

    let add = &block.syllables[1];
    // The movi's destination should appear as one of add's sources.
    assert_eq!(add.srcs[1], Value::Reg(movi.dst.unwrap()));
}
