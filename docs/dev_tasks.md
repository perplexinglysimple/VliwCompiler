# Dev Tasks

Concrete, independent work items derived from
[`dev_plan.md`](dev_plan.md). The plan is the narrative; this file is the
backlog. Each task has explicit inputs, outputs, and acceptance criteria so
it can be picked up in isolation.

## Conventions

- **ID**: stable handle (e.g. `ASM-1`). Use in commits and PR titles.
- **Crate**: where the work lands.
- **Depends on**: blocking task IDs. No entry = pickable today.
- **Size**: rough effort — `S` (≤ half day), `M` (1–3 days), `L` (week+).
- **Done when**: observable acceptance criteria. No "looks good" — always a
  test, a passing verifier, or a diff against a reference output.

The guiding rule from the dev plan still holds: **scalar correctness before
performance.** Any task in the *Optimization* section must be gated on the
scalar baseline (TEST-1 through TEST-6) being green.

## Plan coverage

Each track should either implement or support one of the staged plan
sections:

| Dev plan section | Backlog coverage |
| --- | --- |
| 1. Initial LLVM subset | LLVM-0 through LLVM-6, LLVM-9 |
| 2. Valid `.vliw` structure | ASM-1 through ASM-5, SCHED-2, SCHED-3 |
| 3. Compiler pipeline phases | MIR-1 through MIR-3, LLVM-1 through LLVM-8, SCHED-2 |
| 4. Conservative registers | REGS-1 through REGS-3 |
| 5. Explicit latency | ASM-3, SCHED-1, SCHED-2, TEST-6 |
| 6. Verification tests | TEST-1 through TEST-7, CI-1, CI-2 |
| 7. Scalar baseline forever | DRV-2, SCHED-4, TEST-7 |
| 8. Bundle packing | OPT-1, OPT-3 |
| 9. Local scheduling | OPT-2, OPT-4 |
| 10. Later features | LLVM-7, LATER-1 through LATER-5 |
| Cross-cutting support | DRV-1, DRV-3, DRV-4, DOC-1 through DOC-3 |

---

## Track A — `vliw-asm` data model and emitter

`vliw-asm` is LLVM-free and has the shortest dependency chain. Most of this
track can run in parallel with everything else.

### ASM-1 — Generalize `Processor` beyond the hardcoded 4-slot layout
- **Crate**: `vliw-asm`. **Size**: M. **Depends on**: —.
- Today `Processor` only carries `width`; `emit_header` writes a hardcoded
  4-slot block. Replace with a real model: per-slot unit sets, named unit
  declarations, cache and topology stanzas, and slot aliases.
- **Done when**:
  - `Processor` round-trips both the canonical 4-slot example and a
    contrived 2-slot and 8-slot configuration through `emit`.
  - `emits_canonical_example` test still passes byte-for-byte against the
    current expected substring assertions.
  - No `vliw_verify` regression on `--emit=demo` output.

### ASM-2 — Typed register and immediate operand model
- **Crate**: `vliw-asm`. **Size**: M. **Depends on**: —.
- Replace `Vec<String>` operands with an enum: `Reg(u8)`, `Pred(u8)`,
  `Imm(i64)`, `MemAddr { base: u8, offset: i64 }`, `Label(String)`.
- Keep a `Display` impl that produces today's text. Emitter calls
  `format!("{operand}")` instead of joining strings.
- **Done when**: existing emitter tests pass; new unit test asserts every
  operand variant prints in a form the simulator parser accepts.

### ASM-3 — Opcode and slot-legality table
- **Crate**: `vliw-asm`. **Size**: M. **Depends on**: ASM-2.
- Encode each opcode the contract supports as a typed `Opcode` enum (or
  `&'static OpcodeInfo`) with: mnemonic, allowed unit kinds, latency,
  whether it writes a GPR / predicate / memory.
- Used by ASM-4, SCHED-1, SCHED-2, OPT-1.
- **Done when**: lookup `Opcode::Add.units()` returns `[IntegerAlu]`,
  `Opcode::Mul.latency()` returns 3, etc., for the full set listed in
  `compiler_contract.md`.

