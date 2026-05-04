use std::collections::HashMap;

use vliw_asm::{parse, Item, Operand, Program};
use vliw_backend::{compile, regs::RETVAL_REG, OptimizationLevel, Schedule};

const IF_LT: &str = include_str!("fixtures/cmp/if_lt.ll");
const BRANCH_TAKEN: &str = include_str!("fixtures/branch/branch_taken.ll");
const BRANCH_NOT_TAKEN: &str = include_str!("fixtures/branch/branch_not_taken.ll");

#[test]
fn branch_fixture_emits_labels_and_branch_in_x_slot() {
    let text = compile(IF_LT, OptimizationLevel::None, Schedule::Scalar)
        .expect("if_lt should compile through VLIW emission");
    let program = parse(&text).expect("emitted VLIW should parse");
    let x_slot = x_slot(&program);

    assert!(program
        .items
        .iter()
        .any(|item| matches!(item, Item::Label(label) if label == "entry")));
    assert!(program
        .items
        .iter()
        .any(|item| matches!(item, Item::Label(label) if label == "if.then")));
    assert!(program
        .items
        .iter()
        .any(|item| matches!(item, Item::Label(label) if label == "if.else")));

    let branch_bundle = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Bundle(bundle)
                if bundle.slots[x_slot]
                    .as_ref()
                    .is_some_and(|syl| syl.opcode == "br") =>
            {
                Some(bundle)
            }
            _ => None,
        })
        .expect("conditional branch should be emitted in the X slot");

    assert!(
        branch_bundle
            .slots
            .iter()
            .enumerate()
            .all(|(slot, syl)| slot == x_slot || syl.is_none()),
        "branch bundle should not contain non-X syllables"
    );
}

#[test]
fn branch_fixture_executes_taken_and_fallthrough_paths() {
    let text = compile(IF_LT, OptimizationLevel::None, Schedule::Scalar)
        .expect("if_lt should compile through VLIW emission");
    let program = parse(&text).expect("emitted VLIW should parse");

    assert_eq!(run_if_lt(&program, 2, 7), 1, "taken path should return 1");
    assert_eq!(
        run_if_lt(&program, 9, 3),
        0,
        "fallthrough path should return 0"
    );
}

#[test]
fn unconditional_jump_emits_in_x_slot() {
    let ir = r#"define i32 @jump_only() {
entry:
  br label %exit

exit:
  ret i32 7
}"#;
    let text = compile(ir, OptimizationLevel::None, Schedule::Scalar)
        .expect("unconditional branch should compile through VLIW emission");
    let program = parse(&text).expect("emitted VLIW should parse");
    let x_slot = x_slot(&program);

    assert!(
        program.items.iter().any(|item| {
            matches!(
                item,
                Item::Bundle(bundle)
                    if bundle.slots[x_slot]
                        .as_ref()
                        .is_some_and(|syl| syl.opcode == "jmp")
            )
        }),
        "unconditional jump should be emitted in the X slot"
    );
}

/// Both branch-fixture variants must compile at O1 and preserve a real
/// conditional `br` in the X slot (not just an unconditional `jmp`).
#[test]
fn branch_fixtures_emit_conditional_branch_at_o1() {
    for (name, ir) in [("branch_taken", BRANCH_TAKEN), ("branch_not_taken", BRANCH_NOT_TAKEN)] {
        let text = compile(ir, OptimizationLevel::Less, Schedule::Scalar)
            .unwrap_or_else(|e| panic!("{name}: compile failed: {e}"));
        let program = parse(&text).unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));
        let x_slot = x_slot(&program);

        let has_cmplt = program.items.iter().any(|item| {
            matches!(item, Item::Bundle(b) if b.slots[0].as_ref().is_some_and(|s| s.opcode == "cmplt"))
        });
        assert!(has_cmplt, "{name}: expected a cmplt syllable in I0 slot");

        let has_br = program.items.iter().any(|item| {
            matches!(item, Item::Bundle(b) if b.slots[x_slot].as_ref().is_some_and(|s| s.opcode == "br"))
        });
        assert!(has_br, "{name}: expected a conditional `br` syllable in X slot");
    }
}

fn run_if_lt(program: &Program, x: i64, y: i64) -> i64 {
    let mut labels = HashMap::new();
    for (idx, item) in program.items.iter().enumerate() {
        if let Item::Label(label) = item {
            labels.insert(label.as_str(), idx);
        }
    }

    let (lhs_reg, rhs_reg) = cmp_input_regs(program);
    let mut regs = HashMap::from([
        (vliw_backend::regs::ZERO_REG, 0),
        (lhs_reg, x),
        (rhs_reg, y),
    ]);
    let mut preds = HashMap::new();

    let mut pc = 0usize;
    let mut steps = 0u64;
    while pc < program.items.len() {
        steps += 1;
        assert!(steps < 100, "branch fixture did not terminate");

        match &program.items[pc] {
            Item::Label(_) => pc += 1,
            Item::Bundle(bundle) => {
                let Some(syl) = bundle.slots.iter().find_map(|slot| slot.as_ref()) else {
                    pc += 1;
                    continue;
                };

                match syl.opcode.as_str() {
                    "movi" => {
                        write_reg(&mut regs, reg(&syl.operands[0]), imm(&syl.operands[1]));
                        pc += 1;
                    }
                    "mov" => {
                        let value = read_reg(&regs, reg(&syl.operands[1]));
                        write_reg(&mut regs, reg(&syl.operands[0]), value);
                        pc += 1;
                    }
                    "cmplt" => {
                        preds.insert(
                            pred(&syl.operands[0]),
                            read_reg(&regs, reg(&syl.operands[1]))
                                < read_int_operand(&regs, &syl.operands[2]),
                        );
                        pc += 1;
                    }
                    "br" => {
                        if preds.get(&pred(&syl.operands[0])).copied().unwrap_or(false) {
                            pc = labels[label(&syl.operands[1])];
                        } else {
                            pc += 1;
                        }
                    }
                    "jmp" => pc = labels[label(&syl.operands[0])],
                    "ret" => break,
                    other => panic!("unexpected opcode in branch fixture: {other}"),
                }
            }
        }
    }

    read_reg(&regs, RETVAL_REG)
}

fn cmp_input_regs(program: &Program) -> (u8, u8) {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Bundle(bundle) => bundle.slots.iter().find_map(|slot| {
                let syl = slot.as_ref()?;
                if syl.opcode == "cmplt" {
                    Some((reg(&syl.operands[1]), reg(&syl.operands[2])))
                } else {
                    None
                }
            }),
            Item::Label(_) => None,
        })
        .expect("branch fixture should contain a cmplt")
}

fn x_slot(program: &Program) -> usize {
    program
        .processor
        .slot_aliases
        .iter()
        .find(|alias| alias.name == "X")
        .expect("processor should define X slot")
        .slot
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
