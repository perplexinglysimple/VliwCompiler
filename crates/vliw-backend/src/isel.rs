//! ISel: lower validated LLVM IR to MIR.
//!
//! Supported LLVM IR instructions: `add`, `sub`, `mul`, `and`, `or`, `xor`,
//! `shl`, `lshr`, `ashr`, `icmp` (eq/ne/slt/sgt/ult/ugt), `phi`, `br` (conditional
//! and unconditional), `ret`, `load` (i8/i32/i64), `store` (i8/i32/i64),
//! `getelementptr` (constant indices folded into load/store displacement),
//! `call` (direct calls to functions defined in the same module).
//!
//! Phi lowering: phi nodes are left in place during ISel (a VReg is assigned
//! to the phi result so downstream uses resolve correctly).  After the ISel
//! worklist finishes, [`crate::phi::lower_phi_copies`] inserts parallel-copy
//! `mov` syllables into each predecessor block.  For back-edges (where the
//! false-target of a conditional branch is a block already placed in MIR), a
//! synthetic "latch" block is inserted between the branching block and the
//! real back-edge target; phi copies for the back-edge are placed in the
//! latch so they execute only on the loop-continue path.
//!
//! Memory addressing: pointers are resolved to a `(base: Value, offset: i64)`
//! pair.  A constant `inttoptr` expression folds to `(Imm(addr), 0)`.  A GEP
//! with all-constant indices folds to `(base, byte_offset)` without emitting
//! any extra syllables.  Store srcs layout: `[base, data]` when offset == 0,
//! or `[base, Imm(offset), data]` when offset != 0.  Load srcs layout:
//! `[base]` or `[base, Imm(offset)]`.
//!
//! Block ordering: for conditional branches the false-target block is placed
//! immediately after the branch block in MIR order so that the fall-through
//! path is correct.  A worklist drives the traversal.
//!
//! Call lowering: a `call` instruction in the middle of a LLVM BB splits the
//! MIR at that point.  The pre-call MIR block ends with
//! `Terminator::DirectCall`; a synthetic continuation block captures the
//! post-call instructions.  The caller emits argument moves to r2–r9, saves
//! r31 before the call, and restores r31 at the top of the continuation block.

use std::collections::{HashMap, HashSet, VecDeque};

use inkwell::llvm_sys::core::{
    LLVMConstIntGetSExtValue, LLVMCountBasicBlocks, LLVMCountIncoming, LLVMGetArgOperand,
    LLVMGetCalledValue, LLVMGetCondition, LLVMGetConstOpcode, LLVMGetGEPSourceElementType,
    LLVMGetIncomingBlock, LLVMGetIncomingValue, LLVMGetIntTypeWidth, LLVMGetNumArgOperands,
    LLVMGetOperand, LLVMGetSuccessor, LLVMGetTypeKind, LLVMGetValueName2, LLVMIsAConstantExpr,
    LLVMIsAConstantInt, LLVMIsAFunction,
};
use inkwell::llvm_sys::prelude::LLVMTypeRef;
use inkwell::llvm_sys::{LLVMOpcode, LLVMTypeKind};
use inkwell::module::Module;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{AsValueRef, BasicValueEnum, FunctionValue, InstructionOpcode, Operand};
use inkwell::IntPredicate;
use vliw_asm::opcode::Opcode;

use crate::mir::{Block, Function, Reg, Syllable, Terminator, Value};
use crate::phi::{CopySrc, PhiCopy};
use crate::regs::{ARG_REG_FIRST, LINK_REG, LINK_REG_SAVE_ADDR, RETVAL_REG};
use crate::CompileError;

/// Raw phi incoming value before the vreg_map is fully populated.
enum RawPhiVal {
    Const(i64),
    /// LLVM value pointer; resolved to a VReg after ISel completes.
    LlvmRef(usize),
}

/// Pending phi: collected during ISel, lowered after all blocks are built.
struct PendingPhi {
    /// Label of the block that owns the phi.
    phi_block_label: String,
    /// VReg assigned to the phi result.
    dst: u32,
    /// Per incoming edge: (predecessor block label as seen in LLVM IR, value).
    incoming: Vec<(String, RawPhiVal)>,
}