### ASM-4 — In-bundle hazard verifier
- **Crate**: `vliw-asm`. **Size**: M. **Depends on**: ASM-3.
- Pure-Rust reimplementation of the local subset of `vliw_verify`:
  - syllable in a slot whose declared units allow its opcode
  - no two writers of the same GPR in one bundle (WAW)
  - no in-bundle RAW between syllables
- **Done when**: feeding handcrafted bad bundles returns specific error
  variants; feeding the canonical demo program returns `Ok(())`.

### ASM-5 — Parser (round-trip)
- **Crate**: `vliw-asm`. **Size**: L. **Depends on**: ASM-1, ASM-2.
- Parse `.vliw` text back into `Program`. Used to load reference outputs
  in golden tests without shelling out to the simulator.
- **Done when**: for each generated fixture, `parse(emit(p))` is structurally
  equal to `p` (proptest or a small fixture suite).

---

## Track B — VLIW MIR (machine IR) inside `vliw-backend`

A typed IR sits between LLVM IR and bundle emission. ISel produces it,
register mapping rewrites it, the scheduler consumes it. Doing this track
first unblocks most instruction selection work in track C and the scheduling
track E.

### MIR-1 — Define MIR types
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: ASM-2, ASM-3.
- `Function { name, blocks }`, `Block { label, syllables, terminator }`,
  `Syllable { opcode, dst, srcs }` where regs are `VReg(u32)` (virtual)
  pre-allocation and `PReg(u8)` post-allocation. Terminators are
  `Branch(cond, label)`, `Jump(label)`, `Return`.
- **Done when**: unit tests construct a 2-block function manually and the
  pretty printer (MIR-2) round-trips it.

### MIR-2 — MIR pretty printer
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: MIR-1.
- Plain-text dump. Wired up behind `--emit-mir` (DRV-3).
- **Done when**: dumping the canonical demo MIR matches a checked-in
  golden file.

### MIR-3 — Liveness analysis on MIR
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: MIR-1.
- Per-block live-in / live-out / use-def sets. Iterate to fixpoint.
- **Done when**: a small fixture function with two blocks produces the
  expected live sets (test asserts the bitsets directly).

---

## Track C — LLVM IR ingestion and instruction selection

Requires `inkwell`, so depends on the LLVM 19 dev libraries being
installed. Parsing and optimization can start immediately; instruction
selection depends on MIR-1 from track B.

### LLVM-1 — Parse `.ll` to an inkwell `Module`
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: —.
- Replace the `NotImplemented` body of `compile` with a real parse; produce
  a clear `CompileError::Parse` on failure with the inkwell diagnostic.
- **Done when**: feeding `examples/simple.c`'s `clang-19 -emit-llvm -S -O1`
  output reaches the post-parse codegen hand-off; bad IR returns
  `CompileError::Parse` with the LLVM diagnostic.

### LLVM-0 — Enforce the initial supported subset
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: LLVM-1.
- Walk the parsed module before ISel and reject unsupported surface area
  explicitly: more than one lowered function, calls, unsupported integer
  widths, unsupported atomics, and `phi` until LLVM-7 lands.
- **Done when**: each unsupported feature has a fixture that returns a
  dedicated `UnsupportedFeature` error instead of panicking or silently
  generating bad code.

### LLVM-2 — Pre-codegen LLVM opt pipeline
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: LLVM-1.
- Run the standard `O1` (or selectable) pass pipeline on the parsed module
  before lowering, via inkwell's `PassManager`.
- **Done when**: a function containing `add i32 %x, 0` is reduced to its
  identity input before the ISel hand-off.

### LLVM-3 — ISel: integer ALU ops
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: MIR-1, LLVM-1.
- `add`, `sub`, `and`, `or`, `xor`, `shl`, `lshr`, `ashr`. Constants are
  materialized via `movi` (immediate) syllables.
- **Done when**: each opcode covered by a `.ll` fixture compiles to MIR
  whose only syllables are the corresponding VLIW ops + `movi` for
  constants.

