//! End-to-end harness: `.ll` → `vliwc` → `vliw_verify` → `vliw_simulator` → assert.
//!
//! **Skip conditions** (test returns early with a diagnostic):
//!   - `VLIW_VERIFY_BIN` env var not set
//!   - `VLIW_SIMULATOR_BIN` env var not set
//!   - `vliwc` binary not found (build it first, or set `VLIWC_BIN`)
//!
//! **Fixture discovery**: walks `crates/vliw-backend/tests/fixtures/` recursively
//! and collects every `.ll` that has a sibling `.expect` file.  With none
//! present the test passes trivially after reporting "ran 0 fixture(s)".
//!
//! **`.expect` file format** (UTF-8, `#` = comment):
//! ```text
//! # register assertion
//! r1 = 42
//! # memory assertion (hex or decimal address; optional byte width defaults to 8)
//! mem[0x100] = 42
//! mem[0x100:4] = 42
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

// ---------------------------------------------------------------------------
// Binary location helpers
// ---------------------------------------------------------------------------

/// Return the path to the `vliwc` binary.
///
/// Precedence:
/// 1. `VLIWC_BIN` env var (explicit override)
/// 2. Next to the test binary in `target/{debug,release}/`
///
/// Returns `None` when the binary cannot be found.
fn vliwc_bin() -> Option<PathBuf> {
    if let Ok(p) = env::var("VLIWC_BIN") {
        return Some(PathBuf::from(p));
    }

    // The test binary lives at `target/<profile>/deps/<name>-<hash>`.
    // `vliwc` lives at `target/<profile>/vliwc`.
    let mut path = env::current_exe().ok()?;
    path.pop(); // remove test-binary filename
    if path.file_name().map_or(false, |n| n == "deps") {
        path.pop(); // step out of deps/
    }
    let bin = if cfg!(windows) { "vliwc.exe" } else { "vliwc" };
    path.push(bin);
    path.exists().then_some(path)
}

// ---------------------------------------------------------------------------
// Fixture discovery
// ---------------------------------------------------------------------------

/// Recursively collect `(ll_path, expect_path)` pairs from `dir`.
///
/// Only `.ll` files with a sibling `.expect` file are included.
fn collect_fixtures(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by(|a, b| a.path().cmp(&b.path())); // deterministic order

    let mut out = vec![];
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_fixtures(&path));
        } else if path.extension().map_or(false, |e| e == "ll") {
            let expect = path.with_extension("expect");
            if expect.exists() {
                out.push((path, expect));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Expect-file parsing
// ---------------------------------------------------------------------------

enum Assertion {
    Register { reg: u32, value: i64 },
    Memory { addr: u64, width: usize, value: i64 },
}

#[derive(Clone, Copy)]
enum E2eSchedule {
    Scalar,
    Pack,
}

impl E2eSchedule {
    fn as_str(self) -> &'static str {
        match self {
            E2eSchedule::Scalar => "scalar",
            E2eSchedule::Pack => "pack",
        }
    }
}

fn parse_expect(text: &str) -> Vec<Assertion> {
    let mut out = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let lhs = line[..eq].trim();
        let rhs = line[eq + 1..].trim();
        let value: i64 = rhs
            .parse()
            .unwrap_or_else(|_| panic!("bad numeric value in expect line: {line:?}"));

        if let Some(n) = lhs.strip_prefix('r') {
            let reg: u32 = n
                .parse()
                .unwrap_or_else(|_| panic!("bad register number in expect line: {line:?}"));
            out.push(Assertion::Register { reg, value });
        } else if let Some(inner) = lhs.strip_prefix("mem[").and_then(|s| s.strip_suffix(']')) {
            let (addr_s, width) = parse_mem_query(inner)
                .unwrap_or_else(|| panic!("bad memory address in expect line: {line:?}"));
            let addr = parse_u64(addr_s)
                .unwrap_or_else(|| panic!("bad memory address in expect line: {line:?}"));
            out.push(Assertion::Memory { addr, width, value });
        }
    }
    out
}

