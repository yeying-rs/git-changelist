//! Local repository collector, driven by the system `git` binary.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::classify::{classify, link_reverts};
use crate::github::{compare_versions, parse_slug, tag_prefix};
use crate::model::{RawCommit, Report, SCHEMA_VERSION, Selection, Version};

const FIELD: char = '\x1f';
const RECORD: char = '\x1e';

struct Tag {
  name: String,
  date: String,
  message: Option<String>,
}

pub fn collect(repo: &Path, selection: &Selection) -> Result<Report> {
  git(repo, &["rev-parse", "--git-dir"])
    .with_context(|| format!("{} 不是 git 仓库", repo.display()))?;
  let tags = list_tags(repo)?;

  // (tag, previous); tag `None` means HEAD (unreleased).
  let mut ranges: Vec<(Option<String>, Option<String>)> = Vec::new();
  if let Some((from, to)) = &selection.range {
    ranges.extend(split_range(repo, &tags, from, to)?);
  } else if let Some(tag) = &selection.tag {
    if !tags.iter().any(|t| &t.name == tag) {
      bail!("找不到 Tag：{tag}");
    }
    ranges.push((
      Some(tag.clone()),
      previous_tag(repo, &format!("{tag}^"), Some(tag_prefix(tag))),
    ));
  } else {
    if selection.unreleased || tags.is_empty() {
      ranges.push((None, previous_tag(repo, "HEAD", None)));
    }
    for tag in tags.iter().take(selection.limit) {
      let previous = previous_tag(repo, &format!("{}^", tag.name), Some(tag_prefix(&tag.name)));
      ranges.push((Some(tag.name.clone()), previous));
    }
  }

  let mut versions = Vec::with_capacity(ranges.len());
  let mut warnings = Vec::new();
  for (tag, previous) in ranges {
    let head = tag.as_deref().unwrap_or("HEAD");
    let revisions = revisions(previous.as_deref(), head);
    let total_commits = count_commits(repo, &revisions, selection.merges)?;
    // Only an unreleased range can be legitimately empty; hide it instead of showing noise.
    if tag.is_none() && total_commits == 0 && !tags.is_empty() {
      continue;
    }
    let info = tag
      .as_ref()
      .and_then(|name| tags.iter().find(|t| &t.name == name));
    let mut commits: Vec<_> = log(repo, &revisions, selection)?
      .into_iter()
      .map(classify)
      .collect();
    link_reverts(&mut commits);
    let truncated = total_commits > commits.len();
    if truncated {
      warnings.push(format!(
        "版本 {head} 共 {total_commits} 个提交，已按 --max-commits={} 截断为 {} 个",
        selection.max_commits,
        commits.len()
      ));
    }
    versions.push(Version {
      date: info.map(|t| t.date.clone()),
      tag_message: info.and_then(|t| t.message.clone()),
      commits,
      total_commits,
      truncated,
      release: None,
      tag,
      previous,
    });
  }

  Ok(Report {
    schema_version: SCHEMA_VERSION,
    // `.` makes a useless heading in exported files.
    source: repo
      .canonicalize()
      .unwrap_or_else(|_| repo.to_path_buf())
      .display()
      .to_string(),
    github: origin_url(repo).as_deref().and_then(parse_slug),
    warnings,
    versions,
  })
}

fn origin_url(repo: &Path) -> Option<String> {
  git(repo, &["remote", "get-url", "origin"])
    .ok()
    .map(|s| s.trim().to_string())
}

fn list_tags(repo: &Path) -> Result<Vec<Tag>> {
  let format = "--format=%(refname:strip=2)%1f%(creatordate:short)%1f%(objecttype)%1f\
                %(contents:subject)%1f%(contents:body)%1e";
  // The last --sort key is primary; version order breaks ties between same-second tags.
  let out = git(
    repo,
    &[
      "for-each-ref",
      "--sort=-v:refname",
      "--sort=-creatordate",
      format,
      "refs/tags",
    ],
  )?;
  Ok(
    records(&out)
      .filter_map(|fields| match fields.as_slice() {
        [name, date, kind, subject, body] => Some(Tag {
          name: name.to_string(),
          date: date.to_string(),
          // A lightweight tag's "contents" is the commit message, not a tag note.
          message: (*kind == "tag")
            .then(|| format!("{subject}\n\n{body}").trim().to_string())
            .filter(|m| !m.is_empty()),
        }),
        _ => None,
      })
      .collect(),
  )
}

/// Nearest tag reachable from `rev`; `None` when there is none (e.g. the first tag).
/// With `prefix`, only tags sharing it count, so monorepo packages don't interleave.
fn previous_tag(repo: &Path, rev: &str, prefix: Option<&str>) -> Option<String> {
  let pattern = prefix.map(|prefix| format!("--match={prefix}[0-9]*"));
  let mut args = vec!["describe", "--tags", "--abbrev=0"];
  args.extend(pattern.as_deref());
  args.push(rev);
  git(repo, &args).ok().map(|s| s.trim().to_string())
}

