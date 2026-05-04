//! In-bundle hazard verifier.
//!
//! Checks three classes of error within a single bundle:
//!
//! 1. **Slot legality** – a syllable's opcode must be legal on one of the unit
//!    kinds declared for that slot.
//! 2. **WAW** – no two syllables in the same bundle write the same GPR.
//! 3. **RAW** – no syllable reads a GPR that another syllable in the same
//!    bundle writes (all syllables in a bundle issue simultaneously, so the
//!    write is not yet visible to the co-issued reader).

use std::collections::HashMap;

use crate::opcode::{Opcode, UnitKind};
use crate::{Bundle, Item, Operand, Processor, Program};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HazardError {
    #[error("slot {slot}: unknown opcode '{mnemonic}'")]
    UnknownOpcode { slot: usize, mnemonic: String },

    #[error("slot {slot}: opcode '{mnemonic}' cannot execute on this slot's units")]
    IllegalSlot { slot: usize, mnemonic: String },

    #[error("WAW hazard: r{reg} written by both slot {first} and slot {second}")]
    Waw { reg: u8, first: usize, second: usize },

    #[error("RAW hazard: slot {reader} reads r{reg} written by slot {writer} in the same bundle")]
    Raw { reg: u8, writer: usize, reader: usize },
}

/// Verify one bundle against the given processor configuration.
pub fn verify_bundle(bundle: &Bundle, proc: &Processor) -> Result<(), HazardError> {
    // Build unit name → UnitKind lookup from processor declarations.
    let unit_kind_map: HashMap<&str, UnitKind> = proc
        .units
        .iter()
        .filter_map(|u| UnitKind::from_kind_str(&u.kind).map(|k| (u.name.as_str(), k)))
        .collect();

    let mut gpr_writes: Vec<(u8, usize)> = Vec::new();
    let mut gpr_reads: Vec<(u8, usize)> = Vec::new();

    for slot_idx in 0..(proc.width as usize) {
        let syl = match bundle.slots.get(slot_idx).and_then(|s| s.as_ref()) {
            None => continue,
            Some(s) => s,
        };

        let opcode =
            Opcode::from_mnemonic(&syl.opcode).ok_or_else(|| HazardError::UnknownOpcode {
                slot: slot_idx,
                mnemonic: syl.opcode.clone(),
            })?;

        // Nop is universally legal and produces no reads/writes.
        if opcode == Opcode::Nop {
            continue;
        }

        // --- Slot legality ---
        let slot_unit_kinds: Vec<UnitKind> = proc
            .slot_units
            .get(slot_idx)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| unit_kind_map.get(n.as_str()).copied())
                    .collect()
            })
            .unwrap_or_default();

        let legal = opcode.units().iter().any(|req| slot_unit_kinds.contains(req));
        if !legal {
            return Err(HazardError::IllegalSlot {
                slot: slot_idx,
                mnemonic: syl.opcode.clone(),
            });
        }

        // --- Collect GPR writes ---
        // Convention: for opcodes that write a GPR, the first Reg operand is
        // the destination.
        if opcode.writes_gpr() {
            if let Some(Operand::Reg(n)) = syl.operands.first() {
                gpr_writes.push((*n, slot_idx));
            }
        }

        // --- Collect GPR reads ---
        // Skip the first Reg operand when it is the GPR destination.
        let skip_dest = opcode.writes_gpr();
        let mut dest_skipped = false;
        for op in &syl.operands {
            match op {
                Operand::Reg(n) => {
                    if skip_dest && !dest_skipped {
                        dest_skipped = true;
                    } else {
                        gpr_reads.push((*n, slot_idx));
                    }
                }
                Operand::MemAddr { base, .. } => {
                    gpr_reads.push((*base, slot_idx));
                }
                _ => {}
            }
        }
    }

    // --- WAW check ---
    for i in 0..gpr_writes.len() {
        for j in (i + 1)..gpr_writes.len() {
            if gpr_writes[i].0 == gpr_writes[j].0 {
                return Err(HazardError::Waw {
                    reg: gpr_writes[i].0,
                    first: gpr_writes[i].1,
                    second: gpr_writes[j].1,
                });
            }
        }
    }

    // --- RAW check ---
    // Flag any read of a GPR that is written by a different slot in this bundle.
    for &(read_reg, reader_slot) in &gpr_reads {
        for &(write_reg, writer_slot) in &gpr_writes {
            if read_reg == write_reg && reader_slot != writer_slot {
                return Err(HazardError::Raw {
                    reg: read_reg,
                    writer: writer_slot,
                    reader: reader_slot,
                });
            }
        }
    }

    Ok(())
}

