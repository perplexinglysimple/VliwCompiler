//! Tests for phi lowering (LLVM-7) using the LLVM-8 loop fixture.
//!
//! The loop fixture `loop_iv.ll` computes `count_to_n(n)`:
//!
//! ```llvm
//! define i64 @count_to_n(i64 %n) {
//! entry:
//!   br label %loop
//! loop:
//!   %i = phi i64 [ 0, %entry ], [ %i_next, %loop ]
//!   %i_next = add i64 %i, 1
//!   %done = icmp eq i64 %i_next, %n
//!   br i1 %done, label %exit, label %loop
//! exit:
//!   ret i64 %i_next
//! }
//! ```
//!
//! After phi lowering the MIR must contain:
//! - An `entry` block with a `movi` that initialises the phi destination to 0.
//! - A `loop` block with ALU ops and a conditional branch to `exit`.
//! - A synthetic latch block (fall-through from `loop`) with a `mov` that
//!   copies `i_next` back into the phi destination, then jumps to `loop`.
//! - An `exit` block that copies the result to the return register.

use std::collections::HashMap;
use vliw_asm::opcode::Opcode;

use vliw_backend::mir::{Function, Reg, Terminator, Value};
use vliw_backend::{compile_to_mir, OptimizationLevel};

const LOOP_IV: &str = include_str!("fixtures/loop_iv.ll");

/// The loop fixture must compile without error.
#[test]
fn loop_iv_compiles() {
    compile_to_mir(LOOP_IV, OptimizationLevel::None).expect("loop_iv should compile");
}

/// The existing phi.ll fixture (same loop structure) must also compile now.
#[test]
fn phi_ll_compiles() {
    let ir = include_str!("fixtures/phi.ll");
    compile_to_mir(ir, OptimizationLevel::None).expect("phi.ll should compile after LLVM-7");
}

/// `entry` block must contain a `movi` initialising the phi register to 0.
#[test]
fn entry_contains_phi_init() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let entry = func
        .blocks
        .iter()
        .find(|b| b.label == "entry")
        .expect("entry block");

    let has_movi_zero = entry
        .syllables
        .iter()
        .any(|s| s.opcode == Opcode::MovImm && s.srcs == vec![Value::Imm(0)]);
    assert!(
        has_movi_zero,
        "entry must contain `movi <phi_dst>, 0`; syllables: {entry}"
    );
}

/// `entry` must jump (or fall-through) to the `loop` block.
#[test]
fn entry_jumps_to_loop() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let entry = func
        .blocks
        .iter()
        .find(|b| b.label == "entry")
        .expect("entry block");
    assert!(
        matches!(&entry.terminator, Terminator::Jump(lbl) if lbl == "loop"),
        "entry terminator should be Jump(\"loop\"), got: {}",
        entry.terminator,
    );
}

/// The `loop` block must contain exactly one conditional branch to `exit`.
#[test]
fn loop_block_branches_to_exit() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let loop_block = func
        .blocks
        .iter()
        .find(|b| b.label == "loop")
        .expect("loop block");
    assert!(
        matches!(&loop_block.terminator, Terminator::Branch { label, .. } if label == "exit"),
        "loop terminator should be Branch {{.. exit}}, got: {}",
        loop_block.terminator,
    );
}

/// A synthetic latch block must exist between `loop` and the back-edge jump.
/// It must contain a `mov` (back-edge phi copy) and terminate with `jump loop`.
#[test]
fn latch_block_exists_with_mov_and_back_jump() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();

    // Latch is the synthetic block inserted for the back-edge; its label
    // contains "latch" (set by isel) and it jumps back to "loop".
    let latch = func
        .blocks
        .iter()
        .find(|b| {
            b.label.contains("latch")
                && matches!(&b.terminator, Terminator::Jump(lbl) if lbl == "loop")
        })
        .expect("should have a latch block that jumps back to loop");

    // The latch must contain a Mov syllable (i_next → phi_dst).
    let has_mov = latch.syllables.iter().any(|s| s.opcode == Opcode::Mov);
    assert!(
        has_mov,
        "latch block '{}' must contain a Mov for the back-edge phi copy; syllables: {:?}",
        latch.label, latch.syllables
    );
}

/// The `exit` block must copy the result to the physical return register r1.
#[test]
fn exit_copies_result_to_retval_reg() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let exit = func
        .blocks
        .iter()
        .find(|b| b.label == "exit")
        .expect("exit block");

    let has_ret_copy = exit
        .syllables
        .iter()
        .any(|s| s.dst == Some(Reg::PReg(vliw_backend::regs::RETVAL_REG)));
    assert!(
        has_ret_copy,
        "exit must copy result to r{} (RETVAL_REG)",
        vliw_backend::regs::RETVAL_REG,
    );
}

