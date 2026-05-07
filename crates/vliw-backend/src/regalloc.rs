//! Linear virtual-register allocator with explicit spilling.
//!
//! The allocator walks MIR in program order, rewrites virtual registers to the
//! reserved-register-free GPR range, and inserts ordinary MIR memory syllables
//! for spill stores and reloads.  Those inserted load/store syllables are
//! returned to the scheduler, so spill traffic participates in the same slot,
//! latency, and alias handling as source-program memory operations.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::liveness::{LiveSet, Liveness};
use crate::mir::{Block, Function, Reg, Syllable, Terminator, Value};
use crate::{regs, CompileError};
use vliw_asm::opcode::Opcode;

const SPILL_SLOT_BYTES: i64 = 8;

/// Rewrite all [`Reg::VReg`] operands in `func` to [`Reg::PReg`] operands.
///
/// `memory_size` is the processor's declared memory size in bytes; it controls
/// where the spill area begins (see [`crate::regs::spill_base`]).
pub fn allocate_registers(func: &Function, memory_size: u64) -> Result<Function, CompileError> {
    let last_uses = compute_block_last_uses(func);
    let live_out = Liveness::compute(func)
        .blocks
        .into_iter()
        .map(|block| block.live_out)
        .collect();
    let mut alloc = Allocator::new(last_uses, live_out, regs::spill_base(memory_size));

    let mut blocks = Vec::with_capacity(func.blocks.len());
    let mut pos = 0usize;

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let mut syllables = Vec::with_capacity(block.syllables.len());
        for syl in &block.syllables {
            syllables.extend(alloc.rewrite_syllable(syl, block_idx, pos)?);
            pos += 1;
        }

        let (prefix, terminator) = alloc.rewrite_terminator(&block.terminator, block_idx, pos)?;
        syllables.extend(prefix);
        pos += 1;

        blocks.push(Block {
            label: block.label.clone(),
            syllables,
            terminator,
        });
    }

    Ok(Function {
        name: func.name.clone(),
        blocks,
    })
}

fn compute_block_last_uses(func: &Function) -> Vec<HashMap<u32, usize>> {
    let mut all_last_uses = Vec::with_capacity(func.blocks.len());
    let mut pos = 0usize;

    for block in &func.blocks {
        let mut last_uses = HashMap::new();
        for syl in &block.syllables {
            for src in &syl.srcs {
                if let Value::Reg(Reg::VReg(vreg)) = src {
                    last_uses.insert(*vreg, pos);
                }
            }
            pos += 1;
        }

        if let Terminator::Branch {
            cond: Reg::VReg(vreg),
            ..
        } = &block.terminator
        {
            last_uses.insert(*vreg, pos);
        }
        pos += 1;

        all_last_uses.push(last_uses);
    }

    all_last_uses
}

struct Allocator {
    last_uses: Vec<HashMap<u32, usize>>,
    live_out: Vec<LiveSet>,
    assigned: HashMap<u32, u8>,
    free: VecDeque<u8>,
    spill_slots: HashMap<u32, i64>,
    next_spill_slot: i64,
    spill_base: i64,
}

impl Allocator {
    fn new(last_uses: Vec<HashMap<u32, usize>>, live_out: Vec<LiveSet>, spill_base: i64) -> Self {
        Self {
            last_uses,
            live_out,
            assigned: HashMap::new(),
            free: (regs::FIRST_ALLOCATABLE_GPR..=regs::LAST_ALLOCATABLE_GPR).collect(),
            spill_slots: HashMap::new(),
            next_spill_slot: 0,
            spill_base,
        }
    }

