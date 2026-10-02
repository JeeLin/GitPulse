//! Remote operations: push / pull via the git CLI.
//!
//! The git2 push/pull APIs require non-trivial credential callbacks, so these
//! operations shell out to the `git` CLI inside `spawn_blocking` (design
//! checkpoint 4: keep the runtime thread free while the child process runs).
//! `pull` is strict fast-forward only (convention 7): diverged histories fail
//! with an explicit error instead of merging or rebasing.
//!
//! The start message (0%) is pre-set by the App layer, and the completion is
//! shown by the App layer from the task's return value, so these functions
//! only run the git command and report its result.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use git2::Repository;

use super::show_bytes;

/// Resolve the remote to operate on: `origin` when present, else the first
/// configured remote; errors when the repository has no remote at all.
///
/// A remote name starting with `-` is refused (G28): git config accepts such
/// a name, and it would then be handed to `git push`/`git pull` as a
/// positional argument where git reads it as an option
/// (`--upload-pack=…`-style injection).
fn resolve_remote(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let remotes = repo.remotes()?;
    // `StringArray::get` yields `None` for a name that is not valid UTF-8,
    // which `iter().flatten()` would silently drop (V5): the repository
    // would then look remote-less, or an unexpected remote would win. Walk
    // the entries by index instead so such a name is reported.
    let mut names: Vec<String> = Vec::with_capacity(remotes.len());
    for index in 0..remotes.len() {
        let Some(name) = remotes.get(index) else {
            anyhow::bail!("远端名不是有效的 UTF-8，无法执行远端操作");
        };
        names.push(name.to_string());
    }
    pick_remote(&names)
}

/// Pick the remote to operate on out of the configured remote names:
/// `origin` when present, otherwise the first one.
fn pick_remote(names: &[String]) -> Result<String> {
    if names.iter().any(|name| name == "origin") {
        return Ok("origin".to_string());
    }
    let Some(first) = names.first() else {
        anyhow::bail!("未配置远端，无法执行远端操作");
    };
    if first.starts_with('-') {
        anyhow::bail!("远端名不能以 - 开头: {first}");
    }
    Ok(first.clone())
}

/// Full refspec of a local branch: `refs/heads/<branch>`.
///
/// Handing the branch to git as a bare positional argument lets git read a
/// branch named `--mirror` as an option (which really does mirror-delete
/// remote branches) — G27. The full refspec can never be read as an option,
/// and a `:` inside the name is refused because it would split the refspec
/// into source and destination (V4).
fn branch_refspec(branch: &str) -> Result<String> {
    validate_ref_name(branch, "分支名", false)?;
    Ok(format!("refs/heads/{branch}"))
}

/// Full refspec of a tag: `refs/tags/<tag>`.
///
/// The full refspec keeps git from reading a tag named `--force` as an option
/// (R2); a `:` or a control character in the name is refused because it would
/// split the refspec into source and destination, so `a:b` would update an
/// arbitrary remote ref (V4).
fn tag_refspec(tag: &str) -> Result<String> {
    validate_ref_name(tag, "标签名", true)?;
    Ok(format!("refs/tags/{tag}"))
}

/// Characters a name must not carry to be interpolated into a refspec.
///
/// `:` splits the refspec into source and destination, control characters
/// would corrupt the command line, and a leading `-` makes a positional
/// argument look like an option (`allow_leading_dash` is set for tag names,
/// which always travel as a full refspec).
fn validate_ref_name(name: &str, label: &str, allow_leading_dash: bool) -> Result<()> {
    if !allow_leading_dash && name.starts_with('-') {
        anyhow::bail!("{label}不能以 - 开头: {name}");
    }
    if name.chars().any(|ch| ch == ':' || ch.is_control()) {
        anyhow::bail!("{label}含非法字符: {name}");
    }
    Ok(())
}

/// Resolve the current branch name; errors on detached or unborn HEAD.
///
/// A branch name that is not valid UTF-8 cannot be handed to the git CLI as
/// a `&str` argument, so it is refused with an explicit error carrying the
/// escaped spelling (V8) instead of a generic "无法解析分支名".
fn current_branch(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head().context("无法解析 HEAD")?;
    if !head.is_branch() {
        anyhow::bail!("处于 detached HEAD，无法执行远端操作");
    }
    let Some(name) = head.shorthand() else {
        anyhow::bail!(
            "分支名不是有效的 UTF-8，无法执行远端操作: {}",
            show_bytes(head.shorthand_bytes())
        );
    };
    Ok(name.to_string())
}

