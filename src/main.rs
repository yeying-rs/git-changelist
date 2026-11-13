mod classify;
mod local;
mod model;
mod render;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

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
  if !path.is_dir() {
    bail!("{} 不是目录", cli.target);
  }
  let report = local::collect(&path, &selection)?;
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