### LLVM-4 — ISel: comparisons and conditional branches
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: LLVM-3.
- `icmp eq/ne/slt/sgt/ult/ugt` lowered to predicate-producing ops; `br i1`
  lowered to a predicated branch terminator.
- **Done when**: a fixture `if (x < y) return 1; else return 0;` produces
  exactly one cmp, one conditional branch, two return paths.

### LLVM-5 — ISel: load, store, GEP
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: LLVM-3.
- For now: only flat `i8*`/`i32*`/`i64*` loads and stores; GEP folds into
  `[base + imm]` operands when the index is constant.
- **Done when**: `examples/simple.c` lowers to an `std [r0 + 0x100], rN`
  syllable preceded by the constant materialization of `42`.

### LLVM-6 — ISel: returns
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: LLVM-3, REGS-1.
- `ret void` and `ret i32 N`. Returns go in the `X` (control) slot. The
  return value, if any, lands in `RETVAL_REG` from REGS-1.
- **Done when**: `int main() { return 7; }` produces a single
  `movi RETVAL_REG, 7` followed by a `ret`.

### LLVM-7 — Phi lowering
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: LLVM-3, MIR-3.
- Insert `mov` syllables in predecessor blocks before each phi destination.
  Order them so swap-and-cycle phis don't clobber. Done as a separate MIR
  pass; ISel can leave phis alone.
- **Done when**: a loop fixture (LLVM-8) with an induction variable
  compiles correctly and the simulator reports the right loop trip count.

### LLVM-8 — Loop fixture compiles
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: LLVM-3, LLVM-4,
  LLVM-7.
- A `for (int i = 0; i < 10; ++i) sum += i;` C fixture lowered through the
  whole pipeline.
- **Done when**: simulator reports `sum == 45` in the conventional output
  register or memory location asserted by the test.

### LLVM-9 — ISel: multiply
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: LLVM-3.
- Lower LLVM integer `mul` to the contract's multiplier opcode so the
  latency path in SCHED-1 and SCHED-2 has a real frontend fixture.
- **Done when**: a `.ll` fixture with `mul i32` lowers to MIR containing
  `Opcode::Mul`, and TEST-6 can be expressed as a normal compiler fixture.

---

## Track D — Register allocation (trivial first)

### REGS-1 — Reserved register policy
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: MIR-1.
- Centralize the policy: `r0` is hardware zero, `r31` is link, `r1` is the
  return-value convention. Expose as constants used by ISel and the
  allocator.
- **Done when**: a single module owns the constants; ISel uses them by
  name, not by literal `0` / `31`.

### REGS-2 — Trivial SSA → physical register mapper
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: MIR-3, REGS-1.
- Walk MIR in program order. Assign each fresh `VReg` the next free `PReg`
  in `[2, 30]`. Free a register when its last use is past. No spilling.
  Hard error if free-list empties.
- **Done when**: fixtures with ≤ 29 simultaneously-live values compile;
  one fixture with 40 live values fails with the dedicated
  `OutOfRegisters` error.

### REGS-3 — Liveness-aware reuse (bonus, if free)
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: REGS-2, MIR-3.
- Same algorithm but uses the proper live-out sets from MIR-3 instead of
  "last textual use". Catches the case where a value is dead before its
  last syntactic occurrence (e.g. dead branch).
- **Done when**: a fixture with a live-but-unreferenced value reuses its
  register before the value naturally ends.

---

## Track E — Scalar scheduler and bundle emission

### SCHED-1 — Latency table
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: ASM-3.
- `LatencyTable::result_ready_after(opcode)` returns the cycle delta from
  issue to ready. Source of truth: `compiler_contract.md`.
- **Done when**: a unit test asserts `add` = 1, `mul` = 3, `ld` = whatever
  the contract says.

### SCHED-2 — Scalar scheduler: one syllable per bundle
- **Crate**: `vliw-backend`. **Size**: M. **Depends on**: MIR-1, REGS-2,
  SCHED-1, ASM-1.
- Walk MIR. For each syllable, emit a bundle with that syllable in the
  first compatible slot and `nop` elsewhere. Insert all-`nop` filler
  bundles so every source register is ready (per ready-cycle table).
