//! Commit history: newest-first log, commit details and DAG lane layout.
//!
//! Like the other single-repository helpers in this module every entry point
//! opens the repository on the calling thread; only batch operations need
//! `spawn_blocking`.
//!
//! The data layer ships ahead of its UI consumers, so every item below is
//! only exercised by the tests in this file until the history view lands.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use git2::{BranchType, Commit, ErrorCode, Oid, Repository, Sort};

/// Upper bound of DAG lanes the graph column can draw.
const MAX_LANES: usize = 6;

/// Reference pointing at a commit: HEAD, a local branch or a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefMark {
    /// Which kind of reference this mark represents.
    pub kind: RefKind,
    /// Reference name: always `HEAD` for [`RefKind::Head`], otherwise the
    /// branch / tag name without any `refs/…` prefix.
    pub name: String,
}

/// Kind of reference carried by a [`RefMark`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// The repository HEAD.
    Head,
    /// A local branch.
    Branch,
    /// A tag (lightweight or annotated).
    Tag,
}

/// One commit row for the history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Abbreviated object id (7 hex characters).
    pub id: String,
    /// First line of the commit message.
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Commit time rendered as `YYYY-MM-DD HH:MM` in the offset recorded
    /// with the commit (the same rule `git log` uses).
    pub time: String,
    /// HEAD / local branches / tags pointing at this commit.
    pub refs: Vec<RefMark>,
    /// Abbreviated parent ids, first parent first — the DAG layout input.
    pub parents: Vec<String>,
}

/// Full payload of one commit for the detail pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    /// Complete commit message, subject and body.
    pub message: String,
    /// Paths changed relative to the first parent commit; a root commit is
    /// diffed against the empty tree.
    pub files: Vec<String>,
}

/// One rendered graph cell: a glyph inside a DAG lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphCell {
    /// Character drawn in this cell.
    pub glyph: char,
    /// Zero-based lane index, always `< MAX_LANES`.
    pub lane: usize,
}

/// Walk the history newest-first and return at most `limit` commits.
///
/// The walk starts at HEAD plus every local branch and tag, so commits on
/// unmerged branches show up as well. A repository without a single commit
/// yields an empty vector, a path that is not a repository is an error.
pub fn list_commits(repo_path: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let repo = Repository::open(repo_path)?;
    let marks = collect_ref_marks(&repo);

    // A freshly initialised repository has an unborn HEAD and no history.
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == ErrorCode::UnbornBranch => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let head_commit = head.peel_to_commit()?;

    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TIME)?;
    walk.push(head_commit.id())?;
    for entry in repo.branches(Some(BranchType::Local))?.flatten() {
        if let Ok(commit) = entry.0.get().peel_to_commit() {
            let _ = walk.push(commit.id());
        }
    }
    if let Ok(references) = repo.references_glob("refs/tags/*") {
        for reference in references.flatten() {
            if let Ok(commit) = reference.peel_to_commit() {
                let _ = walk.push(commit.id());
            }
        }
    }

    let mut rows: Vec<(i64, CommitInfo)> = Vec::new();
    for oid in walk {
        if rows.len() >= limit {
            break;
        }
        let commit = repo.find_commit(oid?)?;
        let time = commit.time().seconds();
        rows.push((time, build_info(&commit, &marks)));
    }
    // The revwalk already yields newest-first; the explicit sort keeps that
    // promise deterministic when several commits share a timestamp.
    rows.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    Ok(rows.into_iter().map(|(_, info)| info).collect())
}

