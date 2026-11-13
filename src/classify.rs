//! Commit classification: Conventional Commits first, keyword heuristics second.

use crate::model::{Category, Commit, RawCommit};

const FIRST_WORDS: &[(Category, &[&str])] = &[
  (
    Category::Feature,
    &[
      "add",
      "adds",
      "added",
      "adding",
      "support",
      "supports",
      "implement",
      "implements",
      "implemented",
      "introduce",
      "introduces",
      "introduced",
      "new",
      "allow",
      "allows",
      "enable",
      "enables",
      "expose",
      "exposes",
      "provide",
      "provides",
      "feat",
      "feature",
      "improve",
      "improves",
      "improved",
      "enhance",
      "enhances",
    ],
  ),
  (
    Category::Fix,
    &[
      "fix",
      "fixes",
      "fixed",
      "fixing",
      "resolve",
      "resolves",
      "resolved",
      "correct",
      "corrects",
      "corrected",
      "prevent",
      "prevents",
      "avoid",
      "bugfix",
      "hotfix",
      "repair",
      "workaround",
    ],
  ),
  (
    Category::Perf,
    &[
      "perf",
      "optimize",
      "optimise",
      "optimized",
      "optimised",
      "speed",
      "speedup",
      "accelerate",
    ],
  ),
  (
    Category::Refactor,
    &[
      "refactor",
      "refactored",
      "refactoring",
      "cleanup",
      "clean",
      "simplify",
      "simplified",
      "rename",
      "renamed",
      "restructure",
      "reorganize",
    ],
  ),
  (
    Category::Docs,
    &["doc", "docs", "document", "documented", "readme", "typo"],
  ),
  (
    Category::Chore,
    &[
      "bump", "release", "chore", "ci", "test", "tests", "upgrade", "revert", "format", "fmt",
      "lint", "prepare", "merge",
    ],
  ),
];

const CJK_PREFIXES: &[(Category, &[&str])] = &[
  (
    Category::Feature,
    &["新增", "增加", "添加", "支持", "实现", "引入", "新功能"],
  ),
  (Category::Fix, &["修复", "修正", "解决"]),
  (Category::Perf, &["优化", "性能"]),
  (Category::Refactor, &["重构", "清理", "简化", "重命名"]),
  (Category::Docs, &["文档", "注释"]),
  (Category::Chore, &["升级", "发布", "依赖", "测试", "格式化"]),
];

/// Weaker signals, only used when neither the prefix nor the first word decides.
const CONTAINS: &[(Category, &[&str])] = &[
  (
    Category::Fix,
    &[
      "bug",
      "crash",
      "panic",
      "regression",
      "修复",
      "崩溃",
      "错误",
    ],
  ),
  (Category::Perf, &["performance", "faster", "性能"]),
  (Category::Docs, &["readme", "documentation", "文档"]),
];

pub fn classify(raw: RawCommit) -> Commit {
  let (subject, pr) = strip_pr(raw.subject.trim());
  let body_breaking =
    raw.body.contains("BREAKING CHANGE:") || raw.body.contains("BREAKING-CHANGE:");

  let (category, scope, summary, bang) = match parse_prefix(subject) {
    Some(p) => match conventional_type(p.kind) {
      Some(category) => (category, p.scope, p.rest, p.bang),
      // `net/http: fix ...` style: the prefix is a component, not a type.
      None => (heuristic(p.rest), Some(p.kind), p.rest, p.bang),
    },
    None => (heuristic(subject), None, subject, false),
  };

  Commit {
    sha: raw.sha,
    author: raw.author,
    date: raw.date,
    category,
    breaking: bang || body_breaking,
    scope: scope.map(str::to_string),
    summary: summary.to_string(),
    pr,
  }
}

struct Prefix<'a> {
  kind: &'a str,
  scope: Option<&'a str>,
  bang: bool,
  rest: &'a str,
}

/// Parses `type(scope)!: rest`. The colon must be followed by whitespace so URLs don't match.
fn parse_prefix(subject: &str) -> Option<Prefix<'_>> {
  let colon = subject.find(": ")?;
  let rest = subject[colon + 2..].trim();
  let mut head = &subject[..colon];
  if rest.is_empty() || head.is_empty() {
    return None;
  }
  let bang = head.ends_with('!');
  if bang {
    head = &head[..head.len() - 1];
  }
  let (kind, scope) = match head.find('(') {
    Some(open) if head.ends_with(')') => (&head[..open], Some(&head[open + 1..head.len() - 1])),
    Some(_) => return None,
    None => (head, None),
  };
  if kind.is_empty() || kind.chars().any(char::is_whitespace) {
    return None;
  }
  Some(Prefix {
    kind,
    scope: scope.filter(|s| !s.is_empty()),
    bang,
    rest,
  })
}

fn conventional_type(kind: &str) -> Option<Category> {
  let category = match kind.to_ascii_lowercase().as_str() {
    "feat" | "feature" => Category::Feature,
    "fix" | "bugfix" | "hotfix" => Category::Fix,
    "perf" => Category::Perf,
    "refactor" => Category::Refactor,
    "docs" | "doc" => Category::Docs,
    "test" | "tests" | "build" | "ci" | "chore" | "style" | "deps" | "revert" | "release" => {
      Category::Chore
    }
    _ => return None,
  };
  Some(category)
}