/// Splits `from..to` at every tag of the endpoints' prefix whose version lies between them,
/// newest first, so each change is attributed to the version that introduced it.
/// Version order (not ancestry) decides, so tags on release/hotfix branches count too.
/// Without a tag endpoint there is no prefix to go by, and the range stays one block.
fn split_range(
  repo: &Path,
  tags: &[Tag],
  from: &str,
  to: &str,
) -> Result<Vec<(Option<String>, Option<String>)>> {
  let to_is_tag = tags.iter().any(|t| t.name == to);
  let from_is_tag = tags.iter().any(|t| t.name == from);
  let prefix = match (to_is_tag, from_is_tag) {
    (true, _) => tag_prefix(to),
    (false, true) => tag_prefix(from),
    (false, false) => return Ok(vec![(Some(to.to_string()), Some(from.to_string()))]),
  };
  // A non-tag endpoint has no version, so fall back to ancestry on that side.
  let reachable = |flag: &str, rev: &str| -> Result<HashSet<String>> {
    let out = git(repo, &["tag", "--list", &format!("{flag}={rev}")])?;
    Ok(out.lines().map(str::to_string).collect())
  };
  let below_to = if to_is_tag {
    None
  } else {
    Some(reachable("--merged", to)?)
  };
  let above_from = if from_is_tag {
    None
  } else {
    Some(reachable("--no-merged", from)?)
  };

  let mut between: Vec<&str> = tags
    .iter()
    .map(|t| t.name.as_str())
    .filter(|name| *name != to && *name != from && tag_prefix(name) == prefix)
    .filter(|name| match &below_to {
      Some(set) => set.contains(*name),
      None => compare_versions(name, to) == Ordering::Less,
    })
    .filter(|name| match &above_from {
      Some(set) => set.contains(*name),
      None => compare_versions(name, from) == Ordering::Greater,
    })
    .collect();
  between.sort_by(|a, b| compare_versions(b, a));

  let heads: Vec<&str> = std::iter::once(to).chain(between).collect();
  let previous = heads.iter().skip(1).copied().chain(std::iter::once(from));
  Ok(
    heads
      .iter()
      .zip(previous)
      .map(|(head, previous)| (Some(head.to_string()), Some(previous.to_string())))
      .collect(),
  )
}

/// Revision arguments for the commits `head` brings over `previous`.
/// `A...B --cherry-pick --right-only` drops commits whose patch already shipped in `previous`
/// (back-ports on a release branch); on linear history it is the same as `A..B`.
fn revisions(previous: Option<&str>, head: &str) -> Vec<String> {
  match previous {
    Some(previous) => vec![
      "--cherry-pick".to_string(),
      "--right-only".to_string(),
      "--end-of-options".to_string(),
      format!("{previous}...{head}"),
    ],
    None => vec!["--end-of-options".to_string(), head.to_string()],
  }
}

fn count_commits(repo: &Path, revisions: &[String], merges: bool) -> Result<usize> {
  let mut args = vec!["rev-list", "--count"];
  if !merges {
    args.push("--no-merges");
  }
  args.extend(revisions.iter().map(String::as_str));
  let out = git(repo, &args)?;
  out
    .trim()
    .parse()
    .with_context(|| format!("无法解析提交数：{out}"))
}

fn log(repo: &Path, revisions: &[String], selection: &Selection) -> Result<Vec<RawCommit>> {
  let max = format!("--max-count={}", selection.max_commits);
  let mut args = vec!["log", "--format=%H%x1f%an%x1f%as%x1f%s%x1f%b%x1e", &max];
  if !selection.merges {
    args.push("--no-merges");
  }
  args.extend(revisions.iter().map(String::as_str));
  let out = git(repo, &args)?;
  Ok(
    records(&out)
      .filter_map(|fields| match fields.as_slice() {
        [sha, author, date, subject, body] => Some(RawCommit {
          sha: sha.to_string(),
          author: author.to_string(),
          date: date.to_string(),
          subject: subject.to_string(),
          body: body.to_string(),
        }),
        _ => None,
      })
      .collect(),
  )
}