- **Done when**: a mul-then-dependent-store scheduler fixture emits the
  required all-`nop` latency bundles; once TEST-6 exists, it passes
  `vliw_verify`.

### SCHED-3 — Branch and label emission
- **Crate**: `vliw-backend`. **Size**: S. **Depends on**: SCHED-2.
- Block labels become `Item::Label`. Terminators emit jump/branch
  syllables in the `X` slot.
- **Done when**: TEST-4 (branch fixture) executes the right path in the
  simulator.

### SCHED-4 — Honor the `--schedule=scalar` flag end-to-end
- **Crate**: `vliwc`, `vliw-backend`. **Size**: S. **Depends on**: SCHED-2.
- Plumb the flag from CLI through `compile()`. Default is `scalar`.
- **Done when**: `vliwc --schedule=scalar examples/simple.ll` prints
  at most one non-`nop` syllable per bundle, using the plan's canonical
  slots: ALU in `I0`, memory in `M`, control and multiply in `X`.

---

## Track F — Driver and dev ergonomics

### DRV-1 — Promote `--demo` to `--emit=demo`
- **Crate**: `vliwc`. **Size**: S. **Depends on**: —.
- Cosmetic, but it removes a special case from `main`.
- **Done when**: `--demo` still works as an alias; `--emit=demo` is the
  documented form.

### DRV-2 — `--schedule {scalar,pack}` flag
- **Crate**: `vliwc`. **Size**: S. **Depends on**: —.
- Wires through to `compile()` later. Default `scalar`. Accepts `pack` but
  errors with `not yet implemented` until OPT-1 lands.
- **Done when**: `vliwc --help` lists the flag; `--schedule=pack` returns
  a clean error.

### DRV-3 — `--emit-mir` debug dump
- **Crate**: `vliwc`, `vliw-backend`. **Size**: S. **Depends on**: MIR-2.
- Skips bundle emission and prints MIR instead. Useful for ISel
  development.
- **Done when**: a fixture `.ll` produces a stable text MIR dump.

### DRV-4 — `--emit-bundles` (already the default) keep clean
- **Crate**: `vliwc`. **Size**: S. **Depends on**: —.
- Just make sure the default path stays the bundle emitter, and the doc
  string says so explicitly.
- **Done when**: `vliwc --help` documents the default bundle output path
  and omitting `--emit-*` still writes `.vliw` bundle text.

---

## Track G — Verification and test harness

This track unblocks every other track: nothing else can claim "done"
without passing through it.

### TEST-1 — Cargo-driven end-to-end harness
- **Crate**: `tests/` (workspace integration crate). **Size**: M.
  **Depends on**: —.
- A `#[test]` runner that, for each fixture:
  1. compiles the `.ll` to `.vliw` via `vliwc`
  2. invokes `vliw_verify` (path from env var, e.g.
     `VLIW_VERIFY_BIN`)
  3. invokes `vliw_simulator` (path from env var) capturing trace
  4. asserts a memory-or-register expected result from the fixture's
     sidecar `.expect` file
- **Done when**: the harness runs zero fixtures successfully (the hookup
  itself works), and skips cleanly when the env vars are unset.

### TEST-2 — Fixture: constant return
- **Size**: S. **Depends on**: TEST-1, LLVM-6, REGS-2, SCHED-2.
- `int main() { return 7; }`.
- **Done when**: simulator reports return register == 7.

### TEST-3 — Fixture: constant store
- **Size**: S. **Depends on**: TEST-1, LLVM-5, LLVM-6, REGS-2, SCHED-2.
- `examples/simple.c`. Asserts memory at `0x100` equals 42.
- **Done when**: simulator memory at `0x100` equals 42 and the program
  returns through the normal return path.

### TEST-4 — Fixture: branch
- **Size**: S. **Depends on**: TEST-1, LLVM-4, LLVM-6, REGS-2, SCHED-3.
- A function that takes `argc`-style input (or a constant) and selects one
  of two return values. Both branches covered by separate test cases.
- **Done when**: both branch directions pass `vliw_verify` and the simulator
  reports the expected return value for each case.

