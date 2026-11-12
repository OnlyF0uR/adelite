//! Version numbers, as the releases are tagged: `v0.2.0`.

use std::fmt;

/// A release's version: major, minor, patch. Compares as numbers, so `0.10.0` is newer than `0.9.9`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// The version of this build.
    #[must_use]
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self(0, 0, 0))
    }

    /// Reads `1.2.3` or `v1.2.3`. `None` for anything else, a pre-release (`1.2.3-rc.1`) included.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
        let version = Self(number(parts.next())?, number(parts.next())?, number(parts.next())?);
        parts.next().is_none().then_some(version)
    }
}

/// One part of a version: digits only.
fn number(part: Option<&str>) -> Option<u32> {
    let part = part?;
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_tags() {
        assert_eq!(Version::parse("v0.2.0"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse(" 1.10.3 "), Some(Version(1, 10, 3)));
        for wrong in ["", "v", "1.2", "1.2.3.4", "1.2.3-rc.1", "1.+2.3", "1..3", "x1.2.3", "v1.2.3 beta"] {
            assert_eq!(Version::parse(wrong), None, "{wrong}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert!(Version(0, 2, 1) > Version(0, 2, 0));
        assert_eq!(Version(0, 2, 0).to_string(), "0.2.0");
    }
}
