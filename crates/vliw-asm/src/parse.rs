//! Parser for the `.vliw` text format: inverse of [`crate::emit`].

use crate::{
    Bundle, CacheSpec, Item, Operand, Processor, Program, SlotAlias, Syllable, TopologySpec,
    UnitDecl,
};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("parse error at line {line}: {msg}")]
    Syntax { line: usize, msg: String },
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { lines: input.lines().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.pos).copied()
    }

    fn next_line(&mut self) -> Option<&'a str> {
        let line = self.lines.get(self.pos).copied();
        if line.is_some() {
            self.pos += 1;
        }
        line
    }

    fn skip_blank(&mut self) {
        while matches!(self.peek(), Some(l) if l.trim().is_empty()) {
            self.pos += 1;
        }
    }

    fn expect_line(&mut self) -> Result<&'a str, ParseError> {
        self.next_line().ok_or(ParseError::UnexpectedEof)
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError::Syntax { line: self.pos, msg: msg.into() }
    }

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        self.skip_blank();
        let processor = self.parse_processor()?;
        let mut items = Vec::new();

        loop {
            self.skip_blank();
            let Some(line) = self.peek() else { break };
            let trimmed = line.trim();
            if trimmed == "{" {
                items.push(Item::Bundle(self.parse_bundle(&processor)?));
            } else if let Some(label) = trimmed.strip_suffix(':') {
                items.push(Item::Label(label.to_string()));
                self.pos += 1;
            } else {
                return Err(self.err(format!("unexpected: {trimmed}")));
            }
        }

        Ok(Program { processor, items })
    }

    fn parse_processor(&mut self) -> Result<Processor, ParseError> {
        let line = self.expect_line()?;
        if line.trim() != ".processor {" {
            return Err(
                self.err(format!("expected '.processor {{', got: {}", line.trim()))
            );
        }

        let mut width = 4u32;
        let mut units = Vec::new();
        let mut slot_aliases = Vec::new();
        let mut slot_units: Vec<Vec<String>> = Vec::new();
        let cache = CacheSpec {};
        let mut topology = TopologySpec { cpus: 1 };

        loop {
            self.skip_blank();
            let line = self.expect_line()?;
            let trimmed = line.trim();

            if trimmed == "}" {
                break;
            } else if let Some(rest) = trimmed.strip_prefix("width ") {
                width = rest.trim().parse()
                    .map_err(|_| self.err(format!("bad width: {rest}")))?;
            } else if trimmed == "hardware {" {
                units = self.parse_hardware()?;
            } else if trimmed == "layout slots {" {
                (slot_aliases, slot_units) = self.parse_layout()?;
            } else if trimmed.starts_with("cache {") {
                // `cache { }` — no parameters
            } else if trimmed.starts_with("topology {") {
                topology = parse_topology_inline(trimmed)
                    .ok_or_else(|| self.err(format!("bad topology: {trimmed}")))?;
            } else {
                return Err(self.err(format!("unexpected in .processor: {trimmed}")));
            }
        }

        Ok(Processor { width, units, slot_aliases, slot_units, cache, topology })
    }

    fn parse_hardware(&mut self) -> Result<Vec<UnitDecl>, ParseError> {
        let mut units = Vec::new();
        loop {
            self.skip_blank();
            let line = self.expect_line()?;
            let trimmed = line.trim();
            if trimmed == "}" {
                break;
            }
            let rest = trimmed.strip_prefix("unit ")
                .ok_or_else(|| self.err(format!("expected 'unit ...': {trimmed}")))?;
            let (name, kind) = rest.split_once(" = ")
                .ok_or_else(|| self.err(format!("bad unit decl: {trimmed}")))?;
            units.push(UnitDecl {
                name: name.trim().to_string(),
                kind: kind.trim().to_string(),
            });
        }
        Ok(units)
    }

    fn parse_layout(&mut self) -> Result<(Vec<SlotAlias>, Vec<Vec<String>>), ParseError> {
        let mut aliases = Vec::new();
        let mut indexed: Vec<(usize, Vec<String>)> = Vec::new();

        loop {
            self.skip_blank();
            let line = self.expect_line()?;
            let trimmed = line.trim();
            if trimmed == "}" {
                break;
            }

            if let Some(rest) = trimmed.strip_prefix("alias ") {
                let (name, idx_s) = rest.split_once(" = ")
                    .ok_or_else(|| self.err(format!("bad alias: {trimmed}")))?;
                let slot: usize = idx_s.trim().parse()
                    .map_err(|_| self.err(format!("bad alias index: {idx_s}")))?;
                aliases.push(SlotAlias { name: name.trim().to_string(), slot });
            } else {
                // `N = { unit, ... }`
                let (idx_s, rest) = trimmed.split_once(" = ")
                    .ok_or_else(|| self.err(format!("bad slot decl: {trimmed}")))?;
                let slot_idx: usize = idx_s.trim().parse()
                    .map_err(|_| self.err(format!("bad slot index: {idx_s}")))?;
                let inner = rest.trim().trim_start_matches('{').trim_end_matches('}').trim();
                let us: Vec<String> = if inner.is_empty() {
                    Vec::new()
                } else {
                    inner.split(',').map(|s| s.trim().to_string()).collect()
                };
                indexed.push((slot_idx, us));
            }
        }

        indexed.sort_by_key(|(i, _)| *i);
        let slot_units = if indexed.is_empty() {
            Vec::new()
        } else {
            let max_idx = indexed.iter().map(|(i, _)| *i).max().unwrap();
            let mut result = vec![Vec::new(); max_idx + 1];
            for (i, us) in indexed {
                result[i] = us;
            }
            result
        };

        Ok((aliases, slot_units))
    }

    fn parse_bundle(&mut self, proc: &Processor) -> Result<Bundle, ParseError> {
        let _ = self.expect_line()?; // consume `{`
        let mut slots: Vec<Option<Syllable>> = vec![None; proc.width as usize];
        let mut pos_idx = 0usize;

        loop {
            let line = self.expect_line()?;
            let trimmed = line.trim();
            if trimmed == "}" {
                break;
            }

            let colon = trimmed.find(':')
                .ok_or_else(|| self.err(format!("expected ':' in: {trimmed}")))?;
            let slot_name = trimmed[..colon].trim();
            let rest = trimmed[colon + 1..].trim();

            // Prefer alias lookup; fall back to emit-order position for unnamed slots.
            let slot_idx = proc.slot_aliases.iter()
                .find(|a| a.name == slot_name)
                .map(|a| a.slot)
                .unwrap_or(pos_idx);

            let syllable = if rest == "nop" {
                None
            } else {
                Some(parse_syllable(rest)
                    .ok_or_else(|| self.err(format!("bad syllable: {rest}")))?)
            };

            if slot_idx < slots.len() {
                slots[slot_idx] = syllable;
            }
            pos_idx += 1;
        }

        Ok(Bundle { slots })
    }
}

