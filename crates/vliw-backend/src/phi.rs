//! Phi lowering: insert parallel-copy `mov` syllables in predecessor blocks.
//!
//! Each LLVM phi node `%x = phi [v1, pred1], [v2, pred2]` becomes a copy
//! `x ← v_i` inserted at the tail of `pred_i`'s syllable list.  Parallel
//! copies within the same predecessor are serialised to avoid WAR clobbering:
//! chains are emitted in dependency order; cycles are broken by spilling one
//! source to a fresh virtual register.

use std::collections::HashMap;

use vliw_asm::opcode::Opcode;

use crate::mir::{Block, Reg, Syllable, Value};

/// Source of a phi copy: either an immediate constant or a virtual register.
#[derive(Debug, Clone, Copy)]
pub enum CopySrc {
    Const(i64),
    VReg(u32),
}

/// One copy `dst ← src` to be inserted into a predecessor block.
#[derive(Debug, Clone)]
pub struct PhiCopy {
    pub dst: u32,
    pub src: CopySrc,
}

/// Insert phi copies into predecessor blocks.
///
/// `copies_per_block` maps a MIR block label to the list of parallel copies
/// that must execute on entry to the successor block containing the phis.
/// Copies are appended after all existing syllables in each predecessor.
pub fn lower_phi_copies(
    blocks: &mut Vec<Block>,
    copies_per_block: &HashMap<String, Vec<PhiCopy>>,
    next_vreg: &mut u32,
) {
    for (label, copies) in copies_per_block {
        let idx = blocks
            .iter()
            .position(|b| &b.label == label)
            .unwrap_or_else(|| panic!("phi predecessor block '{label}' not found in MIR"));
        let syllables = serialize_parallel_copies(copies, next_vreg);
        blocks[idx].syllables.extend(syllables);
    }
}

/// Serialize a set of parallel copies into an ordered syllable list.
///
/// Immediate copies are emitted first (no dependency).  Register copies are
/// emitted in dependency order: a copy `dst ← src` where `dst` is not a
/// source of any other pending copy is safe to emit immediately.  Cycles are
/// broken by saving one source to a fresh temp register.
fn serialize_parallel_copies(copies: &[PhiCopy], next_vreg: &mut u32) -> Vec<Syllable> {
    let mut result = Vec::new();
    let mut remaining: Vec<(u32, CopySrc)> = copies
        .iter()
        .filter_map(|copy| match copy.src {
            CopySrc::VReg(src) if src == copy.dst => None,
            src => Some((copy.dst, src)),
        })
        .collect();

    while !remaining.is_empty() {
        // A copy is ready once its destination is no longer needed as the
        // register source of any other pending copy. This applies equally to
        // immediates: `v1 <- 0` must wait behind `v2 <- v1`.
        let ready = remaining.iter().position(|&(dst, _)| {
            !remaining
                .iter()
                .any(|&(_, src)| matches!(src, CopySrc::VReg(src_reg) if src_reg == dst))
        });

        if let Some(pos) = ready {
            let (dst, src) = remaining.remove(pos);
            result.push(syllable_for_copy(dst, src));
        } else {
            // All remaining destinations are still used as register sources,
            // which means the pending register copies contain a cycle. Break
            // it by saving one source to a fresh temp and redirecting uses.
            let src = remaining
                .iter()
                .find_map(|&(_, src)| match src {
                    CopySrc::VReg(src) => Some(src),
                    CopySrc::Const(_) => None,
                })
                .expect("copy cycle must contain a register source");
            let tmp = *next_vreg;
            *next_vreg += 1;

            result.push(syllable_for_copy(tmp, CopySrc::VReg(src)));
            for (_, pending_src) in remaining.iter_mut() {
                if matches!(*pending_src, CopySrc::VReg(s) if s == src) {
                    *pending_src = CopySrc::VReg(tmp);
                }
            }
        }
    }

    result
}

