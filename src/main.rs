mod classify;
mod github;
mod local;
mod model;
mod render;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::github::GitHub;
use crate::model::Selection;
use crate::render::Lang;

/// 按 Tag / Release 查看仓库每个版本的提交，并归类为新增功能、问题修复、破坏性变更等。
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
  /// 本地仓库路径，或 GitHub 地址（https://github.com/o/r、git@github.com:o/r.git、o/r）
  #[arg(default_value = ".")]
  target: String,

  /// 展示最近 N 个版本
  #[arg(short = 'n', long, default_value_t = 5)]
  limit: usize,

  /// 只看某个 Tag
  #[arg(long, conflicts_with = "range")]
  tag: Option<String>,

  /// 只看 A..B 之间的变更
  #[arg(long, value_name = "A..B", value_parser = parse_range)]
  range: Option<(String, String)>,

  /// 额外展示最新 Tag 之后尚未发布的提交
  #[arg(long)]
  unreleased: bool,

  /// 每个版本最多展示的提交数
  #[arg(long, default_value_t = 200)]
  max_commits: usize,

  /// 保留合并提交（默认过滤）
  #[arg(long)]
  merges: bool,

  /// 本地模式下，按 origin 拉取 GitHub Release Notes 合并展示
  #[arg(long)]
  github: bool,

  /// GitHub Token，提升限流额度并支持私有仓库
  #[arg(long, env = "GITHUB_TOKEN", hide_env_values = true)]
  token: Option<String>,

  /// 输出语言
  #[arg(long, value_enum, default_value = "zh")]
  lang: Lang,
}

fn parse_range(value: &str) -> Result<(String, String), String> {
  match value.split_once("..") {
    Some((from, to)) if !from.is_empty() && !to.is_empty() && !to.starts_with('.') => {
      Ok((from.to_string(), to.to_string()))
    }
    _ => Err("格式应为 A..B，例如 v1.0.0..v1.2.0".to_string()),
  }
}

fn main() {
  if let Err(error) = run(Cli::parse()) {
    eprintln!("错误：{error:#}");
    std::process::exit(1);
  }
}

fn run(cli: Cli) -> Result<()> {
  let selection = Selection {
    limit: cli.limit,
    tag: cli.tag,
    range: cli.range,
    unreleased: cli.unreleased,
    max_commits: cli.max_commits,
    merges: cli.merges,
  };
  let path = PathBuf::from(&cli.target);
  let report = if path.is_dir() {
    let mut report = local::collect(&path, &selection)?;
    if cli.github {
      let Some(slug) = report.github.clone() else {
        bail!("--github 需要 origin 指向 GitHub 仓库");
      };
      let mut releases = GitHub::new(cli.token)?.releases(&slug)?;
      for version in &mut report.versions {
        version.release = version.tag.as_ref().and_then(|tag| releases.remove(tag));
      }
    }
    report
  } else if let Some(slug) = github::parse_slug(&cli.target) {
    github::collect(&slug, &selection, &GitHub::new(cli.token)?)?
  } else {
    bail!("{} 既不是本地目录，也不是可识别的 GitHub 地址", cli.target);
  };
  let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
  print!("{}", render::text(&report, cli.lang, color));
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn range_parsing() {
    assert_eq!(parse_range("v1..v2"), Ok(("v1".into(), "v2".into())));
    assert!(parse_range("v1...v2").is_err());
    assert!(parse_range("v1..").is_err());
    assert!(parse_range("v1").is_err());
  }
}