/// Whether the current branch has an upstream tracking branch.
fn has_upstream(repo_path: &Path) -> bool {
    let Ok(repo) = Repository::open(repo_path) else {
        return false;
    };
    let Ok(head) = repo.head() else {
        return false;
    };
    if !head.is_branch() {
        return false;
    }
    let Some(name) = head.shorthand() else {
        return false;
    };
    let Ok(branch) = repo.find_branch(name, git2::BranchType::Local) else {
        return false;
    };
    // The `Result` returned by `upstream` keeps the `Branch` (and with it a
    // borrow of `repo`) alive; bind it before returning so it drops first.
    let upstream = branch.upstream();
    upstream.is_ok()
}

/// Apply the shared non-interactive configuration to a `git` child process.
///
/// Every `git` subprocess GitPulse spawns (push/pull, batch fetch and the CLI
/// helpers in tests) must call this first, otherwise git opens the controlling
/// terminal (`/dev/tty`, *not* stdin) and prints `Username for
/// 'https://github.com':` where the TUI cannot answer it, hanging the batch
/// operation. The TUI has no way to reply, so interactive fallbacks are
/// disabled:
///
/// - `GIT_TERMINAL_PROMPT=0` — git fails fast instead of prompting on the
///   terminal (the fix for the batch-pull credential hang).
/// - `GIT_ASKPASS=` / `SSH_ASKPASS=` — external askpass dialogs would open
///   outside the TUI, so they are disabled as well.
/// - `GIT_SSH_COMMAND` — set to `ssh -oBatchMode=yes` when the parent has no
///   preference of its own (V3). The `GIT_TERMINAL_PROMPT`/`*_ASKPASS` switches
///   only cover git's own credential prompts: OpenSSH reads its host-key
///   confirmation and its key passphrase straight from `/dev/tty` whenever a
///   controlling terminal exists, so an `git@host:` remote would block the
///   child forever on a first connection and — because the task channel then
///   never delivers a terminal event — lock every remote action out for the
///   rest of the session. `BatchMode=yes` makes ssh fail fast instead. A
///   `GIT_SSH_COMMAND` already present in the parent environment is kept: the
///   user's own wrapper is their explicit choice.
///
/// The parent environment is inherited untouched on purpose: `HOME`,
/// `XDG_CONFIG_HOME`, `PATH` and `SSH_AUTH_SOCK` must reach the child so git
/// keeps reading the user's global/system config and reuses the system
/// credential helpers configured there (`gh`, `osxkeychain`, `libsecret`, ...).
/// When no helper can supply credentials, git now exits immediately with
/// `could not read Username ...: terminal prompts disabled`, which callers
/// surface as a readable error instead of blocking forever.
pub fn configure_git_cmd(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    }
    cmd
}

/// Run `git -C <repo> <args>` and return its output, or an error carrying
/// the operation name and the child's stderr (shown on the status line).
///
/// Shared with the batch helpers (G29), so every git child GitPulse spawns
/// reports a failure the same way.
pub(crate) fn git_output(repo_path: &Path, args: &[&str]) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    configure_git_cmd(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("启动 git 进程失败")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let op = args.first().copied().unwrap_or("git");
        anyhow::bail!("{op} 失败: {err}");
    }
    Ok(output)
}

/// Shared body of `push`, `push_tag` and `push_all_tags`: resolve the remote,
/// run `git push` with the arguments `args` builds for that remote, then
/// return its result.
///
/// The running message is emitted by the App layer only (it sets
/// `RemoteStatus::Running` before spawning), and the completion is
/// displayed by the App layer from the returned result.
fn push_refs(repo_path: &Path, args: impl FnOnce(&str) -> Result<Vec<String>>) -> Result<()> {
    let remote = resolve_remote(repo_path)?;
    let mut argv = vec!["push".to_string()];
    argv.extend(args(&remote)?);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    git_output(repo_path, &refs)?;
    Ok(())
}

/// Push the current branch to its remote.
///
/// Uses `git push -u <remote> refs/heads/<branch>` so the first push to a
/// freshly configured remote also establishes the upstream tracking branch,
/// and so a branch name can never be read as a git option (G27).
pub async fn push(repo_path: &Path) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        push_refs(&repo_path, |remote| {
            let branch = current_branch(&repo_path)?;
            // The full refspec keeps a branch named `--mirror` from being
            // read as an option (G27).
            Ok(vec![
                "-u".to_string(),
                remote.to_string(),
                branch_refspec(&branch)?,
            ])
        })
    })
    .await?
}

/// Pull the current branch, fast-forward only.
///
/// Falls back to explicit `<remote> refs/heads/<branch>` arguments when the
/// branch has no upstream yet; diverged histories fail with an explicit error
/// (convention 7) instead of merging or rebasing.
pub async fn pull(repo_path: &Path) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let remote = resolve_remote(&repo_path)?;
        let branch = current_branch(&repo_path)?;
        let refspec = branch_refspec(&branch)?;
        let args: Vec<&str> = if has_upstream(&repo_path) {
            vec!["pull", "--ff-only"]
        } else {
            vec!["pull", "--ff-only", remote.as_str(), refspec.as_str()]
        };
        git_output(&repo_path, &args)?;
        Ok(())
    })
    .await?
}

