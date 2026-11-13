//! GitHub collector: tags via `git ls-remote`, releases and ranges via the REST API.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::classify::{classify, link_reverts, matches_grep};
use crate::model::{RawCommit, Release, Report, SCHEMA_VERSION, Selection, Version};

const API: &str = "https://api.github.com";

/// Extracts `owner/repo` from GitHub URLs or a bare `owner/repo`.
pub fn parse_slug(input: &str) -> Option<String> {
  let s = input.trim();
  let without_scheme = ["https://", "http://", "ssh://"]
    .iter()
    .find_map(|scheme| s.strip_prefix(scheme))
    .unwrap_or(s);
  let without_user = without_scheme
    .strip_prefix("git@")
    .unwrap_or(without_scheme);
  let path = match ["github.com/", "github.com:"]
    .iter()
    .find_map(|h| without_user.strip_prefix(h))
  {
    Some(path) => path,
    // Something with a scheme or user but another host is not a GitHub repo.
    None if without_user.len() != s.len() || s.contains(':') => return None,
    None if s.trim_end_matches('/').matches('/').count() != 1 => return None,
    None => s,
  };
  let mut segments = path.trim_end_matches('/').split('/');
  let owner = segments.next()?;
  let repo = segments.next()?;
  let repo = repo.strip_suffix(".git").unwrap_or(repo);
  let valid = |part: &str| {
    !part.is_empty()
      && part
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
  };
  (valid(owner) && valid(repo)).then(|| format!("{owner}/{repo}"))
}

pub struct GitHub {
  client: Client,
  token: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
  tag_name: String,
  name: Option<String>,
  body: Option<String>,
  published_at: Option<String>,
  prerelease: bool,
  draft: bool,
  html_url: String,
}

#[derive(Deserialize)]
struct ApiCompare {
  total_commits: usize,
  commits: Vec<ApiCommit>,
}

#[derive(Deserialize)]
struct ApiCommit {
  sha: String,
  commit: ApiCommitDetail,
  parents: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct ApiCommitDetail {
  author: Option<ApiSignature>,
  message: String,
}

#[derive(Deserialize)]
struct ApiSignature {
  name: String,
  date: String,
}

#[derive(Deserialize)]
struct ApiRepo {
  default_branch: String,
}

impl GitHub {
  pub fn new(token: Option<String>) -> Result<Self> {
    let client = Client::builder()
      .user_agent(concat!("git-changelist/", env!("CARGO_PKG_VERSION")))
      .timeout(Duration::from_secs(30))
      .build()
      .context("无法创建 HTTP 客户端")?;
    Ok(Self { client, token })
  }

  fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
    let mut request = self
      .client
      .get(format!("{API}{path}"))
      .header("Accept", "application/vnd.github+json")
      .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = &self.token {
      request = request.bearer_auth(token);
    }
    let response = request
      .send()
      .with_context(|| format!("请求 GitHub 失败：{path}"))?;
    let status = response.status();
    if status.is_success() {
      return response
        .json()
        .with_context(|| format!("无法解析 GitHub 响应：{path}"));
    }
    let remaining = response
      .headers()
      .get("x-ratelimit-remaining")
      .and_then(|v| v.to_str().ok());
    if matches!(
      status,
      StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) && remaining == Some("0")
    {
      bail!(
        "GitHub API 已限流（匿名 60 次/小时）。设置环境变量 GITHUB_TOKEN 或 --token 可提升到 5000 次/小时"
      );
    }
    if status == StatusCode::NOT_FOUND {
      bail!("GitHub 返回 404：{path}（仓库或 Tag 不存在；私有仓库请设置 GITHUB_TOKEN）");
    }
    bail!(
      "GitHub 返回 {status}：{path}：{}",
      response.text().unwrap_or_default()
    );
  }