/// Lower all function definitions in `module` to MIR [`Function`]s.
///
/// Declarations (no body) are silently skipped.  Returns one [`Function`]
/// per definition, in module order.
pub fn lower_all_functions(module: &Module) -> Result<Vec<Function>, CompileError> {
    module
        .get_functions()
        .filter(|f| f.count_basic_blocks() > 0)
        .map(|f| lower_function(f))
        .collect()
}

/// Lower the single function definition in `module` to a MIR [`Function`].
///
/// Panics if the module contains no function with a body; call
/// [`crate::check_module`] first to enforce the single-definition
/// constraint.
pub fn lower_module(module: &Module) -> Result<Function, CompileError> {
    lower_all_functions(module)?
        .into_iter()
        .next()
        .ok_or(CompileError::NotImplemented("no function body to lower"))
}

/// Lower a single LLVM function value to a MIR [`Function`].
fn lower_function(llvm_fn: FunctionValue) -> Result<Function, CompileError> {
    let name = llvm_fn.get_name().to_string_lossy().into_owned();

    // Pre-assign stable labels to every basic block, keyed by raw pointer so
    // we can resolve branch targets returned by the LLVM C API.
    let all_blocks = llvm_fn.get_basic_blocks();
    let mut label_of: HashMap<usize, String> = HashMap::new();
    let mut bb_of: HashMap<String, inkwell::basic_block::BasicBlock> = HashMap::new();
    for (idx, bb) in all_blocks.iter().enumerate() {
        let raw = bb.get_name().to_string_lossy().into_owned();
        let label = if raw.is_empty() { format!("bb{idx}") } else { raw };
        label_of.insert(bb.as_mut_ptr() as usize, label.clone());
        bb_of.insert(label, *bb);
    }
    let entry_label = label_of[&(all_blocks[0].as_mut_ptr() as usize)].clone();

    // Map LLVM value pointer → VReg index.
    let mut vreg_map: HashMap<usize, u32> = HashMap::new();
    let mut next_vreg: u32 = 0;

    // GEP folding: maps GEP result value ref → (base, byte_offset).
    let mut gep_map: HashMap<usize, (Value, i64)> = HashMap::new();

    // Collected phi nodes, resolved after ISel completes.
    let mut pending_phis: Vec<PendingPhi> = Vec::new();

    // Back-edge edge remap: (src_block_label, dst_block_label) → latch_label.
    let mut edge_remap: HashMap<(String, String), String> = HashMap::new();

    // Counter for generating unique continuation block labels for call splits.
    let mut call_count: usize = 0;

    // Function parameters arrive in argument registers; give each one a VReg.
    for param in llvm_fn.get_params() {
        vreg_map.insert(bve_key(param), next_vreg);
        next_vreg += 1;
    }

    // Process blocks in worklist order.
    let mut worklist: VecDeque<String> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut mir_blocks: Vec<Block> = Vec::new();

    worklist.push_back(entry_label);

    while let Some(label) = worklist.pop_front() {
        if visited.contains(&label) {
            continue;
        }
        visited.insert(label.clone());

        let bb = *bb_of.get(&label).expect("label not in bb_of");

        // Current MIR block state — may be split by call instructions.
        let mut current_mir_label = label.clone();
        let mut syllables: Vec<Syllable> = Vec::new();
        let mut terminator = Terminator::Return;

        let mut false_succ: Option<String> = None;
        let mut true_succ: Option<String> = None;

        for instr in bb.get_instructions() {
            match instr.get_opcode() {
                InstructionOpcode::Phi => {
                    let phi_ref = instr.as_value_ref();
                    let dst_id = next_vreg;
                    next_vreg += 1;
                    vreg_map.insert(phi_ref as usize, dst_id);

                    let n = unsafe { LLVMCountIncoming(phi_ref) };
                    let mut incoming: Vec<(String, RawPhiVal)> = Vec::new();
                    for i in 0..n {
                        let val_ref = unsafe { LLVMGetIncomingValue(phi_ref, i) };
                        let bb_ref = unsafe { LLVMGetIncomingBlock(phi_ref, i) };
                        let pred_label = label_of[&(bb_ref as usize)].clone();
                        let raw_val = if unsafe { !LLVMIsAConstantInt(val_ref).is_null() } {
                            let v = unsafe { LLVMConstIntGetSExtValue(val_ref) };
                            RawPhiVal::Const(v)
                        } else {
                            RawPhiVal::LlvmRef(val_ref as usize)
                        };
                        incoming.push((pred_label, raw_val));
                    }
                    pending_phis.push(PendingPhi {
                        phi_block_label: label.clone(),
                        dst: dst_id,
                        incoming,
                    });
                }

                InstructionOpcode::Return => {
                    let mut ret_ops = instr.get_operands().flatten();
                    if let Some(Operand::Value(BasicValueEnum::IntValue(iv))) = ret_ops.next() {
                        if iv.is_const() {
                            let imm = iv.get_sign_extended_constant().unwrap_or(0);
                            syllables.push(Syllable {
                                opcode: Opcode::MovImm,
                                dst: Some(Reg::PReg(RETVAL_REG)),
                                srcs: vec![Value::Imm(imm)],
                            });
                        } else {
                            let vreg = *vreg_map
                                .get(&(iv.as_value_ref() as usize))
                                .expect("return value not in vreg_map");
                            syllables.push(Syllable {
                                opcode: Opcode::Mov,
                                dst: Some(Reg::PReg(RETVAL_REG)),
                                srcs: vec![Value::Reg(Reg::VReg(vreg))],
                            });
                        }
                    }
                    terminator = Terminator::Return;
                }

                InstructionOpcode::Br => {
                    if instr.is_conditional().unwrap_or(false) {
                        let cond_raw = unsafe { LLVMGetCondition(instr.as_value_ref()) };
                        let true_raw = unsafe { LLVMGetSuccessor(instr.as_value_ref(), 0) };
                        let false_raw = unsafe { LLVMGetSuccessor(instr.as_value_ref(), 1) };

                        let true_label = label_of[&(true_raw as usize)].clone();
                        let false_label = label_of[&(false_raw as usize)].clone();
                        let cond_vreg = *vreg_map
                            .get(&(cond_raw as usize))
                            .expect("branch condition not in vreg_map");

                        terminator = Terminator::Branch {
                            cond: Reg::VReg(cond_vreg),
                            label: true_label.clone(),
                        };
                        false_succ = Some(false_label);
                        true_succ = Some(true_label);
                    } else {
                        let target_raw = unsafe { LLVMGetSuccessor(instr.as_value_ref(), 0) };
                        let target_label = label_of[&(target_raw as usize)].clone();
                        terminator = Terminator::Jump(target_label.clone());
                        true_succ = Some(target_label);
                    }
                }

                InstructionOpcode::Call => {
                    let call_ref = instr.as_value_ref();
                    let callee = unsafe { LLVMGetCalledValue(call_ref) };

                    if unsafe { LLVMIsAFunction(callee).is_null() } {
                        return Err(CompileError::UnsupportedFeature(
                            "indirect call not supported".into(),
                        ));
                    }
                    if unsafe { LLVMCountBasicBlocks(callee) } == 0 {
                        return Err(CompileError::UnsupportedFeature(
                            "call to external function".into(),
                        ));
                    }

                    // Get callee name.
                    let mut name_len = 0usize;
                    let name_ptr =
                        unsafe { LLVMGetValueName2(callee, &mut name_len as *mut usize) };
                    let target_name = unsafe {
                        std::str::from_utf8(std::slice::from_raw_parts(
                            name_ptr as *const u8,
                            name_len,
                        ))
                        .unwrap_or("unknown")
                        .to_owned()
                    };

                    // Emit argument moves: place each arg in r2, r3, ..., r9.
                    let num_args = unsafe { LLVMGetNumArgOperands(call_ref) } as usize;
                    if num_args > 8 {
                        return Err(CompileError::UnsupportedFeature(
                            "call with more than 8 arguments".into(),
                        ));
                    }
                    for i in 0..num_args {
                        let arg_ref = unsafe { LLVMGetArgOperand(call_ref, i as u32) };
                        let arg_val = if unsafe { !LLVMIsAConstantInt(arg_ref).is_null() } {
                            let imm = unsafe { LLVMConstIntGetSExtValue(arg_ref) };
                            syllables.push(Syllable {
                                opcode: Opcode::MovImm,
                                dst: Some(Reg::PReg(ARG_REG_FIRST + i as u8)),
                                srcs: vec![Value::Imm(imm)],
                            });
                            continue;
                        } else {
                            let k = arg_ref as usize;
                            let vreg = *vreg_map
                                .get(&k)
                                .expect("call argument not in vreg_map");
                            Value::Reg(Reg::VReg(vreg))
                        };
                        syllables.push(Syllable {
                            opcode: Opcode::Mov,
                            dst: Some(Reg::PReg(ARG_REG_FIRST + i as u8)),
                            srcs: vec![arg_val],
                        });
                    }

                    // Save link register before the call.
                    syllables.push(Syllable {
                        opcode: Opcode::StoreD,
                        dst: None,
                        srcs: vec![
                            Value::Imm(LINK_REG_SAVE_ADDR),
                            Value::Reg(Reg::PReg(LINK_REG)),
                        ],
                    });

                    // Finish the pre-call MIR block with a DirectCall terminator.
                    let cont_label =
                        format!("{current_mir_label}__cont__{call_count}");
                    call_count += 1;
                    mir_blocks.push(Block {
                        label: current_mir_label.clone(),
                        syllables: std::mem::take(&mut syllables),
                        terminator: Terminator::DirectCall {
                            target: target_name,
                            cont: cont_label.clone(),
                        },
                    });

                    // Begin the continuation block.
                    current_mir_label = cont_label;
                    terminator = Terminator::Return; // overwritten by subsequent instructions

                    // Restore link register at the top of the continuation.
                    syllables.push(Syllable {
                        opcode: Opcode::LoadD,
                        dst: Some(Reg::PReg(LINK_REG)),
                        srcs: vec![Value::Imm(LINK_REG_SAVE_ADDR)],
                    });

                    // Capture return value (if non-void) from RETVAL_REG into a new vreg.
                    let ret_ty = instr.get_type();
                    let is_void = matches!(ret_ty, inkwell::types::AnyTypeEnum::VoidType(_));
                    if !is_void {
                        let ret_vreg = next_vreg;
                        next_vreg += 1;
                        vreg_map.insert(instr.as_value_ref() as usize, ret_vreg);
                        syllables.push(Syllable {
                            opcode: Opcode::Mov,
                            dst: Some(Reg::VReg(ret_vreg)),
                            srcs: vec![Value::Reg(Reg::PReg(RETVAL_REG))],
                        });
                    }
                    // If void, no result to capture; continue with the next instruction.
                }

                InstructionOpcode::ICmp => {
                    let pred = instr
                        .get_icmp_predicate()
                        .expect("ICmp must have a predicate");
                    let mut operands = instr.get_operands().flatten();
                    let lhs = lower_operand(
                        operands.next().expect("icmp lhs"),
                        &mut vreg_map,
                        &mut next_vreg,
                        &mut syllables,
                    );
                    let rhs = lower_operand(
                        operands.next().expect("icmp rhs"),
                        &mut vreg_map,
                        &mut next_vreg,
                        &mut syllables,
                    );

                    let (opcode, src0, src1) = match pred {
                        IntPredicate::EQ => (Opcode::CmpEq, lhs, rhs),
                        IntPredicate::NE => (Opcode::CmpEq, lhs, rhs),
                        IntPredicate::SLT => (Opcode::CmpLt, lhs, rhs),
                        IntPredicate::SGT => (Opcode::CmpLt, rhs, lhs),
                        IntPredicate::ULT => (Opcode::CmpUlt, lhs, rhs),
                        IntPredicate::UGT => (Opcode::CmpUlt, rhs, lhs),
                        _ => {
                            return Err(CompileError::NotImplemented("unsupported icmp predicate"))
                        }
                    };

                    let cmp_dst = next_vreg;
                    next_vreg += 1;
                    syllables.push(Syllable {
                        opcode,
                        dst: Some(Reg::VReg(cmp_dst)),
                        srcs: vec![src0, src1],
                    });

                    let result_vreg = if pred == IntPredicate::NE {
                        let not_dst = next_vreg;
                        next_vreg += 1;
                        syllables.push(Syllable {
                            opcode: Opcode::PNot,
                            dst: Some(Reg::VReg(not_dst)),
                            srcs: vec![Value::Reg(Reg::VReg(cmp_dst))],
                        });
                        not_dst
                    } else {
                        cmp_dst
                    };

                    vreg_map.insert(instr.as_value_ref() as usize, result_vreg);
                }

                op @ (InstructionOpcode::Add
                | InstructionOpcode::Sub
                | InstructionOpcode::Mul
                | InstructionOpcode::And
                | InstructionOpcode::Or
                | InstructionOpcode::Xor
                | InstructionOpcode::Shl
                | InstructionOpcode::LShr
                | InstructionOpcode::AShr) => {
                    let mut operands = instr.get_operands().flatten();
                    let lhs = lower_operand(
                        operands.next().expect("binary op requires LHS"),
                        &mut vreg_map,
                        &mut next_vreg,
                        &mut syllables,
                    );
                    let rhs = lower_operand(
                        operands.next().expect("binary op requires RHS"),
                        &mut vreg_map,
                        &mut next_vreg,
                        &mut syllables,
                    );

                    let vliw_op = match op {
                        InstructionOpcode::Add => Opcode::Add,
                        InstructionOpcode::Sub => Opcode::Sub,
                        InstructionOpcode::Mul => Opcode::Mul,
                        InstructionOpcode::And => Opcode::And,
                        InstructionOpcode::Or => Opcode::Or,
                        InstructionOpcode::Xor => Opcode::Xor,
                        InstructionOpcode::Shl => Opcode::Shl,
                        InstructionOpcode::LShr => Opcode::Srl,
                        InstructionOpcode::AShr => Opcode::Sra,
                        _ => unreachable!(),
                    };

                    let dst_id = next_vreg;
                    next_vreg += 1;
                    vreg_map.insert(instr.as_value_ref() as usize, dst_id);

                    syllables.push(Syllable {
                        opcode: vliw_op,
                        dst: Some(Reg::VReg(dst_id)),
                        srcs: vec![lhs, rhs],
                    });
                }

                InstructionOpcode::Store => {
                    let mut operands = instr.get_operands().flatten();
                    let val_op = operands.next().expect("store: missing value operand");
                    let ptr_op = operands.next().expect("store: missing pointer operand");

                    let store_opcode = match &val_op {
                        Operand::Value(bve) => match bve.get_type() {
                            BasicTypeEnum::IntType(t) => match t.get_bit_width() {
                                8 => Opcode::StoreB,
                                32 => Opcode::StoreW,
                                64 => Opcode::StoreD,
                                _ => {
                                    return Err(CompileError::NotImplemented(
                                        "unsupported store width",
                                    ))
                                }
                            },
                            _ => return Err(CompileError::NotImplemented("non-integer store")),
                        },
                        _ => {
                            return Err(CompileError::NotImplemented(
                                "store value must be an LLVM Value operand",
                            ))
                        }
                    };

                    let data_val =
                        lower_operand(val_op, &mut vreg_map, &mut next_vreg, &mut syllables);
                    let (base_val, offset) = lower_ptr_operand(ptr_op, &vreg_map, &gep_map)?;

                    let mut srcs = vec![base_val];
                    if offset != 0 {
                        srcs.push(Value::Imm(offset));
                    }
                    srcs.push(data_val);

                    syllables.push(Syllable {
                        opcode: store_opcode,
                        dst: None,
                        srcs,
                    });
                }

                InstructionOpcode::Load => {
                    let mut operands = instr.get_operands().flatten();
                    let ptr_op = operands.next().expect("load: missing pointer operand");

                    let load_opcode = match instr.get_type() {
                        inkwell::types::AnyTypeEnum::IntType(t) => match t.get_bit_width() {
                            8 => Opcode::LoadB,
                            32 => Opcode::LoadW,
                            64 => Opcode::LoadD,
                            _ => {
                                return Err(CompileError::NotImplemented("unsupported load width"))
                            }
                        },
                        _ => return Err(CompileError::NotImplemented("non-integer load")),
                    };

                    let (base_val, offset) = lower_ptr_operand(ptr_op, &vreg_map, &gep_map)?;

                    let dst_id = next_vreg;
                    next_vreg += 1;
                    vreg_map.insert(instr.as_value_ref() as usize, dst_id);

                    let mut srcs = vec![base_val];
                    if offset != 0 {
                        srcs.push(Value::Imm(offset));
                    }

                    syllables.push(Syllable {
                        opcode: load_opcode,
                        dst: Some(Reg::VReg(dst_id)),
                        srcs,
                    });
                }

                InstructionOpcode::GetElementPtr => {
                    let mut operands = instr.get_operands().flatten();
                    let base_op = operands.next().expect("gep: missing base operand");

                    let (base_val, base_offset) = lower_ptr_operand(base_op, &vreg_map, &gep_map)?;

                    let elem_bytes = unsafe {
                        let ty = LLVMGetGEPSourceElementType(instr.as_value_ref());
                        int_type_byte_size(ty)?
                    };

                    let mut total_offset = base_offset;
                    for idx_op in operands {
                        match idx_op {
                            Operand::Value(BasicValueEnum::IntValue(iv)) if iv.is_const() => {
                                let idx = iv.get_sign_extended_constant().unwrap_or(0);
                                total_offset += idx * elem_bytes;
                            }
                            _ => {
                                return Err(CompileError::NotImplemented("non-constant GEP index"))
                            }
                        }
                    }

                    gep_map.insert(instr.as_value_ref() as usize, (base_val, total_offset));
                }

                _ => {
                    return Err(CompileError::NotImplemented(
                        "unsupported instruction in ISel",
                    ));
                }
            }
        }

        mir_blocks.push(Block {
            label: current_mir_label,
            syllables,
            terminator,
        });

        // Enqueue successors.
        if let Some(fl) = false_succ {
            if visited.contains(&fl) {
                let latch_label = format!("{label}__latch__{fl}");
                edge_remap.insert((label.clone(), fl.clone()), latch_label.clone());
                mir_blocks.push(Block {
                    label: latch_label,
                    syllables: vec![],
                    terminator: Terminator::Jump(fl),
                });
            } else {
                worklist.push_front(fl);
            }
        }
        if let Some(tl) = true_succ {
            if !visited.contains(&tl) {
                worklist.push_back(tl);
            }
        }
    }

    // --- Phi lowering ---
    let mut copies_per_block: std::collections::HashMap<String, Vec<PhiCopy>> =
        std::collections::HashMap::new();

    for phi in &pending_phis {
        for (pred_label, raw_val) in &phi.incoming {
            let actual_pred = edge_remap
                .get(&(pred_label.clone(), phi.phi_block_label.clone()))
                .cloned()
                .unwrap_or_else(|| pred_label.clone());

            let src = match raw_val {
                RawPhiVal::Const(imm) => CopySrc::Const(*imm),
                RawPhiVal::LlvmRef(key) => {
                    let vreg = *vreg_map
                        .get(key)
                        .expect("phi incoming value not found in vreg_map after ISel");
                    CopySrc::VReg(vreg)
                }
            };

            copies_per_block
                .entry(actual_pred)
                .or_default()
                .push(PhiCopy { dst: phi.dst, src });
        }
    }

    crate::phi::lower_phi_copies(&mut mir_blocks, &copies_per_block, &mut next_vreg);

    Ok(Function {
        name,
        blocks: mir_blocks,
    })
}

