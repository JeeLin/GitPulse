//! Configuration type definitions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Persistent GitPulse configuration stored in `~/.gitpulse/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories scanned for git repositories at startup.
    ///
    /// Encoded entry by entry so non-UTF-8 paths persist too: UTF-8 paths are
    /// plain TOML strings (the original format), anything else uses the
    /// additive `{ path_hex = .. }` marker documented in the config encoding
    /// module.
    #[serde(with = "crate::config::encoding")]
    pub scan_dirs: Vec<PathBuf>,
    /// Display groups for the repositories found under [`Config::scan_dirs`].
    ///
    /// Absent in every config written before v0.8.0, which is why it needs no
    /// key of its own: the container-level `#[serde(default)]` already turns
    /// "missing key" into an empty list, so an old file loads byte-for-byte
    /// the way it always did.
    pub scan_groups: Vec<GroupDef>,
}

/// One named group of repository paths, presented as a collapsible section in
/// the dashboard (sub-task 5).
///
/// A group owns an ordered list of **directory prefixes** rather than the
/// repositories themselves: which repositories exist is the scanner's
/// answer, and re-stating it here would only add a way for the config to
/// disagree with the filesystem. Order is meaningful — it is the order the
/// groups are presented in — and the list may carry duplicate names, because
/// two groups are legitimately allowed to share a display name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDef {
    /// Display name shown on the group's header row.
    ///
    /// Free-form: §6.1 gives the header no length or character rules, so the
    /// config layer imposes none either (§5.4's width budget is the
    /// presenter's job at render time).
    pub name: String,
    /// Absolute directory paths whose repositories belong to this group.
    ///
    /// Encoded exactly like [`Config::scan_dirs`] — one entry per path, the
    /// plain-string form for UTF-8 and the `{ path_hex = .. }` marker
    /// otherwise — so a non-UTF-8 prefix survives a save/load round trip
    /// through the same, single channel rather than a second one.
    #[serde(with = "crate::config::encoding", default = "Vec::default")]
    pub prefixes: Vec<PathBuf>,
}

#[allow(dead_code)] // Reached through `group_for_path`; see its note.
impl GroupDef {
    /// The longest of this group's prefixes that covers `path`.
    ///
    /// Longest-prefix-wins is what makes a nested layout groupable at all: a
    /// repository under both `work` and `work/client` belongs to the more
    /// specific one (checkpoint ⑥). Unusable prefixes are dropped here, so a
    /// group carrying one bad entry still claims the repositories its usable
    /// prefixes cover.
    fn best_prefix(&self, path: &Path) -> Option<usize> {
        self.prefixes
            .iter()
            .filter(|prefix| Self::is_usable_prefix(prefix))
            .filter(|prefix| path_starts_with(path, prefix))
            .map(|prefix| prefix.components().count())
            .max()
    }

    /// Whether a prefix is usable as a group root at all.
    ///
    /// An empty or relative prefix would match every repository relative to
    /// the process's working directory — a grouping that silently depends on
    /// where the binary was launched. Such an entry is dropped at resolve time
    /// rather than rejecting the whole config (checkpoint ⑥).
    fn is_usable_prefix(prefix: &Path) -> bool {
        prefix.is_absolute() && prefix.components().count() > 0
    }
}

/// Component-wise ancestor test: `path` is `prefix` or sits under it.
///
/// [`Path::starts_with`] already compares whole components, which is exactly
/// the rule needed; this exists as a named seam so the grouping semantics
/// live in one readable place instead of at every call site.
#[allow(dead_code)] // Reached through `group_for_path`; see its note.
fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    path.starts_with(prefix)
}