/// Verify every bundle in a program.
pub fn verify_program(program: &Program) -> Result<(), HazardError> {
    for item in &program.items {
        if let Item::Bundle(bundle) = item {
            verify_bundle(bundle, &program.processor)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bundle, Item, Operand, Processor, Program, Syllable};

    fn syl(op: &str, ops: Vec<Operand>) -> Option<Syllable> {
        Some(Syllable::new(op, ops))
    }

    // ---- slot-legality tests ----

    #[test]
    fn mul_in_alu_slot_is_illegal() {
        // Multiplier opcode in slot 0 (I0, IntegerAlu) should be rejected.
        let bundle = Bundle {
            slots: vec![
                syl("mul", vec![Operand::Reg(3), Operand::Reg(1), Operand::Reg(2)]),
                None,
                None,
                None,
            ],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::IllegalSlot { slot: 0, mnemonic: "mul".into() })
        );
    }

    #[test]
    fn add_in_mem_slot_is_illegal() {
        // IntegerAlu opcode in slot 2 (M, Memory) should be rejected.
        let bundle = Bundle {
            slots: vec![
                None,
                None,
                syl("add", vec![Operand::Reg(3), Operand::Reg(1), Operand::Reg(2)]),
                None,
            ],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::IllegalSlot { slot: 2, mnemonic: "add".into() })
        );
    }

    #[test]
    fn ret_in_alu_slot_is_illegal() {
        let bundle = Bundle {
            slots: vec![syl("ret", vec![]), None, None, None],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::IllegalSlot { slot: 0, mnemonic: "ret".into() })
        );
    }

    #[test]
    fn nop_is_legal_in_any_slot() {
        // Explicit nop syllables must be accepted in every slot.
        let bundle = Bundle {
            slots: vec![
                syl("nop", vec![]),
                syl("nop", vec![]),
                syl("nop", vec![]),
                syl("nop", vec![]),
            ],
        };
        assert_eq!(verify_bundle(&bundle, &Processor::default()), Ok(()));
    }

    #[test]
    fn unknown_opcode_returns_error() {
        let bundle = Bundle {
            slots: vec![syl("bogus_op", vec![]), None, None, None],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::UnknownOpcode { slot: 0, mnemonic: "bogus_op".into() })
        );
    }

    // ---- WAW tests ----

    #[test]
    fn waw_same_dest_register_is_rejected() {
        // Both I0 and I1 write r1 — WAW hazard.
        let bundle = Bundle {
            slots: vec![
                syl("movi", vec![Operand::Reg(1), Operand::Imm(6)]),
                syl("movi", vec![Operand::Reg(1), Operand::Imm(7)]),
                None,
                None,
            ],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::Waw { reg: 1, first: 0, second: 1 })
        );
    }

    #[test]
    fn waw_different_dest_registers_is_ok() {
        let bundle = Bundle {
            slots: vec![
                syl("movi", vec![Operand::Reg(1), Operand::Imm(6)]),
                syl("movi", vec![Operand::Reg(2), Operand::Imm(7)]),
                None,
                None,
            ],
        };
        assert_eq!(verify_bundle(&bundle, &Processor::default()), Ok(()));
    }

    // ---- RAW tests ----

    #[test]
    fn raw_reader_sees_same_bundle_writer_is_rejected() {
        // I0 writes r3; I1 reads r3 — in-bundle RAW.
        let bundle = Bundle {
            slots: vec![
                syl("add", vec![Operand::Reg(3), Operand::Reg(1), Operand::Reg(2)]),
                syl("mov", vec![Operand::Reg(4), Operand::Reg(3)]),
                None,
                None,
            ],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::Raw { reg: 3, writer: 0, reader: 1 })
        );
    }

    #[test]
    fn raw_mem_base_of_store_reads_from_bundle_writer_is_rejected() {
        // I0 writes r5; M reads r5 as the address base for a store — in-bundle RAW.
        let bundle = Bundle {
            slots: vec![
                syl("movi", vec![Operand::Reg(5), Operand::Imm(0x200)]),
                None,
                syl("std", vec![Operand::MemAddr { base: 5, offset: 0 }, Operand::Reg(1)]),
                None,
            ],
        };
        assert_eq!(
            verify_bundle(&bundle, &Processor::default()),
            Err(HazardError::Raw { reg: 5, writer: 0, reader: 2 })
        );
    }

    #[test]
    fn raw_independent_reads_writes_is_ok() {
        // I0 writes r3; M reads r0 and r1 — no overlap, no hazard.
        let bundle = Bundle {
            slots: vec![
                syl("movi", vec![Operand::Reg(3), Operand::Imm(42)]),
                None,
                syl(
                    "std",
                    vec![Operand::MemAddr { base: 0, offset: 0x100 }, Operand::Reg(1)],
                ),
                None,
            ],
        };
        assert_eq!(verify_bundle(&bundle, &Processor::default()), Ok(()));
    }

    // ---- canonical demo program ----

    #[test]
    fn canonical_demo_program_passes() {
        use Operand::*;
        let program = Program {
            processor: Processor::default(),
            items: vec![
                Item::Label("entry".into()),
                // Bundle 1: I0=movi r1,6  I1=movi r2,7  M=nop  X=nop
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", vec![Reg(1), Imm(6)]),
                        syl("movi", vec![Reg(2), Imm(7)]),
                        None,
                        None,
                    ],
                }),
                // Bundle 2: X=mul r3,r1,r2
                Item::Bundle(Bundle {
                    slots: vec![None, None, None, syl("mul", vec![Reg(3), Reg(1), Reg(2)])],
                }),
                // Bundle 3: M=std [r0+0x100],r3  X=ret
                Item::Bundle(Bundle {
                    slots: vec![
                        None,
                        None,
                        syl("std", vec![MemAddr { base: 0, offset: 0x100 }, Reg(3)]),
                        syl("ret", vec![]),
                    ],
                }),
            ],
        };
        assert_eq!(verify_program(&program), Ok(()));
    }
}
