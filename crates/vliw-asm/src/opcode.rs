//! Typed opcode enum with slot-legality and scheduling metadata.

/// Functional unit classifications, matching the simulator's `UnitKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitKind {
    IntegerAlu,
    Memory,
    Control,
    Multiplier,
    FloatingPoint,
    Aes,
}

/// Every ISA opcode the contract supports, with associated metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Opcode {
    // Integer ALU
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    Srl,
    Sra,
    Mov,
    MovImm,
    CmpEq,
    CmpLt,
    CmpUlt,
    // Memory
    LoadB,
    LoadH,
    LoadW,
    LoadD,
    StoreB,
    StoreH,
    StoreW,
    StoreD,
    Lea,
    Prefetch,
    AcqLoad,
    RelStore,
    // Multiplier
    Mul,
    MulH,
    // Control / predicate logic
    Branch,
    Jump,
    Call,
    Ret,
    PAnd,
    POr,
    PXor,
    PNot,
    // Floating point
    FpAdd32,
    FpMul32,
    FpAdd64,
    FpMul64,
    // AES
    AesEnc,
    AesDec,
    // Universal
    Nop,
}

impl UnitKind {
    /// Parse a unit-kind string from a processor `.vliw` declaration.
    pub fn from_kind_str(s: &str) -> Option<UnitKind> {
        match s {
            "integer_alu" => Some(UnitKind::IntegerAlu),
            "memory" => Some(UnitKind::Memory),
            "control" => Some(UnitKind::Control),
            "multiplier" => Some(UnitKind::Multiplier),
            "floating_point" => Some(UnitKind::FloatingPoint),
            "aes" => Some(UnitKind::Aes),
            _ => None,
        }
    }
}

impl Opcode {
    /// Look up an opcode by its canonical mnemonic string.
    pub fn from_mnemonic(s: &str) -> Option<Opcode> {
        match s {
            "add" => Some(Opcode::Add),
            "sub" => Some(Opcode::Sub),
            "and" => Some(Opcode::And),
            "or" => Some(Opcode::Or),
            "xor" => Some(Opcode::Xor),
            "shl" => Some(Opcode::Shl),
            "srl" => Some(Opcode::Srl),
            "sra" => Some(Opcode::Sra),
            "mov" => Some(Opcode::Mov),
            "movi" => Some(Opcode::MovImm),
            "cmpeq" => Some(Opcode::CmpEq),
            "cmplt" => Some(Opcode::CmpLt),
            "cmpult" => Some(Opcode::CmpUlt),
            "ldb" => Some(Opcode::LoadB),
            "ldh" => Some(Opcode::LoadH),
            "ldw" => Some(Opcode::LoadW),
            "ldd" => Some(Opcode::LoadD),
            "stb" => Some(Opcode::StoreB),
            "sth" => Some(Opcode::StoreH),
            "stw" => Some(Opcode::StoreW),
            "std" => Some(Opcode::StoreD),
            "lea" => Some(Opcode::Lea),
            "prefetch" => Some(Opcode::Prefetch),
            "acqload" => Some(Opcode::AcqLoad),
            "relstore" => Some(Opcode::RelStore),
            "mul" => Some(Opcode::Mul),
            "mulh" => Some(Opcode::MulH),
            "br" => Some(Opcode::Branch),
            "jmp" => Some(Opcode::Jump),
            "call" => Some(Opcode::Call),
            "ret" => Some(Opcode::Ret),
            "pand" => Some(Opcode::PAnd),
            "por" => Some(Opcode::POr),
            "pxor" => Some(Opcode::PXor),
            "pnot" => Some(Opcode::PNot),
            "fpadd32" => Some(Opcode::FpAdd32),
            "fpmul32" => Some(Opcode::FpMul32),
            "fpadd64" => Some(Opcode::FpAdd64),
            "fpmul64" => Some(Opcode::FpMul64),
            "aesenc" => Some(Opcode::AesEnc),
            "aesdec" => Some(Opcode::AesDec),
            "nop" => Some(Opcode::Nop),
            _ => None,
        }
    }