    fn rewrite_syllable(
        &mut self,
        syl: &Syllable,
        block_idx: usize,
        pos: usize,
    ) -> Result<Vec<Syllable>, CompileError> {
        let mut prefix = Vec::new();
        let mut src_vregs = Vec::new();
        let mut protected: HashSet<u32> = syl
            .srcs
            .iter()
            .filter_map(|src| match src {
                Value::Reg(Reg::VReg(vreg)) => Some(*vreg),
                _ => None,
            })
            .collect();

        let mut srcs = Vec::with_capacity(syl.srcs.len());
        for src in &syl.srcs {
            srcs.push(self.rewrite_value(*src, &mut src_vregs, &protected, &mut prefix)?);
        }

        let dst_vreg = match syl.dst {
            Some(Reg::VReg(vreg)) => Some(vreg),
            _ => None,
        };

        // Source values whose final use is this syllable are no longer live
        // after the operands have been read, so the destination may reuse one.
        for vreg in src_vregs {
            if Some(vreg) != dst_vreg && self.can_free_after(vreg, block_idx, pos) {
                self.free_vreg(vreg);
                protected.remove(&vreg);
            }
        }

        let dst = match syl.dst {
            Some(Reg::VReg(vreg)) => {
                let preg = self.preg_for_vreg(vreg, &protected, &mut prefix)?;
                if !self.has_later_block_use(vreg, block_idx, pos)
                    && !self.is_live_out(vreg, block_idx)
                {
                    self.free_vreg(vreg);
                }
                Some(Reg::PReg(preg))
            }
            other => other,
        };

        prefix.push(Syllable {
            opcode: syl.opcode,
            dst,
            srcs,
        });
        Ok(prefix)
    }

    fn rewrite_terminator(
        &mut self,
        term: &Terminator,
        block_idx: usize,
        pos: usize,
    ) -> Result<(Vec<Syllable>, Terminator), CompileError> {
        let mut prefix = Vec::new();
        match term {
            Terminator::Branch { cond, label } => {
                let cond = match *cond {
                    Reg::VReg(vreg) => {
                        let mut protected = HashSet::from([vreg]);
                        let preg = self.preg_for_vreg(vreg, &protected, &mut prefix)?;
                        if self.can_free_after(vreg, block_idx, pos) {
                            self.free_vreg(vreg);
                            protected.remove(&vreg);
                        }
                        Reg::PReg(preg)
                    }
                    Reg::PReg(preg) => Reg::PReg(preg),
                };
                Ok((
                    prefix,
                    Terminator::Branch {
                        cond,
                        label: label.clone(),
                    },
                ))
            }
            Terminator::Jump(label) => Ok((prefix, Terminator::Jump(label.clone()))),
            Terminator::Return => Ok((prefix, Terminator::Return)),
            Terminator::DirectCall { target, cont } => Ok((
                prefix,
                Terminator::DirectCall { target: target.clone(), cont: cont.clone() },
            )),
        }
    }

    fn rewrite_value(
        &mut self,
        value: Value,
        src_vregs: &mut Vec<u32>,
        protected: &HashSet<u32>,
        prefix: &mut Vec<Syllable>,
    ) -> Result<Value, CompileError> {
        match value {
            Value::Reg(Reg::VReg(vreg)) => {
                let preg = self.preg_for_vreg(vreg, protected, prefix)?;
                src_vregs.push(vreg);
                Ok(Value::Reg(Reg::PReg(preg)))
            }
            other => Ok(other),
        }
    }

    fn preg_for_vreg(
        &mut self,
        vreg: u32,
        protected: &HashSet<u32>,
        prefix: &mut Vec<Syllable>,
    ) -> Result<u8, CompileError> {
        if let Some(&preg) = self.assigned.get(&vreg) {
            return Ok(preg);
        }

        let preg = self.alloc_preg(vreg, protected, prefix)?;
        self.assigned.insert(vreg, preg);
        if let Some(&slot) = self.spill_slots.get(&vreg) {
            prefix.push(Syllable {
                opcode: Opcode::LoadD,
                dst: Some(Reg::PReg(preg)),
                srcs: vec![Value::Stack(slot)],
            });
        }
        Ok(preg)
    }

