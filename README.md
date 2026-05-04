# VliwCompiler

Rust-based compiler for the VLIW ISA defined by
[LwirSimulator](https://github.com/perplexinglysimple/LwirSimulator). Consumes
LLVM IR (via `inkwell`), runs LLVM's optimization pipeline, then lowers to
scheduled `.vliw` assembly that satisfies the simulator's compiler contract.

## Status

The compiler has a working scalar baseline and an optional packed schedule.
It parses LLVM IR with `inkwell`, lowers the supported integer subset to MIR,
allocates physical registers, emits scheduled `.vliw` bundle text, and checks
the generated program with the Rust-side VLIW verifier before writing output.

Supported lowering currently includes integer ALU ops, multiply, integer
comparisons, conditional and unconditional branches, returns, `i8`/`i32`/`i64`
loads and stores, constant-offset GEPs, phi lowering for loops, and direct
calls to functions defined in the same LLVM module.

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
│   ├── vliw-backend/       # IR -> VLIW lowering, scheduling, packing
│   └── vliwc/              # driver binary
├── docs/
│   ├── dev_plan.md         # staged bring-up plan
│   ├── dev_tasks.md        # implementation backlog
│   ├── architecture.md     # current architecture notes
│   └── mir.md              # MIR design note
└── tests/                  # simulator-backed integration harness
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
the hard-coded demo program, build the simulator's verifier alongside this repo:

```bash
mkdir -p build
cargo run -p vliwc -- --emit=demo -o build/demo.vliw
../LwirSimulator/target/debug/vliw_verify build/demo.vliw
../LwirSimulator/target/debug/vliw_simulator --trace build/demo.vliw
```

## C Flow

[`examples/simple.c`](examples/simple.c) writes `42` to simulator memory at
`0x100` and returns. It lowers through the scalar baseline by default:
one real syllable per bundle, all other slots `nop`, plus full-`nop` latency
padding when needed.

```bash
mkdir -p build/c-flow

clang-19 -S -emit-llvm -O1 -ffreestanding -fno-builtin \
  examples/simple.c \
  -o build/c-flow/simple.ll

cargo run -p vliwc -- build/c-flow/simple.ll -o build/c-flow/simple.vliw

../LwirSimulator/target/debug/vliw_verify build/c-flow/simple.vliw
../LwirSimulator/target/debug/vliw_simulator --trace build/c-flow/simple.vliw
```

Use `--schedule=pack` to enable local scheduling plus greedy bundle packing:

```bash
cargo run -p vliwc -- --schedule=pack build/c-flow/simple.ll -o build/c-flow/simple.pack.vliw
```

## Processor Layouts

By default, `vliwc` targets the canonical four-slot processor layout from
`Processor::default()`. The compiler can also take a different processor
layout and emit `.vliw` output with that layout's header, aliases, width, and
slot capabilities:

```bash
cargo run -p vliwc -- \
  --processor crates/vliw-backend/tests/fixtures/wide8.vliw \
  --schedule=pack \
  build/c-flow/simple.ll \
  -o build/c-flow/simple.wide8.vliw
```

The `--processor` file is any `.vliw` text containing a `.processor { ... }`
block. The parser reads that block into `vliw_asm::Processor`; the backend then
uses the declared slot/unit capabilities when placing scalar syllables and when
packing bundles. This is covered by `layout_test.rs`, including a contrived
8-slot target where `pack` emits denser bundles than the canonical 4-slot
layout.