/// Block order: entry → loop → latch → exit (latch must follow loop in MIR).
#[test]
fn block_order_is_correct() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let labels: Vec<&str> = func.blocks.iter().map(|b| b.label.as_str()).collect();

    let entry_idx = labels.iter().position(|&l| l == "entry").expect("entry");
    let loop_idx = labels.iter().position(|&l| l == "loop").expect("loop");
    let exit_idx = labels.iter().position(|&l| l == "exit").expect("exit");
    let latch_idx = labels
        .iter()
        .position(|&l| l.contains("latch"))
        .expect("latch block");

    assert!(entry_idx < loop_idx, "entry before loop");
    assert!(loop_idx < latch_idx, "loop before latch");
    assert!(latch_idx < exit_idx, "latch before exit");
}

/// The phi destination register set in `entry` must be the same register that
/// the latch's Mov writes to (proving both phi copies target the same VReg).
#[test]
fn phi_dst_consistent_across_copies() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();

    let entry = func.blocks.iter().find(|b| b.label == "entry").unwrap();
    let latch = func
        .blocks
        .iter()
        .find(|b| {
            b.label.contains("latch")
                && matches!(&b.terminator, Terminator::Jump(lbl) if lbl == "loop")
        })
        .unwrap();

    // dst VReg from entry's movi-0 copy.
    let entry_phi_dst = entry
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::MovImm && s.srcs == vec![Value::Imm(0)])
        .and_then(|s| s.dst)
        .expect("entry phi copy dst");

    // dst VReg from latch's mov copy.
    let latch_phi_dst = latch
        .syllables
        .iter()
        .find(|s| s.opcode == Opcode::Mov)
        .and_then(|s| s.dst)
        .expect("latch phi copy dst");

    assert_eq!(
        entry_phi_dst, latch_phi_dst,
        "both phi copies must write to the same VReg"
    );
}

/// Semantic check for the lowered loop: execute the MIR with `n = 5` and
/// verify both the return value and the number of loop-header visits.
#[test]
fn lowered_loop_reports_expected_trip_count() {
    let func = compile_to_mir(LOOP_IV, OptimizationLevel::None).unwrap();
    let (ret, trips) = run_count_to_n_mir(&func, 5);

    assert_eq!(ret, 5, "count_to_n(5) should return 5");
    assert_eq!(trips, 5, "loop header should execute once per trip");
}

fn run_count_to_n_mir(func: &Function, n: i64) -> (i64, u64) {
    let mut regs: HashMap<Reg, i64> = HashMap::from([(Reg::VReg(0), n)]);
    let label_to_idx: HashMap<&str, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(idx, block)| (block.label.as_str(), idx))
        .collect();

    let mut idx = 0usize;
    let mut trips = 0u64;
    let mut steps = 0u64;

    loop {
        steps += 1;
        assert!(steps < 1_000, "MIR interpreter did not terminate");

        let block = &func.blocks[idx];
        if block.label == "loop" {
            trips += 1;
        }

        for syl in &block.syllables {
            match syl.opcode {
                Opcode::MovImm => {
                    regs.insert(syl.dst.expect("movi dst"), read_value(&regs, syl.srcs[0]));
                }
                Opcode::Mov => {
                    regs.insert(syl.dst.expect("mov dst"), read_value(&regs, syl.srcs[0]));
                }
                Opcode::Add => {
                    regs.insert(
                        syl.dst.expect("add dst"),
                        read_value(&regs, syl.srcs[0]) + read_value(&regs, syl.srcs[1]),
                    );
                }
                Opcode::CmpEq => {
                    regs.insert(
                        syl.dst.expect("cmpeq dst"),
                        i64::from(read_value(&regs, syl.srcs[0]) == read_value(&regs, syl.srcs[1])),
                    );
                }
                other => panic!("unexpected opcode in loop fixture: {other:?}"),
            }
        }

        match &block.terminator {
            Terminator::Return => return (regs[&Reg::PReg(vliw_backend::regs::RETVAL_REG)], trips),
            Terminator::Jump(label) => idx = label_to_idx[label.as_str()],
            Terminator::Branch { cond, label } => {
                if regs[&Reg::VReg(match cond {
                    Reg::VReg(n) => *n,
                    Reg::PReg(_) => panic!("branch cond should be virtual"),
                })] != 0
                {
                    idx = label_to_idx[label.as_str()];
                } else {
                    idx += 1;
                }
            }
            Terminator::DirectCall { .. } => {
                panic!("phi_isel interpreter does not support DirectCall")
            }
        }
    }
}

fn read_value(regs: &HashMap<Reg, i64>, value: Value) -> i64 {
    match value {
        Value::Imm(imm) => imm,
        Value::Reg(reg) => regs[&reg],
        Value::Stack(_) => panic!("stack address cannot be read as scalar value"),
    }
}