/// Push a single tag to the remote.
///
/// Uses `git push <remote> refs/tags/<tag>` so that only the named tag is
/// transferred.  The full refspec (`refs/tags/…`) prevents git from
/// misinterpreting tag names that start with `-` (e.g. `--force`,
/// `--delete`) as command-line options — see R2.
///
/// The repository must have at least one remote configured; otherwise an
/// error is returned with a readable reason (matching `push` behavior).
///
/// A tag name containing `:` or a control character is rejected: it would
/// split the refspec into source and destination and update an arbitrary
/// remote ref (V4).
pub async fn push_tag(repo_path: &Path, tag: &str) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    let tag = tag.to_string();
    tokio::task::spawn_blocking(move || {
        push_refs(&repo_path, |remote| {
            Ok(vec![remote.to_string(), tag_refspec(&tag)?])
        })
    })
    .await?
}

/// Push all local tags to the remote.
///
/// Uses `git push <remote> --tags` so that every local tag is transferred.
/// The repository must have at least one remote configured; otherwise an error
/// is returned with a readable reason (matching `push` behavior).
pub async fn push_all_tags(repo_path: &Path) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        push_refs(&repo_path, |remote| {
            Ok(vec![remote.to_string(), "--tags".to_string()])
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn base_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-remote-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path) {
        let repo = Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
    }

    fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
        fs::write(dir.join(name), content).unwrap();
        crate::git::stage(dir, Path::new(name)).unwrap();
        crate::git::commit(dir, message).unwrap();
    }

    /// The shared helper must switch git to non-interactive mode while
    /// leaving the credential-relevant variables to be inherited (no
    /// `env_clear`): `HOME`/`XDG_CONFIG_HOME` carry the global config that
    /// holds the system credential helpers, `SSH_AUTH_SOCK` covers SSH remotes.
    #[test]
    fn configure_git_cmd_disables_prompts_and_keeps_inherited_env() {
        let mut cmd = Command::new("git");
        configure_git_cmd(&mut cmd);

        let envs: std::collections::HashMap<String, Option<String>> = cmd
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();

        assert_eq!(
            envs.get("GIT_TERMINAL_PROMPT").and_then(|v| v.as_deref()),
            Some("0"),
            "{envs:?}"
        );
        assert_eq!(envs.get("GIT_ASKPASS").and_then(|v| v.as_deref()), Some(""));
        assert_eq!(envs.get("SSH_ASKPASS").and_then(|v| v.as_deref()), Some(""));

        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "PATH",
            "SSH_AUTH_SOCK",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ] {
            assert!(
                !envs.contains_key(key),
                "{key} must be inherited from the parent, not overridden: {envs:?}"
            );
        }
    }

    /// Runtime proof that a configured child sees both the non-interactive
    /// flag and the inherited `HOME` of the parent process.
    #[cfg(unix)]
    #[test]
    fn configure_git_cmd_child_inherits_home_at_runtime() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("printf '%s|%s' \"$GIT_TERMINAL_PROMPT\" \"$HOME\"");
        configure_git_cmd(&mut cmd);
        let output = cmd.output().expect("run /bin/sh");
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        let (prompt, home) = text
            .split_once('|')
            .unwrap_or_else(|| panic!("unexpected output: {text}"));
        assert_eq!(prompt, "0", "{text}");
        let expected = std::env::var_os("HOME")
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert_eq!(home, expected, "HOME must be inherited unchanged");
    }

    /// Regression for the batch-pull hang: when no credential helper can
    /// provide credentials, git must exit immediately with a readable reason
    /// instead of printing `Username for ...` on the TUI's terminal.
    #[test]
    fn credential_lookup_fails_fast_instead_of_prompting() {
        use std::io::Write;

        let mut cmd = Command::new("git");
        configure_git_cmd(&mut cmd);
        // Clearing the helper list makes the case deterministic even on
        // machines with a globally configured credential helper.
        cmd.args(["-c", "credential.helper=", "credential", "fill"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn git credential fill");
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(b"protocol=https\nhost=example.invalid\n\n")
            .expect("write credential request");
        let output = child.wait_with_output().expect("wait for git");
        assert!(
            !output.status.success(),
            "credential fill must fail when no helper has credentials"
        );
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            err.contains("terminal prompts disabled"),
            "expected a readable fast-failure, got: {err}"
        );
    }

    #[tokio::test]
    async fn push_without_remote_reports_reason() {
        let dir = base_dir("noremote");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");

        let err = push(&dir).await.unwrap_err();
        assert!(
            err.to_string().contains("未配置远端"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn push_pull_roundtrip_and_ff_only_divergence_error() {
        let root = base_dir("roundtrip");
        let origin_path = root.join("origin.git");
        Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        // Work repo: first push must succeed and establish the upstream.
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");

        push(&work).await.expect("first push must succeed");

        // Second clone advances the remote.
        let other = root.join("other");
        let repo2 = Repository::clone(&origin_str, &other).unwrap();
        let mut config2 = repo2.config().unwrap();
        config2.set_str("user.name", "GitPulse Test").unwrap();
        config2
            .set_str("user.email", "test@gitpulse.local")
            .unwrap();
        drop(config2);
        commit_file(&other, "b.txt", "v2\n", "v2");
        push(&other).await.expect("clone push must succeed");

        // Drop the upstream so the pull exercises the explicit-args fallback.
        let mut untrack = Command::new("git");
        configure_git_cmd(&mut untrack);
        let untrack = untrack
            .arg("-C")
            .arg(&work)
            .arg("branch")
            .arg("--unset-upstream")
            .output()
            .unwrap();
        assert!(untrack.status.success(), "unset-upstream failed");

        pull(&work).await.expect("ff-only pull must succeed");
        assert!(
            work.join("b.txt").exists(),
            "pull did not bring the remote commit"
        );

        // An up-to-date push still succeeds (and re-sets the upstream).
        push(&work).await.expect("up-to-date push must succeed");

        // Diverged histories: local commit + advanced remote -> ff-only error.
        commit_file(&work, "c.txt", "local\n", "local-only");
        commit_file(&other, "d.txt", "remote\n", "remote-only");
        push(&other).await.expect("second clone push must succeed");

        let err = pull(&work).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("pull 失败"), "missing error reason: {msg}");

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn push_tag_publishes_ref_to_remote() {
        let root = base_dir("pushtag");
        let origin_path = root.join("origin.git");
        Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        // Work repo: create a commit and a tag.
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");
        // Create a tag named "v1.0".
        crate::git::create_tag(&work, "v1.0").unwrap();

        push_tag(&work, "v1.0")
            .await
            .expect("push tag must succeed");

        // Verify the tag exists in the bare repo.
        let origin = Repository::open(&origin_path).unwrap();
        assert!(
            origin.find_reference("refs/tags/v1.0").is_ok(),
            "tag should exist in remote"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn push_all_tags_publishes_every_tag() {
        let root = base_dir("pushall");
        let origin_path = root.join("origin.git");
        Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        // Work repo: create a commit and two tags.
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");
        crate::git::create_tag(&work, "v1.0").unwrap();
        crate::git::create_tag(&work, "v2.0").unwrap();

        push_all_tags(&work)
            .await
            .expect("push all tags must succeed");

        // Verify both tags exist in the bare repo.
        let origin = Repository::open(&origin_path).unwrap();
        assert!(
            origin.find_reference("refs/tags/v1.0").is_ok(),
            "tag v1.0 should exist in remote"
        );
        assert!(
            origin.find_reference("refs/tags/v2.0").is_ok(),
            "tag v2.0 should exist in remote"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// R2: a tag whose name starts with `-` must still reach the remote.
    /// The full `refs/tags/<name>` refspec is what keeps git from reading
    /// the name as a command-line option (`-leading-dash`) or as a path.
    #[tokio::test]
    async fn push_tag_with_leading_dash_publishes_full_refspec() {
        let root = base_dir("pushtag-dash");
        let origin_path = root.join("origin.git");
        Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        // Work repo: first push establishes nothing here, the tag is what
        // travels — the tag name itself must never be passed bare.
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");
        // `git tag` itself rejects a leading `-` as a name (git2 wraps that
        // check), so the loose ref is written directly — the tag is still a
        // real one for the push, which is what R2 is about.
        {
            let repo = Repository::open(&work).unwrap();
            let oid = repo
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string();
            let ref_dir = repo.path().join("refs/tags");
            fs::create_dir_all(&ref_dir).unwrap();
            fs::write(ref_dir.join("-leading-dash"), format!("{oid}\n")).unwrap();
        }

        push_tag(&work, "-leading-dash")
            .await
            .expect("pushing a `-`-leading tag must succeed");

        // Verify the tag exists in the bare repo under its full name.
        let origin = Repository::open(&origin_path).unwrap();
        assert!(
            origin.find_reference("refs/tags/-leading-dash").is_ok(),
            "`-leading-dash` must exist in remote as refs/tags/-leading-dash"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn push_tag_without_remote_reports_reason() {
        let dir = base_dir("noremote-tag");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        // Create a tag to push.
        crate::git::create_tag(&dir, "v1.0").unwrap();

        let err = push_tag(&dir, "v1.0").await.unwrap_err();
        assert!(
            err.to_string().contains("未配置远端"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
