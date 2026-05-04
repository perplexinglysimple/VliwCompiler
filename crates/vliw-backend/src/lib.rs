//! VLIW backend: LLVM IR -> scheduled `.vliw` text.

pub mod isel;
pub mod liveness;
pub mod mir;
pub mod phi;
pub mod regalloc;
pub mod regs;

pub use inkwell::OptimizationLevel;
pub use mir::demo_mir;

use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::types::{AnyTypeEnum, BasicMetadataTypeEnum, BasicTypeEnum};
use inkwell::values::{InstructionOpcode, Operand};
use thiserror::Error;
use vliw_asm::opcode::Opcode;
use vliw_asm::{Bundle, Item, Operand as AsmOperand, Processor, Program, Syllable as AsmSyllable};

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
    #[error("LLVM parse error: {0}")]
    Parse(String),
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("optimization error: {0}")]
    OptError(String),
    #[error("out of physical registers while allocating v{vreg}")]
    OutOfRegisters { vreg: u32 },
}

/// Instruction scheduling strategy used during VLIW emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Schedule {
    /// Baseline scalar schedule: at most one non-nop syllable per bundle.
    #[default]
    Scalar,
    /// Post-scalar greedy bundle packer.
    Pack,
}

/// Render a MIR function as a plain-text dump.
pub fn emit_mir(func: &mir::Function) -> String {
    func.to_string()
}

/// Scheduler latency lookup for cycles from issue to result availability.
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyTable;

impl LatencyTable {
    pub fn result_ready_after(&self, opcode: Opcode) -> u32 {
        opcode.latency()
    }
}

/// Walk `module` and reject features not yet supported by the backend.
///
/// Checks performed:
/// - `phi` instructions are accepted and lowered after ISel.
/// - Direct `call` instructions are accepted; indirect calls and calls to
///   external declarations are rejected by ISel.
/// - No `invoke` / `callbr` instructions.
/// - No atomic instructions (`atomicrmw`, `cmpxchg`, `fence`).
/// - Integer types: only i1, i32, and i64 are accepted anywhere in the
///   function signature or instruction stream.
fn check_module(module: &Module) -> Result<(), CompileError> {
    for func in module.get_functions() {
        // Ignore external declarations (no body).
        if func.count_basic_blocks() == 0 {
            continue;
        }

        // Check function signature types.
        let fn_type = func.get_type();
        for param_ty in fn_type.get_param_types() {
            if let BasicMetadataTypeEnum::IntType(t) = param_ty {
                check_int_width(t.get_bit_width())?;
            }
        }
        if let Some(BasicTypeEnum::IntType(t)) = fn_type.get_return_type() {
            check_int_width(t.get_bit_width())?;
        }

        // Check every instruction in the function body.
        for bb in func.get_basic_blocks() {
            for instr in bb.get_instructions() {
                check_instruction(instr)?;
            }
        }
    }

    Ok(())
}

fn check_int_width(bits: u32) -> Result<(), CompileError> {
    match bits {
        1 | 32 | 64 => Ok(()),
        other => Err(CompileError::UnsupportedFeature(format!(
            "integer width i{other}"
        ))),
    }
}

fn check_instruction(instr: inkwell::values::InstructionValue) -> Result<(), CompileError> {
    match instr.get_opcode() {
        // Direct calls are supported; ISel validates target is a defined function.
        InstructionOpcode::Call => {}
        InstructionOpcode::CallBr | InstructionOpcode::Invoke => {
            return Err(CompileError::UnsupportedFeature(
                "callbr/invoke not supported".into(),
            ));
        }
        InstructionOpcode::Phi => {
            // Phi nodes are now supported — lowered by isel::lower_module.
        }
        InstructionOpcode::AtomicRMW
        | InstructionOpcode::AtomicCmpXchg
        | InstructionOpcode::Fence => {
            return Err(CompileError::UnsupportedFeature(
                "atomic instruction".into(),
            ));
        }
        _ => {}
    }

    // Check the instruction's result type.
    if let AnyTypeEnum::IntType(t) = instr.get_type() {
        check_int_width(t.get_bit_width())?;
    }

    // Check operand types (catches e.g. `store i8 42, ptr %p`).
    for operand in instr.get_operands().flatten() {
        if let Operand::Value(v) = operand {
            if let BasicTypeEnum::IntType(t) = v.get_type() {
                check_int_width(t.get_bit_width())?;
            }
        }
    }

    Ok(())
}

/// Run the LLVM new-pass-manager pipeline at the requested optimisation level.
///
/// `OptimizationLevel::None` is a no-op (no target machine is constructed).
/// `Less` → `default<O1>`, `Default` → `default<O2>`, `Aggressive` → `default<O3>`.
fn run_opt_pipeline(module: &Module, level: OptimizationLevel) -> Result<(), CompileError> {
    let passes = match level {
        OptimizationLevel::None => return Ok(()),
        OptimizationLevel::Less => "default<O1>",
        OptimizationLevel::Default => "default<O2>",
        OptimizationLevel::Aggressive => "default<O3>",
    };

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| CompileError::OptError(e))?;

    let triple = TargetMachine::get_default_triple();
    let target =
        Target::from_triple(&triple).map_err(|e| CompileError::OptError(format!("{e}")))?;
    let machine = target
        .create_target_machine(
            &triple,
            TargetMachine::get_host_cpu_name().to_string().as_str(),
            TargetMachine::get_host_cpu_features().to_string().as_str(),
            level,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| CompileError::OptError("failed to create target machine".into()))?;

    module
        .run_passes(passes, &machine, PassBuilderOptions::create())
        .map_err(|e| CompileError::OptError(format!("{e}")))
}

/// Parse LLVM IR text, run the optimisation pipeline, and return the
/// optimised module's textual IR. Useful for inspection and testing.
pub fn parse_and_optimize(ir_text: &str, opt: OptimizationLevel) -> Result<String, CompileError> {
    let context = Context::create();
    let mut bytes = ir_text.as_bytes().to_vec();
    bytes.push(0);
    let buffer = MemoryBuffer::create_from_memory_range(&bytes, "input");
    let module = context
        .create_module_from_ir(buffer)
        .map_err(|e| CompileError::Parse(e.to_string()))?;

    run_opt_pipeline(&module, opt)?;

    Ok(format!(
        "{}",
        module.print_to_string().to_str().unwrap_or("")
    ))
}

/// Parse LLVM IR text and lower it to a MIR [`mir::Function`].
///
/// Runs subset validation and the optional optimisation pipeline before
/// instruction selection.  Only the integer ALU opcodes (`add`, `sub`, `mul`,
/// `and`, `or`, `xor`, `shl`, `lshr`, `ashr`) and `ret` are currently supported;
/// other instructions return [`CompileError::NotImplemented`].
pub fn compile_to_mir(
    ir_text: &str,
    opt: OptimizationLevel,
) -> Result<mir::Function, CompileError> {
    let context = Context::create();
    let mut bytes = ir_text.as_bytes().to_vec();
    bytes.push(0);
    let buffer = MemoryBuffer::create_from_memory_range(&bytes, "input");
    let module = context
        .create_module_from_ir(buffer)
        .map_err(|e| CompileError::Parse(e.to_string()))?;

    check_module(&module)?;
    run_opt_pipeline(&module, opt)?;

    isel::lower_module(&module)
}

