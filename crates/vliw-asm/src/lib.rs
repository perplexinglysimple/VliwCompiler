//! `.vliw` text format data model and emitter.
//!
//! Targets the parser in LwirSimulator. See
//! <https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md>.
//! This crate is intentionally free of LLVM dependencies so it can be
//! unit-tested standalone and so the simulator team can pull it in if useful.

pub mod opcode;
pub mod parse;
pub mod verify;
pub use opcode::{Opcode, UnitKind};
pub use parse::{parse, ParseError};
pub use verify::{verify_bundle, verify_program, HazardError};

use std::fmt::{self, Write};

#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    #[error("formatting failed: {0}")]
    Fmt(#[from] fmt::Error),
}

/// A typed instruction operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// General-purpose register, e.g. `r3`.
    Reg(u8),
    /// Predicate register, e.g. `p0`.
    Pred(u8),
    /// Immediate integer constant, e.g. `42`.
    Imm(i64),
    /// Memory address `[r{base} + offset]`, `[r{base} - offset]`, or `[r{base}]`.
    MemAddr { base: u8, offset: i64 },
    /// Label reference or branch target.
    Label(String),
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Reg(n) => write!(f, "r{n}"),
            Operand::Pred(n) => write!(f, "p{n}"),
            Operand::Imm(n) => write!(f, "{n}"),
            Operand::MemAddr { base, offset } => {
                if *offset == 0 {
                    write!(f, "[r{base}]")
                } else if *offset > 0 {
                    write!(f, "[r{base} + {offset:#x}]")
                } else {
                    write!(f, "[r{base} - {:#x}]", offset.unsigned_abs())
                }
            }
            Operand::Label(s) => write!(f, "{s}"),
        }
    }
}

/// Minimal program model.
#[derive(Debug, Default, PartialEq)]
pub struct Program {
    pub processor: Processor,
    pub items: Vec<Item>,
}

/// A named functional unit declared in the `hardware { }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitDecl {
    pub name: String,
    pub kind: String,
}

/// Maps a human-readable alias to a slot index in the `layout slots { }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotAlias {
    pub name: String,
    pub slot: usize,
}

/// Cache stanza (currently parameterless).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheSpec {}

/// Topology stanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologySpec {
    pub cpus: u32,
}

impl Default for TopologySpec {
    fn default() -> Self {
        Self { cpus: 1 }
    }
}

/// Processor configuration: unit declarations, per-slot unit sets, aliases,
/// cache, and topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Processor {
    pub width: u32,
    /// Named hardware units declared in the `hardware { }` block.
    pub units: Vec<UnitDecl>,
    /// Human-readable slot aliases (`alias NAME = index`).
    pub slot_aliases: Vec<SlotAlias>,
    /// Per-slot capability sets. `slot_units[i]` lists unit names for slot `i`.
    /// Length must equal `width`.
    pub slot_units: Vec<Vec<String>>,
    pub cache: CacheSpec,
    pub topology: TopologySpec,
}