fn parse_topology_inline(trimmed: &str) -> Option<TopologySpec> {
    // `topology { cpus N }`
    let inner = trimmed
        .strip_prefix("topology")?
        .trim()
        .strip_prefix('{')?
        .trim_end_matches('}')
        .trim();
    let rest = inner.strip_prefix("cpus ")?.trim();
    let cpus: u32 = rest.parse().ok()?;
    Some(TopologySpec { cpus })
}

fn parse_syllable(s: &str) -> Option<Syllable> {
    let (opcode, rest) = s.split_once(' ')
        .map(|(op, r)| (op, r.trim()))
        .unwrap_or((s, ""));
    let operands = if rest.is_empty() {
        Vec::new()
    } else {
        parse_operands(rest)?
    };
    Some(Syllable { opcode: opcode.to_string(), operands })
}

fn parse_operands(s: &str) -> Option<Vec<Operand>> {
    let mut operands = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;

    for (i, b) in s.bytes().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => depth -= 1,
            b',' if depth == 0 => {
                operands.push(parse_operand(s[start..i].trim())?);
                start = i + 1;
            }
            _ => {}
        }
    }
    operands.push(parse_operand(s[start..].trim())?);
    Some(operands)
}

fn parse_operand(s: &str) -> Option<Operand> {
    if s.starts_with('[') {
        let inner = s.strip_prefix('[')?.strip_suffix(']')?;
        if let Some((base_s, off_s)) = inner.split_once(" + ") {
            let base = try_parse_reg(base_s.trim())?;
            let offset = parse_hex_or_dec(off_s.trim())?;
            Some(Operand::MemAddr { base, offset })
        } else if let Some((base_s, off_s)) = inner.split_once(" - ") {
            let base = try_parse_reg(base_s.trim())?;
            let abs_off = parse_hex_or_dec(off_s.trim())?;
            Some(Operand::MemAddr { base, offset: -abs_off })
        } else {
            let base = try_parse_reg(inner.trim())?;
            Some(Operand::MemAddr { base, offset: 0 })
        }
    } else if let Some(n) = try_parse_reg(s) {
        Some(Operand::Reg(n))
    } else if let Some(n) = try_parse_pred(s) {
        Some(Operand::Pred(n))
    } else if s.starts_with('-') || s.bytes().next().map_or(false, |b| b.is_ascii_digit()) {
        s.parse::<i64>().ok().map(Operand::Imm)
    } else {
        Some(Operand::Label(s.to_string()))
    }
}

fn try_parse_reg(s: &str) -> Option<u8> {
    s.strip_prefix('r').and_then(|n| n.parse().ok())
}

fn try_parse_pred(s: &str) -> Option<u8> {
    s.strip_prefix('p').and_then(|n| n.parse().ok())
}

