//! Семантические версии и диапазоны, как в npm: `1.2.3`, `^1.2.0`, `~1.2.0`,
//! `>=1.0.0 <2.0.0`, `1.x`, `*`, `1.2.3 - 2.0.0`, `^1.0.0 || ^2.0.0`.
//! Предрелизы (`1.0.0-бета.1`) подходят, только если диапазон называет
//! предрелиз той же версии.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Result<Version, String> {
        let s = s.trim().trim_start_matches('v').trim_start_matches('=');
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.split('.').map(|x| x.to_string()).collect()),
            None => (s.split('+').next().unwrap_or(s), vec![]),
        };
        let nums: Vec<&str> = core.split('.').collect();
        if nums.len() != 3 {
            return Err(format!("версия «{s}» должна быть вида 1.2.3"));
        }
        let n = |x: &str| x.parse::<u64>().map_err(|_| format!("версия «{s}»: «{x}» — не число"));
        Ok(Version { major: n(nums[0])?, minor: n(nums[1])?, patch: n(nums[2])?, pre })
    }

    fn triple(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        self.triple().cmp(&o.triple()).then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(o.pre.iter()) {
                    let c = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        _ => a.cmp(b),
                    };
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                self.pre.len().cmp(&o.pre.len())
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq)]
struct Cmp {
    op: Op,
    v: Version,
}

impl Cmp {
    fn test(&self, v: &Version) -> bool {
        let c = v.cmp(&self.v);
        match self.op {
            Op::Eq => c == Ordering::Equal,
            Op::Gt => c == Ordering::Greater,
            Op::Ge => c != Ordering::Less,
            Op::Lt => c == Ordering::Less,
            Op::Le => c != Ordering::Greater,
        }
    }
}

/// Диапазон версий: наборы условий через `||`, условия в наборе — через пробел.
#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    sets: Vec<Vec<Cmp>>,
}

fn v3(a: u64, b: u64, c: u64) -> Version {
    Version { major: a, minor: b, patch: c, pre: vec![] }
}

/// Неполная версия: `1`, `1.2`, `1.x`, `*` → (числа, сколько задано).
fn partial(s: &str) -> Result<(Vec<u64>, Vec<String>), String> {
    let s = s.trim().trim_start_matches('v');
    if s.is_empty() || s == "*" || s == "x" || s == "X" {
        return Ok((vec![], vec![]));
    }
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, p.split('.').map(|x| x.to_string()).collect()),
        None => (s, vec![]),
    };
    let mut nums = vec![];
    for part in core.split('.') {
        if part == "x" || part == "X" || part == "*" {
            break;
        }
        nums.push(part.parse::<u64>().map_err(|_| format!("«{s}» — не версия"))?);
    }
    if nums.len() > 3 {
        return Err(format!("«{s}» — не версия"));
    }
    Ok((nums, pre))
}

fn full(nums: &[u64], pre: Vec<String>) -> Version {
    Version { major: nums.first().copied().unwrap_or(0), minor: nums.get(1).copied().unwrap_or(0), patch: nums.get(2).copied().unwrap_or(0), pre }
}