  /// Most recent 100 published releases, keyed by tag.
  pub fn releases(&self, slug: &str) -> Result<HashMap<String, Release>> {
    let releases: Vec<ApiRelease> = self.get(&format!("/repos/{slug}/releases?per_page=100"))?;
    Ok(
      releases
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| {
          let release = Release {
            name: r
              .name
              .filter(|n| !n.trim().is_empty())
              .unwrap_or_else(|| r.tag_name.clone()),
            body: r.body.unwrap_or_default(),
            published_at: r.published_at.map(|d| d.chars().take(10).collect()),
            prerelease: r.prerelease,
            url: r.html_url,
          };
          (r.tag_name, release)
        })
        .collect(),
    )
  }

  /// Commits in `base...head`, newest first, plus the range's total commit count.
  fn compare(
    &self,
    slug: &str,
    base: &str,
    head: &str,
    selection: &Selection,
  ) -> Result<(Vec<RawCommit>, usize)> {
    let path = format!("/repos/{slug}/compare/{}...{}", encode(base), encode(head));
    let compare: ApiCompare = self.get(&path)?;
    let returned = compare.commits.len();
    let mut commits: Vec<RawCommit> = compare
      .commits
      .into_iter()
      .rev()
      .filter(|c| selection.merges || c.parents.len() <= 1)
      .map(|c| {
        let (subject, body) = c
          .commit
          .message
          .split_once('\n')
          .unwrap_or((&c.commit.message, ""));
        let (author, date) = c
          .commit
          .author
          .map(|a| (a.name, a.date.chars().take(10).collect()))
          .unwrap_or_default();
        RawCommit {
          sha: c.sha,
          author,
          date,
          subject: subject.to_string(),
          body: body.trim().to_string(),
          files: Vec::new(),
        }
      })
      .collect();
    // When the API returned everything, count what we kept; otherwise only the API total is known.
    let total = if returned >= compare.total_commits {
      commits.len()
    } else {
      compare.total_commits
    };
    commits.truncate(selection.max_commits);
    Ok((commits, total))
  }
}

pub fn collect(slug: &str, selection: &Selection, github: &GitHub) -> Result<Report> {
  let groups = group_tags(remote_tags(slug)?);
  let previous_of = |tag: &str| -> Option<String> {
    groups.iter().find_map(|group| {
      let index = group.iter().position(|t| t == tag)?;
      group.get(index + 1).cloned()
    })
  };
  let ordered: Vec<&String> = groups.iter().flatten().collect();

  // (tag or None for unreleased, previous, head ref)
  let mut ranges: Vec<(Option<String>, Option<String>, String)> = Vec::new();
  if let Some((from, to)) = &selection.range {
    for (head, previous) in split_range(&groups, from, to) {
      ranges.push((Some(head.clone()), Some(previous), head));
    }
  } else if let Some(tag) = &selection.tag {
    if !ordered.contains(&tag) {
      bail!("找不到 Tag：{tag}");
    }
    ranges.push((Some(tag.clone()), previous_of(tag), tag.clone()));
  } else {
    if selection.unreleased
      && let Some(latest) = ordered.first()
    {
      let repo: ApiRepo = github.get(&format!("/repos/{slug}"))?;
      ranges.push((None, Some((*latest).clone()), repo.default_branch));
    }
    for tag in ordered.iter().take(selection.limit) {
      ranges.push((Some((*tag).clone()), previous_of(tag), (*tag).clone()));
    }
  }

  let mut releases = github.releases(slug)?;
  let mut versions = Vec::with_capacity(ranges.len());
  let mut warnings = Vec::new();
  if selection.files {
    warnings.push(
      "GitHub 远端模式不支持 --files（Compare API 不返回每个提交的文件明细）；请克隆到本地仓库运行"
        .to_string(),
    );
  }
  if !selection.paths.is_empty() {
    warnings.push("GitHub 远端模式不支持 --path 按路径过滤；请克隆到本地仓库运行".to_string());
  }
  if selection.range.is_some() && ranges.len() > 1 {
    warnings.push(
      "GitHub 远端模式按版本号切分区间，未做本地 cherry-pick 补丁去重；如需 hotfix 分支精确去重请克隆到本地运行"
        .to_string(),
    );
  }
  for (tag, previous, head) in ranges {
    // The first version has no base to compare against; skip its history to save requests.
    let (raw_commits, mut total_commits) = match &previous {
      Some(base) => github.compare(slug, base, &head, selection)?,
      None => (Vec::new(), 0),
    };
    if tag.is_none() && total_commits == 0 {
      continue;
    }
    let release = tag.as_ref().and_then(|t| releases.remove(t));
    let date = release
      .as_ref()
      .and_then(|r| r.published_at.clone())
      .or_else(|| raw_commits.first().map(|c| c.date.clone()));
    let mut commits: Vec<_> = raw_commits.into_iter().map(classify).collect();
    link_reverts(&mut commits);
    if let Some(query) = &selection.grep {
      commits.retain(|c| matches_grep(c, query));
      total_commits = commits.len();
    }
    let truncated = total_commits > commits.len();
    if truncated {
      warnings.push(format!(
        "版本 {head} 共 {total_commits} 个提交，仅获取到 {} 个（受 --max-commits={} 或 GitHub Compare 250 条上限限制）",
        commits.len(),
        selection.max_commits
      ));
    }
    versions.push(Version {
      tag,
      previous,
      date,
      tag_message: None,
      release,
      commits,
      total_commits,
      truncated,
    });
  }

  Ok(Report {
    schema_version: SCHEMA_VERSION,
    source: format!("https://github.com/{slug}"),
    github: Some(slug.to_string()),
    warnings,
    versions,
  })
}

