//! Local repository collector, driven by the system `git` binary.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::classify::{classify, link_reverts, matches_grep};
use crate::github::{compare_versions, parse_slug, tag_prefix};
use crate::model::{
  Backport, Commit, DepChange, DepsReport, FileChange, FindReport, FoundCommit, LockfileDiff,
  RawCommit, Report, SCHEMA_VERSION, Selection, TagSeries, Version, WhichReport,
};

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
    let mut total_commits = count_commits(repo, &revisions, selection)?;
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
    if let Some(query) = &selection.grep {
      commits.retain(|c| matches_grep(c, query));
      total_commits = commits.len();
      commits.truncate(selection.max_commits);
    }
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

pub fn find(repo: &Path, query: &str) -> Result<FindReport> {
  git(repo, &["rev-parse", "--git-dir"])
    .with_context(|| format!("{} 不是 git 仓库", repo.display()))?;
  let needle = query.trim();
  if needle.is_empty() {
    bail!("查询条件不能为空");
  }
  let out = git(
    repo,
    &[
      "log",
      "--all",
      "--no-merges",
      "--format=%x1e%H%x1f%an%x1f%as%x1f%s%x1f%b%x1f",
    ],
  )?;
  let all_commits: Vec<Commit> = records(&out)
    .filter_map(|fields| match fields.as_slice() {
      [sha, author, date, subject, body, _] => Some(classify(RawCommit {
        sha: sha.to_string(),
        author: author.to_string(),
        date: date.to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        files: Vec::new(),
      })),
      _ => None,
    })
    .collect();

  let pr_query = needle.strip_prefix('#').and_then(|s| s.parse::<u64>().ok());
  let matched: Vec<&Commit> = all_commits
    .iter()
    .filter(|c| match pr_query {
      Some(pr) => c.pr == Some(pr) || c.trailers.iter().any(|t| t.value == needle),
      None => matches_grep(c, needle),
    })
    .collect();

  let matched_shas: HashSet<&str> = matched.iter().map(|c| c.sha.as_str()).collect();
  let is_pick_of_matched = |c: &Commit| {
    c.trailers
      .iter()
      .filter(|t| t.key == "cherry-picked-from")
      .any(|t| {
        matched_shas
          .iter()
          .any(|sha| sha.starts_with(&t.value) || t.value.starts_with(sha))
      })
  };

  let mut matches = Vec::new();
  for primary in matched.into_iter().filter(|c| !is_pick_of_matched(c)) {
    let (first_tag, series) = containing_series(repo, &primary.sha)?;
    let mut backports = Vec::new();
    for candidate in &all_commits {
      if candidate.sha == primary.sha {
        continue;
      }
      let picked_by_trailer = candidate
        .trailers
        .iter()
        .filter(|t| t.key == "cherry-picked-from")
        .any(|t| primary.sha.starts_with(&t.value) || t.value.starts_with(&primary.sha));
      let same_subject =
        candidate.summary == primary.summary && candidate.category == primary.category;
      if picked_by_trailer || same_subject {
        let (bp_first, bp_series) = containing_series(repo, &candidate.sha)?;
        backports.push(Backport {
          commit: candidate.clone(),
          first_tag: bp_first,
          series: bp_series,
        });
      }
    }
    matches.push(FoundCommit {
      commit: primary.clone(),
      first_tag,
      series,
      backports,
    });
  }

  Ok(FindReport {
    schema_version: SCHEMA_VERSION,
    source: canonical_source(repo),
    query: needle.to_string(),
    matches,
  })
}

pub fn which(repo: &Path, rev: &str) -> Result<WhichReport> {
  git(repo, &["rev-parse", "--git-dir"])
    .with_context(|| format!("{} 不是 git 仓库", repo.display()))?;
  let out = git(
    repo,
    &[
      "log",
      "-1",
      "--format=%x1e%H%x1f%an%x1f%as%x1f%s%x1f%b%x1f",
      rev,
      "--",
    ],
  )
  .with_context(|| format!("找不到提交或引用：{rev}"))?;
  let commit = records(&out)
    .find_map(|fields| match fields.as_slice() {
      [sha, author, date, subject, body, _] => Some(classify(RawCommit {
        sha: sha.to_string(),
        author: author.to_string(),
        date: date.to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        files: Vec::new(),
      })),
      _ => None,
    })
    .with_context(|| format!("无法解析提交：{rev}"))?;

  let points_at = git(repo, &["tag", "--points-at", &commit.sha])?;
  let mut exact_tags: Vec<String> = points_at
    .lines()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .collect();
  exact_tags.sort_by(|a, b| compare_versions(a, b));

  let (base_tag, ahead) = if let Some(first) = exact_tags.first() {
    (Some(first.clone()), 0)
  } else if let Ok(desc) = git(repo, &["describe", "--tags", "--long", &commit.sha]) {
    let mut parts = desc.trim().rsplitn(3, '-');
    let _g = parts.next();
    let ahead = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let base = parts.next().map(str::to_string);
    (base, ahead)
  } else {
    (None, 0)
  };

  let (first_tag, contained_in) = containing_series(repo, &commit.sha)?;
  Ok(WhichReport {
    schema_version: SCHEMA_VERSION,
    source: canonical_source(repo),
    rev: rev.to_string(),
    commit,
    exact_tags,
    base_tag,
    ahead,
    first_tag,
    contained_in,
  })
}