fn records(out: &str) -> impl Iterator<Item = Vec<&str>> {
  out
    .split(RECORD)
    .map(|record| record.trim_start_matches('\n'))
    .filter(|record| !record.is_empty())
    .map(|record| record.split(FIELD).collect())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
  let output = Command::new("git")
    .arg("-C")
    .arg(repo)
    .args(args)
    .output()
    .context("无法执行 git，请确认已安装 git")?;
  if !output.status.success() {
    bail!(
      "git {} 失败：{}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::Category;
  use std::path::PathBuf;

  /// A throwaway repository under the system temp dir.
  struct TempRepo(PathBuf);

  impl TempRepo {
    fn new(name: &str) -> Self {
      let dir = std::env::temp_dir().join(format!("taglens-{name}-{}", std::process::id()));
      let _ = std::fs::remove_dir_all(&dir);
      std::fs::create_dir_all(&dir).unwrap();
      let repo = TempRepo(dir);
      repo.run(&["init", "-q", "-b", "main"]);
      repo.run(&["config", "user.name", "Tester"]);
      repo.run(&["config", "user.email", "t@example.com"]);
      repo.run(&["config", "tag.gpgSign", "false"]);
      repo.run(&["config", "commit.gpgSign", "false"]);
      repo
    }

    fn run(&self, args: &[&str]) {
      git(&self.0, args).unwrap();
    }

    fn commit(&self, message: &str) {
      self.run(&["commit", "-q", "--allow-empty", "-m", message]);
    }

    /// A commit with a real diff, so `--cherry-pick` can match it by patch id.
    fn commit_file(&self, file: &str, message: &str) {
      std::fs::write(self.0.join(file), message).unwrap();
      self.run(&["add", file]);
      self.run(&["commit", "-q", "-m", message]);
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  fn selection() -> Selection {
    Selection {
      limit: 10,
      tag: None,
      range: None,
      unreleased: false,
      max_commits: 200,
      merges: false,
    }
  }

  fn sample_repo(name: &str) -> TempRepo {
    let repo = TempRepo::new(name);
    repo.commit("chore: init");
    repo.run(&["tag", "v0.1.0"]);
    repo.commit("feat: add export");
    repo.commit("fix: crash on empty input (#7)");
    repo.run(&["tag", "-a", "v0.2.0", "-m", "Second release"]);
    repo.commit("feat!: drop legacy flag");
    repo
  }

  #[test]
  fn versions_are_split_by_previous_tag() {
    let repo = sample_repo("split");
    let report = collect(&repo.0, &selection()).unwrap();
    let tags: Vec<_> = report.versions.iter().map(|v| v.tag.as_deref()).collect();
    assert_eq!(tags, [Some("v0.2.0"), Some("v0.1.0")]);

    let v2 = &report.versions[0];
    assert_eq!(v2.previous.as_deref(), Some("v0.1.0"));
    assert_eq!(v2.total_commits, 2);
    assert_eq!(v2.tag_message.as_deref(), Some("Second release"));
    let categories: Vec<_> = v2.commits.iter().map(|c| c.category).collect();
    assert_eq!(categories, [Category::Fix, Category::Feature]);
    assert_eq!(v2.commits[0].pr, Some(7));

    let v1 = &report.versions[1];
    assert_eq!(v1.previous, None);
    assert_eq!(v1.tag_message, None, "lightweight tag has no note");
    assert_eq!(v1.total_commits, 1);
  }

  #[test]
  fn unreleased_range_starts_at_latest_reachable_tag() {
    let repo = sample_repo("unreleased");
    let report = collect(
      &repo.0,
      &Selection {
        unreleased: true,
        ..selection()
      },
    )
    .unwrap();
    let unreleased = &report.versions[0];
    assert_eq!(unreleased.tag, None);
    assert_eq!(unreleased.previous.as_deref(), Some("v0.2.0"));
    assert!(unreleased.commits[0].breaking);
  }

  #[test]
  fn tag_range_and_limit_selection() {
    let repo = sample_repo("select");
    let one = collect(
      &repo.0,
      &Selection {
        tag: Some("v0.2.0".into()),
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(one.versions.len(), 1);
    assert!(
      collect(
        &repo.0,
        &Selection {
          tag: Some("nope".into()),
          ..selection()
        }
      )
      .is_err()
    );

    let limited = collect(
      &repo.0,
      &Selection {
        limit: 1,
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(limited.versions.len(), 1);
  }

  fn segments(report: &Report) -> Vec<(String, String, usize)> {
    report
      .versions
      .iter()
      .map(|v| {
        let tag = v.tag.clone().unwrap_or_default();
        (tag, v.previous.clone().unwrap_or_default(), v.total_commits)
      })
      .collect()
  }

  fn range(repo: &TempRepo, from: &str, to: &str) -> Report {
    let range = Some((from.to_string(), to.to_string()));
    collect(
      &repo.0,
      &Selection {
        range,
        ..selection()
      },
    )
    .unwrap()
  }

  /// v0.1.0 · v0.2.0 · react@0.1.0 · v0.2.1 · v0.3.0 · (HEAD)
  fn release_line(name: &str) -> TempRepo {
    let repo = TempRepo::new(name);
    for (message, tag) in [
      ("feat: a", "v0.1.0"),
      ("feat: b", "v0.2.0"),
      ("feat(react): ui", "react@0.1.0"),
      ("fix: c", "v0.2.1"),
      ("feat: d", "v0.3.0"),
    ] {
      repo.commit(message);
      repo.run(&["tag", tag]);
    }
    repo.commit("feat: unreleased");
    repo
  }

  #[test]
  fn range_between_tags_is_split_per_version() {
    let repo = release_line("range-split");
    let expected = [
      ("v0.3.0".to_string(), "v0.2.1".to_string(), 1),
      ("v0.2.1".to_string(), "v0.2.0".to_string(), 2),
      ("v0.2.0".to_string(), "v0.1.0".to_string(), 1),
    ];
    assert_eq!(segments(&range(&repo, "v0.1.0", "v0.3.0")), expected);
  }

  #[test]
  fn range_to_non_tag_head_keeps_it_as_first_segment() {
    let repo = release_line("range-head");
    let expected = [
      ("HEAD".to_string(), "v0.3.0".to_string(), 1),
      ("v0.3.0".to_string(), "v0.2.1".to_string(), 1),
    ];
    assert_eq!(segments(&range(&repo, "v0.2.1", "HEAD")), expected);
  }

  /// main:   v0.1.0 ─ shared ─ crash ─ new(v0.2.0)
  /// hotfix:                 └ crash' (cherry-pick, v0.1.1)
  #[test]
  fn range_includes_hotfix_branch_tags_without_duplicating_backports() {
    let repo = TempRepo::new("range-hotfix");
    repo.commit_file("a", "feat: base");
    repo.run(&["tag", "v0.1.0"]);
    repo.commit_file("b", "feat: shared");
    repo.run(&["branch", "hotfix"]);
    repo.commit_file("c", "fix: crash");
    repo.run(&["checkout", "-q", "hotfix"]);
    // -x changes the message; otherwise the same-second pick can reproduce the original SHA.
    repo.run(&["cherry-pick", "-x", "main"]);
    repo.run(&["tag", "v0.1.1"]);
    repo.run(&["checkout", "-q", "main"]);
    repo.commit_file("d", "feat: new");
    repo.run(&["tag", "v0.2.0"]);

    let report = range(&repo, "v0.1.0", "v0.2.0");
    let expected = [
      ("v0.2.0".to_string(), "v0.1.1".to_string(), 1),
      ("v0.1.1".to_string(), "v0.1.0".to_string(), 2),
    ];
    assert_eq!(segments(&report), expected);
    // The back-ported fix shipped in v0.1.1, so upgrading to v0.2.0 only brings `new`.
    assert_eq!(report.versions[0].commits[0].summary, "new");
  }

  #[test]
  fn range_without_tag_endpoints_is_one_block() {
    let repo = release_line("range-plain");
    let report = range(&repo, "HEAD~3", "HEAD");
    assert_eq!(
      segments(&report),
      [("HEAD".to_string(), "HEAD~3".to_string(), 3)]
    );
  }

  #[test]
  fn max_commits_truncates_but_keeps_total() {
    let repo = sample_repo("truncate");
    let report = collect(
      &repo.0,
      &Selection {
        max_commits: 1,
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(report.versions[0].commits.len(), 1);
    assert_eq!(report.versions[0].total_commits, 2);
    assert!(report.versions[0].truncated);
    assert!(!report.warnings.is_empty());
  }

  #[test]
  fn repo_without_tags_shows_head_as_unreleased() {
    let repo = TempRepo::new("notags");
    repo.commit("feat: first");
    let report = collect(&repo.0, &selection()).unwrap();
    assert_eq!(report.versions.len(), 1);
    assert_eq!(report.versions[0].tag, None);
    assert_eq!(report.versions[0].total_commits, 1);
  }

  #[test]
  fn monorepo_previous_tag_shares_prefix() {
    let repo = TempRepo::new("monorepo");
    repo.commit("feat: core");
    repo.run(&["tag", "core-0.1.0"]);
    repo.commit("feat: macros");
    repo.run(&["tag", "macros-0.1.0"]);
    repo.commit("fix: core bug");
    repo.run(&["tag", "core-0.2.0"]);
    let selection = Selection {
      tag: Some("core-0.2.0".into()),
      ..selection()
    };
    let report = collect(&repo.0, &selection).unwrap();
    assert_eq!(report.versions[0].previous.as_deref(), Some("core-0.1.0"));
    assert_eq!(report.versions[0].total_commits, 2);
  }

  #[test]
  fn non_repo_is_an_error() {
    let dir = std::env::temp_dir().join(format!("taglens-norepo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(collect(&dir, &selection()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
  }
}