fn parse_mem_query(s: &str) -> Option<(&str, usize)> {
    match s.split_once(':') {
        Some((addr, width_s)) => {
            let width = width_s.parse().ok()?;
            Some((addr, width))
        }
        None => Some((s, 8)),
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

// ---------------------------------------------------------------------------
// Assertion checking
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DumpState {
    regs: HashMap<u32, u64>,
    mem: HashMap<(u64, usize), u64>,
}

fn parse_dump_output(text: &str, fixture: &Path) -> DumpState {
    let mut state = DumpState::default();

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("reg ") {
            let mut parts = rest.split_whitespace();
            let reg_s = parts
                .next()
                .unwrap_or_else(|| panic!("{}: bad dump reg line: {line}", fixture.display()));
            let value_s = parts
                .find_map(|p| p.strip_prefix("value="))
                .unwrap_or_else(|| panic!("{}: bad dump reg line: {line}", fixture.display()));

            let reg = reg_s
                .strip_prefix('r')
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("{}: bad dump reg line: {line}", fixture.display()));
            let value = parse_u64(value_s)
                .unwrap_or_else(|| panic!("{}: bad dump reg value: {line}", fixture.display()));
            state.regs.insert(reg, value);
        } else if let Some(rest) = line.strip_prefix("mem ") {
            let mut addr = None;
            let mut width = None;
            let mut value = None;
            let mut in_bounds = None;

            for part in rest.split_whitespace() {
                if let Some(v) = part.strip_prefix("addr=") {
                    addr = parse_u64(v);
                } else if let Some(v) = part.strip_prefix("width=") {
                    width = v.parse().ok();
                } else if let Some(v) = part.strip_prefix("value=") {
                    value = parse_u64(v);
                } else if let Some(v) = part.strip_prefix("in_bounds=") {
                    in_bounds = Some(v == "true");
                }
            }

            let addr =
                addr.unwrap_or_else(|| panic!("{}: bad dump mem line: {line}", fixture.display()));
            let width =
                width.unwrap_or_else(|| panic!("{}: bad dump mem line: {line}", fixture.display()));
            let value =
                value.unwrap_or_else(|| panic!("{}: bad dump mem line: {line}", fixture.display()));
            assert_eq!(
                in_bounds,
                Some(true),
                "{}: memory dump was out of bounds: {line}",
                fixture.display()
            );
            state.mem.insert((addr, width), value);
        }
    }

    state
}

fn expected_bits(value: i64, width_bytes: usize) -> u64 {
    let bits = width_bytes * 8;
    if bits >= 64 {
        value as u64
    } else {
        (value as u64) & ((1u64 << bits) - 1)
    }
}