fn comparator(tok: &str) -> Result<Vec<Cmp>, String> {
    let (op, rest) = if let Some(r) = tok.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = tok.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = tok.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = tok.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = tok.strip_prefix('^') {
        ("^", r)
    } else if let Some(r) = tok.strip_prefix('~') {
        ("~", r.trim_start_matches('>'))
    } else if let Some(r) = tok.strip_prefix('=') {
        ("=", r)
    } else {
        ("", tok)
    };
    let (n, pre) = partial(rest)?;
    let c = |op, v| Cmp { op, v };
    Ok(match op {
        "" | "=" => match n.len() {
            0 => vec![c(Op::Ge, v3(0, 0, 0))],
            1 => vec![c(Op::Ge, v3(n[0], 0, 0)), c(Op::Lt, v3(n[0] + 1, 0, 0))],
            2 => vec![c(Op::Ge, v3(n[0], n[1], 0)), c(Op::Lt, v3(n[0], n[1] + 1, 0))],
            _ => vec![c(Op::Eq, full(&n, pre))],
        },
        "^" => {
            let lo = full(&n, pre);
            let hi = match (n.len(), n.first(), n.get(1)) {
                (_, Some(&m), _) if m > 0 => v3(m + 1, 0, 0),
                (1, _, _) => v3(1, 0, 0),
                (_, _, Some(&mi)) if mi > 0 => v3(0, mi + 1, 0),
                (2, _, _) => v3(0, 1, 0),
                (_, _, _) if n.len() == 3 => v3(0, 0, n[2] + 1),
                _ => v3(u64::MAX, 0, 0),
            };
            vec![c(Op::Ge, lo), c(Op::Lt, hi)]
        }
        "~" => {
            let lo = full(&n, pre);
            let hi = match n.len() {
                0 => v3(u64::MAX, 0, 0),
                1 => v3(n[0] + 1, 0, 0),
                _ => v3(n[0], n[1] + 1, 0),
            };
            vec![c(Op::Ge, lo), c(Op::Lt, hi)]
        }
        ">=" => vec![c(Op::Ge, full(&n, pre))],
        ">" => match n.len() {
            0 => vec![c(Op::Lt, v3(0, 0, 0))],
            1 => vec![c(Op::Ge, v3(n[0] + 1, 0, 0))],
            2 => vec![c(Op::Ge, v3(n[0], n[1] + 1, 0))],
            _ => vec![c(Op::Gt, full(&n, pre))],
        },
        "<" => vec![c(Op::Lt, full(&n, pre))],
        "<=" => match n.len() {
            0 => vec![c(Op::Ge, v3(0, 0, 0))],
            1 => vec![c(Op::Lt, v3(n[0] + 1, 0, 0))],
            2 => vec![c(Op::Lt, v3(n[0], n[1] + 1, 0))],
            _ => vec![c(Op::Le, full(&n, pre))],
        },
        _ => unreachable!(),
    })
}

impl Range {
    pub fn parse(s: &str) -> Result<Range, String> {
        let mut sets = vec![];
        for alt in s.split("||") {
            let alt = alt.trim();
            let mut set = vec![];
            if let Some((a, b)) = alt.split_once(" - ") {
                // 1.2.3 - 2.3.4
                let (lo, lp) = partial(a)?;
                let (hi, hp) = partial(b)?;
                set.push(Cmp { op: Op::Ge, v: full(&lo, lp) });
                set.extend(comparator(&format!("<={}", if hi.len() == 3 { full(&hi, hp).to_string() } else { b.trim().to_string() }))?);
            } else {
                // «>= 1.0» допускается с пробелом после знака
                let mut toks: Vec<String> = vec![];
                for t in alt.split_whitespace() {
                    if let Some(last) = toks.last_mut() {
                        if matches!(last.as_str(), ">" | ">=" | "<" | "<=" | "^" | "~" | "=") {
                            last.push_str(t);
                            continue;
                        }
                    }
                    toks.push(t.to_string());
                }
                if toks.is_empty() {
                    toks.push("*".into());
                }
                for t in toks {
                    set.extend(comparator(&t)?);
                }
            }
            sets.push(set);
        }
        Ok(Range { sets })
    }

    pub fn matches(&self, v: &Version) -> bool {
        self.sets.iter().any(|set| {
            set.iter().all(|c| c.test(v))
                // предрелиз — только если диапазон явно называет предрелиз той же версии
                && (v.pre.is_empty() || set.iter().any(|c| !c.v.pre.is_empty() && c.v.triple() == v.triple()))
        })
    }

    /// Наибольшая подходящая версия из списка.
    pub fn best<'a>(&self, versions: impl IntoIterator<Item = &'a str>) -> Option<String> {
        versions
            .into_iter()
            .filter_map(|s| Version::parse(s).ok().map(|v| (v, s)))
            .filter(|(v, _)| self.matches(v))
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, s)| s.to_string())
    }
}