/// Compile LLVM IR text to a `.vliw` program using the canonical 4-slot layout.
///
/// Parses `ir_text` via inkwell, rejects unsupported features, runs the opt
/// pipeline at `opt`, then hands off to the VLIW codegen pipeline.
/// Multiple function definitions are supported; all are compiled into a single
/// `.vliw` output.  Direct calls to functions defined in the same module are
/// lowered according to the VLIW calling convention (args in r2–r9, return
/// value in r1, link register r31 saved/restored around each call site).
/// Returns [`CompileError::Parse`] with the LLVM diagnostic when the input is
/// not valid LLVM IR.  Returns [`CompileError::UnsupportedFeature`] when the
/// IR uses features outside the currently supported subset.
pub fn compile(
    ir_text: &str,
    opt: OptimizationLevel,
    schedule: Schedule,
) -> Result<String, CompileError> {
    compile_for_processor(ir_text, opt, schedule, Processor::default())
}

/// Compile LLVM IR text to a `.vliw` program targeting a specific processor layout.
///
/// Identical to [`compile`] but uses the provided `processor` configuration
/// instead of the canonical 4-slot default.  Scheduling and bundle packing
/// exploit the slot capabilities declared in the processor (e.g. a wider
/// 8-slot layout allows more syllables per bundle).
pub fn compile_for_processor(
    ir_text: &str,
    opt: OptimizationLevel,
    schedule: Schedule,
    processor: Processor,
) -> Result<String, CompileError> {
    let context = Context::create();
    // create_from_memory_range requires a nul-terminated slice.
    let mut bytes = ir_text.as_bytes().to_vec();
    bytes.push(0);
    let buffer = MemoryBuffer::create_from_memory_range(&bytes, "input");
    let module = context
        .create_module_from_ir(buffer)
        .map_err(|e| CompileError::Parse(e.to_string()))?;

    check_module(&module)?;
    run_opt_pipeline(&module, opt)?;

    let functions = isel::lower_all_functions(&module)?;
    let allocated_fns: Vec<_> = functions
        .iter()
        .map(|f| regalloc::allocate_registers(f))
        .collect::<Result<_, _>>()?;

    match schedule {
        Schedule::Scalar => emit_scalar_vliw_module(&allocated_fns, processor),
        Schedule::Pack => emit_packed_vliw_module(&allocated_fns, processor),
    }
}

#[cfg(test)]
fn emit_packed_vliw(func: &mir::Function) -> Result<String, CompileError> {
    emit_packed_vliw_module(std::slice::from_ref(func), Processor::default())
}

fn emit_scalar_vliw_module(fns: &[mir::Function], processor: Processor) -> Result<String, CompileError> {
    let mut program = Program {
        processor,
        items: Vec::new(),
    };

    for func in fns {
        // Reset latency tracking at each function boundary.
        let mut ctx = CodegenCtx::default();
        for (idx, block) in func.blocks.iter().enumerate() {
            // Emit the function name as a call-target label before the entry block.
            if idx == 0 {
                program.items.push(Item::Label(func.name.clone()));
            }
            program.items.push(Item::Label(block.label.clone()));
            for syl in &block.syllables {
                ctx.emit_syllable(&mut program, syl)?;
            }
            ctx.emit_terminator(&mut program, &block.terminator)?;
        }
    }

    vliw_asm::verify_program(&program)
        .map_err(|_| CompileError::NotImplemented("generated VLIW failed verifier"))?;
    vliw_asm::emit(&program).map_err(|_| CompileError::NotImplemented("VLIW text emission"))
}

fn emit_packed_vliw_module(fns: &[mir::Function], processor: Processor) -> Result<String, CompileError> {
    let mut program = Program {
        processor,
        items: Vec::new(),
    };

    for func in fns {
        let scheduled = list_schedule_function(func);
        // Reset latency tracking at each function boundary.
        let mut ctx = CodegenCtx::default();
        for (idx, block) in scheduled.blocks.iter().enumerate() {
            // Emit the function name as a call-target label before the entry block.
            if idx == 0 {
                program.items.push(Item::Label(func.name.clone()));
            }
            program.items.push(Item::Label(block.label.clone()));
            for syl in &block.syllables {
                ctx.emit_syllable(&mut program, syl)?;
            }
            ctx.emit_terminator(&mut program, &block.terminator)?;
        }
    }

    pack_scalar_program(&mut program);
    vliw_asm::verify_program(&program)
        .map_err(|_| CompileError::NotImplemented("generated packed VLIW failed verifier"))?;
    vliw_asm::emit(&program).map_err(|_| CompileError::NotImplemented("VLIW text emission"))
}

fn list_schedule_function(func: &mir::Function) -> mir::Function {
    mir::Function {
        name: func.name.clone(),
        blocks: func.blocks.iter().map(list_schedule_block).collect(),
    }
}

fn list_schedule_block(block: &mir::Block) -> mir::Block {
    mir::Block {
        label: block.label.clone(),
        syllables: list_schedule_syllables(&block.syllables),
        terminator: block.terminator.clone(),
    }
}

#[derive(Debug, Clone)]
struct SchedNode {
    succs: Vec<usize>,
    unscheduled_preds: usize,
    height: u32,
}

fn list_schedule_syllables(syllables: &[mir::Syllable]) -> Vec<mir::Syllable> {
    if syllables.len() <= 1 {
        return syllables.to_vec();
    }

    let mut nodes = build_sched_dag(syllables);
    compute_heights(&mut nodes, syllables);

    let mut ready: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(idx, node)| (node.unscheduled_preds == 0).then_some(idx))
        .collect();
    let mut scheduled = vec![false; syllables.len()];
    let mut issue_ready = ReadyState::default();
    let mut cycle = 0u64;
    let mut out = Vec::with_capacity(syllables.len());

    while out.len() < syllables.len() {
        let selected_pos = pick_ready_node(syllables, &nodes, &ready, &issue_ready, cycle)
            .unwrap_or_else(|| {
                ready
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, idx)| {
                        (
                            syllable_required_cycle(&syllables[**idx], &issue_ready),
                            std::cmp::Reverse(nodes[**idx].height),
                            **idx,
                        )
                    })
                    .map(|(pos, _)| pos)
                    .expect("DAG scheduler must have a ready node")
            });
        let idx = ready.swap_remove(selected_pos);
        let syl = syllables[idx].clone();
        let required = syllable_required_cycle(&syl, &issue_ready);
        cycle = cycle.max(required);
        update_ready_after_mir_syllable(&syl, cycle, &mut issue_ready);
        cycle += 1;
        scheduled[idx] = true;
        out.push(syl);

        for succ in nodes[idx].succs.clone() {
            nodes[succ].unscheduled_preds -= 1;
            if nodes[succ].unscheduled_preds == 0 && !scheduled[succ] {
                ready.push(succ);
            }
        }
    }

    out
}