fn syllable_for_copy(dst: u32, src: CopySrc) -> Syllable {
    match src {
        CopySrc::Const(imm) => Syllable {
            opcode: Opcode::MovImm,
            dst: Some(Reg::VReg(dst)),
            srcs: vec![Value::Imm(imm)],
        },
        CopySrc::VReg(src) => Syllable {
            opcode: Opcode::Mov,
            dst: Some(Reg::VReg(dst)),
            srcs: vec![Value::Reg(Reg::VReg(src))],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vliw_asm::opcode::Opcode;

    fn opcodes(syls: &[Syllable]) -> Vec<Opcode> {
        syls.iter().map(|s| s.opcode).collect()
    }

    fn dst_vregs(syls: &[Syllable]) -> Vec<u32> {
        syls.iter()
            .filter_map(|s| match s.dst {
                Some(Reg::VReg(n)) => Some(n),
                _ => None,
            })
            .collect()
    }

    /// A single immediate copy: movi dst, imm.
    #[test]
    fn single_imm_copy() {
        let copies = vec![PhiCopy {
            dst: 1,
            src: CopySrc::Const(42),
        }];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        assert_eq!(syls.len(), 1);
        assert_eq!(syls[0].opcode, Opcode::MovImm);
        assert_eq!(syls[0].dst, Some(Reg::VReg(1)));
        assert_eq!(syls[0].srcs, vec![Value::Imm(42)]);
    }

    /// A single register copy: mov dst, src.
    #[test]
    fn single_reg_copy() {
        let copies = vec![PhiCopy {
            dst: 2,
            src: CopySrc::VReg(3),
        }];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        assert_eq!(syls.len(), 1);
        assert_eq!(syls[0].opcode, Opcode::Mov);
    }

    /// A self-copy (dst == src) is a no-op.
    #[test]
    fn self_copy_is_noop() {
        let copies = vec![PhiCopy {
            dst: 5,
            src: CopySrc::VReg(5),
        }];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        assert!(syls.is_empty());
    }

    /// Chain: v2 ← v1, v3 ← v2 — must emit v3←v2 before v2←v1.
    #[test]
    fn chain_copies_correct_order() {
        // v2 ← v1, v3 ← v2 : v2 is both dst and src, so v3←v2 must go first.
        let copies = vec![
            PhiCopy {
                dst: 2,
                src: CopySrc::VReg(1),
            },
            PhiCopy {
                dst: 3,
                src: CopySrc::VReg(2),
            },
        ];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        assert_eq!(syls.len(), 2);
        // First copy emitted must have dst=3 (reads v2 before v2 is overwritten).
        assert_eq!(dst_vregs(&syls)[0], 3);
        assert_eq!(dst_vregs(&syls)[1], 2);
    }

    /// Immediate copies must not clobber a register source still needed by
    /// another copy in the same parallel-copy group.
    #[test]
    fn immediate_copy_waits_for_reader() {
        let copies = vec![
            PhiCopy {
                dst: 1,
                src: CopySrc::Const(0),
            },
            PhiCopy {
                dst: 2,
                src: CopySrc::VReg(1),
            },
        ];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        assert_eq!(syls.len(), 2);
        assert_eq!(syls[0].opcode, Opcode::Mov);
        assert_eq!(syls[0].dst, Some(Reg::VReg(2)));
        assert_eq!(syls[1].opcode, Opcode::MovImm);
        assert_eq!(syls[1].dst, Some(Reg::VReg(1)));
    }

    /// Swap: v1 ← v2, v2 ← v1 — requires a temp.
    #[test]
    fn swap_uses_temp() {
        let copies = vec![
            PhiCopy {
                dst: 1,
                src: CopySrc::VReg(2),
            },
            PhiCopy {
                dst: 2,
                src: CopySrc::VReg(1),
            },
        ];
        let mut nv = 10u32;
        let syls = serialize_parallel_copies(&copies, &mut nv);
        // Needs 3 moves: tmp←v2 (or v1), one real move, then the other.
        assert_eq!(syls.len(), 3, "swap needs 3 moves");
        assert_eq!(nv, 11, "one temp vreg allocated");
        // All should be Mov
        assert!(opcodes(&syls).iter().all(|&op| op == Opcode::Mov));
    }
}
