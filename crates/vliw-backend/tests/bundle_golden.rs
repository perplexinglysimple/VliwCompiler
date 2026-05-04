/// Golden-bundle tests: each fixture is compiled to VLIW text and diffed against
/// a checked-in `.vliw` file.  Run with `UPDATE_GOLDENS=1` to regenerate golden files.
use std::path::{Path, PathBuf};

use vliw_backend::{compile, OptimizationLevel, Schedule};

struct Fixture {
    name: &'static str,
    ir: &'static str,
    opt: OptimizationLevel,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "return_7",
        ir: include_str!("fixtures/ret/return_7.ll"),
        opt: OptimizationLevel::None,
    },
    Fixture {
        name: "simple_store",
        ir: include_str!("fixtures/mem/simple_store.ll"),
        opt: OptimizationLevel::Less,
    },
    Fixture {
        name: "branch_taken",
        ir: include_str!("fixtures/branch/branch_taken.ll"),
        opt: OptimizationLevel::Less,
    },
    Fixture {
        name: "branch_not_taken",
        ir: include_str!("fixtures/branch/branch_not_taken.ll"),
        opt: OptimizationLevel::Less,
    },
    Fixture {
        name: "loop_sum_10",
        ir: include_str!("fixtures/loop_sum_10.ll"),
        opt: OptimizationLevel::Less,
    },
    Fixture {
        name: "mul_dependent_store",
        ir: include_str!("fixtures/mul_dependent_store.ll"),
        opt: OptimizationLevel::None,
    },
];

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(format!("{name}.vliw"))
}

fn unified_diff(golden_path: &Path, golden: &str, got: &str) -> String {
    let golden_lines: Vec<&str> = golden.lines().collect();
    let got_lines: Vec<&str> = got.lines().collect();

    let mut out = String::new();
    out.push_str(&format!(
        "--- {}\n+++ (compiled)\n",
        golden_path.display()
    ));

    let max = golden_lines.len().max(got_lines.len());
    let mut i = 0;
    while i < max {
        let a = golden_lines.get(i).copied().unwrap_or("");
        let b = got_lines.get(i).copied().unwrap_or("");
        if a != b {
            out.push_str(&format!("-{a}\n+{b}\n"));
        } else {
            out.push_str(&format!(" {a}\n"));
        }
        i += 1;
    }
    out
}

fn run_golden(fixture: &Fixture) {
    let got = compile(fixture.ir, fixture.opt, Schedule::Scalar)
        .unwrap_or_else(|e| panic!("{}: compile failed: {e}", fixture.name));

    let path = golden_path(fixture.name);

    if std::env::var("UPDATE_GOLDENS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        return;
    }

    let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: golden file missing at {}; run with UPDATE_GOLDENS=1 to create it: {e}",
            fixture.name,
            path.display()
        )
    });

    if got != golden {
        panic!(
            "{}: VLIW output differs from golden {}\n\n{}",
            fixture.name,
            path.display(),
            unified_diff(&path, &golden, &got)
        );
    }
}

#[test]
fn golden_return_7() {
    run_golden(&FIXTURES[0]);
}

#[test]
fn golden_simple_store() {
    run_golden(&FIXTURES[1]);
}

#[test]
fn golden_branch_taken() {
    run_golden(&FIXTURES[2]);
}

#[test]
fn golden_branch_not_taken() {
    run_golden(&FIXTURES[3]);
}

#[test]
fn golden_loop_sum_10() {
    run_golden(&FIXTURES[4]);
}

#[test]
fn golden_mul_dependent_store() {
    run_golden(&FIXTURES[5]);
}