/// Resolve `oid` (abbreviated, full or a ref name) and return the full commit
/// message together with the paths changed relative to the first parent.
///
/// An unknown object id is an error.
pub fn commit_detail(repo_path: &Path, oid: &str) -> Result<CommitDetail> {
    let repo = Repository::open(repo_path)?;
    let object = repo
        .revparse_single(oid)
        .with_context(|| format!("无法解析提交 {oid}"))?;
    let commit = object
        .peel_to_commit()
        .with_context(|| format!("{oid} 不是提交对象"))?;
    let message = commit.message().unwrap_or_default().to_string();

    let tree = commit.tree().context("读取提交树失败")?;
    // Diffs are taken against the first parent; a root commit has none and is
    // therefore diffed against the empty tree.
    let parent_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree().context("读取父提交树失败")?),
        Err(_) => None,
    };
    let mut opts = git2::DiffOptions::new();
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
        .context("生成提交差异失败")?;
    let mut files = Vec::new();
    for delta in diff.deltas() {
        let path = delta.new_file().path().or_else(|| delta.old_file().path());
        if let Some(path) = path {
            files.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(CommitDetail { message, files })
}

/// Lay out the DAG column: one row of [`GraphCell`]s per input commit, in the
/// same order as `commits`.
///
/// The first parent of a commit keeps the commit's lane, extra parents fork
/// onto a new lane, and a commit awaited by several lanes joins those lanes
/// back together (`┤`) and frees them. Pure function; the lane count never
/// exceeds [`MAX_LANES`].
pub fn layout_graph(commits: &[CommitInfo]) -> Vec<Vec<GraphCell>> {
    // `Some(id)` marks a lane that is waiting for that commit to arrive.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, waiting)| waiting.as_deref() == Some(commit.id.as_str()))
            .map(|(index, _)| index)
            .collect();
        // The oldest lane wins the commit; any further lane joins into it.
        let primary = match waiting.first() {
            Some(&index) => index,
            None => allocate_lane(&mut lanes),
        };
        let joined: Vec<usize> = waiting.into_iter().skip(1).collect();

        // Route the parents: first parent stays on the commit's lane, each
        // additional parent forks onto a lane of its own.
        lanes[primary] = commit.parents.first().cloned();
        for parent in commit.parents.iter().skip(1) {
            let lane = allocate_lane(&mut lanes);
            lanes[lane] = Some(parent.clone());
        }
        // Lanes that joined into this commit are now free again.
        for index in joined.iter() {
            lanes[*index] = None;
        }

        let mut cells = Vec::with_capacity(lanes.len());
        for (index, lane) in lanes.iter().enumerate() {
            if joined.contains(&index) {
                cells.push(GraphCell {
                    glyph: '┤',
                    lane: index,
                });
            } else if index == primary {
                let glyph = if commit.parents.len() > 1 {
                    '┼'
                } else {
                    '●'
                };
                cells.push(GraphCell { glyph, lane: index });
            } else if lane.is_some() {
                cells.push(GraphCell {
                    glyph: '│',
                    lane: index,
                });
            }
        }
        rows.push(cells);
    }
    rows
}

/// Pick a lane for a new edge: reuse a free lane, grow the layout up to
/// [`MAX_LANES`] and, once every lane is busy, fold onto the last lane so the
/// graph degrades instead of overflowing the column.
fn allocate_lane(lanes: &mut Vec<Option<String>>) -> usize {
    if let Some(index) = lanes.iter().position(Option::is_none) {
        return index;
    }
    if lanes.len() < MAX_LANES {
        lanes.push(None);
        return lanes.len() - 1;
    }
    MAX_LANES - 1
}

/// Collect HEAD, local branch and tag marks grouped by target commit.
///
/// Within one commit the marks keep insertion order: HEAD first, then the
/// branches alphabetically, then the tags alphabetically. Unreadable
/// references are skipped instead of failing the whole map.
fn collect_ref_marks(repo: &Repository) -> HashMap<Oid, Vec<RefMark>> {
    let mut marks: HashMap<Oid, Vec<RefMark>> = HashMap::new();
    if let Ok(head) = repo.head() {
        if let Ok(commit) = head.peel_to_commit() {
            marks.entry(commit.id()).or_default().push(RefMark {
                kind: RefKind::Head,
                name: "HEAD".to_string(),
            });
        }
    }

    let mut branches: Vec<(String, Oid)> = Vec::new();
    if let Ok(iter) = repo.branches(Some(BranchType::Local)) {
        for entry in iter.flatten() {
            let (branch, _) = entry;
            let Ok(Some(name)) = branch.name() else {
                continue;
            };
            if let Ok(commit) = branch.get().peel_to_commit() {
                branches.push((name.to_string(), commit.id()));
            }
        }
    }
    branches.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, oid) in branches {
        marks.entry(oid).or_default().push(RefMark {
            kind: RefKind::Branch,
            name,
        });
    }

    let mut tags: Vec<(String, Oid)> = Vec::new();
    if let Ok(references) = repo.references_glob("refs/tags/*") {
        for reference in references.flatten() {
            let Some(name) = reference.shorthand().map(str::to_string) else {
                continue;
            };
            if let Ok(commit) = reference.peel_to_commit() {
                tags.push((name, commit.id()));
            }
        }
    }
    tags.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, oid) in tags {
        marks.entry(oid).or_default().push(RefMark {
            kind: RefKind::Tag,
            name,
        });
    }
    marks
}

