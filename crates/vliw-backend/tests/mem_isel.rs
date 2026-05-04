use vliw_asm::opcode::Opcode;
use vliw_backend::mir::Value;
use vliw_backend::{OptimizationLevel, compile_to_mir};

// --- acceptance test: simple.c ---

/// The canonical simple.c smoke test:
///   volatile u64 *out = (volatile u64 *)0x100;
///   *out = 42;
///
/// Must lower to a `std [0x100], rN` syllable preceded by `movi rN, 42`.
const SIMPLE_C_IR: &str = r#"define i32 @main() {
entry:
  store volatile i64 42, ptr inttoptr (i64 256 to ptr), align 8
  ret i32 0
}"#;

#[test]
fn simple_c_emits_movi42_then_std() {
    let func = compile_to_mir(SIMPLE_C_IR, OptimizationLevel::None)
        .expect("simple.c IR should compile");
    let block = &func.blocks[0];

    // There must be a movi 42 syllable.
    let movi_idx = block
        .syllables
        .iter()
        .position(|s| s.opcode == Opcode::MovImm && s.srcs == [Value::Imm(42)])
        .expect("should have movi 42");
    let movi_dst = block.syllables[movi_idx].dst.unwrap();

    // There must be a StoreD syllable after the movi.
    let std_idx = block
        .syllables
        .iter()
        .position(|s| s.opcode == Opcode::StoreD)
        .expect("should have std syllable");

    assert!(movi_idx < std_idx, "movi 42 must precede std");

    let std_syl = &block.syllables[std_idx];

    // The store must encode address 256 (= 0x100).
    assert!(
        std_syl.srcs.contains(&Value::Imm(256)),
        "std should encode address 0x100 = 256; srcs: {:?}",
        std_syl.srcs
    );

    // The store's data source must be the register produced by movi 42.
    assert!(
        std_syl.srcs.contains(&Value::Reg(movi_dst)),
        "std should use the movi 42 dest as data; srcs: {:?}",
        std_syl.srcs
    );
}

// --- store tests ---

#[test]
fn store_i64_to_ptr_param() {
    let ir = r#"define void @f(ptr %p, i64 %v) {
entry:
  store i64 %v, ptr %p
  ret void
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let block = &func.blocks[0];

    let std_syl = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::StoreD)
        .expect("should have std syllable");

    assert_eq!(std_syl.dst, None);
    // srcs = [base_reg, data_reg]
    assert_eq!(std_syl.srcs.len(), 2);
}

#[test]
fn store_i32_emits_stw() {
    let ir = r#"define void @f(ptr %p, i32 %v) {
entry:
  store i32 %v, ptr %p
  ret void
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let has_stw = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .any(|s| s.opcode == Opcode::StoreW);
    assert!(has_stw, "i32 store should emit stw");
}

// --- load tests ---

#[test]
fn load_i64_from_ptr_param() {
    let ir = r#"define i64 @f(ptr %p) {
entry:
  %v = load i64, ptr %p
  ret i64 %v
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let block = &func.blocks[0];

    let ldd_syl = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::LoadD)
        .expect("should have ldd syllable");

    assert!(ldd_syl.dst.is_some(), "ldd must have a destination");
    assert_eq!(ldd_syl.srcs.len(), 1, "ldd from plain ptr has 1 src (the address)");
}

#[test]
fn load_i32_emits_ldw() {
    let ir = r#"define i32 @f(ptr %p) {
entry:
  %v = load i32, ptr %p
  ret i32 %v
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let has_ldw = func
        .blocks
        .iter()
        .flat_map(|b| b.syllables.iter())
        .any(|s| s.opcode == Opcode::LoadW);
    assert!(has_ldw, "i32 load should emit ldw");
}

#[test]
fn load_result_vreg_usable_by_later_instr() {
    let ir = r#"define i64 @f(ptr %p, i64 %x) {
entry:
  %v = load i64, ptr %p
  %r = add i64 %v, %x
  ret i64 %r
}"#;
    let func = compile_to_mir(ir, OptimizationLevel::None).unwrap();
    let block = &func.blocks[0];

    // ldd must come before add, and ldd's dst must feed add's src.
    let ldd = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::LoadD)
        .expect("should have ldd");
    let ldd_dst = ldd.dst.unwrap();

    let add = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::Add)
        .expect("should have add");

    assert!(
        add.srcs.contains(&Value::Reg(ldd_dst)),
        "add should use the ldd result; add srcs: {:?}",
        add.srcs
    );
}

// --- GEP folding tests ---

#[test]
fn gep_const_index_folds_into_store() {
    let ir = include_str!("fixtures/mem/gep_store.ll");
    let func = compile_to_mir(ir, OptimizationLevel::None)
        .expect("gep_store.ll should compile");
    let block = &func.blocks[0];

    // GEP folding must not emit an Add to compute the address.
    let has_add = block.syllables.iter().any(|s| s.opcode == Opcode::Add);
    assert!(!has_add, "GEP should fold into store, not emit an Add");

    // StoreD must carry the folded offset (8 = 1 * sizeof(i64)).
    let std_syl = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::StoreD)
        .expect("should have std");

    assert!(
        std_syl.srcs.contains(&Value::Imm(8)),
        "store should encode GEP offset 8; srcs: {:?}",
        std_syl.srcs
    );
}

#[test]
fn gep_const_index_folds_into_load() {
    let ir = include_str!("fixtures/mem/gep_load.ll");
    let func = compile_to_mir(ir, OptimizationLevel::None)
        .expect("gep_load.ll should compile");
    let block = &func.blocks[0];

    // GEP folding must not emit an Add to compute the address.
    let has_add = block.syllables.iter().any(|s| s.opcode == Opcode::Add);
    assert!(!has_add, "GEP should fold into load, not emit an Add");

    // LoadD must carry the folded offset (16 = 2 * sizeof(i64)).
    let ldd_syl = block
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::LoadD)
        .expect("should have ldd");

    assert!(
        ldd_syl.srcs.contains(&Value::Imm(16)),
        "load should encode GEP offset 16; srcs: {:?}",
        ldd_syl.srcs
    );
}
