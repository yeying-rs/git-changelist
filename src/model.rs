use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

/// Display order of groups inside a version is the declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
  Feature,
  Fix,
  Perf,
  Refactor,
  Docs,
  Chore,
  Other,
}

impl Category {
  pub const ALL: [Category; 7] = [
    Category::Feature,
    Category::Fix,
    Category::Perf,
    Category::Refactor,
    Category::Docs,
    Category::Chore,
    Category::Other,
  ];
}

/// How a commit's `Category` was decided, so AI consumers know how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClassifiedBy {
  Conventional,
  Keyword,
  None,
}

/// Structured metadata extracted from a commit message body or subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trailer {
  pub key: String,
  pub value: String,
}

/// Per-file diff stat from `git log --numstat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
  pub path: String,
  /// `None` for binary files.
  pub additions: Option<u64>,
  pub deletions: Option<u64>,
}

/// A commit as read from git / GitHub, before classification.
#[derive(Debug, Clone)]
pub struct RawCommit {
  pub sha: String,
  pub author: String,
  pub date: String,
  pub subject: String,
  pub body: String,
  pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
  pub sha: String,
  pub author: String,
  pub date: String,
  pub category: Category,
  pub classified_by: ClassifiedBy,
  pub breaking: bool,
  pub scope: Option<String>,
  /// Subject with the conventional prefix and trailing `(#123)` stripped.
  pub summary: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub body: Option<String>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub trailers: Vec<Trailer>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub files: Vec<FileChange>,
  pub pr: Option<u64>,
  /// SHA of an earlier commit that this commit reverts.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reverts: Option<String>,
  /// SHA of a later commit in the same range that reverted this commit.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reverted_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
  pub name: String,
  pub body: String,
  pub published_at: Option<String>,
  pub prerelease: bool,
  pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Version {
  /// Tag name, or `None` for unreleased commits after the latest tag.
  pub tag: Option<String>,
  pub previous: Option<String>,
  pub date: Option<String>,
  /// Message of an annotated tag.
  pub tag_message: Option<String>,
  pub release: Option<Release>,
  pub commits: Vec<Commit>,
  /// Commits in the range; may exceed `commits.len()` when truncated.
  pub total_commits: usize,
  /// True when `commits.len() < total_commits`.
  pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
  pub schema_version: u32,
  /// Local path or `owner/repo`.
  pub source: String,
  /// `owner/repo` when known, used for PR / commit links.
  pub github: Option<String>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub warnings: Vec<String>,
  pub versions: Vec<Version>,
}

/// Which versions to show; shared by the local and GitHub collectors.
#[derive(Debug, Clone)]
pub struct Selection {
  pub limit: usize,
  pub tag: Option<String>,
  /// `(from, to)` of `--range from..to`.
  pub range: Option<(String, String)>,
  pub unreleased: bool,
  pub max_commits: usize,
  pub merges: bool,
  pub paths: Vec<String>,
  pub grep: Option<String>,
  pub files: bool,
}

/// Tags of one prefix series (`v*`, `react@*`, …) that contain a given commit, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagSeries {
  pub prefix: String,
  pub first_tag: String,
  pub tags: Vec<String>,
}

/// A cherry-picked copy of a commit on another branch/release line.
#[derive(Debug, Clone, Serialize)]
pub struct Backport {
  pub commit: Commit,
  pub first_tag: Option<String>,
  pub series: Vec<TagSeries>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FoundCommit {
  pub commit: Commit,
  pub first_tag: Option<String>,
  pub series: Vec<TagSeries>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub backports: Vec<Backport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindReport {
  pub schema_version: u32,
  pub source: String,
  pub query: String,
  pub matches: Vec<FoundCommit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhichReport {
  pub schema_version: u32,
  pub source: String,
  pub rev: String,
  pub commit: Commit,
  pub exact_tags: Vec<String>,
  pub base_tag: Option<String>,
  pub ahead: usize,
  pub first_tag: Option<String>,
  pub contained_in: Vec<TagSeries>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepChange {
  pub name: String,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub from: Vec<String>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockfileDiff {
  pub file: String,
  pub added: Vec<DepChange>,
  pub removed: Vec<DepChange>,
  pub updated: Vec<DepChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DepsReport {
  pub schema_version: u32,
  pub source: String,
  pub from: String,
  pub to: String,
  pub lockfiles: Vec<LockfileDiff>,
}