pub fn deps(repo: &Path, from: &str, to: &str) -> Result<DepsReport> {
  git(repo, &["rev-parse", "--git-dir"])
    .with_context(|| format!("{} 不是 git 仓库", repo.display()))?;
  let changed = git(repo, &["diff", "--name-only", from, to])
    .with_context(|| format!("无法对比 {from} 与 {to}"))?;

  let mut lockfiles = Vec::new();
  for file in changed.lines().map(str::trim).filter(|s| !s.is_empty()) {
    let Some(name) = Path::new(file).file_name().and_then(|n| n.to_str()) else {
      continue;
    };
    if !matches!(
      name,
      "Cargo.lock" | "uv.lock" | "package-lock.json" | "go.sum"
    ) {
      continue;
    }
    let before = git(repo, &["show", &format!("{from}:{file}")]).unwrap_or_default();
    let after = git(repo, &["show", &format!("{to}:{file}")]).unwrap_or_default();
    let before_map = parse_lockfile(name, &before);
    let after_map = parse_lockfile(name, &after);

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut updated = Vec::new();

    for (pkg, after_vers) in &after_map {
      match before_map.get(pkg) {
        None => added.push(DepChange {
          name: pkg.clone(),
          from: Vec::new(),
          to: after_vers.clone(),
        }),
        Some(before_vers) if before_vers != after_vers => updated.push(DepChange {
          name: pkg.clone(),
          from: before_vers.clone(),
          to: after_vers.clone(),
        }),
        Some(_) => {}
      }
    }
    for (pkg, before_vers) in &before_map {
      if !after_map.contains_key(pkg) {
        removed.push(DepChange {
          name: pkg.clone(),
          from: before_vers.clone(),
          to: Vec::new(),
        });
      }
    }
    if !added.is_empty() || !removed.is_empty() || !updated.is_empty() {
      lockfiles.push(LockfileDiff {
        file: file.to_string(),
        added,
        removed,
        updated,
      });
    }
  }

  Ok(DepsReport {
    schema_version: SCHEMA_VERSION,
    source: canonical_source(repo),
    from: from.to_string(),
    to: to.to_string(),
    lockfiles,
  })
}

fn parse_lockfile(kind: &str, content: &str) -> BTreeMap<String, Vec<String>> {
  let mut sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
  match kind {
    "Cargo.lock" | "uv.lock" => {
      let mut in_pkg = false;
      let mut name: Option<String> = None;
      let mut version: Option<String> = None;
      let flush = |sets: &mut BTreeMap<String, BTreeSet<String>>,
                   name: &mut Option<String>,
                   version: &mut Option<String>| {
        if let (Some(n), Some(v)) = (name.take(), version.take()) {
          sets.entry(n).or_default().insert(v);
        }
      };
      for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
          flush(&mut sets, &mut name, &mut version);
          in_pkg = line == "[[package]]";
          continue;
        }
        if !in_pkg {
          continue;
        }
        if let Some(rest) = line.strip_prefix("name = ")
          && let Some(val) = unquote(rest)
        {
          name = Some(val.to_string());
        } else if let Some(rest) = line.strip_prefix("version = ")
          && let Some(val) = unquote(rest)
        {
          version = Some(val.to_string());
        }
      }
      flush(&mut sets, &mut name, &mut version);
    }
    "package-lock.json" => {
      if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(packages) = json.get("packages").and_then(|v| v.as_object()) {
          for (key, val) in packages {
            let Some(ver) = val.get("version").and_then(|v| v.as_str()) else {
              continue;
            };
            let pkg = key.rsplit("node_modules/").next().unwrap_or("");
            if !key.is_empty() && !pkg.is_empty() {
              sets
                .entry(pkg.to_string())
                .or_default()
                .insert(ver.to_string());
            }
          }
        } else if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
          for (pkg, val) in deps {
            if let Some(ver) = val.get("version").and_then(|v| v.as_str()) {
              sets.entry(pkg.clone()).or_default().insert(ver.to_string());
            }
          }
        }
      }
    }
    "go.sum" => {
      for line in content.lines() {
        let mut parts = line.split_whitespace();
        let (Some(module), Some(ver)) = (parts.next(), parts.next()) else {
          continue;
        };
        let ver = ver.strip_suffix("/go.mod").unwrap_or(ver);
        sets
          .entry(module.to_string())
          .or_default()
          .insert(ver.to_string());
      }
    }
    _ => {}
  }
  sets
    .into_iter()
    .map(|(k, set)| {
      let mut vers: Vec<String> = set.into_iter().collect();
      vers.sort_by(|a, b| compare_versions(a, b));
      (k, vers)
    })
    .collect()
}

