# MIR Design Note

MIR is the backend boundary between LLVM instruction selection and VLIW bundle
emission. It is deliberately close to the target: a function is an ordered list
of basic blocks, each block is a linear list of target opcodes, and control flow
is represented by a single terminator per block.

## Types

The Rust definitions live in `crates/vliw-backend/src/mir.rs`:

- `Function { name, blocks }` is one lowered function.
- `Block { label, syllables, terminator }` is a basic block in final layout
  order.
- `Syllable { opcode, dst, srcs }` is one target-like operation before it has
  been assigned to a VLIW slot.
- `Reg::{VReg, PReg}` distinguishes pre-allocation virtual registers from
  post-allocation physical registers.
- `Value::{Reg, Imm, Stack}` represents register operands, immediates, and
  stack-frame-relative addresses used by scheduler alias analysis.
- `Terminator::{Branch, Jump, Return, DirectCall}` ends every block.

Minimal pre-allocation MIR:

```text
fn add_two:
entry:
  add v2, v0, v1
  mov r1, v2
  ret
```

This example adds two incoming virtual values, moves the result to the return
register, and returns. The `mov r1, v2` shape is allowed before register
allocation because MIR operands may mix fixed physical registers required by
the calling convention with virtual registers that still need allocation.

## Invariants

Before register allocation, instruction selection and phi lowering produce MIR
with these checked invariants:

- The LLVM subset has already been validated: supported integer widths only,
  no atomic instructions, no indirect calls, no external calls, and no
  unsupported control-transfer forms such as `invoke` / `callbr`.
- Every block has exactly one terminator, and normal syllables appear before
  that terminator.
- Phi nodes have been lowered to explicit predecessor-edge `mov` copies, so
  no MIR block contains implicit SSA phi semantics.
- Virtual registers may appear in sources, destinations, and branch
  conditions; physical registers appear only where the ABI or target requires a
  fixed register, such as `r1` for integer returns.

After register allocation, the allocator returns allocated MIR with these
checked invariants:

- All allocatable `Reg::VReg` uses have been rewritten to `Reg::PReg`.
- Assigned general-purpose registers are in the allocator range `r2..r30`;
  `r0`, `r1`, and `r31` keep their fixed roles.
- Liveness and last-use information have been applied when reusing physical
  registers.
- If no physical register is available, allocation inserts explicit spill
  stores and reloads as MIR memory syllables, so spill traffic remains visible
  to scheduling and verification.

Before scheduling, the scheduler consumes allocated MIR with these checked
invariants:

- Syllables use target opcodes whose metadata names legal unit kinds, result
  latency, and write class.
- Branch conditions and predicate-producing comparisons have allocated register
  numbers so final emission can map them to predicate operands (`pN`) where the
  opcode requires it.
- Memory operands have a scheduler-visible address form (`Reg`, `Imm`, or
  `Stack`) so local scheduling can preserve conservative memory dependencies.
- Control flow boundaries are explicit: labels precede blocks, and each block's
  terminator remains the last operation for that block. `DirectCall`
  terminators name both the target function label and the continuation block.

## Mapping To Bundles

MIR does not store bundle slots. Emission walks allocated MIR block by block,
adds a `.vliw` label for each block, converts each `Syllable` to a
`vliw_asm::Syllable`, and places it in a processor-compatible slot. The scalar
schedule emits at most one non-`nop` syllable per bundle and inserts full-`nop`
bundles until source registers, memory base registers, and predicates are ready
at the current cycle; destination ready cycles are then advanced by opcode
latency. The packed schedule first locally reorders independent MIR syllables,
then greedily merges adjacent scalar bundles when slot legality, ready-cycle
checks, and the `vliw-asm` bundle verifier allow it. Its observable-result
contract is to preserve the scalar stream's control-flow boundaries and
state-changing syllables while matching the scalar baseline's expected simulator
results.

The final bundle stream is verifier-checked for local VLIW hazards before text
emission, including same-bundle GPR WAW, same-bundle GPR RAW, and predicate
ordering hazards.
