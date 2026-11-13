use serde::Serialize;

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
  pub breaking: bool,
  pub scope: Option<String>,
  /// Subject with the conventional prefix and trailing `(#123)` stripped.
  pub summary: String,
  pub pr: Option<u64>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
  /// Local path or `owner/repo`.
  pub source: String,
  /// `owner/repo` when known, used for PR / commit links.
  pub github: Option<String>,
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
