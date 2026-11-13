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

/// A commit as read from git / GitHub, before classification.
#[derive(Debug, Clone)]
pub struct RawCommit {
  pub sha: String,
  pub author: String,
  pub date: String,
  pub subject: String,
  pub body: String,
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
}
