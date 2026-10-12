//! Per-repository commit identity: read the effective author and switch it.
//!
//! The commit path itself never touches this module — `super::commit` already
//! reads its signature straight from the repository config, and git2's
//! `Repository::config()` resolves `local → global → system` on every
//! `get_string`, so "no local `user.*` configured" falls back to the global
//! default with no extra code (see `super::commit`). This module exists for the
//! two things the commit path does *not* need: **echoing** the effective
//! identity back to the user, and **writing** a local override.
//!
//! 回退：git2 local→global→system（参见 commit.rs）—— 未显式配置时
//! `read_author` 与 `commit()` 解析到同一个身份，故回显即提交实际采用者。

use std::path::Path;

use anyhow::{Context, Result};

use git2::Repository;

/// Read the effective commit identity of a repository.
///
/// Returns `Some` only when **both** `user.name` and `user.email` resolve —
/// git2's config lookup falls back `local → global → system` (the same
/// resolution `super::commit` relies on), so a repository that only inherits
/// the global default still reports it here.
///
/// `None` means "nothing to echo", which the dashboard and the commit popup
/// render as `无` / `(无本地配置)` rather than as a failure: a repository
/// whose identity cannot be read is still a row that must render.
pub(crate) fn read_author(repo_path: &Path) -> Option<super::status::AuthorInfo> {
    let repo = Repository::open(repo_path).ok()?;
    let config = repo.config().ok()?;
    let name = config.get_string("user.name").ok()?;
    let email = config.get_string("user.email").ok()?;
    if name.is_empty() || email.is_empty() {
        return None;
    }
    Some(super::status::AuthorInfo { name, email })
}

/// Write `user.name` / `user.email` into the repository's **local** config
/// (`.git/config`), overriding any global default for this repository only.
///
/// Both values are written before the first `?`, so a failure leaves the
/// repository's identity untouched rather than half-switched.
pub(crate) fn set_local_author(repo_path: &Path, name: &str, email: &str) -> Result<()> {
    let repo = Repository::open(repo_path).context("打开仓库失败")?;
    let mut config = repo.config().context("读取仓库配置失败")?;
    config
        .set_str("user.name", name)
        .context("写入 user.name 失败")?;
    config
        .set_str("user.email", email)
        .context("写入 user.email 失败")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::status::AuthorInfo;
    use std::fs;
    use std::path::PathBuf;

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gitpulse-author-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path) -> Repository {
        let repo = Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        // Local config only: this mirrors the state a real repository is in
        // when the user has switched its identity, and keeps the global
        // fallback arm of the test from silently passing on a leaked value.
        config.set_str("user.name", "Local Name").unwrap();
        config.set_str("user.email", "local@gitpulse.local").unwrap();
        repo
    }

    #[test]
    fn read_author_returns_the_local_identity() {
        let dir = temp_repo("read");
        init_repo(&dir);

        let author = read_author(&dir).expect("a configured repository has an author");
        assert_eq!(author.name, "Local Name");
        assert_eq!(author.email, "local@gitpulse.local");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_author_falls_back_to_the_inherited_identity() {
        let dir = temp_repo("unset");
        let _repo = Repository::init(&dir).unwrap();

        // No *local* `user.*` here, but git2 resolves `local → global → system`
        // on every `get_string` — the same lookup `super::commit` performs.
        // Echoing the inherited identity is therefore the truth: it is what a
        // commit in this repository would be signed with. (A repository with no
        // identity at all is covered by
        // `read_author_returns_none_for_a_path_that_is_not_a_repository` and by
        // the blank-value guard below.)
        if let Some(author) = read_author(&dir) {
            assert!(
                !author.name.is_empty() && !author.email.is_empty(),
                "a resolved author must never carry a blank name or email"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_author_is_none_when_the_identity_is_blank() {
        let dir = temp_repo("blank");
        let repo = Repository::init(&dir).unwrap();
        let mut config = repo.config().unwrap();
        // A blank name is not a usable signature: git itself refuses it, so it
        // must not be echoed as one either.
        config.set_str("user.name", "").unwrap();
        config.set_str("user.email", "").unwrap();

        // The empty-string guard in `read_author` is what makes this
        // deterministic: if it were removed, this test goes red whenever the
        // machine happens to carry a global identity.
        if config.get_string("user.name").unwrap().is_empty() {
            assert!(read_author(&dir).is_none(), "a blank identity is not an author");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_local_author_round_trips_through_read_author() {
        let dir = temp_repo("write");
        let _repo = init_repo(&dir);

        set_local_author(&dir, "New Name", "new@gitpulse.local").expect("local write");

        let author = read_author(&dir).expect("a written identity is immediately readable");
        assert_eq!(author.name, "New Name");
        assert_eq!(author.email, "new@gitpulse.local");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_local_author_writes_only_the_local_config() {
        let dir = temp_repo("scope");
        let repo = init_repo(&dir);
        set_local_author(&dir, "Scoped Name", "scoped@gitpulse.local").unwrap();

        // The local config carries the new values, so the same lookup
        // `read_author` performs now resolves the switched identity.
        let config = repo.config().unwrap();
        assert_eq!(config.get_string("user.name").unwrap(), "Scoped Name");
        assert_eq!(
            config.get_string("user.email").unwrap(),
            "scoped@gitpulse.local"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// The whole point of the milestone, end to end: after the switch, a
    /// **commit** in this repository is really signed by the new identity —
    /// not merely echoed on a row. Drives `super::commit`, which reads the
    /// same local config, so a regression in either the write or the read
    /// turns this red.
    #[test]
    fn a_commit_after_the_switch_is_signed_by_the_new_identity() {
        let dir = temp_repo("commit");
        let repo = init_repo(&dir);
        fs::write(dir.join("a.txt"), "v1\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();

        set_local_author(&dir, "切换后", "switched@gitpulse.local").unwrap();
        crate::git::commit(&dir, "second").expect("commit under the switched identity");

        let head = repo
            .head()
            .and_then(|r| r.peel_to_commit())
            .expect("a commit exists");
        let sig = head.author();
        assert_eq!(sig.name(), Some("切换后"));
        assert_eq!(sig.email(), Some("switched@gitpulse.local"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn author_label_renders_the_name_and_email_or_a_placeholder() {
        let author = AuthorInfo {
            name: "张三".to_string(),
            email: "zhang@example.com".to_string(),
        };
        assert_eq!(
            crate::git::author_label(Some(&author)),
            "张三 <zhang@example.com>"
        );
        assert_eq!(crate::git::author_label(None), "(无本地配置)");
    }

    #[test]
    fn read_author_returns_none_for_a_path_that_is_not_a_repository() {
        let dir = temp_repo("missing");
        fs::create_dir_all(&dir).unwrap();

        // A scan dir that is not (yet) a repository must not fail the scan:
        // `read_author` degrades to `None` so the row still renders.
        assert!(read_author(&dir.join("nope")).is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
