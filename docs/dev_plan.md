# Dev Plan

The guiding design choice: **make correctness scalar and boring first, then
make performance an optional scheduling problem.** A scalar schedule that
trivially satisfies the contract is the permanent oracle; every later
optimization must reproduce its observable result and pass the same verifier.

Reference docs:
- ISA / compiler contract:
  [`docs/compiler_contract.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/compiler_contract.md)
- `.vliw` text format:
  [`docs/vliw_asm_format.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md)
- Processor layout plan:
  [`docs/processor_layout_plan.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/processor_layout_plan.md)

---

## 1. Define the initial LLVM subset

Start small and explicit:

- one function, probably `main`
- integer ops first: `add`, `sub`, `and`, `or`, `xor`, shifts
- `icmp eq`, signed/unsigned less-than
- `load`, `store`, `getelementptr` or a simplified address form
- basic blocks, conditional branch, unconditional branch, return
- no calls, no `phi` nodes initially, or handle phi lowering separately

## 2. Emit valid `.vliw` structure first

Always emit the current required header:

```text
.processor {
  width 4
  hardware {
    unit alu = integer_alu
    unit mem = memory
    unit ctrl = control
    unit mul = multiplier
  }
  layout slots {
    0 = { alu }
    1 = { alu }
    2 = { mem }
    3 = { ctrl, mul }
  }
  cache { }
  topology { cpus 1 }
}
```

Then emit one bundle per selected instruction:

- ALU ops in `i0`
- memory ops in `m`
- branch / jump / ret in `x`
- multiply in `x`
- all other slots `nop`

## 3. Build a simple compiler pipeline

Use clear internal phases:

```text
LLVM IR
  -> parsed module/function
  -> compiler IR with virtual registers and basic blocks
  -> instruction selection to VLIW syllables
  -> scalar scheduler: one syllable per bundle + required latency nops
  -> text emitter
  -> verifier/simulator test
```

Keep instruction selection separate from scheduling. That separation is
what lets the scalar schedule remain a permanent correctness baseline.

## 4. Handle registers conservatively

LLVM IR is SSA; the simulator uses physical GPRs. For the first version:

- map each LLVM SSA value to a physical register while under 32 live values
- reserve `r0` as zero
- reserve `r31` for link / return behavior
- fail clearly if a test exceeds available registers

Do not build a real allocator first. A simple deterministic mapping is
enough for the baseline.

## 5. Handle latency explicitly

Maintain a ready-cycle table for physical registers. When emitting a
syllable at cycle `C`:

- check source registers' ready cycles
- insert full-nop bundles until sources are ready
- after emitting a destination write, set its ready cycle based on opcode
  latency

This makes the scalar baseline pass `vliw_verify`, not just run by relying
on simulator stalls.

## 6. Create verification tests immediately

For every compiler fixture:

- compile `.ll` to `.vliw`
- run `vliw_verify output.vliw` (from the simulator's debug build)
- run the simulator
- assert expected memory / register-visible result

Start with tiny programs:

- constant return / store
- add / sub
- load / store
- branch
- loop
- multiply with dependent use, proving latency nop insertion works

## 7. Keep the scalar baseline forever

Add a compiler flag:

```text
--schedule=scalar
--schedule=pack
```

The scalar schedule is the oracle. Optimized schedules must produce the
same observable result and pass the same verifier.

## 8. Then improve packing

Once scalar output is correct:

- pack independent adjacent syllables into the same bundle when
  slot-compatible
- forbid same-bundle RAW / WAW / predicate hazards
- respect per-slot unit legality
- preserve control-flow boundaries at first
- compare cycle count against the scalar baseline

## 9. Then improve local scheduling

After packing works:

- reorder inside a basic block only
- use dependency DAGs
- schedule around `mul` / load latency
- keep branches / labels fixed initially
- only move memory ops when alias analysis says it is safe; at first,
  treat all memory ops as ordering barriers

## 10. Later: harder features

Add these only after the baseline is stable:

- phi lowering
- real register allocation / spilling
- calls
- wider layouts
- predication
- inter-basic-block scheduling
- memory alias analysis
- multi-CPU / acquire-release support
