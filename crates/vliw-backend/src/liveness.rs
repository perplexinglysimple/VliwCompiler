//! Liveness analysis over MIR functions.
//!
//! Computes per-block live-in / live-out / upward-exposed-use (UEVar) /
//! variable-kill (VarKill) sets using classic backward dataflow fixpoint
//! iteration.  Only virtual registers (`Reg::VReg`) are tracked; physical
//! registers that appear in post-allocation code are ignored.

use std::collections::HashMap;

use crate::mir::{Function, Reg, Terminator, Value};

/// A set of virtual-register indices stored as a 64-bit bitmask.
///
/// Bit *i* is set when `VReg(i)` is a member of the set.  Supports up to
/// 64 virtual registers — sufficient for unit tests and small functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveSet(pub u64);

impl LiveSet {
    pub const fn empty() -> Self {
        LiveSet(0)
    }

    pub fn insert(&mut self, idx: u32) {
        debug_assert!(idx < 64, "VReg index out of LiveSet range");
        self.0 |= 1u64 << idx;
    }

    pub fn contains(self, idx: u32) -> bool {
        idx < 64 && (self.0 >> idx) & 1 == 1
    }

    pub fn union(self, other: LiveSet) -> LiveSet {
        LiveSet(self.0 | other.0)
    }

    pub fn difference(self, other: LiveSet) -> LiveSet {
        LiveSet(self.0 & !other.0)
    }
}

/// Per-block liveness information produced by [`Liveness::compute`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockInfo {
    /// Variables used in this block before any definition (upward-exposed uses).
    pub use_before_def: LiveSet,
    /// Variables defined (killed) in this block.
    pub def: LiveSet,
    /// Variables live at the *start* of this block.
    pub live_in: LiveSet,
    /// Variables live at the *end* of this block.
    pub live_out: LiveSet,
}

/// Liveness analysis result for an entire function.
pub struct Liveness {
    /// One [`BlockInfo`] per block, indexed by block position in
    /// [`Function::blocks`].
    pub blocks: Vec<BlockInfo>,
}