    fn alloc_preg(
        &mut self,
        requesting_vreg: u32,
        protected: &HashSet<u32>,
        prefix: &mut Vec<Syllable>,
    ) -> Result<u8, CompileError> {
        if let Some(preg) = self.free.pop_front() {
            return Ok(preg);
        }

        let victim = self
            .assigned
            .iter()
            .find_map(|(&vreg, &preg)| (!protected.contains(&vreg)).then_some((vreg, preg)))
            .ok_or(CompileError::OutOfRegisters {
                vreg: requesting_vreg,
            })?;
        let (victim_vreg, preg) = victim;
        self.assigned.remove(&victim_vreg);
        let slot = self.spill_slot(victim_vreg);
        prefix.push(Syllable {
            opcode: Opcode::StoreD,
            dst: None,
            srcs: vec![Value::Stack(slot), Value::Reg(Reg::PReg(preg))],
        });
        Ok(preg)
    }

    fn spill_slot(&mut self, vreg: u32) -> i64 {
        *self.spill_slots.entry(vreg).or_insert_with(|| {
            let slot = self.spill_base + self.next_spill_slot * SPILL_SLOT_BYTES;
            self.next_spill_slot += 1;
            slot
        })
    }

    fn can_free_after(&self, vreg: u32, block_idx: usize, pos: usize) -> bool {
        self.last_uses
            .get(block_idx)
            .and_then(|uses| uses.get(&vreg))
            == Some(&pos)
            && !self.is_live_out(vreg, block_idx)
    }

    fn has_later_block_use(&self, vreg: u32, block_idx: usize, pos: usize) -> bool {
        self.last_uses
            .get(block_idx)
            .and_then(|uses| uses.get(&vreg))
            .is_some_and(|last_use| *last_use > pos)
    }

    fn is_live_out(&self, vreg: u32, block_idx: usize) -> bool {
        self.live_out
            .get(block_idx)
            .copied()
            .unwrap_or_default()
            .contains(vreg)
    }

