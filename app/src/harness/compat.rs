//! Which Vibe versions this build was tested with, and how the version Vibe
//! reports in its `initialize` answer (`agentInfo.version`) compares.
//!
//! Catwright depends on Vibe's ACP interface and on Vibe's own extension
//! payloads; a Vibe release outside the tested range may change either, so
//! the app says so instead of letting a feature fail silently.

use crate::i18n::trf;

/// Oldest and newest Vibe release this build was tested with. The README
/// names the same range ("Requirements" and "Known limits"); a test keeps
/// both in step.
pub const TESTED_MIN: Version = Version(3, 1, 0);
pub const TESTED_MAX: Version = Version(3, 1, 3);

/// A `major.minor.patch` release number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

impl Version {
    /// The leading `major.minor[.patch]` of a version string. A `v` prefix
    /// and anything after the numbers (`rc1`, `.dev0`, `+build`) are
    /// ignored, so a pre-release counts as its release. Major and minor
    /// must be numbers; a missing patch is 0.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let mut parts = s.split('.');
        let num = |p: Option<&str>| -> Option<u32> {
            let p = p?;
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        };
        let major = num(parts.next())?;
        let minor = num(parts.next())?;
        let patch = num(parts.next()).unwrap_or(0);
        Some(Version(major, minor, patch))
    }
}

/// How the connected Vibe relates to the tested range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VibeCompat {
    /// No usable version reported: nothing to judge, nothing shown.
    Unknown,
    /// Inside the tested range.
    Tested,
    /// Older than the oldest tested release: features may be missing.
    Older(Version),
    /// Newer than the newest tested release: untested.
    Newer(Version),
}

impl VibeCompat {
    /// Compare the version string Vibe reported (`agentInfo.version`).
    pub fn of(reported: &str) -> VibeCompat {
        match Version::parse(reported) {
            None => VibeCompat::Unknown,
            Some(v) if v < TESTED_MIN => VibeCompat::Older(v),
            Some(v) if v > TESTED_MAX => VibeCompat::Newer(v),
            Some(_) => VibeCompat::Tested,
        }
    }

    /// The explanation shown once as a toast, as the sidebar footer's
    /// tooltip and on the settings page; `None` when there is nothing to say.
    pub fn notice(&self) -> Option<String> {
        let (min, max) = (TESTED_MIN.to_string(), TESTED_MAX.to_string());
        match self {
            VibeCompat::Unknown | VibeCompat::Tested => None,
            VibeCompat::Older(v) => Some(trf(
                "Catwright was tested with Vibe {min} to {max}. Vibe {version} is older: some features may be missing, please update Vibe.",
                &[("min", &min), ("max", &max), ("version", &v.to_string())],
            )),
            VibeCompat::Newer(v) => Some(trf(
                "Catwright was tested with Vibe {min} to {max}. Vibe {version} is newer: if something does not work, please report it.",
                &[("min", &min), ("max", &max), ("version", &v.to_string())],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_takes_the_leading_release_numbers() {
        assert_eq!(Version::parse("3.1.0"), Some(Version(3, 1, 0)));
        assert_eq!(Version::parse(" v3.1.3 "), Some(Version(3, 1, 3)));
        assert_eq!(Version::parse("3.2"), Some(Version(3, 2, 0)));
        // pre-releases and builds count as their release
        assert_eq!(Version::parse("3.2.0rc1"), Some(Version(3, 2, 0)));
        assert_eq!(Version::parse("3.2.0.dev0"), Some(Version(3, 2, 0)));
        assert_eq!(Version::parse("3.1.4+abc"), Some(Version(3, 1, 4)));
        assert_eq!(Version::parse("10.0.12"), Some(Version(10, 0, 12)));
    }

    #[test]
    fn parse_refuses_what_is_not_a_version() {
        for s in ["", "3", "abc", "3.x", "x.1.0", ".1.0", "v"] {
            assert_eq!(Version::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn compat_compares_against_the_tested_range() {
        assert_eq!(VibeCompat::of("3.1.0"), VibeCompat::Tested);
        assert_eq!(VibeCompat::of("3.1.2"), VibeCompat::Tested);
        assert_eq!(VibeCompat::of("3.1.3"), VibeCompat::Tested);
        assert_eq!(VibeCompat::of("3.0.9"), VibeCompat::Older(Version(3, 0, 9)));
        assert_eq!(VibeCompat::of("2.9.0"), VibeCompat::Older(Version(2, 9, 0)));
        assert_eq!(VibeCompat::of("3.1.4"), VibeCompat::Newer(Version(3, 1, 4)));
        assert_eq!(
            VibeCompat::of("3.2.0rc1"),
            VibeCompat::Newer(Version(3, 2, 0))
        );
        assert_eq!(VibeCompat::of("4.0.0"), VibeCompat::Newer(Version(4, 0, 0)));
        assert_eq!(VibeCompat::of(""), VibeCompat::Unknown);
        assert_eq!(VibeCompat::of("dev"), VibeCompat::Unknown);
    }

    #[test]
    fn notice_only_outside_the_tested_range_and_names_the_versions() {
        assert_eq!(VibeCompat::Tested.notice(), None);
        assert_eq!(VibeCompat::Unknown.notice(), None);
        let newer = VibeCompat::Newer(Version(3, 2, 0)).notice().unwrap();
        assert!(newer.contains("3.1.0") && newer.contains("3.1.3") && newer.contains("3.2.0"));
        let older = VibeCompat::Older(Version(3, 0, 1)).notice().unwrap();
        assert!(older.contains("3.0.1") && older.contains("update"));
    }

    /// The README states the tested range twice; it must be the one the
    /// app checks against.
    #[test]
    fn readme_names_the_tested_range() {
        let readme = include_str!("../../../README.md");
        let flat = readme.split_whitespace().collect::<Vec<_>>().join(" ");
        let range = format!("Vibe {TESTED_MIN} to {TESTED_MAX}");
        assert_eq!(
            flat.matches(&range).count(),
            2,
            "README must say \"{range}\" under Requirements and Known limits"
        );
    }
}