fn heuristic(text: &str) -> Category {
  let lower = text.trim().to_lowercase();
  let first = lower
    .split(|c: char| !c.is_ascii_alphanumeric())
    .next()
    .unwrap_or("");
  let by_first_word = FIRST_WORDS.iter().find(|(_, words)| words.contains(&first));
  let by_cjk_prefix = || {
    CJK_PREFIXES
      .iter()
      .find(|(_, words)| words.iter().any(|w| lower.starts_with(w)))
  };
  let by_contains = || {
    CONTAINS
      .iter()
      .find(|(_, words)| words.iter().any(|w| lower.contains(w)))
  };
  by_first_word
    .or_else(by_cjk_prefix)
    .or_else(by_contains)
    .map(|(category, _)| *category)
    .unwrap_or_else(|| {
      if is_version_bump(&lower) {
        Category::Chore
      } else {
        Category::Other
      }
    })
}

/// Release commits titled just `15.2.0` or `grep-pcre2-0.1.10`.
fn is_version_bump(text: &str) -> bool {
  !text.is_empty()
    && !text.contains(char::is_whitespace)
    && text.rsplit(['-', 'v']).next().is_some_and(|tail| {
      tail.contains('.') && tail.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}

/// Extracts a PR number from `... (#123)` or `Merge pull request #123 from ...`.
fn strip_pr(subject: &str) -> (&str, Option<u64>) {
  if let Some(inner) = subject.strip_suffix(')') {
    if let Some(open) = inner.rfind("(#") {
      if let Ok(number) = inner[open + 2..].parse::<u64>() {
        return (inner[..open].trim_end(), Some(number));
      }
    }
  }
  if let Some(rest) = subject.strip_prefix("Merge pull request #") {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    return (subject, digits.parse().ok());
  }
  (subject, None)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn commit(subject: &str, body: &str) -> Commit {
    classify(RawCommit {
      sha: "abc".into(),
      author: "a".into(),
      date: "2026-01-01".into(),
      subject: subject.into(),
      body: body.into(),
    })
  }

  #[test]
  fn conventional_types_map_to_categories() {
    let cases = [
      ("feat: add x", Category::Feature),
      ("fix(parser): handle y", Category::Fix),
      ("perf: faster z", Category::Perf),
      ("refactor: split", Category::Refactor),
      ("docs: readme", Category::Docs),
      ("ci: cache", Category::Chore),
      ("chore(deps): bump serde", Category::Chore),
    ];
    for (subject, expected) in cases {
      assert_eq!(commit(subject, "").category, expected, "{subject}");
    }
  }

  #[test]
  fn scope_bang_and_summary_are_extracted() {
    let c = commit("feat(api)!: drop v1 endpoints", "");
    assert_eq!(c.scope.as_deref(), Some("api"));
    assert!(c.breaking);
    assert_eq!(c.summary, "drop v1 endpoints");
  }

  #[test]
  fn breaking_change_footer_marks_breaking() {
    assert!(
      commit(
        "fix: rename flag",
        "body\n\nBREAKING CHANGE: --foo is now --bar"
      )
      .breaking
    );
    assert!(!commit("fix: rename flag", "no break here").breaking);
  }

  #[test]
  fn unknown_type_is_treated_as_scope() {
    let c = commit("net/http: fix race in Transport", "");
    assert_eq!(c.category, Category::Fix);
    assert_eq!(c.scope.as_deref(), Some("net/http"));
    assert_eq!(c.summary, "fix race in Transport");
  }

  #[test]
  fn heuristics_on_free_form_subjects() {
    let cases = [
      ("Add support for TOML", Category::Feature),
      ("Fixed crash on empty input", Category::Fix),
      ("Update README", Category::Docs),
      ("Handle panic when file missing", Category::Fix),
      ("Bump version to 1.2.0", Category::Chore),
      ("新增导出功能", Category::Feature),
      ("修复登录失败的问题", Category::Fix),
      ("调整一下按钮颜色", Category::Other),
      ("Tweak colors", Category::Other),
      ("15.2.0", Category::Chore),
      ("grep-pcre2-0.1.10", Category::Chore),
      ("v1.2.3", Category::Chore),
      ("utf-8", Category::Other),
      ("x86_64", Category::Other),
    ];
    for (subject, expected) in cases {
      assert_eq!(commit(subject, "").category, expected, "{subject}");
    }
  }

  #[test]
  fn urls_and_colons_without_space_are_not_prefixes() {
    let c = commit("See https://example.com for details", "");
    assert_eq!(c.scope, None);
    assert_eq!(c.category, Category::Other);
  }

  #[test]
  fn pr_number_is_extracted_and_stripped() {
    let c = commit("feat: add thing (#123)", "");
    assert_eq!(c.pr, Some(123));
    assert_eq!(c.summary, "add thing");
    assert_eq!(commit("Merge pull request #45 from a/b", "").pr, Some(45));
    assert_eq!(commit("fix: not a pr (#abc)", "").pr, None);
  }
}
