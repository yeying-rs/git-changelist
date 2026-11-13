//! Turns a `Report` into human-readable text.

use std::fmt::Write;

use clap::ValueEnum;

use crate::model::{Category, Commit, Report, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Lang {
  Zh,
  En,
}

struct Texts {
  breaking: &'static str,
  categories: [&'static str; 7],
  unreleased: &'static str,
  commits: &'static str,
  tag_note: &'static str,
  release: &'static str,
  first_version: &'static str,
  truncated: &'static str,
  no_versions: &'static str,
  no_commits: &'static str,
}

const ZH: Texts = Texts {
  breaking: "⚠️  破坏性变更",
  categories: ["✨ 新增功能", "🐛 问题修复", "⚡ 性能优化", "♻️  重构", "📝 文档", "🔧 维护", "📦 其他"],
  unreleased: "未发布",
  commits: "个提交",
  tag_note: "Tag 说明",
  release: "Release 说明",
  first_version: "首个版本",
  truncated: "仅显示 {shown} / 共 {total} 个提交",
  no_versions: "没有找到任何版本",
  no_commits: "（无提交明细）",
};

const EN: Texts = Texts {
  breaking: "⚠️  Breaking Changes",
  categories: [
    "✨ Features",
    "🐛 Bug Fixes",
    "⚡ Performance",
    "♻️  Refactoring",
    "📝 Documentation",
    "🔧 Chores",
    "📦 Other",
  ],
  unreleased: "Unreleased",
  commits: "commits",
  tag_note: "Tag note",
  release: "Release notes",
  first_version: "first version",
  truncated: "showing {shown} of {total} commits",
  no_versions: "No versions found",
  no_commits: "(no commit details)",
};

impl Lang {
  fn texts(self) -> &'static Texts {
    match self {
      Lang::Zh => &ZH,
      Lang::En => &EN,
    }
  }
}

/// Breaking commits get their own leading group and are not repeated in their category.
fn groups<'a>(version: &'a Version, texts: &Texts) -> Vec<(&'static str, Vec<&'a Commit>)> {
  let breaking: Vec<_> = version.commits.iter().filter(|c| c.breaking).collect();
  let by_category = Category::ALL.iter().zip(texts.categories).map(|(category, label)| {
    let commits = version.commits.iter().filter(|c| !c.breaking && c.category == *category);
    (label, commits.collect::<Vec<_>>())
  });
  std::iter::once((texts.breaking, breaking))
    .chain(by_category)
    .filter(|(_, commits)| !commits.is_empty())
    .collect()
}

fn version_title(version: &Version, texts: &Texts) -> String {
  version.tag.clone().unwrap_or_else(|| texts.unreleased.to_string())
}

/// `2026-11-13 · v0.1.0..v0.2.0 · 2 个提交`
fn version_meta(version: &Version, texts: &Texts) -> String {
  let head = version.tag.as_deref().unwrap_or("HEAD");
  let range = match &version.previous {
    Some(previous) => format!("{previous}..{head}"),
    None => texts.first_version.to_string(),
  };
  let count = format!("{} {}", version.total_commits, texts.commits);
  [version.date.clone(), Some(range), Some(count)].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

fn truncation_note(version: &Version, texts: &Texts) -> Option<String> {
  (version.total_commits > version.commits.len() && !version.commits.is_empty()).then(|| {
    texts
      .truncated
      .replace("{shown}", &version.commits.len().to_string())
      .replace("{total}", &version.total_commits.to_string())
  })
}

pub fn text(report: &Report, lang: Lang, color: bool) -> String {
  let texts = lang.texts();
  let paint = |s: &str, code: &str| if color { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() };
  let mut out = String::new();

  let _ = writeln!(out, "{}", paint(&report.source, "1"));
  if report.versions.is_empty() {
    let _ = writeln!(out, "\n{}", texts.no_versions);
  }
  for version in &report.versions {
    let _ = writeln!(
      out,
      "\n{} {}  {}",
      paint("▌", "36"),
      paint(&version_title(version, texts), "1;36"),
      paint(&version_meta(version, texts), "2")
    );
    if let Some(note) = &version.tag_message {
      let _ = writeln!(out, "  {}: {}", paint(texts.tag_note, "1"), note.replace('\n', "\n    "));
    }
    if let Some(release) = &version.release {
      let _ = writeln!(out, "  {}: {}  {}", paint(texts.release, "1"), release.name, paint(&release.url, "2"));
      for line in release.body.lines().filter(|l| !l.trim().is_empty()) {
        let _ = writeln!(out, "    {}", paint(line.trim_end(), "2"));
      }
    }
    let groups = groups(version, texts);
    if groups.is_empty() && version.release.is_none() {
      let _ = writeln!(out, "  {}", paint(texts.no_commits, "2"));
    }
    for (label, commits) in groups {
      let code = if label == texts.breaking { "1;31" } else { "1" };
      let _ = writeln!(out, "  {} ({})", paint(label, code), commits.len());
      for commit in commits {
        let scope = commit.scope.as_ref().map(|s| format!("[{s}] ")).unwrap_or_default();
        let pr = commit.pr.map(|n| format!(" (#{n})")).unwrap_or_default();
        let sha = commit.sha.get(..7).unwrap_or(&commit.sha);
        let _ = writeln!(out, "    • {scope}{}{pr}  {}", commit.summary, paint(sha, "2"));
      }
    }
    if let Some(note) = truncation_note(version, texts) {
      let _ = writeln!(out, "  {}", paint(&format!("… {note}"), "2"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::Commit;

  fn commit(category: Category, breaking: bool, summary: &str) -> Commit {
    Commit {
      sha: "0123456789".into(),
      author: "a".into(),
      date: "2026-01-01".into(),
      category,
      breaking,
      scope: None,
      summary: summary.into(),
      pr: None,
    }
  }

  fn report() -> Report {
    Report {
      source: "demo".into(),
      github: None,
      versions: vec![Version {
        tag: Some("v1.0.0".into()),
        previous: Some("v0.9.0".into()),
        date: Some("2026-01-02".into()),
        tag_message: None,
        release: None,
        commits: vec![
          commit(Category::Fix, false, "fix one"),
          commit(Category::Feature, true, "drop api"),
          commit(Category::Feature, false, "add two"),
        ],
        total_commits: 5,
      }],
    }
  }

  #[test]
  fn breaking_first_then_categories_in_order() {
    let out = text(&report(), Lang::Zh, false);
    let breaking = out.find("破坏性变更").unwrap();
    let feature = out.find("新增功能").unwrap();
    let fix = out.find("问题修复").unwrap();
    assert!(breaking < feature && feature < fix, "{out}");
    assert_eq!(out.matches("drop api").count(), 1, "breaking commit listed once");
    assert!(out.contains("v0.9.0..v1.0.0"));
    assert!(out.contains("仅显示 3 / 共 5 个提交"));
    assert!(!out.contains('\x1b'));
  }

  #[test]
  fn english_labels() {
    let out = text(&report(), Lang::En, true);
    assert!(out.contains("Bug Fixes") && out.contains("showing 3 of 5 commits"));
    assert!(out.contains('\x1b'));
  }
}
