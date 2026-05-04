# VliwCompiler Architecture

The compiler uses LLVM as a frontend and optimization library, then lowers to
the simulator's textual `.vliw` format with Rust-owned MIR, register mapping,
scheduling, verification, and emission.

## Pipeline

```text
LLVM IR text
   |  inkwell: parse module
   v
Validated LLVM subset
   |  inkwell: optional new-pass-manager pipeline
   v
Optimized LLVM IR
   |  vliw-backend: hand-written instruction selection
   v
VLIW MIR  (basic blocks, virtual registers, typed opcodes)
   |  vliw-backend: phi copy lowering + liveness-aware physical mapping
   v
Allocated MIR  (physical GPR/predicate numbers)
   |  vliw-backend: scalar scheduling, optional local list scheduling + packing
   v
Bundle stream  (slot-assigned, latency-padded, verifier-checked)
   |  vliw-asm: text emission
   v
.vliw text
```

## Crate Boundaries

- `vliw-asm` owns the `.vliw` data model, typed operands, opcode metadata,
  header emission, parsing, and local bundle/program verification. It has no
  LLVM dependency.
- `vliw-backend` owns LLVM IR ingestion, subset validation, MIR, instruction
  selection, phi lowering, liveness, register mapping, scheduling, packing,
  and conversion to `vliw-asm` programs.
- `vliwc` is the driver: argument parsing, file I/O, emit-mode selection, and
  error reporting.

## Target Header

The emitted `.processor { ... }` header comes from a structured
`vliw_asm::Processor`. If no target is provided, `vliwc` uses
`Processor::default()`, the canonical four-slot layout:

- width 4
- units: `alu = integer_alu`, `mem = memory`, `ctrl = control`,
  `mul = multiplier`
- aliases: `I0 = 0`, `I1 = 1`, `M = 2`, `X = 3`
- slot units: `0={alu}`, `1={alu}`, `2={mem}`, `3={ctrl,mul}`
- empty cache stanza and `topology { cpus 1 }`

`Processor` is already a real structured model rather than a string template,
so `vliw-asm` can emit other layouts. End-to-end target selection is available
through `vliwc --processor <config.vliw>` and through the
`compile_for_processor` backend API. The config file may be any `.vliw` text
that contains a `.processor { ... }` block; the parser reads the processor
block and ignores program items for target selection.

Scheduling is layout-aware. Scalar emission chooses the first slot whose
declared units can execute the opcode, and `pack` uses the same slot/unit
legality checks when merging bundles. Wider layouts therefore affect both the
output header and the generated bundle density.

## LLVM Subset And ISel

Instruction selection is hand-written Rust over inkwell/LLVM instruction
opcodes, not SelectionDAG, GlobalISel, BURG, or a separate tiler. LLVM remains
the parser and optimizer; the VLIW backend owns target-specific lowering.

Supported lowering includes:

- integer ALU: `add`, `sub`, `mul`, `and`, `or`, `xor`, `shl`, `lshr`, `ashr`
- constants materialized as `movi` when needed
- integer comparisons `eq`, `ne`, `slt`, `sgt`, `ult`, `ugt`
- conditional and unconditional branches
- `ret void` and integer returns
- `load` / `store` for `i8`, `i32`, and `i64`
- `getelementptr` when indices fold to a constant byte offset
- `phi` nodes lowered after ISel by inserting predecessor-edge `mov` copies
- direct calls to functions defined in the same LLVM module

The validator rejects unsupported integer widths, unsupported atomic
instructions, `invoke` / `callbr`, indirect calls, and calls to external
declarations before or during ISel. Multiple function definitions are supported
and are emitted into one `.vliw` output with function-name labels as direct
call targets.

## MIR

MIR is the backend boundary between LLVM-shaped code and scheduled bundles:

- `Function { name, blocks }`
- `Block { label, syllables, terminator }`
- `Syllable { opcode, dst, srcs }`
- `Reg::VReg` before allocation and `Reg::PReg` after allocation
- `Terminator::{Branch, Jump, Return, DirectCall}`

MIR keeps instruction selection separate from register allocation and
scheduling. Phi lowering is a MIR pass: ISel assigns a vreg to the phi result,
then `phi::lower_phi_copies` inserts parallel-copy moves into predecessor
blocks. Back-edge copies can be placed in a synthetic latch block so loop
continue paths preserve fall-through behavior.

## Registers And Calling Convention

The fixed register policy lives in `vliw-backend::regs`:

- `r0` is the hardware zero register.
- `r1` is the integer return-value register.
- `r2..r30` are allocatable.
- `r31` is the link register reserved for calls.