impl Default for Processor {
    /// Returns the canonical 4-slot layout (`I0, I1, M, X`).
    fn default() -> Self {
        Self {
            width: 4,
            units: vec![
                UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
                UnitDecl { name: "mem".into(), kind: "memory".into() },
                UnitDecl { name: "ctrl".into(), kind: "control".into() },
                UnitDecl { name: "mul".into(), kind: "multiplier".into() },
            ],
            slot_aliases: vec![
                SlotAlias { name: "I0".into(), slot: 0 },
                SlotAlias { name: "I1".into(), slot: 1 },
                SlotAlias { name: "M".into(), slot: 2 },
                SlotAlias { name: "X".into(), slot: 3 },
            ],
            slot_units: vec![
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["mem".into()],
                vec!["ctrl".into(), "mul".into()],
            ],
            cache: CacheSpec {},
            topology: TopologySpec { cpus: 1 },
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Item {
    Label(String),
    Bundle(Bundle),
}

/// Block-form bundle. Index = slot. `None` means `nop`.
#[derive(Debug, Default, PartialEq)]
pub struct Bundle {
    pub slots: Vec<Option<Syllable>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Syllable {
    pub opcode: String,
    pub operands: Vec<Operand>,
}

impl Syllable {
    pub fn new(opcode: impl Into<String>, operands: impl IntoIterator<Item = Operand>) -> Self {
        Self {
            opcode: opcode.into(),
            operands: operands.into_iter().collect(),
        }
    }
}

pub fn emit(program: &Program) -> Result<String, EmitError> {
    let mut out = String::new();
    emit_header(&mut out, &program.processor)?;
    out.push('\n');
    for item in &program.items {
        match item {
            Item::Label(name) => writeln!(out, "{name}:")?,
            Item::Bundle(b) => emit_bundle(&mut out, b, &program.processor)?,
        }
    }
    Ok(out)
}

fn emit_header(out: &mut String, p: &Processor) -> fmt::Result {
    writeln!(out, ".processor {{")?;
    writeln!(out, "  width {}", p.width)?;
    writeln!(out)?;
    writeln!(out, "  hardware {{")?;
    for unit in &p.units {
        writeln!(out, "    unit {} = {}", unit.name, unit.kind)?;
    }
    writeln!(out, "  }}")?;
    writeln!(out)?;
    writeln!(out, "  layout slots {{")?;
    for alias in &p.slot_aliases {
        writeln!(out, "    alias {} = {}", alias.name, alias.slot)?;
    }
    writeln!(out)?;
    for (i, units) in p.slot_units.iter().enumerate() {
        writeln!(out, "    {} = {{ {} }}", i, units.join(", "))?;
    }
    writeln!(out, "  }}")?;
    writeln!(out)?;
    writeln!(out, "  cache {{ }}")?;
    writeln!(out, "  topology {{ cpus {} }}", p.topology.cpus)?;
    writeln!(out, "}}")
}

/// Returns display names for each slot, padded to a uniform width.
///
/// Width is determined by the longest alias name; slots without an alias
/// fall back to "??".
fn slot_display_names(p: &Processor) -> Vec<String> {
    let max_width = p.slot_aliases.iter().map(|a| a.name.len()).max().unwrap_or(1);
    (0..p.width as usize)
        .map(|slot| {
            let name = p
                .slot_aliases
                .iter()
                .find(|a| a.slot == slot)
                .map(|a| a.name.as_str())
                .unwrap_or("??");
            format!("{:<max_width$}", name)
        })
        .collect()
}

fn emit_bundle(out: &mut String, b: &Bundle, p: &Processor) -> fmt::Result {
    let names = slot_display_names(p);
    writeln!(out, "{{")?;
    for slot in 0..p.width as usize {
        let name = names.get(slot).map(|s| s.as_str()).unwrap_or("??");
        match b.slots.get(slot).and_then(|s| s.as_ref()) {
            None => writeln!(out, "  {name}: nop")?,
            Some(syl) => {
                if syl.operands.is_empty() {
                    writeln!(out, "  {name}: {}", syl.opcode)?;
                } else {
                    let ops: Vec<String> = syl.operands.iter().map(|o| format!("{o}")).collect();
                    writeln!(out, "  {name}: {} {}", syl.opcode, ops.join(", "))?;
                }
            }
        }
    }
    writeln!(out, "}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syl(op: &str, ops: Vec<Operand>) -> Option<Syllable> {
        Some(Syllable::new(op, ops))
    }

    /// Reproduces the minimal canonical example from
    /// <https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md>.
    #[test]
    fn emits_canonical_example() {
        use Operand::*;
        let program = Program {
            processor: Processor::default(),
            items: vec![
                Item::Label("entry".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", vec![Reg(1), Imm(6)]),
                        syl("movi", vec![Reg(2), Imm(7)]),
                        None,
                        None,
                    ],
                }),
                Item::Bundle(Bundle {
                    slots: vec![None, None, None, syl("mul", vec![Reg(3), Reg(1), Reg(2)])],
                }),
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

        let out = emit(&program).unwrap();
        assert!(out.contains(".processor {"));
        assert!(out.contains("width 4"));
        assert!(out.contains("entry:"));
        assert!(out.contains("I0: movi r1, 6"));
        assert!(out.contains("X : mul r3, r1, r2"));
        assert!(out.contains("M : std [r0 + 0x100], r3"));
        assert!(out.contains("X : ret"));
    }

    #[test]
    fn round_trips_2slot() {
        use Operand::*;
        let proc = Processor {
            width: 2,
            units: vec![
                UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
                UnitDecl { name: "mem".into(), kind: "memory".into() },
            ],
            slot_aliases: vec![
                SlotAlias { name: "I".into(), slot: 0 },
                SlotAlias { name: "M".into(), slot: 1 },
            ],
            slot_units: vec![vec!["alu".into()], vec!["mem".into()]],
            cache: CacheSpec {},
            topology: TopologySpec { cpus: 1 },
        };

        let program = Program {
            processor: proc,
            items: vec![
                Item::Label("start".into()),
                Item::Bundle(Bundle {
                    slots: vec![syl("movi", vec![Reg(1), Imm(1)]), None],
                }),
                Item::Bundle(Bundle {
                    slots: vec![None, syl("std", vec![MemAddr { base: 0, offset: 0 }, Reg(1)])],
                }),
            ],
        };

        let out = emit(&program).unwrap();
        assert!(out.contains("width 2"));
        assert!(out.contains("unit alu = integer_alu"));
        assert!(out.contains("unit mem = memory"));
        assert!(out.contains("alias I = 0"));
        assert!(out.contains("alias M = 1"));
        assert!(out.contains("0 = { alu }"));
        assert!(out.contains("1 = { mem }"));
        assert!(out.contains("start:"));
        assert!(out.contains("I: movi r1, 1"));
        assert!(out.contains("M: nop"));
        assert!(out.contains("I: nop"));
        assert!(out.contains("M: std [r0], r1"));
    }

    #[test]
    fn round_trips_8slot() {
        use Operand::*;
        let proc = Processor {
            width: 8,
            units: vec![
                UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
                UnitDecl { name: "mem".into(), kind: "memory".into() },
                UnitDecl { name: "ctrl".into(), kind: "control".into() },
                UnitDecl { name: "mul".into(), kind: "multiplier".into() },
                UnitDecl { name: "fp".into(), kind: "floating_point".into() },
            ],
            slot_aliases: vec![
                SlotAlias { name: "I0".into(), slot: 0 },
                SlotAlias { name: "I1".into(), slot: 1 },
                SlotAlias { name: "I2".into(), slot: 2 },
                SlotAlias { name: "I3".into(), slot: 3 },
                SlotAlias { name: "M0".into(), slot: 4 },
                SlotAlias { name: "M1".into(), slot: 5 },
                SlotAlias { name: "X".into(), slot: 6 },
                SlotAlias { name: "FP".into(), slot: 7 },
            ],
            slot_units: vec![
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["mem".into()],
                vec!["mem".into()],
                vec!["ctrl".into(), "mul".into()],
                vec!["fp".into()],
            ],
            cache: CacheSpec {},
            topology: TopologySpec { cpus: 2 },
        };

        let program = Program {
            processor: proc,
            items: vec![
                Item::Label("wide".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", vec![Reg(1), Imm(42)]),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        syl("fadd", vec![Label("f0".into()), Label("f1".into()), Label("f2".into())]),
                    ],
                }),
            ],
        };

        let out = emit(&program).unwrap();
        assert!(out.contains("width 8"));
        assert!(out.contains("unit fp = floating_point"));
        assert!(out.contains("alias I0 = 0"));
        assert!(out.contains("alias FP = 7"));
        assert!(out.contains("6 = { ctrl, mul }"));
        assert!(out.contains("topology { cpus 2 }"));
        assert!(out.contains("wide:"));
        // max alias name length is 2 ("I0", "I1", "M0", "M1", "FP"); "X" pads to "X "
        assert!(out.contains("I0: movi r1, 42"));
        assert!(out.contains("I1: nop"));
        assert!(out.contains("X : nop"));
        assert!(out.contains("FP: fadd f0, f1, f2"));
    }

    /// Asserts every `Operand` variant prints in a form the simulator parser accepts.
    #[test]
    fn operand_display_forms() {
        use Operand::*;
        assert_eq!(format!("{}", Reg(3)), "r3");
        assert_eq!(format!("{}", Reg(0)), "r0");
        assert_eq!(format!("{}", Pred(0)), "p0");
        assert_eq!(format!("{}", Pred(7)), "p7");
        assert_eq!(format!("{}", Imm(0)), "0");
        assert_eq!(format!("{}", Imm(42)), "42");
        assert_eq!(format!("{}", Imm(-1)), "-1");
        assert_eq!(format!("{}", MemAddr { base: 0, offset: 0 }), "[r0]");
        assert_eq!(format!("{}", MemAddr { base: 1, offset: 0x100 }), "[r1 + 0x100]");
        assert_eq!(format!("{}", MemAddr { base: 2, offset: -8 }), "[r2 - 0x8]");
        assert_eq!(format!("{}", Label("loop_top".into())), "loop_top");
        assert_eq!(format!("{}", Label("entry".into())), "entry");
    }
}