/// All tag names of a GitHub repo, in one round trip and without API quota.
fn remote_tags(slug: &str) -> Result<Vec<String>> {
  let output = Command::new("git")
    .args([
      "ls-remote",
      "--tags",
      "--refs",
      &format!("https://github.com/{slug}.git"),
    ])
    .env("GIT_TERMINAL_PROMPT", "0")
    .output()
    .context("无法执行 git，请确认已安装 git")?;
  if !output.status.success() {
    bail!(
      "无法获取 {slug} 的 Tag 列表：{}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(
    String::from_utf8_lossy(&output.stdout)
      .lines()
      .filter_map(|line| {
        line
          .split_once("\trefs/tags/")
          .map(|(_, name)| name.to_string())
      })
      .collect(),
  )
}

/// `(head, previous)` segments of `from..to`, split at every tag of the endpoints' prefix group
/// whose version lies between them, newest first. Without a tag endpoint it stays one block.
fn split_range(groups: &[Vec<String>], from: &str, to: &str) -> Vec<(String, String)> {
  let Some(group) = groups
    .iter()
    .find(|g| g.iter().any(|t| t == to || t == from))
  else {
    return vec![(to.to_string(), from.to_string())];
  };
  let to_is_tag = group.iter().any(|t| t == to);
  let from_is_tag = group.iter().any(|t| t == from);
  let mut heads: Vec<&str> = vec![to];
  heads.extend(group.iter().map(String::as_str).filter(|t| {
    let below_to = !to_is_tag || compare_versions(t, to) == Ordering::Less;
    let above_from = !from_is_tag || compare_versions(t, from) == Ordering::Greater;
    *t != to && *t != from && below_to && above_from
  }));
  let previous = heads.iter().skip(1).copied().chain(std::iter::once(from));
  heads
    .iter()
    .zip(previous)
    .map(|(head, previous)| (head.to_string(), previous.to_string()))
    .collect()
}

/// Groups tags by their non-numeric prefix, each sorted newest version first;
/// groups with more tags come first (the main package of a monorepo).
fn group_tags(tags: Vec<String>) -> Vec<Vec<String>> {
  let mut groups: HashMap<String, Vec<String>> = HashMap::new();
  for tag in tags {
    groups
      .entry(tag_prefix(&tag).to_string())
      .or_default()
      .push(tag);
  }
  let mut groups: Vec<(String, Vec<String>)> = groups.into_iter().collect();
  for (_, tags) in &mut groups {
    tags.sort_by(|a, b| compare_versions(b, a));
  }
  groups.sort_by(|(pa, a), (pb, b)| b.len().cmp(&a.len()).then_with(|| pa.cmp(pb)));
  groups.into_iter().map(|(_, tags)| tags).collect()
}

/// Non-numeric prefix of a tag: `tokio-macros-2.1.0` → `tokio-macros-`, `v1.0` → `v`.
pub fn tag_prefix(tag: &str) -> &str {
  version_key(tag).0
}

/// `tokio-1.40.0-rc.1` → (`tokio-`, [1, 40, 0], `-rc.1`)
fn version_key(tag: &str) -> (&str, Vec<u64>, &str) {
  let Some(start) = tag.find(|c: char| c.is_ascii_digit()) else {
    return (tag, Vec::new(), "");
  };
  let rest = &tag[start..];
  let end = rest
    .find(|c: char| !c.is_ascii_digit() && c != '.')
    .unwrap_or(rest.len());
  let numbers = rest[..end]
    .split('.')
    .filter_map(|n| n.parse().ok())
    .collect();
  (&tag[..start], numbers, rest[end..].trim_start_matches('.'))
}

pub fn compare_versions(a: &str, b: &str) -> Ordering {
  let (prefix_a, numbers_a, suffix_a) = version_key(a);
  let (prefix_b, numbers_b, suffix_b) = version_key(b);
  prefix_a
    .cmp(prefix_b)
    .then_with(|| numbers_a.cmp(&numbers_b))
    .then_with(|| {
      match (suffix_a.is_empty(), suffix_b.is_empty()) {
        (true, true) => Ordering::Equal,
        // A suffix marks a pre-release, which sorts before the release.
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => compare_versions(suffix_a, suffix_b),
      }
    })
}

/// Percent-encodes a ref for a URL path, keeping `/` which GitHub accepts in refs.
fn encode(reference: &str) -> String {
  reference
    .bytes()
    .map(|b| match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
        (b as char).to_string()
      }
      _ => format!("%{b:02X}"),
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn slugs_from_various_forms() {
    let ok = [
      "https://github.com/BurntSushi/ripgrep",
      "https://github.com/BurntSushi/ripgrep.git",
      "https://github.com/BurntSushi/ripgrep/releases/tag/14.0.0",
      "github.com/BurntSushi/ripgrep/",
      "git@github.com:BurntSushi/ripgrep.git",
      "ssh://git@github.com/BurntSushi/ripgrep.git",
      "BurntSushi/ripgrep",
    ];
    for input in ok {
      assert_eq!(
        parse_slug(input).as_deref(),
        Some("BurntSushi/ripgrep"),
        "{input}"
      );
    }
    let bad = [
      "https://gitlab.com/a/b",
      "git@gitlab.com:a/b.git",
      "a/b/c",
      "ripgrep",
      "./a/b",
      "a b/c",
    ];
    for input in bad {
      assert_eq!(parse_slug(input), None, "{input}");
    }
  }

  #[test]
  fn versions_sort_numerically_with_prereleases_first() {
    let mut tags = vec![
      "v1.10.0",
      "v1.2.0",
      "v1.10.0-rc.2",
      "v1.10.0-rc.10",
      "v1.9.9",
    ];
    tags.sort_by(|a, b| compare_versions(b, a));
    assert_eq!(
      tags,
      [
        "v1.10.0",
        "v1.10.0-rc.10",
        "v1.10.0-rc.2",
        "v1.9.9",
        "v1.2.0"
      ]
    );
  }

  #[test]
  fn monorepo_tags_are_grouped_by_prefix() {
    let tags = [
      "tokio-1.2.0",
      "tokio-macros-0.1.0",
      "tokio-1.10.0",
      "tokio-1.9.0",
      "tokio-macros-0.2.0",
    ];
    let groups = group_tags(tags.iter().map(|t| t.to_string()).collect());
    assert_eq!(groups[0], ["tokio-1.10.0", "tokio-1.9.0", "tokio-1.2.0"]);
    assert_eq!(groups[1], ["tokio-macros-0.2.0", "tokio-macros-0.1.0"]);
  }

  #[test]
  fn range_is_split_within_prefix_group() {
    let tags = [
      "v0.5.4.1",
      "v0.5.4",
      "v0.5.3",
      "react@0.1.9",
      "v0.5.2",
      "v0.5.1.1",
      "v0.5.1",
    ];
    let groups = group_tags(tags.iter().map(|t| t.to_string()).collect());
    let pairs = |from, to| -> Vec<(String, String)> { split_range(&groups, from, to) };
    let expected = [
      ("v0.5.4.1", "v0.5.4"),
      ("v0.5.4", "v0.5.3"),
      ("v0.5.3", "v0.5.2"),
      ("v0.5.2", "v0.5.1.1"),
    ];
    let expected: Vec<_> = expected
      .iter()
      .map(|(a, b)| (a.to_string(), b.to_string()))
      .collect();
    assert_eq!(pairs("v0.5.1.1", "v0.5.4.1"), expected);
    // Branch head: every tag above `from` precedes it.
    assert_eq!(
      pairs("v0.5.4", "main")[..2],
      [
        ("main".into(), "v0.5.4.1".into()),
        ("v0.5.4.1".into(), "v0.5.4".into())
      ]
    );
    assert_eq!(
      pairs("abc123", "def456"),
      [("def456".to_string(), "abc123".to_string())]
    );
  }

  #[test]
  fn refs_are_percent_encoded() {
    assert_eq!(encode("release/v1.0+build"), "release/v1.0%2Bbuild");
  }
}