/// The group a repository at `path` belongs to, or `None` when no group
/// claims it (sub-task 5, checkpoint ⑥).
///
/// `pub` so sub-task 6's view can resolve a group without reaching through
/// `App`; currently reached only from [`crate::app::App::group_of`], which the
/// dashboard starts calling when grouping is drawn.
///
/// Resolution is **longest prefix wins**, because that is the only rule under
/// which nested directories can be grouped sensibly. On an exact tie — two
/// groups offering the same prefix length for the same path — the group
/// listed first in [`Config::scan_groups`] wins, so the answer never depends
/// on hash or iteration order the config does not already fix.
///
/// A repository claimed by no group is *not* an error: the config only adds
/// presentation, so an ungrouped repository still appears, just ungrouped.
pub fn group_for_path<'a>(groups: &'a [GroupDef], path: &Path) -> Option<&'a GroupDef> {
    groups
        .iter()
        .filter_map(|group| group.best_prefix(path).map(|len| (len, group)))
        // `max_by_key` keeps the *last* of equal maxima, so the index breaks
        // the tie: scanning in config order and keeping the first candidate of
        // a given length is what makes "first listed wins" true.
        .fold(
            None,
            |best: Option<(usize, &'a GroupDef)>, (len, group)| match best {
                Some((best_len, _)) if best_len >= len => best,
                _ => Some((len, group)),
            },
        )
        .map(|(_, group)| group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, prefixes: &[&str]) -> GroupDef {
        GroupDef {
            name: name.to_string(),
            prefixes: prefixes.iter().map(PathBuf::from).collect(),
        }
    }

    /// The longest matching prefix wins, so a nested directory can be carved
    /// out of a broader group without editing the broader one.
    #[test]
    fn the_longest_matching_prefix_wins() {
        let groups = vec![
            group("work", &["/srv/work"]),
            group("client", &["/srv/work/client"]),
        ];
        assert_eq!(
            group_for_path(&groups, Path::new("/srv/work/client/api")).map(|g| g.name.as_str()),
            Some("client")
        );
        assert_eq!(
            group_for_path(&groups, Path::new("/srv/work/api")).map(|g| g.name.as_str()),
            Some("work")
        );
    }

    /// Matching is component-wise: a prefix is a directory, so a longer name
    /// that merely *starts with* the same text is outside the group.
    #[test]
    fn a_prefix_never_matches_a_longer_sibling_name() {
        let groups = vec![group("bc", &["/a/bc"])];
        assert_eq!(
            group_for_path(&groups, Path::new("/a/bcd")).map(|g| g.name.as_str()),
            None,
            "/a/bcd is a different directory, not a child of /a/bc"
        );
        assert_eq!(
            group_for_path(&groups, Path::new("/a/bc")).map(|g| g.name.as_str()),
            Some("bc")
        );
    }

    /// Two groups may legitimately offer the same prefix; the config's own
    /// order breaks the tie so the result is reproducible rather than
    /// order-dependent.
    #[test]
    fn equal_length_prefixes_resolve_to_the_first_group() {
        let groups = vec![
            group("first", &["/srv/shared"]),
            group("second", &["/srv/shared"]),
        ];
        assert_eq!(
            group_for_path(&groups, Path::new("/srv/shared/repo")).map(|g| g.name.as_str()),
            Some("first")
        );
    }

    /// A relative or empty prefix would make grouping depend on the working
    /// directory the binary was launched from, so it is ignored rather than
    /// silently capturing unrelated repositories.
    #[test]
    fn unusable_prefixes_claim_nothing() {
        for prefix in ["", "relative/dir", "."] {
            let groups = vec![group("bad", &[prefix])];
            assert_eq!(
                group_for_path(&groups, Path::new("/anywhere/repo")),
                None,
                "prefix {prefix:?} must not claim a repository"
            );
        }
    }

    /// An unclaimed repository is a normal state, not a failure: grouping only
    /// adds presentation.
    #[test]
    fn an_unclaimed_repository_resolves_to_no_group() {
        let groups = vec![group("work", &["/srv/work"])];
        assert!(group_for_path(&groups, Path::new("/home/user/repo")).is_none());
        assert!(group_for_path(&[], Path::new("/home/user/repo")).is_none());
    }

    /// One bad entry does not disqualify the group: a group whose *other*
    /// prefixes are usable still claims the repositories they cover.
    #[test]
    fn a_bad_prefix_does_not_disqualify_its_siblings() {
        let groups = vec![group("work", &["relative", "/srv/work"])];
        assert_eq!(
            group_for_path(&groups, Path::new("/srv/work/repo")).map(|g| g.name.as_str()),
            Some("work")
        );
    }

    /// §6.1 renders groups in the order the config lists them, which is why
    /// the schema is a sequence rather than a map (checkpoint ⑥).
    #[test]
    fn group_order_is_the_config_order() {
        let groups = [group("first", &["/a"]), group("second", &["/b"])];
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["first", "second"]);
    }
}