/// Assemble one history row from a commit and the pre-collected ref marks.
fn build_info(commit: &Commit<'_>, marks: &HashMap<Oid, Vec<RefMark>>) -> CommitInfo {
    let time = commit.time();
    CommitInfo {
        id: short_id(commit.id()),
        summary: first_line(commit.message().unwrap_or_default()),
        author: commit.author().name().unwrap_or("unknown").to_string(),
        time: format_time(time.seconds(), time.offset_minutes()),
        refs: marks.get(&commit.id()).cloned().unwrap_or_default(),
        parents: commit.parent_ids().map(short_id).collect(),
    }
}

/// Abbreviate an object id to the 7 hex characters `git log --oneline` shows.
fn short_id(oid: Oid) -> String {
    let text = oid.to_string();
    text[..7].to_string()
}

/// First (trimmed) line of a commit message.
fn first_line(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Render a timestamp as `YYYY-MM-DD HH:MM` in the offset recorded with the
/// commit.
fn format_time(seconds: i64, offset_minutes: i32) -> String {
    let local = seconds + i64::from(offset_minutes) * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (secs / 3600, (secs % 3600) / 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian date
/// (Howard Hinnant's `civil_from_days` algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { y + 1 } else { y }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Commit timestamps start here (2023-11-14 22:13:20 UTC) and advance by
    /// one minute per commit so the expected order is unambiguous.
    const BASE_TIME: i64 = 1_700_000_000;

    fn base_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-history-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Repository fixture: `c1 -> c2` on the main branch, `feature` forked at
    /// `c2` with `f1 -> f2`, `m1` on main and a merge of both sides, plus a
    /// lightweight tag on `c2`.
    struct Fixture {
        dir: PathBuf,
        /// Name of the branch created by `Repository::init`.
        main: String,
        /// Full object ids oldest-first: c1, c2, f1, f2, m1, merge.
        ids: Vec<String>,
    }

    /// Commit `files` (the complete file set of the resulting tree) on top of
    /// `parents`, writing each file and rebuilding the index on the way.
    fn commit_files(
        repo: &Repository,
        dir: &Path,
        files: &[(&str, &str)],
        message: &str,
        when: i64,
        parents: &[&Commit<'_>],
    ) -> Oid {
        for (path, content) in files {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(target, content).unwrap();
        }

        let mut index = repo.index().unwrap();
        let keep: Vec<String> = files.iter().map(|(path, _)| (*path).to_string()).collect();
        let stale: Vec<String> = index
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
            .filter(|path| !keep.contains(path))
            .collect();
        for path in stale {
            index.remove_path(Path::new(&path)).unwrap();
            let _ = fs::remove_file(dir.join(&path));
        }
        for (path, _) in files {
            index.add_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::new(
            "GitPulse Test",
            "test@gitpulse.local",
            &git2::Time::new(when, 0),
        )
        .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, parents)
            .unwrap()
    }

    fn build_fixture(tag: &str) -> Fixture {
        let dir = base_dir(tag);
        let repo = Repository::init(&dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
        drop(config);

        let c1 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v1\n")],
            "initial commit",
            BASE_TIME,
            &[],
        );
        let main = repo.head().unwrap().shorthand().unwrap().to_string();

        let c1_commit = repo.find_commit(c1).unwrap();
        let c2 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v2\n")],
            "second commit",
            BASE_TIME + 60,
            &[&c1_commit],
        );
        let c2_commit = repo.find_commit(c2).unwrap();
        repo.branch("feature", &c2_commit, false).unwrap();
        repo.tag_lightweight("v1.0.0", c2_commit.as_object(), false)
            .unwrap();

        repo.set_head("refs/heads/feature").unwrap();
        let f1 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v2\n"), ("b.txt", "f1\n")],
            "feature one",
            BASE_TIME + 120,
            &[&c2_commit],
        );
        let f1_commit = repo.find_commit(f1).unwrap();
        let f2 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v2\n"), ("b.txt", "f2\n")],
            "feature two",
            BASE_TIME + 180,
            &[&f1_commit],
        );

        repo.set_head(&format!("refs/heads/{main}")).unwrap();
        let m1 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v3\n")],
            "main work\n\nBody line one.\nBody line two.",
            BASE_TIME + 240,
            &[&c2_commit],
        );
        let m1_commit = repo.find_commit(m1).unwrap();
        let f2_commit = repo.find_commit(f2).unwrap();
        let merge = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v3\n"), ("b.txt", "f2\n")],
            "Merge branch 'feature'",
            BASE_TIME + 300,
            &[&m1_commit, &f2_commit],
        );

        Fixture {
            dir,
            main,
            ids: vec![
                c1.to_string(),
                c2.to_string(),
                f1.to_string(),
                f2.to_string(),
                m1.to_string(),
                merge.to_string(),
            ],
        }
    }

    #[test]
    fn list_commits_orders_newest_first_with_short_hashes() {
        let fixture = build_fixture("order");
        let commits = list_commits(&fixture.dir, 200).unwrap();

        let expected: Vec<String> = fixture
            .ids
            .iter()
            .rev()
            .map(|id| id[..7].to_string())
            .collect();
        assert_eq!(commits.len(), 6);
        assert_eq!(
            commits.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            expected
        );
        assert!(commits.iter().all(|c| c.id.len() == 7));

        // Newest commit is BASE_TIME + 300 = 2023-11-14 22:18 UTC.
        assert_eq!(commits[0].time, "2023-11-14 22:18");
        let times: Vec<&str> = commits.iter().map(|c| c.time.as_str()).collect();
        assert!(times.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(times
            .iter()
            .all(|t| t.len() == 16 && t.as_bytes()[10] == b' '));

        assert_eq!(commits[0].summary, "Merge branch 'feature'");
        assert_eq!(commits[1].summary, "main work");
        assert!(commits.iter().all(|c| c.author == "GitPulse Test"));

        // The limit keeps only the newest rows.
        let top = list_commits(&fixture.dir, 2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].id, expected[0]);
        assert_eq!(top[1].id, expected[1]);

        let _ = fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn list_commits_marks_head_branch_and_tag_refs() {
        let fixture = build_fixture("refs");
        let commits = list_commits(&fixture.dir, 200).unwrap();
        let by_id: HashMap<&str, &CommitInfo> =
            commits.iter().map(|c| (c.id.as_str(), c)).collect();
        let at = |full: &str| by_id.get(&full[..7]).copied().unwrap();

        assert_eq!(
            at(&fixture.ids[5]).refs,
            vec![
                RefMark {
                    kind: RefKind::Head,
                    name: "HEAD".to_string(),
                },
                RefMark {
                    kind: RefKind::Branch,
                    name: fixture.main.clone(),
                },
            ]
        );
        assert_eq!(
            at(&fixture.ids[3]).refs,
            vec![RefMark {
                kind: RefKind::Branch,
                name: "feature".to_string(),
            }]
        );
        assert_eq!(
            at(&fixture.ids[1]).refs,
            vec![RefMark {
                kind: RefKind::Tag,
                name: "v1.0.0".to_string(),
            }]
        );
        assert!(at(&fixture.ids[0]).refs.is_empty());
        assert!(at(&fixture.ids[2]).refs.is_empty());
        assert!(at(&fixture.ids[4]).refs.is_empty());

        let _ = fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn list_commits_returns_empty_for_fresh_repository() {
        let dir = base_dir("empty");
        Repository::init(&dir).unwrap();
        let commits = list_commits(&dir, 200).unwrap();
        assert!(commits.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_commits_fails_on_non_repository() {
        let dir = base_dir("norepo");
        assert!(list_commits(&dir, 200).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_detail_returns_message_and_changed_files() {
        let fixture = build_fixture("detail");

        // Merge commit: full message, changed against its first parent.
        let merge = commit_detail(&fixture.dir, &fixture.ids[5]).unwrap();
        assert!(merge.message.starts_with("Merge branch 'feature'"));
        assert_eq!(merge.files, vec!["b.txt".to_string()]);

        // Multi-line message including the body.
        let m1 = commit_detail(&fixture.dir, &fixture.ids[4]).unwrap();
        assert!(m1.message.starts_with("main work"));
        assert!(m1.message.contains("Body line one."));
        assert!(m1.message.contains("Body line two."));
        assert_eq!(m1.files, vec!["a.txt".to_string()]);

        // Side branch commits are resolvable by their short hash too.
        let f1 = commit_detail(&fixture.dir, &fixture.ids[2][..7]).unwrap();
        assert_eq!(f1.message, "feature one");
        assert_eq!(f1.files, vec!["b.txt".to_string()]);

        let _ = fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn commit_detail_diffs_root_commit_against_empty_tree() {
        let fixture = build_fixture("root");
        let detail = commit_detail(&fixture.dir, &fixture.ids[0]).unwrap();
        assert!(detail.message.starts_with("initial commit"));
        assert_eq!(detail.files, vec!["a.txt".to_string()]);
        let _ = fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn commit_detail_fails_for_unknown_hash() {
        let fixture = build_fixture("missing-hash");
        let err = commit_detail(&fixture.dir, "0000000000000000000000000000000000000000");
        assert!(err.is_err());
        assert!(commit_detail(&fixture.dir, "nosuchref").is_err());
        let _ = fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn layout_graph_single_chain_is_one_lane_of_dots() {
        let dir = base_dir("chain");
        let repo = Repository::init(&dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
        drop(config);
        // Linear chain: a root commit plus two follow-up commits.
        let root = commit_files(&repo, &dir, &[("a.txt", "v1\n")], "root", BASE_TIME, &[]);
        let c1 = repo.find_commit(root).unwrap();
        let c2 = commit_files(
            &repo,
            &dir,
            &[("a.txt", "v2\n")],
            "second",
            BASE_TIME + 60,
            &[&c1],
        );
        let c2 = repo.find_commit(c2).unwrap();
        commit_files(
            &repo,
            &dir,
            &[("a.txt", "v3\n")],
            "third",
            BASE_TIME + 120,
            &[&c2],
        );

        let commits = list_commits(&dir, 200).unwrap();
        assert_eq!(commits.len(), 3);
        let rows = layout_graph(&commits);
        assert_eq!(rows.len(), commits.len());
        for row in &rows {
            assert_eq!(row.len(), 1);
            assert_eq!(row[0].lane, 0);
            assert_eq!(row[0].glyph, '●');
        }
        assert!(layout_graph(&[]).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn layout_graph_merge_uses_extra_lane_and_joins_back() {
        let fixture = build_fixture("merge-graph");
        let commits = list_commits(&fixture.dir, 200).unwrap();
        let rows = layout_graph(&commits);

        assert_eq!(rows.len(), commits.len());
        assert!(rows.iter().flatten().all(|cell| cell.lane < MAX_LANES));

        // The merge commit forks onto a second lane and crosses lanes.
        assert!(rows[0].len() >= 2);
        assert!(rows[0].iter().any(|cell| cell.glyph == '┼'));

        // The side branch runs on its own lane…
        let feature_row = &rows[2];
        assert!(feature_row
            .iter()
            .any(|cell| cell.lane == 1 && cell.glyph == '●'));

        // …and joins the main lane back together, freeing its lane.
        assert!(rows
            .iter()
            .any(|row| row.iter().any(|cell| cell.glyph == '┤')));
        let max_lane = rows.iter().flatten().map(|cell| cell.lane).max().unwrap();
        assert_eq!(max_lane, 1);

        let _ = fs::remove_dir_all(&fixture.dir);
    }
}
