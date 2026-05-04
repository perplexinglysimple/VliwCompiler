# VliwCompiler

Rust-based compiler for the VLIW ISA defined by
[LwirSimulator](https://github.com/perplexinglysimple/LwirSimulator). Consumes
LLVM IR (via `inkwell`), runs LLVM's optimization pipeline, then lowers to
scheduled `.vliw` assembly that satisfies the simulator's compiler contract.

## Status

Scaffolding only. No code generation yet.

## Why Rust + LLVM-as-a-library (not an LLVM out-of-tree backend)

Out-of-tree LLVM backends are C++-only — TableGen, `MachineInstr`,
SelectionDAG/GlobalISel, MC layer, `AsmPrinter`. For a VLIW with explicit
bundles and custom slot/unit legality, those frameworks tend to fight you
more than help; existing in-tree VLIW targets (e.g. Hexagon) override large
pieces of MachineScheduler anyway.

This repo treats LLVM as a *frontend/optimization library* via `inkwell`,
and writes the VLIW-specific lowering, scheduling, bundling, and `.vliw`
text emission in pure Rust. That matches the simulator's Rust toolchain and
keeps the bundler under direct control.

## Target contract

Backend output must satisfy
[`docs/compiler_contract.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/compiler_contract.md).
Output text format:
[`docs/vliw_asm_format.md`](https://github.com/perplexinglysimple/LwirSimulator/blob/master/docs/vliw_asm_format.md).

The staged bring-up plan is in [`docs/dev_plan.md`](docs/dev_plan.md).

Headline rules:
- Every program starts with `.processor { ... }` declaring `width`, hardware
  units, per-slot unit sets, cache, and `topology`.
- Bundle width in `[1, 256]`.
- Each syllable in a slot whose declared units can execute its opcode.
- No same-bundle GPR RAW / GPR WAW / predicate hazards.
- Stall-free schedule under the cache- and topology-derived worst-case load
  latency.
- Multi-CPU: memory ops on the issuing CPU's bus slot only; every `acqload`
  polling loop has a matching producer `relstore` on another CPU.

## Layout

```
VliwCompiler/
├── Cargo.toml              # workspace
├── crates/
│   ├── vliw-asm/           # .vliw text emitter (no LLVM dep)
│   ├── vliw-backend/       # IR -> VLIW lowering (will pull in inkwell)
│   └── vliwc/              # driver binary
├── docs/
│   ├── dev_plan.md         # staged bring-up plan
│   └── architecture.md     # design notes — to be filled in
└── tests/                  # integration tests (later)
```

## Build

Install LLVM 19 development libraries first. The workspace links `inkwell`
dynamically against LLVM 19.

```bash
sudo apt-get install clang-19 llvm-19-dev
cargo build
cargo test
cargo run -p vliwc -- --help
```

## Current Simulator Smoke Test

The asm emitter targets the format parsed by
[LwirSimulator](https://github.com/perplexinglysimple/LwirSimulator). To verify
the emitter before LLVM lowering exists, build the simulator's verifier
alongside this repo and run the hard-coded demo program:

```bash
mkdir -p build
cargo run -p vliwc -- --demo -o build/demo.vliw
../LwirSimulator/target/debug/vliw_verify build/demo.vliw
../LwirSimulator/target/debug/vliw_simulator --trace build/demo.vliw
```

## Planned C Flow

The first compiler smoke test is a deliberately small C program:
[`examples/simple.c`](examples/simple.c). It writes `42` to simulator memory at
`0x100` and returns. That should eventually lower through the scalar baseline:
one real syllable per bundle, all other slots `nop`, plus full-`nop` latency
padding when needed.

The intended end-to-end flow is:

```bash
mkdir -p build/c-flow

clang-19 -S -emit-llvm -O1 -ffreestanding -fno-builtin \
  examples/simple.c \
  -o build/c-flow/simple.ll

cargo run -p vliwc -- build/c-flow/simple.ll -o build/c-flow/simple.vliw

../LwirSimulator/target/debug/vliw_verify build/c-flow/simple.vliw
../LwirSimulator/target/debug/vliw_simulator --trace build/c-flow/simple.vliw
```

Current expected status: the `vliwc` command fails with
`not implemented: LLVM IR -> VLIW codegen`. The command sequence and CI are
there to show the shape of the flow; once scalar lowering lands, remove the
temporary `continue-on-error` from `.github/workflows/c-flow.yml`.