fn unquote(s: &str) -> Option<&str> {
  s.trim().strip_prefix('"')?.strip_suffix('"')
}

fn canonical_source(repo: &Path) -> String {
  repo
    .canonicalize()
    .unwrap_or_else(|_| repo.to_path_buf())
    .display()
    .to_string()
}

/// Tags containing `sha`, grouped by tag prefix and sorted oldest-version-first within each group.
fn containing_series(repo: &Path, sha: &str) -> Result<(Option<String>, Vec<TagSeries>)> {
  let out = git(repo, &["tag", "--contains", sha])?;
  let mut groups: HashMap<String, Vec<String>> = HashMap::new();
  for tag in out.lines().map(str::trim).filter(|s| !s.is_empty()) {
    groups
      .entry(tag_prefix(tag).to_string())
      .or_default()
      .push(tag.to_string());
  }
  let mut series: Vec<TagSeries> = groups
    .into_iter()
    .filter_map(|(prefix, mut tags)| {
      tags.sort_by(|a, b| compare_versions(a, b));
      let first_tag = tags.first()?.clone();
      Some(TagSeries {
        prefix,
        first_tag,
        tags,
      })
    })
    .collect();
  series.sort_by(|a, b| {
    b.tags
      .len()
      .cmp(&a.tags.len())
      .then_with(|| a.prefix.cmp(&b.prefix))
  });
  let first_tag = series.first().map(|s| s.first_tag.clone());
  Ok((first_tag, series))
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

fn count_commits(repo: &Path, revisions: &[String], selection: &Selection) -> Result<usize> {
  let mut args = vec!["rev-list", "--count"];
  if !selection.merges {
    args.push("--no-merges");
  }
  args.extend(revisions.iter().map(String::as_str));
  if !selection.paths.is_empty() {
    args.push("--");
    args.extend(selection.paths.iter().map(String::as_str));
  }
  let out = git(repo, &args)?;
  out
    .trim()
    .parse()
    .with_context(|| format!("无法解析提交数：{out}"))
}

fn log(repo: &Path, revisions: &[String], selection: &Selection) -> Result<Vec<RawCommit>> {
  let max = format!("--max-count={}", selection.max_commits);
  // Put RECORD at the start of the format so trailing `--numstat` lines stay inside the same record.
  let mut args = vec!["log", "--format=%x1e%H%x1f%an%x1f%as%x1f%s%x1f%b%x1f"];
  if selection.grep.is_none() {
    args.push(&max);
  }
  if !selection.merges {
    args.push("--no-merges");
  }
  if selection.files {
    args.push("--numstat");
  }
  args.extend(revisions.iter().map(String::as_str));
  if !selection.paths.is_empty() {
    args.push("--");
    args.extend(selection.paths.iter().map(String::as_str));
  }
  let out = git(repo, &args)?;
  Ok(
    records(&out)
      .filter_map(|fields| match fields.as_slice() {
        [sha, author, date, subject, body, stat] => Some(RawCommit {
          sha: sha.to_string(),
          author: author.to_string(),
          date: date.to_string(),
          subject: subject.to_string(),
          body: body.to_string(),
          files: parse_numstat(stat),
        }),
        _ => None,
      })
      .collect(),
  )
}

fn parse_numstat(stat: &str) -> Vec<FileChange> {
  stat
    .lines()
    .filter_map(|line| {
      let mut parts = line.trim().splitn(3, '\t');
      let added = parts.next()?;
      let deleted = parts.next()?;
      let path = parts.next()?.trim();
      if path.is_empty() {
        return None;
      }
      Some(FileChange {
        path: path.to_string(),
        additions: added.parse().ok(),
        deletions: deleted.parse().ok(),
      })
    })
    .collect()
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
      let path = self.0.join(file);
      if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
      }
      std::fs::write(path, message).unwrap();
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
      paths: Vec::new(),
      grep: None,
      files: false,
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

  #[test]
  fn files_flag_collects_numstat_and_path_and_grep_filter_commits() {
    let repo = TempRepo::new("files-path-grep");
    repo.commit_file("src/a.rs", "feat: init a");
    repo.run(&["tag", "v0.1.0"]);
    repo.commit_file("src/a.rs", "fix(core): memory timeout in a");
    repo.commit_file("docs/readme.md", "docs: update guide");
    repo.run(&["tag", "v0.2.0"]);

    let with_files = collect(
      &repo.0,
      &Selection {
        tag: Some("v0.2.0".into()),
        files: true,
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(with_files.versions[0].commits.len(), 2);
    assert_eq!(
      with_files.versions[0].commits[0].files[0].path,
      "docs/readme.md"
    );
    assert_eq!(
      with_files.versions[0].commits[0].files[0].additions,
      Some(1)
    );
    assert_eq!(with_files.versions[0].commits[1].files[0].path, "src/a.rs");

    let by_path = collect(
      &repo.0,
      &Selection {
        tag: Some("v0.2.0".into()),
        paths: vec!["src/".into()],
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(by_path.versions[0].total_commits, 1);
    assert_eq!(by_path.versions[0].commits.len(), 1);
    assert_eq!(
      by_path.versions[0].commits[0].summary,
      "memory timeout in a"
    );

    let by_grep = collect(
      &repo.0,
      &Selection {
        tag: Some("v0.2.0".into()),
        grep: Some("TIMEOUT".into()),
        ..selection()
      },
    )
    .unwrap();
    assert_eq!(by_grep.versions[0].total_commits, 1);
    assert_eq!(by_grep.versions[0].commits.len(), 1);
    assert_eq!(
      by_grep.versions[0].commits[0].summary,
      "memory timeout in a"
    );
  }

  #[test]
  fn find_locates_first_tag_and_hotfix_backport() {
    let repo = TempRepo::new("find-backport");
    repo.commit_file("a", "feat: base");
    repo.run(&["tag", "v0.1.0"]);
    repo.run(&["branch", "hotfix"]);
    repo.commit_file("b", "fix: null pointer (#42)");
    let main_sha = git(&repo.0, &["rev-parse", "HEAD"])
      .unwrap()
      .trim()
      .to_string();
    repo.run(&["tag", "v0.2.0"]);
    repo.commit_file("c", "feat: later");
    repo.run(&["tag", "v0.3.0"]);

    repo.run(&["checkout", "-q", "hotfix"]);
    repo.run(&["cherry-pick", "-x", &main_sha]);
    let hotfix_sha = git(&repo.0, &["rev-parse", "HEAD"])
      .unwrap()
      .trim()
      .to_string();
    repo.run(&["tag", "v0.1.1"]);

    // Search by short SHA of the main commit: finds v0.2.0 as first_tag and v0.1.1 as backport.
    let by_sha = find(&repo.0, &main_sha[..8]).unwrap();
    assert_eq!(by_sha.matches.len(), 1);
    let m = &by_sha.matches[0];
    assert_eq!(m.commit.sha, main_sha);
    assert_eq!(m.first_tag.as_deref(), Some("v0.2.0"));
    assert_eq!(m.series[0].tags, ["v0.2.0", "v0.3.0"]);
    assert_eq!(m.backports.len(), 1);
    assert_eq!(m.backports[0].commit.sha, hotfix_sha);
    assert_eq!(m.backports[0].first_tag.as_deref(), Some("v0.1.1"));

    // Search by PR number `#42`: deduplicates the cherry-pick into `backports`.
    let by_pr = find(&repo.0, "#42").unwrap();
    assert_eq!(by_pr.matches.len(), 1);
    assert_eq!(by_pr.matches[0].first_tag.as_deref(), Some("v0.2.0"));
    assert_eq!(by_pr.matches[0].backports.len(), 1);
    assert_eq!(
      by_pr.matches[0].backports[0].first_tag.as_deref(),
      Some("v0.1.1")
    );
  }

  #[test]
  fn which_identifies_base_tag_ahead_and_containing_tag() {
    let repo = TempRepo::new("which-rev");
    repo.commit_file("a", "feat: init");
    repo.run(&["tag", "v0.1.0"]);
    repo.commit_file("b", "fix: first");
    let mid_sha = git(&repo.0, &["rev-parse", "HEAD"])
      .unwrap()
      .trim()
      .to_string();
    repo.commit_file("c", "feat: second");
    repo.run(&["tag", "v0.2.0"]);
    repo.commit_file("d", "feat: unreleased work");

    let mid = which(&repo.0, &mid_sha[..7]).unwrap();
    assert_eq!(mid.commit.sha, mid_sha);
    assert!(mid.exact_tags.is_empty());
    assert_eq!(mid.base_tag.as_deref(), Some("v0.1.0"));
    assert_eq!(mid.ahead, 1);
    assert_eq!(mid.first_tag.as_deref(), Some("v0.2.0"));

    let head = which(&repo.0, "HEAD").unwrap();
    assert_eq!(head.base_tag.as_deref(), Some("v0.2.0"));
    assert_eq!(head.ahead, 1);
    assert_eq!(head.first_tag, None, "HEAD is after the latest tag");
  }

  #[test]
  fn deps_diffs_cargo_uv_npm_and_go_lockfiles() {
    let repo = TempRepo::new("deps-diff");
    repo.commit_file(
      "Cargo.lock",
      "[[package]]\nname = \"serde\"\nversion = \"1.0.200\"\n\n\
       [[package]]\nname = \"old-crate\"\nversion = \"0.1.0\"\n",
    );
    repo.commit_file(
      "uv.lock",
      "[[package]]\nname = \"pydantic\"\nversion = \"2.8.0\"\n",
    );
    repo.commit_file(
      "package-lock.json",
      r#"{"packages":{"":{"name":"app"},"node_modules/react":{"version":"18.2.0"}}}"#,
    );
    repo.commit_file("go.sum", "golang.org/x/sys v0.19.0 h1:abc\n");
    repo.run(&["tag", "v0.1.0"]);

    repo.commit_file(
      "Cargo.lock",
      "[[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n\n\
       [[package]]\nname = \"new-crate\"\nversion = \"0.2.0\"\n",
    );
    repo.commit_file(
      "uv.lock",
      "[[package]]\nname = \"pydantic\"\nversion = \"2.9.2\"\n",
    );
    repo.commit_file(
      "package-lock.json",
      r#"{"packages":{"":{"name":"app"},"node_modules/react":{"version":"18.3.1"}}}"#,
    );
    repo.commit_file("go.sum", "golang.org/x/sys v0.20.0 h1:def\n");
    repo.run(&["tag", "v0.2.0"]);

    let report = deps(&repo.0, "v0.1.0", "v0.2.0").unwrap();
    assert_eq!(report.lockfiles.len(), 4);

    let cargo = report
      .lockfiles
      .iter()
      .find(|l| l.file == "Cargo.lock")
      .unwrap();
    assert_eq!(cargo.added[0].name, "new-crate");
    assert_eq!(cargo.added[0].to, ["0.2.0"]);
    assert_eq!(cargo.removed[0].name, "old-crate");
    assert_eq!(cargo.removed[0].from, ["0.1.0"]);
    assert_eq!(cargo.updated[0].name, "serde");
    assert_eq!(cargo.updated[0].from, ["1.0.200"]);
    assert_eq!(cargo.updated[0].to, ["1.0.210"]);

    let uv = report
      .lockfiles
      .iter()
      .find(|l| l.file == "uv.lock")
      .unwrap();
    assert_eq!(uv.updated[0].name, "pydantic");
    assert_eq!(uv.updated[0].from, ["2.8.0"]);
    assert_eq!(uv.updated[0].to, ["2.9.2"]);

    let npm = report
      .lockfiles
      .iter()
      .find(|l| l.file == "package-lock.json")
      .unwrap();
    assert_eq!(npm.updated[0].name, "react");
    assert_eq!(npm.updated[0].from, ["18.2.0"]);
    assert_eq!(npm.updated[0].to, ["18.3.1"]);

    let go = report
      .lockfiles
      .iter()
      .find(|l| l.file == "go.sum")
      .unwrap();
    assert_eq!(go.updated[0].name, "golang.org/x/sys");
    assert_eq!(go.updated[0].from, ["v0.19.0"]);
    assert_eq!(go.updated[0].to, ["v0.20.0"]);
  }
}