fn pick_ready_node(
    syllables: &[mir::Syllable],
    nodes: &[SchedNode],
    ready: &[usize],
    issue_ready: &ReadyState,
    cycle: u64,
) -> Option<usize> {
    ready
        .iter()
        .enumerate()
        .filter(|(_, idx)| syllable_required_cycle(&syllables[**idx], issue_ready) <= cycle)
        .max_by_key(|(_, idx)| (nodes[**idx].height, std::cmp::Reverse(**idx)))
        .map(|(pos, _)| pos)
}

fn build_sched_dag(syllables: &[mir::Syllable]) -> Vec<SchedNode> {
    let mut succ_sets: Vec<std::collections::BTreeSet<usize>> =
        vec![std::collections::BTreeSet::new(); syllables.len()];
    let mut last_writer: std::collections::HashMap<RegKey, usize> =
        std::collections::HashMap::new();
    let mut last_readers: std::collections::HashMap<RegKey, Vec<usize>> =
        std::collections::HashMap::new();
    let mut last_unknown_memory_barrier: Option<usize> = None;
    let mut prior_memory: Vec<(usize, MemoryBucket)> = Vec::new();

    for (idx, syl) in syllables.iter().enumerate() {
        if let Some(barrier) = last_unknown_memory_barrier {
            succ_sets[barrier].insert(idx);
        }

        for src in source_reg_keys(syl) {
            if let Some(writer) = last_writer.get(&src) {
                succ_sets[*writer].insert(idx);
            }
            last_readers.entry(src).or_default().push(idx);
        }

        if let Some(dst) = dest_reg_key(syl) {
            if let Some(writer) = last_writer.insert(dst, idx) {
                succ_sets[writer].insert(idx);
            }
            if let Some(readers) = last_readers.remove(&dst) {
                for reader in readers {
                    if reader != idx {
                        succ_sets[reader].insert(idx);
                    }
                }
            }
        }

        if is_memory_opcode(syl.opcode) {
            let bucket = memory_bucket(syl);
            if bucket == MemoryBucket::Unknown {
                for prior in 0..idx {
                    succ_sets[prior].insert(idx);
                }
                last_unknown_memory_barrier = Some(idx);
            } else {
                for (prior, prior_bucket) in &prior_memory {
                    if memory_buckets_may_alias(*prior_bucket, bucket) {
                        succ_sets[*prior].insert(idx);
                    }
                }
            }
            prior_memory.push((idx, bucket));
        }
    }

    let mut pred_counts = vec![0usize; syllables.len()];
    for succs in &succ_sets {
        for succ in succs {
            pred_counts[*succ] += 1;
        }
    }

    succ_sets
        .into_iter()
        .enumerate()
        .map(|(idx, succs)| SchedNode {
            succs: succs.into_iter().collect(),
            unscheduled_preds: pred_counts[idx],
            height: 0,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryBucket {
    Stack,
    Global,
    Unknown,
}

fn memory_bucket(syl: &mir::Syllable) -> MemoryBucket {
    let Some(base) = memory_base_value(syl) else {
        return MemoryBucket::Unknown;
    };
    match base {
        mir::Value::Stack(_) => MemoryBucket::Stack,
        mir::Value::Imm(_) => MemoryBucket::Global,
        mir::Value::Reg(_) => MemoryBucket::Unknown,
    }
}

fn memory_base_value(syl: &mir::Syllable) -> Option<mir::Value> {
    match syl.opcode {
        Opcode::LoadB | Opcode::LoadH | Opcode::LoadW | Opcode::LoadD => syl.srcs.first().copied(),
        Opcode::StoreB | Opcode::StoreH | Opcode::StoreW | Opcode::StoreD => {
            syl.srcs.first().copied()
        }
        _ => None,
    }
}

fn memory_buckets_may_alias(lhs: MemoryBucket, rhs: MemoryBucket) -> bool {
    lhs == MemoryBucket::Unknown || rhs == MemoryBucket::Unknown || lhs == rhs
}

fn compute_heights(nodes: &mut [SchedNode], syllables: &[mir::Syllable]) {
    for idx in (0..nodes.len()).rev() {
        let succ_height = nodes[idx]
            .succs
            .iter()
            .map(|succ| nodes[*succ].height)
            .max()
            .unwrap_or(0);
        nodes[idx].height = syllables[idx].opcode.latency() + succ_height;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RegClass {
    Gpr,
    Pred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RegKey {
    class: RegClass,
    reg: mir::Reg,
}

fn dest_reg_key(syl: &mir::Syllable) -> Option<RegKey> {
    let class = if syl.opcode.writes_gpr() {
        RegClass::Gpr
    } else if syl.opcode.writes_pred() {
        RegClass::Pred
    } else {
        return None;
    };
    Some(RegKey {
        class,
        reg: syl.dst?,
    })
}

fn source_reg_keys(syl: &mir::Syllable) -> Vec<RegKey> {
    let class = if predicate_sources(syl.opcode) {
        RegClass::Pred
    } else {
        RegClass::Gpr
    };
    syl.srcs
        .iter()
        .filter_map(|src| match src {
            mir::Value::Reg(reg) => Some(RegKey { class, reg: *reg }),
            mir::Value::Imm(_) | mir::Value::Stack(_) => None,
        })
        .collect()
}

fn predicate_sources(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::PNot | Opcode::PAnd | Opcode::POr | Opcode::PXor
    )
}

fn syllable_required_cycle(syl: &mir::Syllable, ready: &ReadyState) -> u64 {
    source_reg_keys(syl)
        .iter()
        .map(|key| match key.class {
            RegClass::Gpr => physical_reg(key.reg)
                .and_then(|reg| ready.gpr.get(&reg).copied())
                .unwrap_or(0),
            RegClass::Pred => physical_reg(key.reg)
                .and_then(|reg| ready.pred.get(&reg).copied())
                .unwrap_or(0),
        })
        .max()
        .unwrap_or(0)
}

fn update_ready_after_mir_syllable(syl: &mir::Syllable, cycle: u64, ready: &mut ReadyState) {
    let Some(dst) = dest_reg_key(syl) else {
        return;
    };
    let Some(reg) = physical_reg(dst.reg) else {
        return;
    };
    let result_ready = cycle + u64::from(syl.opcode.latency());

    match dst.class {
        RegClass::Gpr if reg != regs::ZERO_REG => {
            ready.gpr.insert(reg, result_ready);
        }
        RegClass::Pred => {
            ready.pred.insert(reg, result_ready);
        }
        RegClass::Gpr => {}
    }
}

fn physical_reg(reg: mir::Reg) -> Option<u8> {
    match reg {
        mir::Reg::PReg(reg) => Some(reg),
        mir::Reg::VReg(_) => None,
    }
}

#[derive(Default)]
struct CodegenCtx {
    ready_gpr: std::collections::HashMap<u8, u64>,
    ready_pred: std::collections::HashMap<u8, u64>,
    latency_table: LatencyTable,
    cycle: u64,
}

impl CodegenCtx {
    fn emit_syllable(
        &mut self,
        program: &mut Program,
        syl: &mir::Syllable,
    ) -> Result<(), CompileError> {
        let asm = self.lower_syllable(syl)?;
        self.emit_asm_syllable(program, syl.opcode, asm)?;
        Ok(())
    }

    fn emit_terminator(
        &mut self,
        program: &mut Program,
        term: &mir::Terminator,
    ) -> Result<(), CompileError> {
        match term {
            mir::Terminator::Return => {
                self.emit_control_syllable(program, Opcode::Ret, AsmSyllable::new("ret", []))?;
            }
            mir::Terminator::Jump(label) => {
                self.emit_control_syllable(
                    program,
                    Opcode::Jump,
                    AsmSyllable::new("jmp", [AsmOperand::Label(label.clone())]),
                )?;
            }
            mir::Terminator::Branch { cond, label } => {
                let pred = self.pred_for_reg(*cond);
                self.emit_control_syllable(
                    program,
                    Opcode::Branch,
                    AsmSyllable::new(
                        "br",
                        [AsmOperand::Pred(pred), AsmOperand::Label(label.clone())],
                    ),
                )?;
            }
            mir::Terminator::DirectCall { target, cont: _ } => {
                // `call target` saves PC+1 into r31 (LINK_REG) and jumps to
                // target.  The continuation block is emitted immediately after
                // this in program order, so the hardware return address lands
                // at the first bundle of the continuation block.
                self.emit_control_syllable(
                    program,
                    Opcode::Call,
                    AsmSyllable::new("call", [AsmOperand::Label(target.clone())]),
                )?;
            }
        }
        Ok(())
    }

    fn lower_syllable(&mut self, syl: &mir::Syllable) -> Result<AsmSyllable, CompileError> {
        let opcode = syl.opcode;
        let mnemonic = opcode.mnemonic();

        if matches!(
            opcode,
            Opcode::StoreB | Opcode::StoreH | Opcode::StoreW | Opcode::StoreD
        ) {
            let (addr, data) = self.store_operands(&syl.srcs)?;
            return Ok(AsmSyllable::new(mnemonic, [addr, data]));
        }

        if matches!(
            opcode,
            Opcode::LoadB | Opcode::LoadH | Opcode::LoadW | Opcode::LoadD
        ) {
            let dst = self.gpr_for_reg(syl.dst.expect("load dst"));
            let addr = self.load_address(&syl.srcs)?;
            return Ok(AsmSyllable::new(mnemonic, [AsmOperand::Reg(dst), addr]));
        }

        if opcode.writes_pred() {
            let dst = self.pred_for_reg(syl.dst.expect("predicate dst"));
            let mut ops = vec![AsmOperand::Pred(dst)];
            for src in &syl.srcs {
                ops.push(self.pred_or_gpr_operand(
                    *src,
                    matches!(
                        opcode,
                        Opcode::PNot | Opcode::PAnd | Opcode::POr | Opcode::PXor
                    ),
                )?);
            }
            return Ok(AsmSyllable::new(mnemonic, ops));
        }

        let mut ops = Vec::new();
        if let Some(dst) = syl.dst {
            ops.push(AsmOperand::Reg(self.gpr_for_reg(dst)));
        }
        for src in &syl.srcs {
            ops.push(self.gpr_or_imm_operand(*src)?);
        }
        Ok(AsmSyllable::new(mnemonic, ops))
    }

    fn emit_asm_syllable(
        &mut self,
        program: &mut Program,
        opcode: Opcode,
        syl: AsmSyllable,
    ) -> Result<(), CompileError> {
        let slot = scalar_slot(opcode, &program.processor)?;
        self.emit_asm_syllable_in_slot(program, opcode, syl, slot)
    }

    fn emit_control_syllable(
        &mut self,
        program: &mut Program,
        opcode: Opcode,
        syl: AsmSyllable,
    ) -> Result<(), CompileError> {
        let slot = x_slot(&program.processor)?;
        self.emit_asm_syllable_in_slot(program, opcode, syl, slot)
    }

    fn emit_asm_syllable_in_slot(
        &mut self,
        program: &mut Program,
        opcode: Opcode,
        syl: AsmSyllable,
        slot: usize,
    ) -> Result<(), CompileError> {
        if !slot_supports_opcode(slot, opcode, &program.processor) {
            return Err(CompileError::NotImplemented(
                "requested slot is not compatible with opcode",
            ));
        }

        let required_cycle = self.required_cycle_for_sources(opcode, &syl);
        self.wait_until(required_cycle, program);

        let dst = syl.operands.first().cloned();
        program.items.push(Item::Bundle(bundle_with(
            program.processor.width as usize,
            slot,
            syl,
        )));
        let ready = self.cycle + u64::from(self.latency_table.result_ready_after(opcode));
        if opcode.writes_gpr() {
            if let Some(AsmOperand::Reg(reg)) = dst {
                if reg != regs::ZERO_REG {
                    self.ready_gpr.insert(reg, ready);
                }
            }
        }
        if opcode.writes_pred() {
            if let Some(AsmOperand::Pred(pred)) = dst {
                self.ready_pred.insert(pred, ready);
            }
        }
        self.cycle += 1;
        Ok(())
    }

    fn required_cycle_for_sources(&self, opcode: Opcode, syl: &AsmSyllable) -> u64 {
        let mut skip_gpr_dest = opcode.writes_gpr();
        let mut skip_pred_dest = opcode.writes_pred();

        syl.operands
            .iter()
            .map(|op| match op {
                AsmOperand::Reg(r) if skip_gpr_dest => {
                    skip_gpr_dest = false;
                    0
                }
                AsmOperand::Pred(p) if skip_pred_dest => {
                    skip_pred_dest = false;
                    0
                }
                AsmOperand::Reg(r) => self.ready_gpr.get(r).copied().unwrap_or(0),
                AsmOperand::Pred(p) => self.ready_pred.get(p).copied().unwrap_or(0),
                AsmOperand::MemAddr { base, .. } => self.ready_gpr.get(base).copied().unwrap_or(0),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    fn wait_until(&mut self, cycle: u64, program: &mut Program) {
        while self.cycle < cycle {
            program.items.push(Item::Bundle(Bundle::default()));
            self.cycle += 1;
        }
    }

    fn gpr_or_imm_operand(&self, value: mir::Value) -> Result<AsmOperand, CompileError> {
        match value {
            mir::Value::Imm(imm) => Ok(AsmOperand::Imm(imm)),
            mir::Value::Reg(reg) => Ok(AsmOperand::Reg(self.gpr_for_reg(reg))),
            mir::Value::Stack(_) => Err(CompileError::NotImplemented(
                "stack address used where scalar operand expected",
            )),
        }
    }

    fn pred_or_gpr_operand(
        &self,
        value: mir::Value,
        pred_src: bool,
    ) -> Result<AsmOperand, CompileError> {
        match value {
            mir::Value::Imm(imm) => Ok(AsmOperand::Imm(imm)),
            mir::Value::Reg(reg) if pred_src => Ok(AsmOperand::Pred(self.pred_for_reg(reg))),
            mir::Value::Reg(reg) => Ok(AsmOperand::Reg(self.gpr_for_reg(reg))),
            mir::Value::Stack(_) => Err(CompileError::NotImplemented(
                "stack address used where scalar operand expected",
            )),
        }
    }

    fn store_operands(
        &self,
        srcs: &[mir::Value],
    ) -> Result<(AsmOperand, AsmOperand), CompileError> {
        let (base, offset, data) = match srcs {
            [base, data] => (*base, 0, *data),
            [base, mir::Value::Imm(offset), data] => (*base, *offset, *data),
            _ => {
                return Err(CompileError::NotImplemented(
                    "unsupported store operand shape",
                ))
            }
        };
        let addr = self.address_operand(base, offset)?;
        let data = self.gpr_or_imm_operand(data)?;
        Ok((addr, data))
    }

    fn load_address(&self, srcs: &[mir::Value]) -> Result<AsmOperand, CompileError> {
        match srcs {
            [base] => self.address_operand(*base, 0),
            [base, mir::Value::Imm(offset)] => self.address_operand(*base, *offset),
            _ => Err(CompileError::NotImplemented(
                "unsupported load operand shape",
            )),
        }
    }

    fn address_operand(&self, base: mir::Value, offset: i64) -> Result<AsmOperand, CompileError> {
        match base {
            mir::Value::Imm(addr) | mir::Value::Stack(addr) => Ok(AsmOperand::MemAddr {
                base: regs::ZERO_REG,
                offset: addr + offset,
            }),
            mir::Value::Reg(reg) => Ok(AsmOperand::MemAddr {
                base: self.gpr_for_reg(reg),
                offset,
            }),
        }
    }

    fn gpr_for_reg(&self, reg: mir::Reg) -> u8 {
        match reg {
            mir::Reg::PReg(n) => n,
            mir::Reg::VReg(_) => panic!("codegen received unallocated virtual GPR"),
        }
    }

    fn pred_for_reg(&self, reg: mir::Reg) -> u8 {
        match reg {
            mir::Reg::PReg(n) => n,
            mir::Reg::VReg(_) => panic!("codegen received unallocated virtual predicate"),
        }
    }
}

#[derive(Clone, Default)]
struct ReadyState {
    gpr: std::collections::HashMap<u8, u64>,
    pred: std::collections::HashMap<u8, u64>,
}

fn pack_scalar_program(program: &mut Program) {
    let mut packed = Vec::with_capacity(program.items.len());
    let mut segment = Vec::new();
    let mut ready = ReadyState::default();
    let mut cycle = 0u64;

    for item in std::mem::take(&mut program.items) {
        match item {
            Item::Label(label) => {
                let bundles = pack_bundle_segment(
                    &program.processor,
                    std::mem::take(&mut segment),
                    cycle,
                    &ready,
                );
                update_ready_after_packed_bundles(&bundles, cycle, &mut ready);
                cycle += bundles.len() as u64;
                packed.extend(bundles.into_iter().map(Item::Bundle));
                packed.push(Item::Label(label));
            }
            Item::Bundle(bundle) if bundle_is_terminator(&bundle) => {
                let bundles = pack_bundle_segment(
                    &program.processor,
                    std::mem::take(&mut segment),
                    cycle,
                    &ready,
                );
                update_ready_after_packed_bundles(&bundles, cycle, &mut ready);
                cycle += bundles.len() as u64;
                packed.extend(bundles.into_iter().map(Item::Bundle));
                update_ready_after_bundle(&bundle, cycle, &mut ready);
                cycle += 1;
                packed.push(Item::Bundle(bundle));
            }
            Item::Bundle(bundle) => segment.push(bundle),
        }
    }

    let bundles = pack_bundle_segment(&program.processor, segment, cycle, &ready);
    packed.extend(bundles.into_iter().map(Item::Bundle));
    program.items = packed;
}

fn pack_bundle_segment(
    processor: &Processor,
    segment: Vec<Bundle>,
    start_cycle: u64,
    initial_ready: &ReadyState,
) -> Vec<Bundle> {
    let mut out: Vec<Bundle> = Vec::new();

    for bundle in segment {
        if bundle_is_empty(&bundle) {
            out.push(bundle);
            continue;
        }

        if try_merge_into_previous(processor, &mut out, &bundle, start_cycle, initial_ready) {
            continue;
        }

        while !bundle_sources_ready_at(
            &bundle,
            start_cycle + out.len() as u64,
            &ready_after_bundles(initial_ready, start_cycle, &out),
        ) {
            out.push(Bundle::default());
        }
        out.push(bundle);
    }

    out
}

fn try_merge_into_previous(
    processor: &Processor,
    out: &mut [Bundle],
    bundle: &Bundle,
    start_cycle: u64,
    initial_ready: &ReadyState,
) -> bool {
    let Some(last_idx) = out.len().checked_sub(1) else {
        return false;
    };
    if bundle_is_empty(&out[last_idx]) || bundle_is_terminator(&out[last_idx]) {
        return false;
    }

    if has_pending_result_at(
        &ready_after_bundles(initial_ready, start_cycle, out),
        start_cycle + out.len() as u64,
    ) {
        return false;
    }

    let ready_before_last = ready_after_bundles(initial_ready, start_cycle, &out[..last_idx]);
    if !bundle_sources_ready_at(bundle, start_cycle + last_idx as u64, &ready_before_last) {
        return false;
    }

    let Some(candidate) = merge_bundles(processor, &out[last_idx], bundle) else {
        return false;
    };
    if vliw_asm::verify_bundle(&candidate, processor).is_err() {
        return false;
    }

    out[last_idx] = candidate;
    true
}

fn has_pending_result_at(ready: &ReadyState, cycle: u64) -> bool {
    ready
        .gpr
        .values()
        .chain(ready.pred.values())
        .any(|ready_cycle| *ready_cycle > cycle)
}

fn merge_bundles(processor: &Processor, a: &Bundle, b: &Bundle) -> Option<Bundle> {
    let mut merged = Bundle {
        slots: normalize_slots(&a.slots, processor.width as usize),
    };

    for syl in b.slots.iter().flatten() {
        let opcode = Opcode::from_mnemonic(&syl.opcode)?;
        let slot = first_open_compatible_slot(&merged, opcode, processor)?;
        merged.slots[slot] = Some(syl.clone());
    }

    Some(merged)
}

fn first_open_compatible_slot(
    bundle: &Bundle,
    opcode: Opcode,
    processor: &Processor,
) -> Option<usize> {
    (0..processor.width as usize).find(|&slot| {
        bundle.slots.get(slot).is_some_and(Option::is_none)
            && slot_supports_opcode(slot, opcode, processor)
    })
}

fn normalize_slots(slots: &[Option<AsmSyllable>], width: usize) -> Vec<Option<AsmSyllable>> {
    let mut out = slots.to_vec();
    out.resize_with(width, || None);
    out
}

fn bundle_is_empty(bundle: &Bundle) -> bool {
    bundle.slots.iter().all(Option::is_none)
}

fn bundle_is_terminator(bundle: &Bundle) -> bool {
    bundle.slots.iter().flatten().any(|syl| {
        matches!(
            Opcode::from_mnemonic(&syl.opcode),
            Some(Opcode::Ret | Opcode::Jump | Opcode::Branch | Opcode::Call)
        )
    })
}

fn ready_after_bundles(initial: &ReadyState, start_cycle: u64, bundles: &[Bundle]) -> ReadyState {
    let mut ready = initial.clone();
    update_ready_after_packed_bundles(bundles, start_cycle, &mut ready);
    ready
}

fn update_ready_after_packed_bundles(bundles: &[Bundle], start_cycle: u64, ready: &mut ReadyState) {
    for (cycle, bundle) in bundles.iter().enumerate() {
        update_ready_after_bundle(bundle, start_cycle + cycle as u64, ready);
    }
}

fn bundle_sources_ready_at(bundle: &Bundle, cycle: u64, ready: &ReadyState) -> bool {
    bundle.slots.iter().flatten().all(|syl| {
        Opcode::from_mnemonic(&syl.opcode)
            .map(|opcode| sources_ready_at(opcode, syl, cycle, ready))
            .unwrap_or(false)
    })
}

fn sources_ready_at(opcode: Opcode, syl: &AsmSyllable, cycle: u64, ready: &ReadyState) -> bool {
    let mut skip_gpr_dest = opcode.writes_gpr();
    let mut skip_pred_dest = opcode.writes_pred();

    syl.operands.iter().all(|operand| match operand {
        AsmOperand::Reg(_) if skip_gpr_dest => {
            skip_gpr_dest = false;
            true
        }
        AsmOperand::Pred(_) if skip_pred_dest => {
            skip_pred_dest = false;
            true
        }
        AsmOperand::Reg(reg) => ready.gpr.get(reg).copied().unwrap_or(0) <= cycle,
        AsmOperand::Pred(pred) => ready.pred.get(pred).copied().unwrap_or(0) <= cycle,
        AsmOperand::MemAddr { base, .. } => ready.gpr.get(base).copied().unwrap_or(0) <= cycle,
        _ => true,
    })
}

fn update_ready_after_bundle(bundle: &Bundle, cycle: u64, ready: &mut ReadyState) {
    for syl in bundle.slots.iter().flatten() {
        let Some(opcode) = Opcode::from_mnemonic(&syl.opcode) else {
            continue;
        };
        let result_ready = cycle + u64::from(opcode.latency());

        if opcode.writes_gpr() {
            if let Some(AsmOperand::Reg(reg)) = syl.operands.first() {
                if *reg != regs::ZERO_REG {
                    ready.gpr.insert(*reg, result_ready);
                }
            }
        }
        if opcode.writes_pred() {
            if let Some(AsmOperand::Pred(pred)) = syl.operands.first() {
                ready.pred.insert(*pred, result_ready);
            }
        }
    }
}

fn bundle_with(width: usize, slot: usize, syl: AsmSyllable) -> Bundle {
    let mut slots = vec![None; width];
    slots[slot] = Some(syl);
    Bundle { slots }
}

fn first_compatible_slot(opcode: Opcode, processor: &Processor) -> Result<usize, CompileError> {
    if opcode == Opcode::Nop {
        return Ok(0);
    }

    let unit_kind_by_name: std::collections::HashMap<&str, vliw_asm::UnitKind> = processor
        .units
        .iter()
        .filter_map(|unit| {
            vliw_asm::UnitKind::from_kind_str(&unit.kind).map(|kind| (unit.name.as_str(), kind))
        })
        .collect();

    for slot in 0..processor.width as usize {
        if slot_supports_opcode_with_units(slot, opcode, processor, &unit_kind_by_name) {
            return Ok(slot);
        }
    }

    Err(CompileError::NotImplemented(
        "no compatible slot for opcode in processor layout",
    ))
}

fn scalar_slot(opcode: Opcode, processor: &Processor) -> Result<usize, CompileError> {
    let canonical_alias = if is_memory_opcode(opcode) {
        Some("M")
    } else if is_control_or_multiplier_opcode(opcode) {
        Some("X")
    } else if is_integer_alu_opcode(opcode) {
        Some("I0")
    } else {
        None
    };

    if let Some(alias) = canonical_alias {
        if let Some(slot) = slot_by_alias(processor, alias) {
            if slot_supports_opcode(slot, opcode, processor) {
                return Ok(slot);
            }
        }
    }

    first_compatible_slot(opcode, processor)
}

fn slot_by_alias(processor: &Processor, name: &str) -> Option<usize> {
    processor
        .slot_aliases
        .iter()
        .find(|alias| alias.name == name)
        .map(|alias| alias.slot)
}

fn x_slot(processor: &Processor) -> Result<usize, CompileError> {
    processor
        .slot_aliases
        .iter()
        .find(|alias| alias.name == "X")
        .map(|alias| alias.slot)
        .ok_or(CompileError::NotImplemented(
            "processor layout has no X slot",
        ))
}

fn is_integer_alu_opcode(opcode: Opcode) -> bool {
    opcode.units() == [vliw_asm::UnitKind::IntegerAlu]
}

fn is_memory_opcode(opcode: Opcode) -> bool {
    opcode.units() == [vliw_asm::UnitKind::Memory]
}

fn is_control_or_multiplier_opcode(opcode: Opcode) -> bool {
    opcode.units() == [vliw_asm::UnitKind::Control]
        || opcode.units() == [vliw_asm::UnitKind::Multiplier]
}

fn slot_supports_opcode(slot: usize, opcode: Opcode, processor: &Processor) -> bool {
    if opcode == Opcode::Nop {
        return slot < processor.width as usize;
    }

    let unit_kind_by_name: std::collections::HashMap<&str, vliw_asm::UnitKind> = processor
        .units
        .iter()
        .filter_map(|unit| {
            vliw_asm::UnitKind::from_kind_str(&unit.kind).map(|kind| (unit.name.as_str(), kind))
        })
        .collect();

    slot_supports_opcode_with_units(slot, opcode, processor, &unit_kind_by_name)
}

fn slot_supports_opcode_with_units(
    slot: usize,
    opcode: Opcode,
    processor: &Processor,
    unit_kind_by_name: &std::collections::HashMap<&str, vliw_asm::UnitKind>,
) -> bool {
    let Some(slot_units) = processor.slot_units.get(slot) else {
        return false;
    };

    slot_units
        .iter()
        .filter_map(|unit| unit_kind_by_name.get(unit.as_str()))
        .any(|kind| opcode.units().contains(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal valid IR for: int main(void) { volatile u64 *p = (void*)0x100; *p = 42; return 0; }
    const SIMPLE_IR: &str = r#"; ModuleID = 'simple.c'
source_filename = "simple.c"

define i32 @main() {
entry:
  store volatile i64 42, ptr inttoptr (i64 256 to ptr), align 8
  ret i32 0
}
"#;

    #[test]
    fn valid_ir_emits_vliw() {
        let text = compile(SIMPLE_IR, OptimizationLevel::Less, Schedule::Scalar)
            .expect("simple IR should compile");
        assert!(text.contains(".processor {"));
        assert!(text.contains("std [r0 + 0x100]"));
        assert!(text.contains("ret"));
    }

    #[test]
    fn scalar_compile_emits_one_syllable_per_bundle_in_canonical_slots() {
        let text = compile(SIMPLE_IR, OptimizationLevel::Less, Schedule::Scalar)
            .expect("simple IR should compile");
        let program = vliw_asm::parse(&text).expect("emitted VLIW should parse");

        for item in &program.items {
            let Item::Bundle(bundle) = item else {
                continue;
            };
            assert!(
                bundle.slots.iter().filter(|slot| slot.is_some()).count() <= 1,
                "scalar schedule must emit at most one non-nop syllable per bundle:\n{text}"
            );
            if let Some(syl) = bundle.slots[0].as_ref() {
                assert!(
                    matches!(
                        syl.opcode.as_str(),
                        "movi"
                            | "add"
                            | "sub"
                            | "and"
                            | "or"
                            | "xor"
                            | "shl"
                            | "srl"
                            | "sra"
                            | "mov"
                            | "cmpeq"
                            | "cmplt"
                            | "cmpult"
                    ),
                    "I0 should contain only ALU syllables in scalar output: {syl:?}"
                );
            }
            if let Some(syl) = bundle.slots[2].as_ref() {
                assert!(
                    matches!(
                        syl.opcode.as_str(),
                        "ldb"
                            | "ldh"
                            | "ldw"
                            | "ldd"
                            | "stb"
                            | "sth"
                            | "stw"
                            | "std"
                            | "lea"
                            | "prefetch"
                            | "acqload"
                            | "relstore"
                    ),
                    "M should contain only memory syllables in scalar output: {syl:?}"
                );
            }
            if let Some(syl) = bundle.slots[3].as_ref() {
                assert!(
                    matches!(
                        syl.opcode.as_str(),
                        "mul"
                            | "mulh"
                            | "br"
                            | "jmp"
                            | "call"
                            | "ret"
                            | "pand"
                            | "por"
                            | "pxor"
                            | "pnot"
                    ),
                    "X should contain only control or multiply syllables in scalar output: {syl:?}"
                );
            }
        }
    }

    #[test]
    fn bad_ir_returns_parse_error() {
        let bad = "this is not valid LLVM IR\n";
        match compile(bad, OptimizationLevel::None, Schedule::Scalar) {
            Err(CompileError::Parse(msg)) => {
                assert!(!msg.is_empty(), "parse error message should not be empty");
            }
            other => panic!("expected Parse error, got {:?}", other),
        }
    }

    #[test]
    fn add_zero_folded_by_o1() {
        let ir = r#"define i32 @identity_add(i32 %x) {
entry:
  %r = add i32 %x, 0
  ret i32 %r
}
"#;
        let optimized =
            parse_and_optimize(ir, OptimizationLevel::Less).expect("optimization should succeed");
        assert!(
            !optimized.contains("add i32"),
            "add i32 %x, 0 should be folded away by O1 instcombine; optimized IR:\n{optimized}"
        );
    }

    #[test]
    fn codegen_uses_allocated_physical_registers() {
        let ctx = CodegenCtx::default();

        assert_eq!(
            ctx.gpr_for_reg(mir::Reg::PReg(2)),
            regs::FIRST_ALLOCATABLE_GPR
        );
        assert_eq!(ctx.pred_for_reg(mir::Reg::PReg(3)), 3);
    }

    #[test]
    fn latency_table_reports_contract_result_ready_deltas() {
        let latencies = LatencyTable::default();

        assert_eq!(latencies.result_ready_after(Opcode::Add), 1);
        assert_eq!(latencies.result_ready_after(Opcode::Mul), 3);
        assert_eq!(latencies.result_ready_after(Opcode::LoadW), 3);
    }

    #[test]
    fn scheduler_does_not_wait_on_overwritten_gpr_destination() {
        let mut ctx = CodegenCtx::default();
        let mut program = Program {
            processor: Processor::default(),
            items: Vec::new(),
        };

        ctx.emit_asm_syllable(
            &mut program,
            Opcode::LoadW,
            AsmSyllable::new(
                "ldw",
                [
                    AsmOperand::Reg(5),
                    AsmOperand::MemAddr {
                        base: regs::ZERO_REG,
                        offset: 0,
                    },
                ],
            ),
        )
        .unwrap();
        ctx.emit_asm_syllable(
            &mut program,
            Opcode::MovImm,
            AsmSyllable::new("movi", [AsmOperand::Reg(5), AsmOperand::Imm(7)]),
        )
        .unwrap();

        assert_eq!(
            program.items.len(),
            2,
            "overwriting a pending destination should not emit latency filler bundles"
        );
    }

    #[test]
    fn scheduler_uses_first_processor_compatible_slot() {
        let mut processor = Processor::default();
        processor.slot_units.swap(0, 1);
        processor.slot_units[0] = vec!["mem".into()];
        processor.slot_units[1] = vec!["alu".into()];

        let slot = first_compatible_slot(Opcode::Add, &processor).unwrap();

        assert_eq!(slot, 1);
    }

    #[test]
    fn scalar_scheduler_uses_canonical_slots() {
        let processor = Processor::default();

        assert_eq!(scalar_slot(Opcode::Add, &processor).unwrap(), 0);
        assert_eq!(scalar_slot(Opcode::StoreD, &processor).unwrap(), 2);
        assert_eq!(scalar_slot(Opcode::Mul, &processor).unwrap(), 3);
        assert_eq!(scalar_slot(Opcode::Ret, &processor).unwrap(), 3);
    }

    #[test]
    fn pack_scheduler_merges_adjacent_independent_scalar_bundles() {
        let func = mir::Function {
            name: "pack_adjacent".into(),
            blocks: vec![mir::Block {
                label: "entry".into(),
                syllables: vec![
                    mir::Syllable {
                        opcode: Opcode::Add,
                        dst: Some(mir::Reg::PReg(2)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(3)),
                            mir::Value::Reg(mir::Reg::PReg(4)),
                        ],
                    },
                    mir::Syllable {
                        opcode: Opcode::Sub,
                        dst: Some(mir::Reg::PReg(5)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(6)),
                            mir::Value::Reg(mir::Reg::PReg(7)),
                        ],
                    },
                ],
                terminator: mir::Terminator::Return,
            }],
        };

        let text = emit_packed_vliw(&func).expect("packed emission should succeed");
        let program = vliw_asm::parse(&text).expect("packed output should parse");
        let bundles: Vec<_> = program.items.iter().filter_map(bundle_item).collect();

        assert_eq!(
            bundles[0].slots.iter().flatten().count(),
            2,
            "adjacent independent ALU syllables should share a bundle:\n{text}"
        );
        assert!(
            bundles[0].slots[0]
                .as_ref()
                .is_some_and(|syl| syl.opcode == "add")
                && bundles[0].slots[1]
                    .as_ref()
                    .is_some_and(|syl| syl.opcode == "sub"),
            "second ALU syllable should move to the other compatible slot:\n{text}"
        );
    }

    #[test]
    fn pack_scheduler_preserves_latency_padding_and_control_boundaries() {
        let func = mir::Function {
            name: "pack_boundaries".into(),
            blocks: vec![
                mir::Block {
                    label: "entry".into(),
                    syllables: vec![
                        mir::Syllable {
                            opcode: Opcode::Mul,
                            dst: Some(mir::Reg::PReg(2)),
                            srcs: vec![
                                mir::Value::Reg(mir::Reg::PReg(3)),
                                mir::Value::Reg(mir::Reg::PReg(4)),
                            ],
                        },
                        mir::Syllable {
                            opcode: Opcode::StoreW,
                            dst: None,
                            srcs: vec![mir::Value::Imm(0x100), mir::Value::Reg(mir::Reg::PReg(2))],
                        },
                    ],
                    terminator: mir::Terminator::Jump("exit".into()),
                },
                mir::Block {
                    label: "exit".into(),
                    syllables: vec![mir::Syllable {
                        opcode: Opcode::Add,
                        dst: Some(mir::Reg::PReg(8)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(9)),
                            mir::Value::Reg(mir::Reg::PReg(10)),
                        ],
                    }],
                    terminator: mir::Terminator::Return,
                },
            ],
        };

        let text = emit_packed_vliw(&func).expect("packed emission should succeed");
        let program = vliw_asm::parse(&text).expect("packed output should parse");
        let bundles: Vec<_> = program.items.iter().filter_map(bundle_item).collect();
        let mul_idx = bundles
            .iter()
            .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "mul"))
            .expect("mul should be emitted");
        let store_idx = bundles
            .iter()
            .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "stw"))
            .expect("store should be emitted");
        let jump_idx = bundles
            .iter()
            .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "jmp"))
            .expect("jump should be emitted");
        let jump_item_idx = program
            .items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    Item::Bundle(bundle)
                        if bundle.slots.iter().flatten().any(|syl| syl.opcode == "jmp")
                )
            })
            .expect("jump item should be emitted");

        assert!(
            store_idx >= mul_idx + 3,
            "dependent store must keep mul latency padding:\n{text}"
        );
        assert_eq!(
            bundles[jump_idx].slots.iter().flatten().count(),
            1,
            "terminator bundle should not be packed with neighboring syllables:\n{text}"
        );
        assert!(
            matches!(program.items[jump_item_idx + 1], Item::Label(ref label) if label == "exit"),
            "label should remain immediately after the terminator bundle:\n{text}"
        );
    }

    #[test]
    fn pack_scheduler_reorders_independent_ops_into_mul_latency_window() {
        let func = mir::Function {
            name: "list_schedule_latency".into(),
            blocks: vec![mir::Block {
                label: "entry".into(),
                syllables: vec![
                    mir::Syllable {
                        opcode: Opcode::Mul,
                        dst: Some(mir::Reg::PReg(2)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(3)),
                            mir::Value::Reg(mir::Reg::PReg(4)),
                        ],
                    },
                    mir::Syllable {
                        opcode: Opcode::Add,
                        dst: Some(mir::Reg::PReg(5)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(2)),
                            mir::Value::Reg(mir::Reg::PReg(6)),
                        ],
                    },
                    mir::Syllable {
                        opcode: Opcode::Sub,
                        dst: Some(mir::Reg::PReg(7)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(8)),
                            mir::Value::Reg(mir::Reg::PReg(9)),
                        ],
                    },
                    mir::Syllable {
                        opcode: Opcode::Xor,
                        dst: Some(mir::Reg::PReg(10)),
                        srcs: vec![
                            mir::Value::Reg(mir::Reg::PReg(11)),
                            mir::Value::Reg(mir::Reg::PReg(12)),
                        ],
                    },
                ],
                terminator: mir::Terminator::Return,
            }],
        };

        let text = emit_packed_vliw(&func).expect("packed emission should succeed");
        let program = vliw_asm::parse(&text).expect("packed output should parse");
        let bundles: Vec<_> = program.items.iter().filter_map(bundle_item).collect();
        let mul_idx = bundles
            .iter()
            .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "mul"))
            .expect("mul should be emitted");
        let add_idx = bundles
            .iter()
            .position(|bundle| bundle.slots.iter().flatten().any(|syl| syl.opcode == "add"))
            .expect("dependent add should be emitted");

        assert_eq!(
            add_idx,
            mul_idx + 3,
            "dependent add should issue as soon as mul latency is satisfied:\n{text}"
        );
        assert!(
            bundles[mul_idx + 1..add_idx]
                .iter()
                .all(|bundle| bundle.slots.iter().flatten().count() > 0),
            "independent syllables should occupy the mul latency window without filler bundles:\n{text}"
        );
    }

    #[test]
    fn list_scheduler_reorders_disjoint_stack_and_global_memory() {
        let syllables = vec![
            mir::Syllable {
                opcode: Opcode::LoadD,
                dst: Some(mir::Reg::PReg(2)),
                srcs: vec![mir::Value::Imm(0x100)],
            },
            mir::Syllable {
                opcode: Opcode::Add,
                dst: Some(mir::Reg::PReg(4)),
                srcs: vec![
                    mir::Value::Reg(mir::Reg::PReg(2)),
                    mir::Value::Reg(mir::Reg::PReg(3)),
                ],
            },
            mir::Syllable {
                opcode: Opcode::StoreW,
                dst: None,
                srcs: vec![mir::Value::Stack(0), mir::Value::Reg(mir::Reg::PReg(5))],
            },
        ];

        let scheduled = list_schedule_syllables(&syllables);
        let opcodes: Vec<_> = scheduled.iter().map(|syl| syl.opcode).collect();

        assert_eq!(
            opcodes,
            vec![Opcode::LoadD, Opcode::StoreW, Opcode::Add],
            "disjoint stack/global memory should be free to fill the load latency window"
        );
    }

    #[test]
    fn list_scheduler_preserves_unknown_alias_scalar_ordering() {
        let unknown_base = mir::Value::Reg(mir::Reg::PReg(8));
        let syllables = vec![
            mir::Syllable {
                opcode: Opcode::LoadD,
                dst: Some(mir::Reg::PReg(2)),
                srcs: vec![unknown_base],
            },
            mir::Syllable {
                opcode: Opcode::Add,
                dst: Some(mir::Reg::PReg(4)),
                srcs: vec![
                    mir::Value::Reg(mir::Reg::PReg(2)),
                    mir::Value::Reg(mir::Reg::PReg(3)),
                ],
            },
            mir::Syllable {
                opcode: Opcode::StoreW,
                dst: None,
                srcs: vec![unknown_base, mir::Value::Reg(mir::Reg::PReg(5))],
            },
        ];

        let scheduled = list_schedule_syllables(&syllables);
        let opcodes: Vec<_> = scheduled.iter().map(|syl| syl.opcode).collect();

        assert_eq!(
            opcodes,
            vec![Opcode::LoadD, Opcode::Add, Opcode::StoreW],
            "unknown-alias memory must remain a scalar-order barrier"
        );
    }

    fn bundle_item(item: &Item) -> Option<&Bundle> {
        match item {
            Item::Bundle(bundle) => Some(bundle),
            Item::Label(_) => None,
        }
    }
}
