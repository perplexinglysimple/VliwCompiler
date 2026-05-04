//! `vliwc` -- VLIW compiler driver.
//!
//! Today: reads an LLVM IR file path and either (a) emits a hard-coded
//! canonical-example `.vliw` program when `--demo` is passed, or (b) calls
//! into `vliw_backend::compile`, which is not yet implemented.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "vliwc", about = "VLIW compiler driver", version)]
struct Args {
    /// Input LLVM IR file (.ll). Ignored when --demo is set.
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// Output .vliw file. `-` or omitted writes to stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Skip codegen and emit the canonical example program from the spec.
    /// Useful for round-tripping against the simulator before the backend
    /// is implemented.
    #[arg(long)]
    demo: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let vliw_text = if args.demo {
        emit_demo()?
    } else {
        let path = args
            .input
            .as_ref()
            .context("INPUT is required unless --demo is set")?;
        let ir = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        vliw_backend::compile(&ir).map_err(anyhow::Error::from)?
    };

    match args.output {
        None => {
            io::stdout().write_all(vliw_text.as_bytes())?;
        }
        Some(p) => {
            fs::write(&p, vliw_text)
                .with_context(|| format!("writing {}", p.display()))?;
        }
    }
    Ok(())
}

/// Hard-coded canonical example (see vliw_asm_format.md in LwirSimulator)
/// so we can smoke-test the emitter against the simulator's parser today.
fn emit_demo() -> Result<String> {
    use vliw_asm::{Bundle, Item, Processor, Program, Syllable};

    fn syl(op: &str, ops: &[&str]) -> Option<Syllable> {
        Some(Syllable::new(op, ops.iter().map(|s| s.to_string())))
    }

    let program = Program {
        processor: Processor { width: 4 },
        items: vec![
            Item::Label("entry".into()),
            Item::Bundle(Bundle {
                slots: vec![syl("movi", &["r1", "6"]), syl("movi", &["r2", "7"]), None, None],
            }),
            Item::Bundle(Bundle {
                slots: vec![None, None, None, syl("mul", &["r3", "r1", "r2"])],
            }),
            // Two filler bundles cover MUL's 3-cycle latency so r3 is
            // scoreboard-ready at the std below (compiler_contract.md rule 6).
            Item::Bundle(Bundle::default()),
            Item::Bundle(Bundle::default()),
            Item::Bundle(Bundle {
                slots: vec![
                    None,
                    None,
                    syl("std", &["[r0 + 0x100]", "r3"]),
                    syl("ret", &[]),
                ],
            }),
        ],
    };

    Ok(vliw_asm::emit(&program)?)
}
