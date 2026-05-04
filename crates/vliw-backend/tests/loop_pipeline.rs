use std::collections::HashMap;

use vliw_asm::{parse, Item, Operand};
use vliw_backend::{compile, regs::RETVAL_REG, OptimizationLevel, Schedule};

const LOOP_SUM_10: &str = include_str!("fixtures/loop_sum_10.ll");

#[test]
fn loop_sum_10_compiles_and_reports_45_in_return_register() {
    let text = compile(LOOP_SUM_10, OptimizationLevel::Less, Schedule::Scalar)
        .expect("loop_sum_10 should compile through VLIW emission");
    assert!(text.contains("loop:"), "fixture should keep its loop shape");
    assert!(
        text.contains("br p"),
        "fixture should emit a conditional branch"
    );
    let program = parse(&text).expect("emitted VLIW should parse");
    let regs = run_scalar_program(&program);

    assert_eq!(
        regs.get(&RETVAL_REG).copied().unwrap_or(0),
        45,
        "sum should be reported in RETVAL_REG (r{RETVAL_REG})"
    );
}

fn run_scalar_program(program: &vliw_asm::Program) -> HashMap<u8, i64> {
    let mut labels = HashMap::new();
    for (idx, item) in program.items.iter().enumerate() {
        if let Item::Label(label) = item {
            labels.insert(label.as_str(), idx);
        }
    }

    let mut regs = HashMap::new();
    let mut preds = HashMap::new();
    regs.insert(vliw_backend::regs::ZERO_REG, 0);

    let mut pc = 0usize;
    let mut steps = 0u64;
    while pc < program.items.len() {
        steps += 1;
        assert!(steps < 1_000, "scalar program did not terminate");

        match &program.items[pc] {
            Item::Label(_) => pc += 1,
            Item::Bundle(bundle) => {
                let syl = bundle.slots.iter().find_map(|slot| slot.as_ref());
                let Some(syl) = syl else {
                    pc += 1;
                    continue;
                };

                match syl.opcode.as_str() {
                    "movi" => {
                        let dst = reg(&syl.operands[0]);
                        let imm = imm(&syl.operands[1]);
                        write_reg(&mut regs, dst, imm);
                        pc += 1;
                    }
                    "mov" => {
                        let dst = reg(&syl.operands[0]);
                        let value = read_reg(&regs, reg(&syl.operands[1]));
                        write_reg(&mut regs, dst, value);
                        pc += 1;
                    }
                    "add" => {
                        let dst = reg(&syl.operands[0]);
                        let lhs = read_reg(&regs, reg(&syl.operands[1]));
                        let rhs = read_int_operand(&regs, &syl.operands[2]);
                        write_reg(&mut regs, dst, lhs + rhs);
                        pc += 1;
                    }
                    "cmpeq" => {
                        let dst = pred(&syl.operands[0]);
                        let lhs = read_reg(&regs, reg(&syl.operands[1]));
                        let rhs = read_int_operand(&regs, &syl.operands[2]);
                        preds.insert(dst, lhs == rhs);
                        pc += 1;
                    }
                    "cmplt" => {
                        let dst = pred(&syl.operands[0]);
                        let lhs = read_reg(&regs, reg(&syl.operands[1]));
                        let rhs = read_int_operand(&regs, &syl.operands[2]);
                        preds.insert(dst, lhs < rhs);
                        pc += 1;
                    }
                    "jmp" => pc = labels[label(&syl.operands[0])],
                    "br" => {
                        if preds.get(&pred(&syl.operands[0])).copied().unwrap_or(false) {
                            pc = labels[label(&syl.operands[1])];
                        } else {
                            pc += 1;
                        }
                    }
                    "ret" => break,
                    other => panic!("unexpected opcode in loop pipeline fixture: {other}"),
                }
            }
        }
    }

    regs
}

fn write_reg(regs: &mut HashMap<u8, i64>, reg: u8, value: i64) {
    if reg != vliw_backend::regs::ZERO_REG {
        regs.insert(reg, value);
    }
}

fn read_reg(regs: &HashMap<u8, i64>, reg: u8) -> i64 {
    if reg == vliw_backend::regs::ZERO_REG {
        0
    } else {
        regs.get(&reg).copied().unwrap_or(0)
    }
}

fn read_int_operand(regs: &HashMap<u8, i64>, op: &Operand) -> i64 {
    match op {
        Operand::Reg(reg) => read_reg(regs, *reg),
        Operand::Imm(imm) => *imm,
        other => panic!("expected integer operand, got {other:?}"),
    }
}

fn reg(op: &Operand) -> u8 {
    match op {
        Operand::Reg(reg) => *reg,
        other => panic!("expected register operand, got {other:?}"),
    }
}

fn pred(op: &Operand) -> u8 {
    match op {
        Operand::Pred(pred) => *pred,
        other => panic!("expected predicate operand, got {other:?}"),
    }
}

fn imm(op: &Operand) -> i64 {
    match op {
        Operand::Imm(imm) => *imm,
        other => panic!("expected immediate operand, got {other:?}"),
    }
}

fn label(op: &Operand) -> &str {
    match op {
        Operand::Label(label) => label,
        other => panic!("expected label operand, got {other:?}"),
    }
}
