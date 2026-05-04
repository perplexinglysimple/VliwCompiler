//! `vliwc` -- VLIW compiler driver.
//!
//! By default, reads an LLVM IR file and writes `.vliw` bundle text to the
//! output path (or stdout).  Pass `--emit=demo` for a hard-coded canonical
//! example, or `--emit-mir` for a plain-text MIR dump.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ScheduleArg {
    Scalar,
    Pack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum EmitArg {
    /// Compile INPUT and write `.vliw` bundle text (the default).
    Bundles,
    /// Emit the hard-coded canonical-example `.vliw` program; INPUT is ignored.
    Demo,
}

impl From<ScheduleArg> for vliw_backend::Schedule {
    fn from(value: ScheduleArg) -> Self {
        match value {
            ScheduleArg::Scalar => vliw_backend::Schedule::Scalar,
            ScheduleArg::Pack => vliw_backend::Schedule::Pack,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "vliwc", about = "VLIW compiler driver", version)]
struct Args {
    /// Input LLVM IR file (.ll). Ignored when --emit=demo is set.
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// Output `.vliw` bundle file. Omitted or `-` writes to stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// What to emit: `bundles` (default) compiles INPUT and writes `.vliw` bundle text;
    /// `demo` emits a hard-coded canonical example (INPUT is ignored).
    #[arg(long, value_enum, default_value_t = EmitArg::Bundles, value_name = "KIND")]
    emit: EmitArg,

    /// Alias for --emit=demo.
    #[arg(long, hide = true)]
    demo: bool,

    /// Emit a plain-text MIR dump instead of a bundle stream.
    #[arg(long)]
    emit_mir: bool,

    /// Scheduler to use for bundle emission.
    #[arg(long, value_enum, default_value_t = ScheduleArg::Scalar)]
    schedule: ScheduleArg,