fn parse_hex_or_dec(s: &str) -> Option<i64> {
    if let Some(hex) = s.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse `.vliw` text into a [`Program`]. This is the inverse of [`crate::emit`].
pub fn parse(input: &str) -> Result<Program, ParseError> {
    Parser::new(input).parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{emit, Operand::*};

    fn syl(op: &str, ops: Vec<crate::Operand>) -> Option<Syllable> {
        Some(Syllable::new(op, ops))
    }

    fn canonical_program() -> Program {
        Program {
            processor: Processor::default(),
            items: vec![
                Item::Label("entry".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", vec![Reg(1), Imm(6)]),
                        syl("movi", vec![Reg(2), Imm(7)]),
                        None,
                        None,
                    ],
                }),
                Item::Bundle(Bundle {
                    slots: vec![None, None, None, syl("mul", vec![Reg(3), Reg(1), Reg(2)])],
                }),
                Item::Bundle(Bundle {
                    slots: vec![
                        None,
                        None,
                        syl("std", vec![MemAddr { base: 0, offset: 0x100 }, Reg(3)]),
                        syl("ret", vec![]),
                    ],
                }),
            ],
        }
    }

    #[test]
    fn round_trip_canonical() {
        let p = canonical_program();
        let text = emit(&p).unwrap();
        let p2 = parse(&text).expect("parse failed");
        assert_eq!(p, p2);
    }

    #[test]
    fn round_trip_2slot() {
        let proc = Processor {
            width: 2,
            units: vec![
                UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
                UnitDecl { name: "mem".into(), kind: "memory".into() },
            ],
            slot_aliases: vec![
                SlotAlias { name: "I".into(), slot: 0 },
                SlotAlias { name: "M".into(), slot: 1 },
            ],
            slot_units: vec![vec!["alu".into()], vec!["mem".into()]],
            cache: CacheSpec {},
            topology: TopologySpec { cpus: 1 },
        };
        let p = Program {
            processor: proc,
            items: vec![
                Item::Label("start".into()),
                Item::Bundle(Bundle {
                    slots: vec![syl("movi", vec![Reg(1), Imm(1)]), None],
                }),
                Item::Bundle(Bundle {
                    slots: vec![None, syl("std", vec![MemAddr { base: 0, offset: 0 }, Reg(1)])],
                }),
            ],
        };
        let text = emit(&p).unwrap();
        let p2 = parse(&text).expect("parse failed");
        assert_eq!(p, p2);
    }

    #[test]
    fn round_trip_8slot() {
        let proc = Processor {
            width: 8,
            units: vec![
                UnitDecl { name: "alu".into(), kind: "integer_alu".into() },
                UnitDecl { name: "mem".into(), kind: "memory".into() },
                UnitDecl { name: "ctrl".into(), kind: "control".into() },
                UnitDecl { name: "mul".into(), kind: "multiplier".into() },
                UnitDecl { name: "fp".into(), kind: "floating_point".into() },
            ],
            slot_aliases: vec![
                SlotAlias { name: "I0".into(), slot: 0 },
                SlotAlias { name: "I1".into(), slot: 1 },
                SlotAlias { name: "I2".into(), slot: 2 },
                SlotAlias { name: "I3".into(), slot: 3 },
                SlotAlias { name: "M0".into(), slot: 4 },
                SlotAlias { name: "M1".into(), slot: 5 },
                SlotAlias { name: "X".into(), slot: 6 },
                SlotAlias { name: "FP".into(), slot: 7 },
            ],
            slot_units: vec![
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["alu".into()],
                vec!["mem".into()],
                vec!["mem".into()],
                vec!["ctrl".into(), "mul".into()],
                vec!["fp".into()],
            ],
            cache: CacheSpec {},
            topology: TopologySpec { cpus: 2 },
        };
        let p = Program {
            processor: proc,
            items: vec![
                Item::Label("wide".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("movi", vec![Reg(1), Imm(42)]),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        syl("fadd", vec![Label("f0".into()), Label("f1".into()), Label("f2".into())]),
                    ],
                }),
            ],
        };
        let text = emit(&p).unwrap();
        let p2 = parse(&text).expect("parse failed");
        assert_eq!(p, p2);
    }

    #[test]
    fn round_trip_all_operand_kinds() {
        let p = Program {
            processor: Processor::default(),
            items: vec![
                Item::Label("ops".into()),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("add", vec![Reg(0), Reg(31), Reg(5)]),
                        syl("movi", vec![Reg(1), Imm(-1)]),
                        syl("std", vec![MemAddr { base: 2, offset: -8 }, Reg(3)]),
                        syl("br", vec![Label("ops".into())]),
                    ],
                }),
                Item::Bundle(Bundle {
                    slots: vec![
                        syl("cmpeq", vec![Pred(0), Reg(1), Reg(2)]),
                        syl("movi", vec![Reg(4), Imm(0)]),
                        syl("ldd", vec![Reg(5), MemAddr { base: 0, offset: 0x100 }]),
                        None,
                    ],
                }),
            ],
        };
        let text = emit(&p).unwrap();
        let p2 = parse(&text).expect("parse failed");
        assert_eq!(p, p2);
    }

    #[test]
    fn parse_error_on_bad_input() {
        assert!(parse("not a vliw file").is_err());
    }
}
