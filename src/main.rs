mod classify;
mod github;
mod local;
mod model;
mod render;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

use crate::github::GitHub;
use crate::model::Selection;
use crate::render::Lang;

/// 按 Tag / Release 查看仓库每个版本的提交，并归类为新增功能、问题修复、破坏性变更等。
#[derive(Debug, Parser)]
#[command(version, about, subcommand_precedence_over_arg = true)]
struct Cli {
  #[command(subcommand)]
  command: Option<Command>,

  /// 本地仓库路径，或 GitHub 地址（https://github.com/o/r、git@github.com:o/r.git、o/r）
  #[arg(default_value = ".")]
  target: String,

  /// 展示最近 N 个版本
  #[arg(short = 'n', long, default_value_t = 5)]
  limit: usize,

  /// 只看某个 Tag
  #[arg(long, conflicts_with = "range")]
  tag: Option<String>,

  /// 只看 A 与 B 之间的变更：--range A..B 或 --range A B
  #[arg(long, num_args = 1..=2, value_names = ["A..B", "B"])]
  range: Option<Vec<String>>,

  /// 额外展示最新 Tag 之后尚未发布的提交
  #[arg(long)]
  unreleased: bool,

  /// 每个版本最多展示的提交数
  #[arg(long, default_value_t = 200)]
  max_commits: usize,

  /// 保留合并提交（默认过滤）
  #[arg(long)]
  merges: bool,

  /// 只看触及指定文件或目录的提交（可多次传入，仅本地仓库）
  #[arg(long = "path", value_name = "PATH")]
  paths: Vec<String>,

  /// 只看标题、正文、SHA 或 PR 匹配关键词的提交（不区分大小写）
  #[arg(long, value_name = "TEXT")]
  grep: Option<String>,

  /// 附带每个提交修改的文件与增删行数（仅本地仓库）
  #[arg(long)]
  files: bool,

  /// 本地模式下，按 origin 拉取 GitHub Release Notes 合并展示
  #[arg(long)]
  github: bool,

  /// GitHub Token，提升限流额度并支持私有仓库
  #[arg(long, env = "GITHUB_TOKEN", hide_env_values = true)]
  token: Option<String>,

  /// 输出语言
  #[arg(long, value_enum, default_value = "zh", global = true)]
  lang: Lang,

  /// 输出格式（brief：每行「分类: 变化」，适合看升级带来的改变）
  #[arg(short, long, value_enum, default_value = "text", global = true)]
  format: Format,

  /// 按升级顺序从旧到新排列版本（默认从新到旧）
  #[arg(long)]
  oldest_first: bool,

  /// 写入文件而不是标准输出
  #[arg(short, long, global = true)]
  output: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
  /// 查找某个提交（SHA、#PR 或关键词）最早进入哪个版本，以及是否回移到 hotfix 分支
  Find {
    /// 提交 SHA、#PR 编号或标题/正文关键词
    query: String,
    /// 本地仓库路径
    #[arg(default_value = ".")]
    target: PathBuf,
  },
  /// 查看某个提交或引用（如线上运行的 SHA）所处的版本位置与前后 Tag
  Which {
    /// 提交 SHA、分支或引用（如 HEAD、a1b2c3d）
    rev: String,
    /// 本地仓库路径
    #[arg(default_value = ".")]
    target: PathBuf,
  },
  /// 对比两个版本之间的依赖锁文件变更（Cargo.lock / uv.lock / package-lock.json / go.sum）
  Deps {
    /// 起始版本或引用
    from: String,
    /// 目标版本或引用
    to: String,
    /// 本地仓库路径
    #[arg(default_value = ".")]
    target: PathBuf,
  },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
  Text,
  Brief,
  Markdown,
  Json,
}

fn parse_range(value: &str) -> Result<(String, String), String> {
  match value.split_once("..") {
    Some((from, to)) if !from.is_empty() && !to.is_empty() && !to.starts_with('.') => {
      Ok((from.to_string(), to.to_string()))
    }
    _ => Err("格式应为 A..B，例如 v1.0.0..v1.2.0".to_string()),
  }
}