    /// Processor configuration file (.vliw). The `.processor { }` block is
    /// parsed from this file and used instead of the canonical 4-slot default.
    #[arg(long, value_name = "CONFIG")]
    processor: Option<PathBuf>,
}

impl Args {
    fn effective_emit(&self) -> EmitArg {
        if self.demo {
            EmitArg::Demo
        } else {
            self.emit
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    validate_args(&args)?;

    let vliw_text = match (args.effective_emit(), args.emit_mir) {
        (EmitArg::Demo, true) => vliw_backend::emit_mir(&vliw_backend::demo_mir()),
        (EmitArg::Demo, false) => emit_demo()?,
        (EmitArg::Bundles, true) => {
            let path = args
                .input
                .as_ref()
                .context("INPUT is required with --emit-mir")?;
            let ir =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            vliw_backend::compile_to_mir(&ir, vliw_backend::OptimizationLevel::None)
                .map(|mir| vliw_backend::emit_mir(&mir))
                .map_err(anyhow::Error::from)?
        }
        (EmitArg::Bundles, false) => {
            let path = args
                .input
                .as_ref()
                .context("INPUT is required unless --emit=demo is set")?;
            let ir =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let processor = load_processor(args.processor.as_deref())?;
            vliw_backend::compile_for_processor(
                &ir,
                vliw_backend::OptimizationLevel::Less,
                args.schedule.into(),
                processor,
            )
            .map_err(anyhow::Error::from)?
        }
    };

    match args.output {
        None => {
            io::stdout().write_all(vliw_text.as_bytes())?;
        }
        Some(p) => {
            fs::write(&p, vliw_text).with_context(|| format!("writing {}", p.display()))?;
        }
    }
    Ok(())
}

fn validate_args(args: &Args) -> Result<()> {
    let _ = args;
    Ok(())
}

/// Load a processor configuration from a `.vliw` config file, or return the
/// canonical 4-slot default when no path is given.
fn load_processor(config_path: Option<&std::path::Path>) -> Result<vliw_asm::Processor> {
    match config_path {
        None => Ok(vliw_asm::Processor::default()),
        Some(path) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("reading processor config {}", path.display()))?;
            vliw_asm::parse(&text)
                .map(|prog| prog.processor)
                .map_err(|e| anyhow::anyhow!("processor config parse error in {}: {e}", path.display()))
        }
    }
}

/// Hard-coded canonical example (see vliw_asm_format.md in LwirSimulator)
/// so we can smoke-test the emitter against the simulator's parser today.
fn emit_demo() -> Result<String> {
    use vliw_asm::{Bundle, Item, Operand, Processor, Program, Syllable};

    fn syl(op: &str, ops: Vec<Operand>) -> Option<Syllable> {
        Some(Syllable::new(op, ops))
    }

    let program = Program {
        processor: Processor::default(),
        items: vec![
            Item::Label("entry".into()),
            Item::Bundle(Bundle {
                slots: vec![
                    syl("movi", vec![Operand::Reg(1), Operand::Imm(6)]),
                    syl("movi", vec![Operand::Reg(2), Operand::Imm(7)]),
                    None,
                    None,
                ],
            }),
            Item::Bundle(Bundle {
                slots: vec![
                    None,
                    None,
                    None,
                    syl(
                        "mul",
                        vec![Operand::Reg(3), Operand::Reg(1), Operand::Reg(2)],
                    ),
                ],
            }),
            // Two filler bundles cover MUL's 3-cycle latency so r3 is
            // scoreboard-ready at the std below (compiler_contract.md rule 6).
            Item::Bundle(Bundle::default()),
            Item::Bundle(Bundle::default()),
            Item::Bundle(Bundle {
                slots: vec![
                    None,
                    None,
                    syl(
                        "std",
                        vec![
                            Operand::MemAddr {
                                base: 0,
                                offset: 0x100,
                            },
                            Operand::Reg(3),
                        ],
                    ),
                    syl("ret", vec![]),
                ],
            }),
        ],
    };

    Ok(vliw_asm::emit(&program)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn emit_demo_selects_demo_mode() {
        let args = Args::try_parse_from(["vliwc", "--emit=demo"]).unwrap();

        assert_eq!(args.effective_emit(), EmitArg::Demo);
        assert!(!args.demo);
    }

    #[test]
    fn demo_flag_remains_an_alias() {
        let args = Args::try_parse_from(["vliwc", "--demo"]).unwrap();

        assert_eq!(args.effective_emit(), EmitArg::Demo);
        assert!(args.demo);
    }

    #[test]
    fn no_emit_flag_defaults_to_bundles() {
        let args = Args::try_parse_from(["vliwc", "input.ll"]).unwrap();

        assert_eq!(args.effective_emit(), EmitArg::Bundles);
    }

    #[test]
    fn help_documents_emit_demo_not_demo_alias() {
        let help = Args::command().render_help().to_string();

        assert!(help.contains("--emit <KIND>"));
        assert!(!help.contains("--demo"));
        assert!(help.contains("bundles"));
    }

    #[test]
    fn help_documents_default_bundle_emit() {
        let help = Args::command().render_help().to_string();

        assert!(help.contains("[default: bundles]"));
        assert!(help.contains(".vliw"));
    }

    #[test]
    fn help_documents_schedule_choices() {
        let help = Args::command().render_help().to_string();

        assert!(help.contains("--schedule <SCHEDULE>"));
        assert!(help.contains("[default: scalar]"));
        assert!(help.contains("[possible values: scalar, pack]"));
    }

    #[test]
    fn pack_schedule_is_accepted() {
        let args = Args::try_parse_from(["vliwc", "--schedule=pack"]).unwrap();

        validate_args(&args).unwrap();
        assert_eq!(args.schedule, ScheduleArg::Pack);
    }

    #[test]
    fn processor_flag_is_accepted() {
        let args =
            Args::try_parse_from(["vliwc", "--processor", "wide8.vliw", "input.ll"]).unwrap();

        assert_eq!(args.processor.as_deref(), Some(std::path::Path::new("wide8.vliw")));
    }

    #[test]
    fn load_processor_default_is_4slot() {
        let proc = load_processor(None).expect("default processor should load");

        assert_eq!(proc.width, 4);
    }

    #[test]
    fn load_processor_from_wide8_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../vliw-backend/tests/fixtures/wide8.vliw");
        let proc = load_processor(Some(&path)).expect("wide8.vliw should load");

        assert_eq!(proc.width, 8);
        assert!(
            proc.slot_aliases.iter().any(|a| a.name == "X"),
            "wide8 processor should have an X slot alias"
        );
    }
}
