//! Versions as tags: `1.4.2` (and `v1.4.2`), the next one after the newest
//! (`--bump`), and what a range means among a repository's tags (`1`,
//! `1.4`, `^1.4`, `~1.4.2`). Plain semver, without build metadata (a hub tag
//! can't hold `+`).

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// `rc.1` in `2.0.0-rc.1`; empty for a release.
    pub pre: String,
    /// Whether it was written `v1.4.2`.
    pub v: bool,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let (v, rest) = match s.strip_prefix('v') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (core, pre) = match rest.split_once('-') {
            Some((c, p)) if !p.is_empty() && p.split('.').all(|x| !x.is_empty()) => (c, p.to_string()),
            Some(_) => return None,
            None => (rest, String::new()),
        };
        let nums: Vec<&str> = core.split('.').collect();
        if nums.len() != 3 {
            return None;
        }
        let num = |x: &str| -> Option<u64> {
            if x.is_empty() || (x.len() > 1 && x.starts_with('0')) || !x.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                x.parse().ok()
            }
        };
        Some(Version { major: num(nums[0])?, minor: num(nums[1])?, patch: num(nums[2])?, pre, v })
    }

    pub fn is_release(&self) -> bool {
        self.pre.is_empty()
    }

    /// The next version: patch, minor or major.
    pub fn bump(&self, part: &str) -> Option<Version> {
        let mut n = Version { pre: String::new(), ..self.clone() };
        match part {
            "patch" => {
                // (1.4.0-rc.1 bumped is 1.4.0, as npm does.)
                if self.pre.is_empty() {
                    n.patch += 1;
                }
            }
            // (1.5.0-rc.1 bumped minor is 1.5.0; 2.0.0-rc.1 bumped major, 2.0.0.)
            "minor" if !self.pre.is_empty() && self.patch == 0 => {}
            "minor" => {
                n.minor += 1;
                n.patch = 0;
            }
            "major" if !self.pre.is_empty() && self.minor == 0 && self.patch == 0 => {}
            "major" => {
                n.major += 1;
                n.minor = 0;
                n.patch = 0;
            }
            _ => return None,
        }
        Some(n)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = if self.v { "v" } else { "" };
        write!(f, "{v}{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| {
            match (self.pre.is_empty(), o.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => cmp_pre(&self.pre, &o.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Pre-release identifiers, as semver orders them: numbers by value and
/// before words; more identifiers after fewer.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// The newest release among these tags.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Version> {
    tags.into_iter().filter_map(Version::parse).filter(Version::is_release).max()
}

/// A range of versions, as a tag that isn't one: `1`, `1.4`, `^1.4`,
/// `~1.4.2`, `v1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    kind: char,
    nums: Vec<u64>,
}

impl Range {
    pub fn parse(s: &str) -> Option<Range> {
        let (kind, rest) = match s.chars().next()? {
            '^' => ('^', &s[1..]),
            '~' => ('~', &s[1..]),
            _ => ('=', s),
        };
        let rest = rest.strip_prefix('v').unwrap_or(rest);
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.is_empty() || parts.len() > 3 || (kind == '=' && parts.len() == 3) {
            return None; // (x.y.z is a tag, not a range)
        }
        let nums: Option<Vec<u64>> = parts
            .iter()
            .map(|p| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None })
            .collect();
        Some(Range { kind, nums: nums? })
    }

    pub fn matches(&self, v: &Version) -> bool {
        if !v.is_release() {
            return false;
        }
        let have = [v.major, v.minor, v.patch];
        let at = |i: usize| self.nums.get(i).copied().unwrap_or(0);
        let floor = Version { major: at(0), minor: at(1), patch: at(2), pre: String::new(), v: false };
        match self.kind {
            // 1.4: every 1.4.x; 1: every 1.x.y.
            '=' => self.nums.iter().enumerate().all(|(i, n)| have[i] == *n),
            // ~1.4.2: 1.4.x from 1.4.2; ~1: 1.x.y.
            '~' => {
                let fixed = if self.nums.len() == 1 { 1 } else { 2 };
                *v >= floor && (0..fixed).all(|i| have[i] == self.nums[i])
            }
            // ^1.4: 1.x.y from 1.4.0; ^0.4: 0.4.x (the first number that isn't 0 is fixed).
            _ => {
                let fixed = self.nums.iter().position(|n| *n != 0).map(|i| i + 1).unwrap_or(self.nums.len());
                *v >= floor && (0..fixed.min(3)).all(|i| have[i] == at(i))
            }
        }
    }

    /// The newest of these tags in the range, and its tag as written.
    pub fn pick<'a>(&self, tags: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
        tags.into_iter()
            .filter_map(|t| Version::parse(t).filter(|v| self.matches(v)).map(|v| (v, t)))
            .max()
            .map(|(_, t)| t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_order() {
        let v = |s| Version::parse(s).unwrap();
        assert_eq!(v("1.4.2").to_string(), "1.4.2");
        assert_eq!(v("v2.0.0-rc.1").to_string(), "v2.0.0-rc.1");
        for bad in ["1.4", "1.4.2.1", "01.2.3", "1.2.x", "1.2.3-", "latest", ""] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
        assert!(v("1.10.0") > v("1.9.9"));
        assert!(v("2.0.0") > v("2.0.0-rc.2"));
        assert!(v("2.0.0-rc.10") > v("2.0.0-rc.2"));
        assert!(v("2.0.0-rc.1") > v("2.0.0-beta.9"));
        assert_eq!(newest(["latest", "1.2.0", "1.10.1", "2.0.0-rc.1", "staging"]).unwrap().to_string(), "1.10.1");
    }

    #[test]
    fn bumps() {
        let b = |s: &str, p| Version::parse(s).unwrap().bump(p).unwrap().to_string();
        assert_eq!(b("1.4.2", "patch"), "1.4.3");
        assert_eq!(b("1.4.2", "minor"), "1.5.0");
        assert_eq!(b("1.4.2", "major"), "2.0.0");
        assert_eq!(b("v0.9.9", "minor"), "v0.10.0");
        assert_eq!(b("2.0.0-rc.1", "major"), "2.0.0");
        assert_eq!(b("1.5.0-rc.1", "minor"), "1.5.0");
        assert!(Version::parse("1.0.0").unwrap().bump("huge").is_none());
    }

    #[test]
    fn ranges() {
        let tags = ["latest", "0.4.1", "0.4.9", "0.5.0", "1.3.0", "1.4.0", "1.4.7", "1.10.2", "2.0.0-rc.1", "v3.1.0"];
        let pick = |r: &str| Range::parse(r).and_then(|r| r.pick(tags));
        assert_eq!(pick("1"), Some("1.10.2"));
        assert_eq!(pick("1.4"), Some("1.4.7"));
        assert_eq!(pick("^1.4"), Some("1.10.2"));
        assert_eq!(pick("~1.4"), Some("1.4.7"));
        assert_eq!(pick("~1.4.5"), Some("1.4.7"));
        assert_eq!(pick("^1.11"), None);
        assert_eq!(pick("^0.4"), Some("0.4.9"));
        assert_eq!(pick("2"), None); // (pre-releases only when asked for by name)
        assert_eq!(pick("3"), Some("v3.1.0"));
        assert_eq!(pick("v1.4"), Some("1.4.7"));
        assert!(Range::parse("1.4.2").is_none());
        assert!(Range::parse("latest").is_none());
        assert!(Range::parse("1.x").is_none());
    }
}