impl Liveness {
    /// Run backward dataflow liveness analysis on `func` to fixpoint.
    pub fn compute(func: &Function) -> Self {
        let n = func.blocks.len();
        let mut infos: Vec<BlockInfo> = vec![BlockInfo::default(); n];

        // Pass 1 — compute use_before_def and def per block.
        for (i, block) in func.blocks.iter().enumerate() {
            let info = &mut infos[i];
            for syl in &block.syllables {
                for src in &syl.srcs {
                    if let Value::Reg(Reg::VReg(idx)) = src {
                        if !info.def.contains(*idx) {
                            info.use_before_def.insert(*idx);
                        }
                    }
                }
                if let Some(Reg::VReg(idx)) = syl.dst {
                    info.def.insert(idx);
                }
            }
            // Terminator register uses happen after all syllable defs.
            if let Terminator::Branch { cond: Reg::VReg(idx), .. } = &block.terminator {
                if !info.def.contains(*idx) {
                    info.use_before_def.insert(*idx);
                }
            }
        }

        // Build label → block-index map for resolving branch/jump targets.
        let label_to_idx: HashMap<&str, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.label.as_str(), i))
            .collect();

        // Successor lists derived from terminators.
        let successors: Vec<Vec<usize>> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                let mut succs: Vec<usize> = Vec::new();
                match &block.terminator {
                    Terminator::Branch { label, .. } => {
                        if let Some(&j) = label_to_idx.get(label.as_str()) {
                            succs.push(j);
                        }
                        // Conditional branch also has a fall-through to the next block.
                        let fall = i + 1;
                        if fall < n && !succs.contains(&fall) {
                            succs.push(fall);
                        }
                    }
                    Terminator::Jump(label) => {
                        if let Some(&j) = label_to_idx.get(label.as_str()) {
                            succs.push(j);
                        }
                    }
                    Terminator::Return => {}
                    Terminator::DirectCall { cont, .. } => {
                        if let Some(&j) = label_to_idx.get(cont.as_str()) {
                            succs.push(j);
                        }
                    }
                }
                succs
            })
            .collect();

        // Pass 2 — iterate backward dataflow to fixpoint.
        //
        // live_out[B] = ∪  live_in[S]  for S in succs(B)
        // live_in[B]  = use_before_def[B] ∪ (live_out[B] − def[B])
        loop {
            let mut changed = false;
            for i in (0..n).rev() {
                let new_live_out = successors[i]
                    .iter()
                    .fold(LiveSet::empty(), |acc, &s| acc.union(infos[s].live_in));

                let use_before_def = infos[i].use_before_def;
                let def = infos[i].def;
                let new_live_in = use_before_def.union(new_live_out.difference(def));

                if new_live_out != infos[i].live_out || new_live_in != infos[i].live_in {
                    changed = true;
                    infos[i].live_out = new_live_out;
                    infos[i].live_in = new_live_in;
                }
            }
            if !changed {
                break;
            }
        }

        Liveness { blocks: infos }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{Block, Function, Reg, Syllable, Terminator, Value};
    use vliw_asm::opcode::Opcode;

    /// Two-block loop requiring fixpoint iteration to converge:
    ///
    /// ```text
    /// fn simple_loop:
    /// header:                        (index 0)
    ///   add v0, v0, 1    # uses v0 (upward-exposed); defs v0
    ///   cmpeq v1, v0, 10 # uses v0 (killed above);   defs v1
    ///   branch v1, body  # uses v1 (killed above)
    ///                    # fall-through also reaches body
    /// body:                          (index 1)
    ///   jump header      # back-edge — no register uses
    /// ```
    ///
    /// Expected (derived by hand):
    ///
    /// | block  | UBD  | def   | live_in | live_out |
    /// |--------|------|-------|---------|----------|
    /// | header | {v0} | {v0,v1} | {v0} | {v0}   |
    /// | body   | {}   | {}    | {v0}    | {v0}   |
    fn simple_loop() -> Function {
        Function {
            name: "simple_loop".into(),
            blocks: vec![
                Block {
                    label: "header".into(),
                    syllables: vec![
                        Syllable {
                            opcode: Opcode::Add,
                            dst: Some(Reg::VReg(0)),
                            srcs: vec![Value::Reg(Reg::VReg(0)), Value::Imm(1)],
                        },
                        Syllable {
                            opcode: Opcode::CmpEq,
                            dst: Some(Reg::VReg(1)),
                            srcs: vec![Value::Reg(Reg::VReg(0)), Value::Imm(10)],
                        },
                    ],
                    terminator: Terminator::Branch {
                        cond: Reg::VReg(1),
                        label: "body".into(),
                    },
                },
                Block {
                    label: "body".into(),
                    syllables: vec![],
                    terminator: Terminator::Jump("header".into()),
                },
            ],
        }
    }

    #[test]
    fn two_block_loop_liveness() {
        let func = simple_loop();
        let liv = Liveness::compute(&func);

        // bit 0 = VReg(0) = v0, bit 1 = VReg(1) = v1
        let v0 = LiveSet(0b01);
        let v0v1 = LiveSet(0b11);
        let empty = LiveSet::empty();

        // header
        assert_eq!(liv.blocks[0].use_before_def, v0,   "header UBD");
        assert_eq!(liv.blocks[0].def,            v0v1,  "header def");
        assert_eq!(liv.blocks[0].live_in,        v0,   "header live_in");
        assert_eq!(liv.blocks[0].live_out,       v0,   "header live_out");

        // body
        assert_eq!(liv.blocks[1].use_before_def, empty, "body UBD");
        assert_eq!(liv.blocks[1].def,            empty, "body def");
        assert_eq!(liv.blocks[1].live_in,        v0,   "body live_in");
        assert_eq!(liv.blocks[1].live_out,       v0,   "body live_out");
    }

    /// Verify use_before_def correctly distinguishes upward-exposed uses
    /// from uses of locally-defined registers.
    #[test]
    fn use_before_def_and_def_sets() {
        let func = simple_loop();
        let liv = Liveness::compute(&func);

        // v1 is defined in header before any use of v1 leaks upward.
        assert!(!liv.blocks[0].use_before_def.contains(1), "v1 must not be upward-exposed");
        // v0 IS used before it is re-defined in header.
        assert!(liv.blocks[0].use_before_def.contains(0), "v0 must be upward-exposed");
        // Both v0 and v1 are killed in header.
        assert!(liv.blocks[0].def.contains(0), "v0 must be in def[header]");
        assert!(liv.blocks[0].def.contains(1), "v1 must be in def[header]");
    }

    /// A single-block function with only a return terminates immediately;
    /// all live sets are empty.
    #[test]
    fn single_block_return_all_empty() {
        let func = Function {
            name: "trivial".into(),
            blocks: vec![Block {
                label: "entry".into(),
                syllables: vec![],
                terminator: Terminator::Return,
            }],
        };
        let liv = Liveness::compute(&func);
        assert_eq!(liv.blocks[0].use_before_def, LiveSet::empty());
        assert_eq!(liv.blocks[0].def,            LiveSet::empty());
        assert_eq!(liv.blocks[0].live_in,        LiveSet::empty());
        assert_eq!(liv.blocks[0].live_out,       LiveSet::empty());
    }
}
