/// End-to-end tests for wider target layouts (LATER-3).
///
/// Verifies that the same LLVM IR can be compiled for both the canonical 4-slot
/// layout and a contrived 8-slot layout, that both outputs pass `vliw_verify`,
/// and that the pack scheduler exploits the wider slot set in the 8-slot case.
use vliw_asm::{CacheSpec, Item, Processor, SlotAlias, TopologySpec, UnitDecl, DEFAULT_MEMORY_SIZE};
use vliw_backend::{compile_for_processor, OptimizationLevel, Schedule};

fn wide_8slot_processor() -> Processor {
    Processor {
        width: 8,
        units: vec![
            UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
            UnitDecl { name: "mem".into(), kind: "memory".into() },
            UnitDecl { name: "ctrl".into(), kind: "control".into() },
            UnitDecl { name: "mul".into(), kind: "multiplier".into() },
        ],
        slot_aliases: vec![
            SlotAlias { name: "I0".into(), slot: 0 },
            SlotAlias { name: "I1".into(), slot: 1 },
            SlotAlias { name: "I2".into(), slot: 2 },
            SlotAlias { name: "I3".into(), slot: 3 },
            SlotAlias { name: "M0".into(), slot: 4 },
            SlotAlias { name: "M1".into(), slot: 5 },
            SlotAlias { name: "X".into(), slot: 6 },
            SlotAlias { name: "I4".into(), slot: 7 },
        ],
        slot_units: vec![
            vec!["alu".into()],
            vec!["alu".into()],
            vec!["alu".into()],
            vec!["alu".into()],
            vec!["mem".into()],
            vec!["mem".into()],
            vec!["ctrl".into(), "mul".into()],
            vec!["alu".into()],
        ],
        cache: CacheSpec {},
        topology: TopologySpec { cpus: 1 },
        memory_size: DEFAULT_MEMORY_SIZE,
    }
}

/// Four independent ALU operations followed by their consumers.
/// t1,t2,t3,t4 have no dependencies on each other; pack can issue all four
/// in a single 8-slot bundle but is limited to two per bundle on 4-slot.
const WIDE_OPS_IR: &str = r#"define i64 @wide_ops(i64 %a, i64 %b, i64 %c, i64 %d, i64 %e, i64 %f) {
entry:
  %t1 = add i64 %a, %b
  %t2 = add i64 %c, %d
  %t3 = add i64 %e, %f
  %t4 = add i64 %a, %c
  %t5 = add i64 %t1, %t2
  %t6 = add i64 %t3, %t4
  %t7 = add i64 %t5, %t6
  ret i64 %t7
}
"#;

#[test]
fn canonical_4slot_scalar_compiles_and_verifies() {
    let text = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Scalar,
        Processor::default(),
    )
    .expect("4-slot scalar compilation should succeed");

    let program = vliw_asm::parse(&text).expect("4-slot scalar output should parse");
    vliw_asm::verify_program(&program).expect("4-slot scalar output should pass vliw_verify");

    assert!(text.contains("width 4"), "output should declare 4-slot processor");
}

#[test]
fn wide_8slot_scalar_compiles_and_verifies() {
    let text = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Scalar,
        wide_8slot_processor(),
    )
    .expect("8-slot scalar compilation should succeed");

    let program = vliw_asm::parse(&text).expect("8-slot scalar output should parse");
    vliw_asm::verify_program(&program).expect("8-slot scalar output should pass vliw_verify");

    assert!(text.contains("width 8"), "output should declare 8-slot processor");
}

#[test]
fn pack_exploits_wider_8slot_layout() {
    let text_4 = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Pack,
        Processor::default(),
    )
    .expect("4-slot pack compilation should succeed");

    let text_8 = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Pack,
        wide_8slot_processor(),
    )
    .expect("8-slot pack compilation should succeed");

    let prog_4 = vliw_asm::parse(&text_4).expect("4-slot pack output should parse");
    let prog_8 = vliw_asm::parse(&text_8).expect("8-slot pack output should parse");

    vliw_asm::verify_program(&prog_4).expect("4-slot pack output should pass vliw_verify");
    vliw_asm::verify_program(&prog_8).expect("8-slot pack output should pass vliw_verify");

    let max_syllables = |prog: &vliw_asm::Program| -> usize {
        prog.items
            .iter()
            .filter_map(|item| match item {
                Item::Bundle(b) => Some(b.slots.iter().flatten().count()),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    };

    let max_4 = max_syllables(&prog_4);
    let max_8 = max_syllables(&prog_8);

    assert!(
        max_8 > max_4,
        "8-slot pack should produce denser bundles than 4-slot pack \
         (4-slot max={max_4}, 8-slot max={max_8})\n\n\
         --- 4-slot ---\n{text_4}\n--- 8-slot ---\n{text_8}"
    );
    assert!(
        max_8 >= 3,
        "8-slot pack should fit at least 3 syllables into a single bundle \
         (max={max_8})\n\n{text_8}"
    );
}

#[test]
fn processor_config_loaded_from_file_matches_inline() {
    let config_text = include_str!("fixtures/wide8.vliw");
    let parsed_proc = vliw_asm::parse(config_text)
        .expect("wide8.vliw should parse")
        .processor;

    assert_eq!(parsed_proc.width, 8);
    assert_eq!(parsed_proc.slot_units.len(), 8);

    let text_inline = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Pack,
        wide_8slot_processor(),
    )
    .expect("inline 8-slot pack should succeed");

    let text_file = compile_for_processor(
        WIDE_OPS_IR,
        OptimizationLevel::None,
        Schedule::Pack,
        parsed_proc,
    )
    .expect("file-loaded 8-slot pack should succeed");

    assert_eq!(
        text_inline, text_file,
        "inline and file-loaded processor configs should produce identical output"
    );
}
