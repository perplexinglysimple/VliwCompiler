//! VLIW backend: LLVM IR -> scheduled `.vliw` text.
//!
//! Stub. The dev plan will define the lowering pipeline. Today this crate
//! exposes a single `compile` entry point that returns "not implemented".

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Compile LLVM IR text to a `.vliw` program.
///
/// Eventually: parse `ir_text` via inkwell, run opt, lower to VLIW MIR,
/// schedule, bundle-pack, and emit via `vliw_asm::emit`.
pub fn compile(_ir_text: &str) -> Result<String, CompileError> {
    Err(CompileError::NotImplemented("LLVM IR -> VLIW codegen"))
}