    fn free_vreg(&mut self, vreg: u32) {
        if let Some(preg) = self.assigned.remove(&vreg) {
            self.free.push_front(preg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vliw_asm::opcode::Opcode;

    #[test]
    fn rewrites_virtual_registers_to_allocatable_physical_registers() {
        let func = Function {
            name: "simple".into(),
            blocks: vec![Block {
                label: "entry".into(),
                syllables: vec![
                    Syllable {
                        opcode: Opcode::MovImm,
                        dst: Some(Reg::VReg(0)),
                        srcs: vec![Value::Imm(1)],
                    },
                    Syllable {
                        opcode: Opcode::MovImm,
                        dst: Some(Reg::VReg(1)),
                        srcs: vec![Value::Imm(2)],
                    },
                    Syllable {
                        opcode: Opcode::Add,
                        dst: Some(Reg::VReg(2)),
                        srcs: vec![Value::Reg(Reg::VReg(0)), Value::Reg(Reg::VReg(1))],
                    },
                    Syllable {
                        opcode: Opcode::Mov,
                        dst: Some(Reg::PReg(regs::RETVAL_REG)),
                        srcs: vec![Value::Reg(Reg::VReg(2))],
                    },
                ],
                terminator: Terminator::Return,
            }],
        };

        let allocated = allocate_registers(&func, vliw_asm::DEFAULT_MEMORY_SIZE).expect("allocation should succeed");
        let block = &allocated.blocks[0];

        assert_eq!(block.syllables[0].dst, Some(Reg::PReg(2)));
        assert_eq!(block.syllables[1].dst, Some(Reg::PReg(3)));
        assert_eq!(block.syllables[2].dst, Some(Reg::PReg(3)));
        assert_eq!(block.syllables[3].srcs, [Value::Reg(Reg::PReg(3))]);
    }

    #[test]
    fn reuses_register_after_last_use() {
        let func = Function {
            name: "reuse".into(),
            blocks: vec![Block {
                label: "entry".into(),
                syllables: vec![
                    Syllable {
                        opcode: Opcode::MovImm,
                        dst: Some(Reg::VReg(0)),
                        srcs: vec![Value::Imm(1)],
                    },
                    Syllable {
                        opcode: Opcode::MovImm,
                        dst: Some(Reg::VReg(1)),
                        srcs: vec![Value::Imm(2)],
                    },
                    Syllable {
                        opcode: Opcode::Add,
                        dst: Some(Reg::VReg(2)),
                        srcs: vec![Value::Reg(Reg::VReg(0)), Value::Reg(Reg::VReg(1))],
                    },
                ],
                terminator: Terminator::Return,
            }],
        };

        let allocated = allocate_registers(&func, vliw_asm::DEFAULT_MEMORY_SIZE).expect("allocation should succeed");

        assert_eq!(allocated.blocks[0].syllables[2].dst, Some(Reg::PReg(3)));
    }

    #[test]
    fn reuses_register_when_liveness_kills_before_later_syntactic_use() {
        let func = Function {
            name: "dead_later_block".into(),
            blocks: vec![
                Block {
                    label: "entry".into(),
                    syllables: vec![
                        Syllable {
                            opcode: Opcode::MovImm,
                            dst: Some(Reg::VReg(0)),
                            srcs: vec![Value::Imm(1)],
                        },
                        Syllable {
                            opcode: Opcode::MovImm,
                            dst: Some(Reg::VReg(1)),
                            srcs: vec![Value::Imm(2)],
                        },
                    ],
                    terminator: Terminator::Jump("exit".into()),
                },
                Block {
                    label: "exit".into(),
                    syllables: vec![],
                    terminator: Terminator::Return,
                },
                Block {
                    label: "dead".into(),
                    syllables: vec![Syllable {
                        opcode: Opcode::StoreW,
                        dst: None,
                        srcs: vec![Value::Imm(0x100), Value::Reg(Reg::VReg(0))],
                    }],
                    terminator: Terminator::Return,
                },
            ],
        };

        let allocated = allocate_registers(&func, vliw_asm::DEFAULT_MEMORY_SIZE).expect("allocation should succeed");

        assert_eq!(allocated.blocks[0].syllables[0].dst, Some(Reg::PReg(2)));
        assert_eq!(
            allocated.blocks[0].syllables[1].dst,
            Some(Reg::PReg(2)),
            "v0 is not live-out of entry, so v1 may reuse its register even though a later non-successor block mentions v0"
        );
    }

    #[test]
    fn spills_when_more_than_allocatable_registers_are_live() {
        let mut syllables = Vec::new();
        for vreg in 0..u32::from(regs::ALLOCATABLE_GPR_COUNT) {
            syllables.push(Syllable {
                opcode: Opcode::MovImm,
                dst: Some(Reg::VReg(vreg)),
                srcs: vec![Value::Imm(i64::from(vreg))],
            });
        }
        syllables.push(Syllable {
            opcode: Opcode::Add,
            dst: Some(Reg::VReg(u32::from(regs::ALLOCATABLE_GPR_COUNT))),
            srcs: vec![
                Value::Reg(Reg::VReg(0)),
                Value::Reg(Reg::VReg(u32::from(regs::ALLOCATABLE_GPR_COUNT - 1))),
            ],
        });
        for vreg in 0..u32::from(regs::ALLOCATABLE_GPR_COUNT) {
            syllables.push(Syllable {
                opcode: Opcode::StoreW,
                dst: None,
                srcs: vec![
                    Value::Imm(0x100 + i64::from(vreg) * 4),
                    Value::Reg(Reg::VReg(vreg)),
                ],
            });
        }

        let func = Function {
            name: "too_many".into(),
            blocks: vec![Block {
                label: "entry".into(),
                syllables,
                terminator: Terminator::Return,
            }],
        };

        let allocated = allocate_registers(&func, vliw_asm::DEFAULT_MEMORY_SIZE).expect("allocation should spill");
        let spill_stores = allocated.blocks[0]
            .syllables
            .iter()
            .filter(|syl| {
                syl.opcode == Opcode::StoreD
                    && matches!(syl.srcs.first(), Some(Value::Stack(_)))
            })
            .count();
        let spill_loads = allocated.blocks[0]
            .syllables
            .iter()
            .filter(|syl| {
                syl.opcode == Opcode::LoadD
                    && matches!(syl.srcs.first(), Some(Value::Stack(_)))
            })
            .count();

        assert!(spill_stores > 0, "allocator should emit spill stores");
        assert!(spill_loads > 0, "allocator should emit spill reloads");
    }
}
