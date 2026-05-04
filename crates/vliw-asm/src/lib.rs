//! `.vliw` text format data model and emitter.
//!
//! Targets the parser in LwirSimulator. See
//! <https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md>.
//! This crate is intentionally free of LLVM dependencies so it can be
//! unit-tested standalone and so the simulator team can pull it in if useful.
//!
//! Only the canonical 4-slot layout (`I, I, M, X`) is modeled today —
//! enough to round-trip the example program in the spec. Generalizing to
//! arbitrary `.processor { ... }` layouts is part of the dev plan.

use std::fmt::{self, Write};

#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    #[error("formatting failed: {0}")]
    Fmt(#[from] fmt::Error),
}

/// Minimal program model. Just enough to emit the canonical example
/// from `vliw_asm_format.md` end-to-end.
#[derive(Debug, Default)]
pub struct Program {
    pub processor: Processor,
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub struct Processor {
    pub width: u32,
}

impl Default for Processor {
    fn default() -> Self {
        Self { width: 4 }
    }
}

#[derive(Debug)]
pub enum Item {
    Label(String),
    Bundle(Bundle),
}

/// Block-form bundle. Index = slot. `None` means `nop`.
#[derive(Debug, Default)]
pub struct Bundle {
    pub slots: Vec<Option<Syllable>>,
}

#[derive(Debug)]
pub struct Syllable {
    pub opcode: String,
    pub operands: Vec<String>,
}

impl Syllable {
    pub fn new(opcode: impl Into<String>, operands: impl IntoIterator<Item = String>) -> Self {
        Self {
            opcode: opcode.into(),
            operands: operands.into_iter().collect(),
        }
    }
}

/// Slot aliases for the canonical 4-slot layout.
const CANONICAL_SLOT_NAMES: &[&str] = &["I0", "I1", "M ", "X "];

pub fn emit(program: &Program) -> Result<String, EmitError> {
    let mut out = String::new();
    emit_header(&mut out, &program.processor)?;
    out.push('\n');
    for item in &program.items {
        match item {
            Item::Label(name) => writeln!(out, "{name}:")?,
            Item::Bundle(b) => emit_bundle(&mut out, b, program.processor.width)?,
        }
    }
    Ok(out)
}

fn emit_header(out: &mut String, p: &Processor) -> fmt::Result {
    // Hardcoded canonical 4-slot layout for now. Generalizing this is
    // tracked in docs/architecture.md.
    writeln!(out, ".processor {{")?;
    writeln!(out, "  width {}", p.width)?;
    writeln!(out)?;
    writeln!(out, "  hardware {{")?;
    writeln!(out, "    unit alu = integer_alu")?;
    writeln!(out, "    unit mem = memory")?;
    writeln!(out, "    unit ctrl = control")?;
    writeln!(out, "    unit mul = multiplier")?;
    writeln!(out, "  }}")?;
    writeln!(out)?;
    writeln!(out, "  layout slots {{")?;
    writeln!(out, "    alias I0 = 0")?;
    writeln!(out, "    alias I1 = 1")?;
    writeln!(out, "    alias M = 2")?;
    writeln!(out, "    alias X = 3")?;
    writeln!(out)?;
    writeln!(out, "    0 = {{ alu }}")?;
    writeln!(out, "    1 = {{ alu }}")?;
    writeln!(out, "    2 = {{ mem }}")?;
    writeln!(out, "    3 = {{ ctrl, mul }}")?;
    writeln!(out, "  }}")?;
    writeln!(out)?;
    writeln!(out, "  cache {{ }}")?;
    writeln!(out, "  topology {{ cpus 1 }}")?;
    writeln!(out, "}}")
}

fn emit_bundle(out: &mut String, b: &Bundle, width: u32) -> fmt::Result {
    writeln!(out, "{{")?;
    for slot in 0..width as usize {
        let name = CANONICAL_SLOT_NAMES
            .get(slot)
            .copied()
            .unwrap_or("?? ");
        match b.slots.get(slot).and_then(|s| s.as_ref()) {
            None => writeln!(out, "  {name}: nop")?,
            Some(syl) => {
                if syl.operands.is_empty() {
                    writeln!(out, "  {name}: {}", syl.opcode)?;
                } else {
                    writeln!(out, "  {name}: {} {}", syl.opcode, syl.operands.join(", "))?;
                }
            }
        }
    }
    writeln!(out, "}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syl(op: &str, ops: &[&str]) -> Option<Syllable> {
        Some(Syllable::new(
            op,
            ops.iter().map(|s| s.to_string()),
        ))
    }

    /// Reproduces the minimal canonical example from
    /// <https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md>.
    #[test]
    fn emits_canonical_example() {
        let program = Program {
            processor: Processor { width: 4 },
            items: vec![
                Item::Label("entry".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", &["r1", "6"]),
                        syl("movi", &["r2", "7"]),
                        None,
                        None,
                    ],
                }),
                Item::Bundle(Bundle {
                    slots: vec![
                        None,
                        None,
                        None,
                        syl("mul", &["r3", "r1", "r2"]),
                    ],
                }),
                Item::Bundle(Bundle {
                    slots: vec![
                        None,
                        None,
                        syl("std", &["[r0 + 0x100]", "r3"]),
                        syl("ret", &[]),
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
}