The current allocator walks MIR in program order, assigns fresh virtual
registers from `r2..r30`, and uses MIR liveness plus last-use information to
free registers. When allocatable GPRs are exhausted, it inserts explicit spill
stores and reloads into MIR; spill traffic is ordinary scheduler-visible
memory traffic.

The current calling convention supports integer returns and direct calls to
functions defined in the same LLVM module. Call arguments are moved into
`r2..r9`, returns come back in `r1`, and the caller saves/restores `r31` around
each direct call site. External calls, indirect calls, varargs, and a richer
clobber model remain unsupported.

## Predicates

Comparisons produce predicate-valued MIR results using `cmpeq`, `cmplt`, or
`cmpult`. `ne` lowers as `cmpeq` followed by `pnot`; signed/unsigned
greater-than lower by swapping operands into less-than forms.

Predicate results use the same MIR `Reg` numbering as other virtual values
until allocation. During emission, predicate-writing opcodes map the allocated
number to `pN` operands and use a separate predicate ready-cycle table. Branch
terminators lower to `br pN, label`. Full if-conversion and inter-block
predicated scheduling are deferred to `LATER-4`.

## Scheduling And Packing

The default schedule is scalar and remains the correctness oracle:

- emit at most one non-`nop` syllable per bundle
- choose the first compatible slot from the emitted processor layout
- insert all-`nop` bundles until every source register or predicate is ready
- update ready cycles from opcode latency metadata
- verify the generated program with `vliw-asm::verify_program`

The scalar oracle has three invariants that optimized schedules must preserve.
First, each scalar bundle contains zero or one real syllable: padding bundles
are all `nop`, and a non-padding bundle has exactly one non-`nop` syllable in a
processor-compatible slot. Second, before any scalar syllable is emitted at
cycle `C`, every source GPR, source predicate, and memory base register it reads
must have ready cycle `<= C`; otherwise the scheduler inserts full-`nop`
bundles until that condition holds, then records destination ready cycles as
`C + opcode.latency()`. Third, `pack` may change bundle grouping and local
order only when the resulting program has the same observable result as the
scalar stream: the same labels and terminators delimit control flow, the same
state-changing syllables execute with dependencies satisfied, generated bundles
pass the `vliw-asm` verifier, and fixture/simulator expected results must match
the scalar baseline.

The optional `--schedule=pack` path first applies a local per-basic-block list
scheduler over MIR syllables, then greedily packs scalar bundle segments while
respecting slot legality, in-bundle hazards, labels, and terminators. The local
scheduler builds a dependence DAG for register RAW/WAR/WAW, predicate hazards,
and memory dependencies. Memory aliasing is conservative: stack, absolute
global, and unknown buckets may reorder only when proven disjoint.

Scheduling metadata comes from `vliw-asm::Opcode`: allowed unit kinds, result
latency, and write classes. Current key latencies are ALU/predicate/control 1,
loads and multiply 3, stores 1, fp32 4, fp64 6, AES 4.

## Memory Ordering

Plain LLVM loads and stores lower to width-specific `ld*` and `st*` opcodes.
GEP is folded into a base plus immediate displacement when all indices are
constant. Stack/absolute addresses use `r0` as the address base during final
emission.

LLVM atomic instructions are not lowered today. The subset validator rejects
`atomicrmw`, `cmpxchg`, and `fence` with `UnsupportedFeature("atomic
instruction")`. Acquire/release lowering to `acqload` / `relstore`, including
multi-CPU topology and producer/consumer verification, is deferred to
`LATER-5`.

## Deferred Decisions

| Area | Current answer | Deferred task |
| --- | --- | --- |
| Header source | `Processor::default()` or `--processor <config.vliw>` / `compile_for_processor` | richer target configuration if needed |
| External / indirect calls | Rejected before lowering | remaining `LATER-2` ABI work |
| Spilling allocator | Implemented as explicit MIR load/store spill traffic | more advanced allocation heuristics |
| Full if-conversion/inter-block predication | Not implemented | `LATER-4` |
| LLVM atomics / acquire-release | Rejected by subset validation | `LATER-5` |

## Reference

- ISA contract:
  [`docs/compiler_contract.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/compiler_contract.md)
- Asm format:
  [`docs/vliw_asm_format.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md)
- Layout plan:
  [`docs/processor_layout_plan.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/processor_layout_plan.md)
- MIR design note: [`mir.md`](mir.md)
- Staged bring-up: [`dev_plan.md`](dev_plan.md)
- Concrete backlog: [`dev_tasks.md`](dev_tasks.md)
