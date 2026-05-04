//! MIR (Machine Intermediate Representation) type definitions.
//!
//! Pre-allocation instructions use `Reg::VReg`; post-allocation use `Reg::PReg`.

use std::fmt;
use vliw_asm::opcode::Opcode;

/// A register operand: virtual (pre-allocation) or physical (post-allocation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg {
    VReg(u32),
    PReg(u8),
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reg::VReg(n) => write!(f, "v{n}"),
            Reg::PReg(n) => write!(f, "r{n}"),
        }
    }
}

/// A source operand: either a register or an inline immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Reg(Reg),
    Imm(i64),
    /// Stack-frame-relative byte address used by scheduler alias analysis.
    Stack(i64),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Reg(r) => write!(f, "{r}"),
            Value::Imm(n) => write!(f, "{n}"),
            Value::Stack(n) => write!(f, "stack[{n}]"),
        }
    }
}

/// One operation inside a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syllable {
    pub opcode: Opcode,
    pub dst: Option<Reg>,
    pub srcs: Vec<Value>,
}

impl fmt::Display for Syllable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.opcode.mnemonic())?;
        if let Some(dst) = self.dst {
            write!(f, " {dst}")?;
        }
        for src in &self.srcs {
            write!(f, ", {src}")?;
        }
        Ok(())
    }
}

/// Block-ending control transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    /// Conditional branch: branch to `label` if `cond` is true.
    Branch { cond: Reg, label: String },
    /// Unconditional jump.
    Jump(String),
    /// Return from function.
    Return,
    /// Direct call to `target`; execution resumes at `cont` after return.
    ///
    /// The caller is responsible for setting up argument registers and saving
    /// the link register before this terminator, and for restoring the link
    /// register in the first syllables of `cont`.
    DirectCall { target: String, cont: String },
}

impl fmt::Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Branch { cond, label } => write!(f, "branch {cond}, {label}"),
            Terminator::Jump(label) => write!(f, "jump {label}"),
            Terminator::Return => write!(f, "ret"),
            Terminator::DirectCall { target, cont } => write!(f, "call {target}, {cont}"),
        }
    }
}

/// A basic block: a linear sequence of syllables followed by a terminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub label: String,
    pub syllables: Vec<Syllable>,
    pub terminator: Terminator,
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}:", self.label)?;
        for syl in &self.syllables {
            writeln!(f, "  {syl}")?;
        }
        writeln!(f, "  {}", self.terminator)
    }
}

/// A MIR function: an ordered list of basic blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub blocks: Vec<Block>,
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fn {}:", self.name)?;
        for block in &self.blocks {
            write!(f, "{block}")?;
        }
        Ok(())
    }
}

/// Canonical demo MIR: compute 6×7, store result to address 0x100, return.
///
/// Mirrors the hard-coded demo in `vliwc --emit=demo` but expressed as pre-allocation
/// MIR (all virtual registers) rather than a scheduled bundle stream.
pub fn demo_mir() -> Function {
    Function {
        name: "demo".into(),
        blocks: vec![Block {
            label: "entry".into(),
            syllables: vec![
                Syllable {
                    opcode: Opcode::MovImm,
                    dst: Some(Reg::VReg(0)),
                    srcs: vec![Value::Imm(6)],
                },
                Syllable {
                    opcode: Opcode::MovImm,
                    dst: Some(Reg::VReg(1)),
                    srcs: vec![Value::Imm(7)],
                },
                Syllable {
                    opcode: Opcode::Mul,
                    dst: Some(Reg::VReg(2)),
                    srcs: vec![Value::Reg(Reg::VReg(0)), Value::Reg(Reg::VReg(1))],
                },
                // Store v2 to absolute address 0x100 (base implicit 0).
                Syllable {
                    opcode: Opcode::StoreD,
                    dst: None,
                    srcs: vec![Value::Imm(0x100), Value::Reg(Reg::VReg(2))],
                },
            ],
            terminator: Terminator::Return,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple two-block function:
    ///
    /// ```text
    /// fn add_or_ret:
    /// entry:
    ///   add v0, v1, v2
    ///   cmp_eq v3, v0, 0
    ///   branch v3, exit
    /// exit:
    ///   ret
    /// ```
    fn two_block_function() -> Function {
        Function {
            name: "add_or_ret".into(),
            blocks: vec![
                Block {
                    label: "entry".into(),
                    syllables: vec![
                        Syllable {
                            opcode: Opcode::Add,
                            dst: Some(Reg::VReg(0)),
                            srcs: vec![Value::Reg(Reg::VReg(1)), Value::Reg(Reg::VReg(2))],
                        },
                        Syllable {
                            opcode: Opcode::CmpEq,
                            dst: Some(Reg::VReg(3)),
                            srcs: vec![Value::Reg(Reg::VReg(0)), Value::Imm(0)],
                        },
                    ],
                    terminator: Terminator::Branch {
                        cond: Reg::VReg(3),
                        label: "exit".into(),
                    },
                },
                Block {
                    label: "exit".into(),
                    syllables: vec![],
                    terminator: Terminator::Return,
                },
            ],
        }
    }

    #[test]
    fn construct_two_block_function() {
        let func = two_block_function();
        assert_eq!(func.blocks.len(), 2);
        assert_eq!(func.blocks[0].label, "entry");
        assert_eq!(func.blocks[1].label, "exit");
        assert_eq!(func.blocks[0].syllables.len(), 2);
        assert!(func.blocks[1].syllables.is_empty());
        assert_eq!(func.blocks[1].terminator, Terminator::Return);
    }

    #[test]
    fn pretty_printer_round_trip() {
        let func = two_block_function();
        let text = func.to_string();

        // Round-trip: printing twice produces the same output.
        let text2 = func.to_string();
        assert_eq!(text, text2);

        // Spot-check the printed form.
        assert!(text.contains("fn add_or_ret:"), "missing function header");
        assert!(text.contains("entry:"), "missing entry label");
        assert!(text.contains("exit:"), "missing exit label");
        assert!(text.contains("add v0, v1, v2"), "missing add instruction");
        assert!(text.contains("cmpeq v3, v0, 0"), "missing cmpeq instruction");
        assert!(text.contains("branch v3, exit"), "missing branch terminator");
        assert!(text.contains("ret"), "missing ret terminator");
    }

    #[test]
    fn display_reg() {
        assert_eq!(Reg::VReg(5).to_string(), "v5");
        assert_eq!(Reg::PReg(3).to_string(), "r3");
    }

    #[test]
    fn display_value() {
        assert_eq!(Value::Reg(Reg::VReg(0)).to_string(), "v0");
        assert_eq!(Value::Imm(-7).to_string(), "-7");
    }

    #[test]
    fn display_terminator() {
        assert_eq!(
            Terminator::Branch { cond: Reg::VReg(1), label: "loop".into() }.to_string(),
            "branch v1, loop"
        );
        assert_eq!(Terminator::Jump("done".into()).to_string(), "jump done");
        assert_eq!(Terminator::Return.to_string(), "ret");
    }

    #[test]
    fn physical_regs_post_alloc() {
        let syl = Syllable {
            opcode: Opcode::Add,
            dst: Some(Reg::PReg(0)),
            srcs: vec![Value::Reg(Reg::PReg(1)), Value::Reg(Reg::PReg(2))],
        };
        assert_eq!(syl.to_string(), "add r0, r1, r2");
    }
}