/// `--range A..B` or `--range A B`.
fn range_from(values: Option<Vec<String>>) -> Result<Option<(String, String)>, String> {
  match values.as_deref() {
    None => Ok(None),
    Some([single]) => parse_range(single).map(Some),
    Some([first, second]) if first.contains("..") => Err(format!(
      "--range 已用 A..B 写法，多出的参数 {second} 会被误读；请把仓库路径放在 --range 前面"
    )),
    Some([from, to]) => Ok(Some((from.clone(), to.clone()))),
    Some(_) => unreachable!("clap limits --range to 1..=2 values"),
  }
}

fn main() {
  if let Err(error) = run(Cli::parse()) {
    eprintln!("错误：{error:#}");
    std::process::exit(1);
  }
}

fn run(cli: Cli) -> Result<()> {
  if let Some(command) = cli.command {
    let output = match command {
      Command::Find { query, target } => {
        let report = local::find(&target, &query)?;
        match cli.format {
          Format::Json => serde_json::to_string_pretty(&report)? + "\n",
          _ => render::find_text(&report, cli.lang),
        }
      }
      Command::Which { rev, target } => {
        let report = local::which(&target, &rev)?;
        match cli.format {
          Format::Json => serde_json::to_string_pretty(&report)? + "\n",
          _ => render::which_text(&report, cli.lang),
        }
      }
      Command::Deps { from, to, target } => {
        let report = local::deps(&target, &from, &to)?;
        match cli.format {
          Format::Json => serde_json::to_string_pretty(&report)? + "\n",
          _ => render::deps_text(&report, cli.lang),
        }
      }
    };
    return write_output(cli.output.as_deref(), &output);
  }

  let selection = Selection {
    limit: cli.limit,
    tag: cli.tag,
    range: range_from(cli.range).map_err(anyhow::Error::msg)?,
    unreleased: cli.unreleased,
    max_commits: cli.max_commits,
    merges: cli.merges,
    paths: cli.paths,
    grep: cli.grep,
    files: cli.files,
  };
  let path = PathBuf::from(&cli.target);
  let mut report = if path.is_dir() {
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
  if cli.oldest_first {
    report.versions.reverse();
  }
  let output = match cli.format {
    Format::Text => {
      let color = cli.output.is_none()
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none();
      render::text(&report, cli.lang, color)
    }
    Format::Brief => render::brief(&report, cli.lang),
    Format::Markdown => render::markdown(&report, cli.lang),
    Format::Json => serde_json::to_string_pretty(&report)? + "\n",
  };
  write_output(cli.output.as_deref(), &output)
}

fn write_output(path: Option<&std::path::Path>, output: &str) -> Result<()> {
  match path {
    Some(file) => {
      std::fs::write(file, output).with_context(|| format!("无法写入 {}", file.display()))?
    }
    None => print!("{output}"),
  }
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

  fn cli_range(args: &[&str]) -> Result<Option<(String, String)>, String> {
    let cli = Cli::try_parse_from(args).map_err(|e| e.to_string())?;
    range_from(cli.range)
  }

  #[test]
  fn range_accepts_dots_or_two_values() {
    let expected = Some(("v0.5.1.1".to_string(), "v0.5.4.1".to_string()));
    assert_eq!(
      cli_range(&["taglens", ".", "--range", "v0.5.1.1..v0.5.4.1"]),
      Ok(expected.clone())
    );
    assert_eq!(
      cli_range(&["taglens", ".", "--range", "v0.5.1.1", "v0.5.4.1"]),
      Ok(expected)
    );
    assert_eq!(cli_range(&["taglens", "."]), Ok(None));
    // `--range A..B PATH` swallows PATH as a second value; say so instead of misreading it.
    let swallowed = cli_range(&["taglens", "--range", "v1..v2", "."]).unwrap_err();
    assert!(swallowed.contains("放在 --range 前面"), "{swallowed}");
    let trailing = Cli::try_parse_from(["taglens", "--range", "a", "b", "repo"]).unwrap();
    assert_eq!(trailing.target, "repo");
    assert_eq!(
      range_from(trailing.range),
      Ok(Some(("a".into(), "b".into())))
    );
  }
}