    /// The canonical text mnemonic accepted by the `.vliw` assembler.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Opcode::Add => "add",
            Opcode::Sub => "sub",
            Opcode::And => "and",
            Opcode::Or => "or",
            Opcode::Xor => "xor",
            Opcode::Shl => "shl",
            Opcode::Srl => "srl",
            Opcode::Sra => "sra",
            Opcode::Mov => "mov",
            Opcode::MovImm => "movi",
            Opcode::CmpEq => "cmpeq",
            Opcode::CmpLt => "cmplt",
            Opcode::CmpUlt => "cmpult",
            Opcode::LoadB => "ldb",
            Opcode::LoadH => "ldh",
            Opcode::LoadW => "ldw",
            Opcode::LoadD => "ldd",
            Opcode::StoreB => "stb",
            Opcode::StoreH => "sth",
            Opcode::StoreW => "stw",
            Opcode::StoreD => "std",
            Opcode::Lea => "lea",
            Opcode::Prefetch => "prefetch",
            Opcode::AcqLoad => "acqload",
            Opcode::RelStore => "relstore",
            Opcode::Mul => "mul",
            Opcode::MulH => "mulh",
            Opcode::Branch => "br",
            Opcode::Jump => "jmp",
            Opcode::Call => "call",
            Opcode::Ret => "ret",
            Opcode::PAnd => "pand",
            Opcode::POr => "por",
            Opcode::PXor => "pxor",
            Opcode::PNot => "pnot",
            Opcode::FpAdd32 => "fpadd32",
            Opcode::FpMul32 => "fpmul32",
            Opcode::FpAdd64 => "fpadd64",
            Opcode::FpMul64 => "fpmul64",
            Opcode::AesEnc => "aesenc",
            Opcode::AesDec => "aesdec",
            Opcode::Nop => "nop",
        }
    }

    /// Unit kinds whose slots can execute this opcode.
    ///
    /// Returns an empty slice for `Nop`, which is legal in any slot regardless
    /// of unit declarations.
    pub fn units(self) -> &'static [UnitKind] {
        use UnitKind::*;
        match self {
            Opcode::Add
            | Opcode::Sub
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::Shl
            | Opcode::Srl
            | Opcode::Sra
            | Opcode::Mov
            | Opcode::MovImm
            | Opcode::CmpEq
            | Opcode::CmpLt
            | Opcode::CmpUlt => &[IntegerAlu],

            Opcode::LoadB
            | Opcode::LoadH
            | Opcode::LoadW
            | Opcode::LoadD
            | Opcode::StoreB
            | Opcode::StoreH
            | Opcode::StoreW
            | Opcode::StoreD
            | Opcode::Lea
            | Opcode::Prefetch
            | Opcode::AcqLoad
            | Opcode::RelStore => &[Memory],

            Opcode::Branch
            | Opcode::Jump
            | Opcode::Call
            | Opcode::Ret
            | Opcode::PAnd
            | Opcode::POr
            | Opcode::PXor
            | Opcode::PNot => &[Control],

            Opcode::Mul | Opcode::MulH => &[Multiplier],

            Opcode::FpAdd32
            | Opcode::FpMul32
            | Opcode::FpAdd64
            | Opcode::FpMul64 => &[FloatingPoint],

            Opcode::AesEnc | Opcode::AesDec => &[Aes],

            Opcode::Nop => &[],
        }
    }

    /// Cycles from issue to when the result is ready for dependent reads.
    ///
    /// Matches the default `LatencyTable` in the simulator. Load latency (3)
    /// reflects the default cache miss latency. `Nop` produces no result (0).
    pub fn latency(self) -> u32 {
        match self {
            Opcode::Add
            | Opcode::Sub
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::Shl
            | Opcode::Srl
            | Opcode::Sra
            | Opcode::Mov
            | Opcode::MovImm
            | Opcode::CmpEq
            | Opcode::CmpLt
            | Opcode::CmpUlt => 1,

            Opcode::LoadB
            | Opcode::LoadH
            | Opcode::LoadW
            | Opcode::LoadD
            | Opcode::AcqLoad => 3,

            Opcode::StoreB
            | Opcode::StoreH
            | Opcode::StoreW
            | Opcode::StoreD
            | Opcode::RelStore
            | Opcode::Lea
            | Opcode::Prefetch => 1,

            Opcode::Mul | Opcode::MulH => 3,

            Opcode::Branch
            | Opcode::Jump
            | Opcode::Call
            | Opcode::Ret
            | Opcode::PAnd
            | Opcode::POr
            | Opcode::PXor
            | Opcode::PNot => 1,

            Opcode::FpAdd32 | Opcode::FpMul32 => 4,
            Opcode::FpAdd64 | Opcode::FpMul64 => 6,

            Opcode::AesEnc | Opcode::AesDec => 4,

            Opcode::Nop => 0,
        }
    }

    /// Returns `true` if this opcode writes a GPR destination (including
    /// `Call`'s implicit write to the link register).
    pub fn writes_gpr(self) -> bool {
        matches!(
            self,
            Opcode::Add
                | Opcode::Sub
                | Opcode::And
                | Opcode::Or
                | Opcode::Xor
                | Opcode::Shl
                | Opcode::Srl
                | Opcode::Sra
                | Opcode::Mov
                | Opcode::MovImm
                | Opcode::Mul
                | Opcode::MulH
                | Opcode::Lea
                | Opcode::LoadB
                | Opcode::LoadH
                | Opcode::LoadW
                | Opcode::LoadD
                | Opcode::AcqLoad
                | Opcode::FpAdd32
                | Opcode::FpMul32
                | Opcode::FpAdd64
                | Opcode::FpMul64
                | Opcode::AesEnc
                | Opcode::AesDec
                | Opcode::Call
        )
    }

    /// Returns `true` if this opcode writes a predicate register destination.
    pub fn writes_pred(self) -> bool {
        matches!(
            self,
            Opcode::CmpEq
                | Opcode::CmpLt
                | Opcode::CmpUlt
                | Opcode::PAnd
                | Opcode::POr
                | Opcode::PXor
                | Opcode::PNot
        )
    }

    /// Returns `true` if this opcode commits a value to memory.
    pub fn writes_mem(self) -> bool {
        matches!(
            self,
            Opcode::StoreB
                | Opcode::StoreH
                | Opcode::StoreW
                | Opcode::StoreD
                | Opcode::RelStore
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Opcode, UnitKind};

    // --- units() ---

    #[test]
    fn integer_alu_ops_execute_on_integer_alu() {
        let alu_ops = [
            Opcode::Add,
            Opcode::Sub,
            Opcode::And,
            Opcode::Or,
            Opcode::Xor,
            Opcode::Shl,
            Opcode::Srl,
            Opcode::Sra,
            Opcode::Mov,
            Opcode::MovImm,
            Opcode::CmpEq,
            Opcode::CmpLt,
            Opcode::CmpUlt,
        ];
        for op in alu_ops {
            assert_eq!(op.units(), &[UnitKind::IntegerAlu], "{op:?}.units()");
        }
    }

    #[test]
    fn memory_ops_execute_on_memory_unit() {
        let mem_ops = [
            Opcode::LoadB,
            Opcode::LoadH,
            Opcode::LoadW,
            Opcode::LoadD,
            Opcode::StoreB,
            Opcode::StoreH,
            Opcode::StoreW,
            Opcode::StoreD,
            Opcode::Lea,
            Opcode::Prefetch,
            Opcode::AcqLoad,
            Opcode::RelStore,
        ];
        for op in mem_ops {
            assert_eq!(op.units(), &[UnitKind::Memory], "{op:?}.units()");
        }
    }

    #[test]
    fn control_ops_execute_on_control_unit() {
        let ctrl_ops = [
            Opcode::Branch,
            Opcode::Jump,
            Opcode::Call,
            Opcode::Ret,
            Opcode::PAnd,
            Opcode::POr,
            Opcode::PXor,
            Opcode::PNot,
        ];
        for op in ctrl_ops {
            assert_eq!(op.units(), &[UnitKind::Control], "{op:?}.units()");
        }
    }

    #[test]
    fn mul_ops_execute_on_multiplier() {
        assert_eq!(Opcode::Mul.units(), &[UnitKind::Multiplier]);
        assert_eq!(Opcode::MulH.units(), &[UnitKind::Multiplier]);
    }

    #[test]
    fn fp_ops_execute_on_fp_unit() {
        let fp_ops = [Opcode::FpAdd32, Opcode::FpMul32, Opcode::FpAdd64, Opcode::FpMul64];
        for op in fp_ops {
            assert_eq!(op.units(), &[UnitKind::FloatingPoint], "{op:?}.units()");
        }
    }

    #[test]
    fn aes_ops_execute_on_aes_unit() {
        assert_eq!(Opcode::AesEnc.units(), &[UnitKind::Aes]);
        assert_eq!(Opcode::AesDec.units(), &[UnitKind::Aes]);
    }

    #[test]
    fn nop_has_no_required_units() {
        assert_eq!(Opcode::Nop.units(), &[] as &[UnitKind]);
    }

    // --- latency() ---

    #[test]
    fn alu_ops_have_latency_1() {
        let ops = [
            Opcode::Add,
            Opcode::Sub,
            Opcode::And,
            Opcode::Or,
            Opcode::Xor,
            Opcode::Shl,
            Opcode::Srl,
            Opcode::Sra,
            Opcode::Mov,
            Opcode::MovImm,
            Opcode::CmpEq,
            Opcode::CmpLt,
            Opcode::CmpUlt,
        ];
        for op in ops {
            assert_eq!(op.latency(), 1, "{op:?}.latency()");
        }
    }

    #[test]
    fn load_ops_have_latency_3() {
        let ops = [Opcode::LoadB, Opcode::LoadH, Opcode::LoadW, Opcode::LoadD, Opcode::AcqLoad];
        for op in ops {
            assert_eq!(op.latency(), 3, "{op:?}.latency()");
        }
    }

    #[test]
    fn store_and_misc_mem_ops_have_latency_1() {
        let ops = [
            Opcode::StoreB,
            Opcode::StoreH,
            Opcode::StoreW,
            Opcode::StoreD,
            Opcode::RelStore,
            Opcode::Lea,
            Opcode::Prefetch,
        ];
        for op in ops {
            assert_eq!(op.latency(), 1, "{op:?}.latency()");
        }
    }

    #[test]
    fn mul_latency_is_3() {
        assert_eq!(Opcode::Mul.latency(), 3);
        assert_eq!(Opcode::MulH.latency(), 3);
    }

    #[test]
    fn control_ops_have_latency_1() {
        let ops = [
            Opcode::Branch,
            Opcode::Jump,
            Opcode::Call,
            Opcode::Ret,
            Opcode::PAnd,
            Opcode::POr,
            Opcode::PXor,
            Opcode::PNot,
        ];
        for op in ops {
            assert_eq!(op.latency(), 1, "{op:?}.latency()");
        }
    }

    #[test]
    fn fp32_latency_is_4() {
        assert_eq!(Opcode::FpAdd32.latency(), 4);
        assert_eq!(Opcode::FpMul32.latency(), 4);
    }

    #[test]
    fn fp64_latency_is_6() {
        assert_eq!(Opcode::FpAdd64.latency(), 6);
        assert_eq!(Opcode::FpMul64.latency(), 6);
    }

    #[test]
    fn aes_latency_is_4() {
        assert_eq!(Opcode::AesEnc.latency(), 4);
        assert_eq!(Opcode::AesDec.latency(), 4);
    }

    #[test]
    fn nop_latency_is_0() {
        assert_eq!(Opcode::Nop.latency(), 0);
    }

    // --- writes_gpr() ---

    #[test]
    fn gpr_writers() {
        let writers = [
            Opcode::Add,
            Opcode::Sub,
            Opcode::And,
            Opcode::Or,
            Opcode::Xor,
            Opcode::Shl,
            Opcode::Srl,
            Opcode::Sra,
            Opcode::Mov,
            Opcode::MovImm,
            Opcode::Mul,
            Opcode::MulH,
            Opcode::Lea,
            Opcode::LoadB,
            Opcode::LoadH,
            Opcode::LoadW,
            Opcode::LoadD,
            Opcode::AcqLoad,
            Opcode::FpAdd32,
            Opcode::FpMul32,
            Opcode::FpAdd64,
            Opcode::FpMul64,
            Opcode::AesEnc,
            Opcode::AesDec,
            Opcode::Call,
        ];
        for op in writers {
            assert!(op.writes_gpr(), "{op:?} should write GPR");
        }
    }

    #[test]
    fn non_gpr_writers() {
        let non_writers = [
            Opcode::CmpEq,
            Opcode::CmpLt,
            Opcode::CmpUlt,
            Opcode::StoreB,
            Opcode::StoreH,
            Opcode::StoreW,
            Opcode::StoreD,
            Opcode::RelStore,
            Opcode::Prefetch,
            Opcode::Branch,
            Opcode::Jump,
            Opcode::Ret,
            Opcode::PAnd,
            Opcode::POr,
            Opcode::PXor,
            Opcode::PNot,
            Opcode::Nop,
        ];
        for op in non_writers {
            assert!(!op.writes_gpr(), "{op:?} should not write GPR");
        }
    }

    // --- writes_pred() ---

    #[test]
    fn pred_writers() {
        let writers = [
            Opcode::CmpEq,
            Opcode::CmpLt,
            Opcode::CmpUlt,
            Opcode::PAnd,
            Opcode::POr,
            Opcode::PXor,
            Opcode::PNot,
        ];
        for op in writers {
            assert!(op.writes_pred(), "{op:?} should write pred");
        }
    }

    #[test]
    fn add_does_not_write_pred() {
        assert!(!Opcode::Add.writes_pred());
        assert!(!Opcode::Mul.writes_pred());
        assert!(!Opcode::LoadD.writes_pred());
        assert!(!Opcode::Ret.writes_pred());
        assert!(!Opcode::Nop.writes_pred());
    }

    // --- writes_mem() ---

    #[test]
    fn mem_writers() {
        let writers = [
            Opcode::StoreB,
            Opcode::StoreH,
            Opcode::StoreW,
            Opcode::StoreD,
            Opcode::RelStore,
        ];
        for op in writers {
            assert!(op.writes_mem(), "{op:?} should write memory");
        }
    }

    #[test]
    fn loads_do_not_write_mem() {
        assert!(!Opcode::LoadB.writes_mem());
        assert!(!Opcode::LoadD.writes_mem());
        assert!(!Opcode::AcqLoad.writes_mem());
        assert!(!Opcode::Add.writes_mem());
        assert!(!Opcode::Prefetch.writes_mem());
    }

    // --- mnemonic() spot checks ---

    #[test]
    fn mnemonic_spot_checks() {
        assert_eq!(Opcode::Add.mnemonic(), "add");
        assert_eq!(Opcode::MovImm.mnemonic(), "movi");
        assert_eq!(Opcode::Mul.mnemonic(), "mul");
        assert_eq!(Opcode::StoreD.mnemonic(), "std");
        assert_eq!(Opcode::LoadD.mnemonic(), "ldd");
        assert_eq!(Opcode::Ret.mnemonic(), "ret");
        assert_eq!(Opcode::Branch.mnemonic(), "br");
        assert_eq!(Opcode::Nop.mnemonic(), "nop");
    }
}