/// Resolve one operand of a binary instruction to a MIR [`Value`].
fn lower_operand(
    operand: Operand,
    vreg_map: &mut HashMap<usize, u32>,
    next_vreg: &mut u32,
    syllables: &mut Vec<Syllable>,
) -> Value {
    match operand {
        Operand::Value(bve) => match bve {
            BasicValueEnum::IntValue(iv) if iv.is_const() => {
                let imm = iv.get_sign_extended_constant().unwrap_or(0);
                let dst_id = *next_vreg;
                *next_vreg += 1;
                syllables.push(Syllable {
                    opcode: Opcode::MovImm,
                    dst: Some(Reg::VReg(dst_id)),
                    srcs: vec![Value::Imm(imm)],
                });
                Value::Reg(Reg::VReg(dst_id))
            }
            BasicValueEnum::IntValue(iv) => {
                let k = iv.as_value_ref() as usize;
                Value::Reg(Reg::VReg(
                    *vreg_map
                        .get(&k)
                        .expect("VReg not found; value used before definition"),
                ))
            }
            other => panic!("non-integer operand in integer instruction: {:?}", other),
        },
        Operand::Block(_) => {
            panic!("unexpected basic-block operand in integer instruction")
        }
    }
}

/// Resolve a pointer operand to `(base: Value, byte_offset: i64)`.
fn lower_ptr_operand(
    operand: Operand,
    vreg_map: &HashMap<usize, u32>,
    gep_map: &HashMap<usize, (Value, i64)>,
) -> Result<(Value, i64), CompileError> {
    match operand {
        Operand::Value(BasicValueEnum::PointerValue(pv)) => {
            if pv.is_const() {
                unsafe {
                    let val_ref = pv.as_value_ref();
                    if !LLVMIsAConstantExpr(val_ref).is_null() {
                        let opcode = LLVMGetConstOpcode(val_ref);
                        if opcode == LLVMOpcode::LLVMIntToPtr {
                            let int_op = LLVMGetOperand(val_ref, 0);
                            let addr = LLVMConstIntGetSExtValue(int_op);
                            return Ok((Value::Imm(addr), 0));
                        }
                    }
                }
                Err(CompileError::NotImplemented(
                    "unsupported constant pointer expression",
                ))
            } else {
                let key = pv.as_value_ref() as usize;
                if let Some(&(base, offset)) = gep_map.get(&key) {
                    Ok((base, offset))
                } else if let Some(&vreg) = vreg_map.get(&key) {
                    Ok((Value::Reg(Reg::VReg(vreg)), 0))
                } else {
                    Err(CompileError::NotImplemented(
                        "pointer not found in vreg_map or gep_map",
                    ))
                }
            }
        }
        _ => Err(CompileError::NotImplemented(
            "non-pointer operand where pointer expected",
        )),
    }
}

/// Return the byte size of an integer LLVM type.
unsafe fn int_type_byte_size(ty: LLVMTypeRef) -> Result<i64, CompileError> {
    let kind = LLVMGetTypeKind(ty);
    if kind == LLVMTypeKind::LLVMIntegerTypeKind {
        Ok(LLVMGetIntTypeWidth(ty) as i64 / 8)
    } else {
        Err(CompileError::NotImplemented(
            "GEP with non-integer element type",
        ))
    }
}

/// Extract a stable identity key from a `BasicValueEnum` (LLVM value pointer).
fn bve_key(v: BasicValueEnum) -> usize {
    match v {
        BasicValueEnum::IntValue(iv) => iv.as_value_ref() as usize,
        BasicValueEnum::PointerValue(pv) => pv.as_value_ref() as usize,
        other => panic!("unsupported function parameter type: {:?}", other),
    }
}