### TEST-5 — Fixture: loop
- **Size**: S. **Depends on**: TEST-1, LLVM-7, LLVM-8, REGS-2, SCHED-3.
- Sum 0..10 = 45.
- **Done when**: the compiled loop passes `vliw_verify` and the simulator
  reports 45 in the fixture's expected register or memory location.

### TEST-6 — Fixture: mul with dependent use
- **Size**: S. **Depends on**: TEST-1, LLVM-9, REGS-2, SCHED-1, SCHED-2.
- Designed to fail without correct latency-nop insertion: `r3 = r1 * r2;
  std [addr], r3` with no padding will read `r3` before it's ready and
  trip the verifier even if the simulator stalls. Confirms SCHED-2 is
  doing its job.
- **Done when**: the generated scalar output contains the required latency
  padding, passes `vliw_verify`, and stores the multiplied value.

### TEST-7 — Golden bundle outputs
- **Size**: S. **Depends on**: TEST-2, TEST-3, TEST-4, TEST-5, TEST-6.
- For each fixture, check in the expected `.vliw` text. Test diffs against
  it. Catches accidental schedule changes during refactors.
- **Done when**: changing scalar output for any fixture produces a readable
  golden diff unless the checked-in expectation is intentionally updated.

### CI-1 — GitHub Actions: build + unit tests
- **Crate**: `.github/workflows`. **Size**: S. **Depends on**: —.
- LLVM 19 install + `cargo build` + `cargo test`. No simulator required.
- **Done when**: the workflow runs on pull requests and fails on Rust
  compile errors or unit test failures.

### CI-2 — GitHub Actions: simulator round-trip
- **Crate**: `.github/workflows`. **Size**: M. **Depends on**: TEST-1, CI-1.
- Build LwirSimulator's debug binaries, expose paths via env vars, run
  the integration harness. Remove the temporary `continue-on-error` from
  `c-flow.yml` once TEST-2/TEST-3 are green.
- **Done when**: simulator-backed fixture failures fail CI, and TEST-2 plus
  TEST-3 no longer rely on `continue-on-error`.

---

## Track H — Optimization (gated on scalar baseline green)

Do not start any task in this track until TEST-1 through TEST-6 pass.

### OPT-1 — Bundle packer (post-scalar)
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: SCHED-2, ASM-4,
  TEST-1, TEST-2, TEST-3, TEST-4, TEST-5, TEST-6.
- Greedy pass over the scalar bundle stream: merge consecutive bundles
  when slot legality allows and ASM-4 reports no in-bundle RAW/WAW.
- Preserve control-flow boundaries — never merge across a label or
  terminator.
- Wired up behind `--schedule=pack` (DRV-2).
- **Done when**: every existing fixture produces fewer-or-equal bundles
  than the scalar baseline and still passes the same expected-result
  assertions and `vliw_verify`.

### OPT-2 — Local list scheduler (intra-block)
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: OPT-1, SCHED-1.
- Build a per-block dependence DAG. Schedule by ready-cycle priority,
  height, or critical-path heuristic. Treat all memory ops as ordering
  barriers initially.
- **Done when**: TEST-6 (mul + dependent use) emits without filler bundles
  by reordering an independent op into the latency window.

### OPT-3 — Cycle-count regression bench
- **Crate**: `tests/`. **Size**: M. **Depends on**: TEST-7, OPT-1.
- For every fixture, record cycle counts from the simulator under
  `scalar` and `pack`. Fail CI if `pack` regresses past `scalar`.
- **Done when**: CI reports scalar and pack cycle counts per fixture and
  rejects any pack result with a higher cycle count than scalar.

### OPT-4 — Memory dependence relaxation
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: OPT-2.
- Simple alias buckets: distinguish stack-frame, global, and unknown
  pointers. Allow reordering only when buckets are disjoint.
- **Done when**: a fixture with disjoint stack/global memory ops reorders
  successfully, while an unknown-alias fixture preserves scalar ordering.

---

## Track I — Documentation and architecture

### DOC-1 — Fill out `architecture.md` open questions
- **Size**: M. **Depends on**: at least MIR-1 + LLVM-1 + REGS-2 done so the
  answers are real, not speculative.