fn check_assertions(assertions: &[Assertion], dump: &str, fixture: &Path) {
    let state = parse_dump_output(dump, fixture);

    for a in assertions {
        match a {
            Assertion::Register { reg, value } => {
                let got = state.regs.get(reg).copied().unwrap_or_else(|| {
                    panic!(
                        "{}: missing dump for r{reg}\n--- dump ---\n{dump}",
                        fixture.display()
                    )
                });
                assert_eq!(
                    got,
                    expected_bits(*value, 8),
                    "{}: expected r{reg} = {value}, got {got:#x}\n--- dump ---\n{dump}",
                    fixture.display()
                );
            }
            Assertion::Memory { addr, width, value } => {
                let got = state.mem.get(&(*addr, *width)).copied().unwrap_or_else(|| {
                    panic!(
                        "{}: missing dump for mem[0x{addr:x}:{width}]\n--- dump ---\n{dump}",
                        fixture.display()
                    )
                });
                assert_eq!(
                    got,
                    expected_bits(*value, *width),
                    "{}: expected mem[0x{addr:x}:{width}] = {value}, got {got:#x}\n--- dump ---\n{dump}",
                    fixture.display()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-fixture runner
// ---------------------------------------------------------------------------

fn run_fixture(
    ll: &Path,
    expect: &Path,
    vliwc: &Path,
    verify: &Path,
    sim: &Path,
    tmp: &Path,
    schedule: E2eSchedule,
) -> u64 {
    // Unique output name inside CARGO_TARGET_TMPDIR to avoid collisions.
    let stem = ll.file_stem().unwrap_or_default().to_string_lossy();
    let vliw_out = tmp.join(format!("{stem}.{}.vliw", schedule.as_str()));

    // 1. Compile .ll → .vliw
    let out = Command::new(vliwc)
        .arg(format!("--schedule={}", schedule.as_str()))
        .arg(ll)
        .arg("-o")
        .arg(&vliw_out)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn vliwc ({}): {e}", vliwc.display()));
    assert!(
        out.status.success(),
        "vliwc failed on {}:\nstdout: {}\nstderr: {}",
        ll.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 2. Verify .vliw
    let out = Command::new(verify)
        .arg(&vliw_out)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn vliw_verify ({}): {e}", verify.display()));
    assert!(
        out.status.success(),
        "vliw_verify failed on {}:\nstdout: {}\nstderr: {}",
        vliw_out.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 3. Simulate and dump exactly the final architectural state requested
    // by the fixture expectations. Dump mode includes zero-valued registers.
    let expect_text =
        fs::read_to_string(expect).unwrap_or_else(|e| panic!("reading {}: {e}", expect.display()));
    let assertions = parse_expect(&expect_text);

    let mut sim_cmd = Command::new(sim);
    for assertion in &assertions {
        match assertion {
            Assertion::Register { reg, .. } => {
                sim_cmd.arg("--dump-reg").arg(format!("r{reg}"));
            }
            Assertion::Memory { addr, width, .. } => {
                sim_cmd.arg("--dump-mem").arg(format!("0x{addr:x}:{width}"));
            }
        }
    }

    let out = sim_cmd
        .arg(&vliw_out)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn vliw_simulator ({}): {e}", sim.display()));
    assert!(
        out.status.success(),
        "vliw_simulator failed on {}:\nstdout: {}\nstderr: {}",
        vliw_out.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 4. Assert expectations
    let dump = String::from_utf8_lossy(&out.stdout);
    check_assertions(&assertions, &dump, ll);

    // 5. Report the simulator cycle count for OPT-3 pack-vs-scalar regression checks.
    let out = Command::new(sim)
        .arg("--json")
        .arg(&vliw_out)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn vliw_simulator ({}): {e}", sim.display()));
    assert!(
        out.status.success(),
        "vliw_simulator --json failed on {}:\nstdout: {}\nstderr: {}",
        vliw_out.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    parse_json_cycle(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or_else(|| panic!("{}: simulator JSON did not include cycle", ll.display()))
}

fn parse_json_cycle(json: &str) -> Option<u64> {
    for line in json.lines() {
        let line = line.trim();
        if let Some((_, rest)) = line.split_once("\"cycle\"") {
            let rest = rest.trim_start().strip_prefix(':')?.trim();
            let value: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return value.parse().ok();
        }
    }
    None
}

#[test]
fn parses_pretty_json_cycle() {
    let json = r#"{
  "format": "vliw-sim-final-state-v1",
  "cycle": 7,
  "halted": true
}"#;

    assert_eq!(parse_json_cycle(json), Some(7));
}

#[test]
fn parses_inline_json_cycle() {
    let json = r#"{"format":"vliw-sim-final-state-v1","cycle":12,"halted":true}"#;

    assert_eq!(parse_json_cycle(json), Some(12));
}

// ---------------------------------------------------------------------------
// Test entry point
// ---------------------------------------------------------------------------

#[test]
fn e2e_fixtures() {
    // --- Skip guards ------------------------------------------------------ //

    let verify_bin = match env::var("VLIW_VERIFY_BIN") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            eprintln!("VLIW_VERIFY_BIN not set — skipping e2e tests");
            return;
        }
    };

    let sim_bin = match env::var("VLIW_SIMULATOR_BIN") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            eprintln!("VLIW_SIMULATOR_BIN not set — skipping e2e tests");
            return;
        }
    };

    let vliwc = match vliwc_bin() {
        Some(p) => p,
        None => {
            eprintln!(
                "vliwc binary not found — build the workspace first or set VLIWC_BIN; \
                 skipping e2e tests"
            );
            return;
        }
    };

    // --- Fixture discovery ------------------------------------------------ //

    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root from CARGO_MANIFEST_DIR")
        .join("crates/vliw-backend/tests/fixtures");

    let fixtures = collect_fixtures(&fixtures_dir);

    let tmp = env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::temp_dir());

    // --- Run -------------------------------------------------------------- //

    for (ll, expect) in &fixtures {
        let scalar_cycles = run_fixture(
            ll,
            expect,
            &vliwc,
            &verify_bin,
            &sim_bin,
            &tmp,
            E2eSchedule::Scalar,
        );
        let pack_cycles = run_fixture(
            ll,
            expect,
            &vliwc,
            &verify_bin,
            &sim_bin,
            &tmp,
            E2eSchedule::Pack,
        );
        eprintln!(
            "e2e cycles {} scalar={} pack={}",
            ll.display(),
            scalar_cycles,
            pack_cycles
        );
        assert!(
            pack_cycles <= scalar_cycles,
            "{}: pack schedule regressed cycle count: scalar={scalar_cycles}, pack={pack_cycles}",
            ll.display()
        );
    }

    eprintln!("e2e: ran {} fixture(s)", fixtures.len());
}
