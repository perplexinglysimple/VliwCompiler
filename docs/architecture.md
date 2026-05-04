# VliwCompiler Architecture (placeholder)

Stub — fill in when the dev plan lands.

## Pipeline (intended)

```
LLVM IR (text/bitcode)
   |  inkwell: parse + run opt pipeline
   v
Optimized LLVM IR
   |  vliw-backend: instruction selection (custom, Rust)
   v
VLIW MIR  (typed Rust IR — slots not yet assigned)
   |  vliw-backend: register alloc, scheduling, bundle packing
   v
Bundle stream  (slot-assigned, hazard-free, scoreboard-ready)
   |  vliw-asm: text emission
   v
.vliw text
```

## Crate boundaries

- `vliw-asm` — pure data model + text writer. No LLVM dep. Independently
  testable against the simulator's parser.
- `vliw-backend` — owns the lowering and scheduling. Pulls in `inkwell` for
  IR ingestion. Depends on `vliw-asm` for the emit step.
- `vliwc` — thin driver. Argument parsing, file I/O, error reporting.

## Open questions for the dev plan

- How is the `.processor { ... }` header sourced? CLI flags, sidecar TOML,
  IR metadata, or compiled-in defaults?
- ISel approach: pattern-matching DAG, BURG-style tiler, hand-written
  match on LLVM IR opcodes?
- Register allocator: linear scan, graph coloring, or borrow from regalloc2?
- Scheduling model: list scheduler with reservation tables per slot/unit?
  Trace scheduling? Modulo scheduling for loops?
- Predicate register modeling and if-conversion strategy.
- Calling convention (r31 = link, per simulator).
- Memory ordering lowering: `acqload` / `relstore` from LLVM atomics.

## Reference

- ISA contract:
  [`docs/compiler_contract.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/compiler_contract.md)
- Asm format:
  [`docs/vliw_asm_format.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md)
- Layout plan:
  [`docs/processor_layout_plan.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/processor_layout_plan.md)
- Staged bring-up: [`dev_plan.md`](dev_plan.md)