- Resolve: header sourcing, ISel approach taken, allocator taken, scheduling
  model taken, predicate strategy, calling convention, memory ordering.
- **Done when**: every open question in `architecture.md` has either a
  settled answer or an explicitly deferred task ID.

### DOC-2 — MIR design note
- **Size**: S. **Depends on**: MIR-1.
- One short page in `docs/` describing MIR types, invariants
  (pre/post regalloc), and how it maps to bundles.
- **Done when**: the note includes a minimal MIR example and names the
  invariants checked before register allocation, after register allocation,
  and before scheduling.

### DOC-3 — Scalar scheduler invariants
- **Size**: S. **Depends on**: SCHED-2.
- Document the rule the scalar baseline guarantees so the optimization
  track has a written oracle to match.
- **Done when**: the doc states the one-syllable-per-bundle rule, the
  ready-cycle padding rule, and the observable-result contract used by
  `pack`.

---

## Track J — Later feature gates

These are intentionally parked behind the scalar compiler and local
optimization work. They exist so section 10 of the dev plan is visible in
the backlog without distracting the baseline bring-up.

### LATER-1 — Real register allocator and spilling
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: REGS-2, TEST-7.
- Replace the no-spill mapper with a real allocator once the scalar oracle
  exists. Spills must lower to explicit memory syllables and remain
  scheduler-visible.
- **Done when**: a fixture with more live values than physical registers
  compiles, passes `vliw_verify`, and preserves the scalar expected result.

### LATER-2 — Calls and calling convention
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: REGS-1, LLVM-6,
  SCHED-3.
- Define argument, return, clobber, and link-register rules; lower direct
  calls after the return path is stable.
- **Done when**: a two-function `.ll` fixture calls a helper, returns its
  result through `RETVAL_REG`, and preserves `r31` behavior.

### LATER-3 — Wider target layouts end-to-end
- **Crate**: `vliw-asm`, `vliw-backend`, `vliwc`. **Size**: L.
  **Depends on**: ASM-1, SCHED-2, OPT-1.
- Source processor layout from a real config path or CLI option, then make
  scheduling and packing target that layout instead of assuming the
  canonical four slots.
- **Done when**: the same fixture compiles for canonical 4-slot and a
  contrived 8-slot layout, both outputs pass `vliw_verify`, and `pack`
  exploits the wider legal slots.

### LATER-4 — Predication and inter-block scheduling
- **Crate**: `vliw-backend`. **Size**: L. **Depends on**: OPT-2, LLVM-4.
- Add predicate-aware hazards, then allow scheduling across basic-block
  boundaries only where labels, branches, and predicate lifetimes remain
  valid.
- **Done when**: a branch-heavy fixture produces fewer cycles than local
  scheduling alone and still matches the scalar oracle.

### LATER-5 — Multi-CPU and acquire/release lowering
- **Crate**: `vliw-backend`, `tests/`. **Size**: L. **Depends on**: TEST-7,
  LATER-3.
- Lower LLVM atomics to `acqload` / `relstore` patterns with explicit CPU
  topology and bus-slot legality from the contract.
- **Done when**: a two-CPU producer/consumer fixture verifies that every
  polling `acqload` has a matching producer `relstore` and the simulator
  observes the expected memory result.

---

## Suggested first-week ordering

If picking up this plan from scratch and parallelizing across two or three
people, a sensible first wave is:

- **Independent immediately**: ASM-1, ASM-2, ASM-3, LLVM-1, LLVM-2, DRV-1,
  DRV-2, CI-1.
- **Unlocked by ASM-3**: ASM-4, SCHED-1.
- **Unlocked by ASM-2 + ASM-3**: MIR-1.
- **Unlocked by LLVM-1**: LLVM-0.
- **Unlocked by MIR-1**: MIR-2, MIR-3, REGS-1, LLVM-3..6, LLVM-9.
- **First end-to-end milestone**: SCHED-2 + REGS-2 + LLVM-3 + LLVM-5 +
  LLVM-6 + TEST-1 + TEST-2 + TEST-3 — at this point `examples/simple.c`
  compiles and runs in the simulator.
