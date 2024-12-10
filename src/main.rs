use directories::BaseDirs;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::{path::PathBuf, time::Duration};
use time::OffsetDateTime;
use tokio::{fs::File, io::AsyncWriteExt, process::Command};

use clap::Parser;
use serde::Deserialize;

const PUPDATE_CONFIG_FILENAME: &str = ".pupdate";
const SPINNER_STYLE: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
const SPINNER_TIME_MILLIS: u64 = 80;

/// arguments pupdate has received
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Args {
	/// list of remotes to run pupdates on
	remotes: Option<Vec<String>>,
	/// whether to only run pupdates locally
	#[arg(long)]
	local_only: bool,
	/// whether to skip local pupdates
	#[arg(long)]
	skip_local: bool,
	/// the directory to log to
	#[arg(short, long)]
	log_dir: Option<PathBuf>,
	/// the config to use as a base
	#[arg(short, long)]
	config: Option<PathBuf>,
}

/// pupdate config
#[derive(Debug, Default, Deserialize)]
struct Config {
	/// the remotes to pupdate if none are provided
	#[serde(default)]
	remotes: Vec<String>,
	/// the directory to log to, no logs if missing
	#[serde(default)]
	log_dir: Option<PathBuf>,
	/// the package managers to pupdate the local system with
	#[serde(default)]
	package_managers: Vec<PackageManager>,
}

/// enum for supported package managers
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PackageManager {
	AptGet,
	Homebrew,
	Pacman,
	Yay,
}

impl PackageManager {
	/// gets the packaage manager's name
	pub fn get_name(&self) -> &'static str {
		match self {
			Self::AptGet => "apt-get",
			Self::Homebrew => "homebrew",
			Self::Pacman => "pacman",
			Self::Yay => "yay",
		}
	}

	/// pupdates the local system
	pub async fn pupdate(&self, log_dir: Option<PathBuf>) -> eyre::Result<bool> {
		match self {
			Self::AptGet => pupdate_apt(log_dir).await,
			Self::Homebrew => pupdate_homebrew(log_dir).await,
			Self::Pacman => pupdate_pacman(log_dir).await,
			Self::Yay => pupdate_yay(log_dir).await,
		}
	}
}

/// pupdates a remote target through ssh
/// TODO: build pupdate daemon and pupdate through that instead
async fn pupdate_remote(
	remote: String,
	log_dir: Option<PathBuf>,
	pb: ProgressBar,
	finished_style: ProgressStyle,
	overall: ProgressBar,
) -> eyre::Result<(String, bool)> {
	pb.set_message("pupdating...");
	let start = OffsetDateTime::now_utc();
	let output = Command::new("ssh")
		.arg(&remote)
		.arg("sudo pupdate")
		.output()
		.await?;
	let end = OffsetDateTime::now_utc();
	let success = output.status.success();
	if let Some(log_dir) = log_dir {
		let mut stdout = File::create(log_dir.join(format!("{remote}.stdout.log"))).await?;
		stdout.write_all(&output.stdout).await?;
		let mut stderr = File::create(log_dir.join(format!("{remote}.stderr.log"))).await?;
		stderr.write_all(&output.stderr).await?;
	}
	let duration = end - start;
	pb.set_style(finished_style);
	pb.finish_with_message(format!(
		"finished in {} seconds: {}",
		duration.whole_seconds(),
		if success { "succeeded" } else { "failed" }
	));
	overall.inc(1);
	Ok((remote, success))
}

/// helper for writing log files from multiple processes
async fn log_helper(
	outputs: &[std::process::Output],
	log_dir: Option<PathBuf>,
) -> eyre::Result<bool> {
	if let Some(log_dir) = log_dir {
		let mut stdout = File::create(log_dir.join("local.stdout.log")).await?;
		let mut stderr = File::create(log_dir.join("local.stderr.log")).await?;
		for output in outputs {
			stdout.write_all(&output.stdout).await?;
			stderr.write_all(&output.stderr).await?;
		}
	}
	for output in outputs {
		if !output.status.success() {
			return Ok(false);
		}
	}
	Ok(true)
}

/// helper to prefix a command with sudo if requested
fn command_maybe_with_sudo(command: &str, with_sudo: bool) -> Command {
	if with_sudo {
		let mut cmd = Command::new("sudo");
		cmd.arg(command);
		cmd
	} else {
		Command::new(command)
	}
}

/// helper for pupdates which both follow the "X update && X upgrade" set of commands (e.g. apt-get and brew)
async fn update_and_upgrade(
	log_dir: Option<PathBuf>,
	command: &str,
	with_sudo: bool,
	yes: bool,
) -> eyre::Result<bool> {
	let update_output = command_maybe_with_sudo(command, with_sudo)
		.arg("update")
		.output()
		.await?;
	if !update_output.status.success() {
		return log_helper(&[update_output], log_dir).await;
	}
	let mut upgrade_command = command_maybe_with_sudo(command, with_sudo);
	upgrade_command.arg("upgrade");
	if yes {
		upgrade_command.arg("-y");
	}
	let upgrade_output = upgrade_command.output().await?;
	log_helper(&[update_output, upgrade_output], log_dir).await
}

/// pupdates the local system using apt-get
async fn pupdate_apt(log_dir: Option<PathBuf>) -> eyre::Result<bool> {
	update_and_upgrade(log_dir, "apt-get", true, true).await
}

/// pupdates the local system using brew, untested
async fn pupdate_homebrew(log_dir: Option<PathBuf>) -> eyre::Result<bool> {
	update_and_upgrade(log_dir, "brew", false, false).await
}

/// helper for pupdates which follow the pacman-style command format
async fn syu(
	log_dir: Option<PathBuf>,
	command: &str,
	with_sudo: bool,
	yes: bool,
) -> eyre::Result<bool> {
	let mut command = command_maybe_with_sudo(command, with_sudo);
	command.arg("-Syu");
	if yes {
		command.arg("--noconfirm");
	}
	let output = command.output().await?;
	log_helper(&[output], log_dir).await
}

/// pupdates the local system using pacman
async fn pupdate_pacman(log_dir: Option<PathBuf>) -> eyre::Result<bool> {
	syu(log_dir, "pacman", true, true).await
}

/// pupdates the local system using yay
async fn pupdate_yay(log_dir: Option<PathBuf>) -> eyre::Result<bool> {
	syu(log_dir, "yay", false, true).await
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
	let args = Args::parse();
	let base_config_path = {
		BaseDirs::new()
			.map(|bd| bd.home_dir().join(PUPDATE_CONFIG_FILENAME))
			.and_then(|p| std::fs::exists(&p).ok().and(Some(p)))
	};
	let config_path = args.config.or(base_config_path);
	let config: Config = if let Some(config) = config_path {
		if config.exists() {
			serde_json::from_str(&std::fs::read_to_string(config)?)?
		} else {
			Config::default()
		}
	} else {
		Config::default()
	};

	let log_dir = args.log_dir.or(config.log_dir).map(|log_dir| {
		let log_dir = log_dir.join(
			OffsetDateTime::now_utc()
				.format(&time::format_description::well_known::Rfc3339)
				.expect("should never fail, surely"),
		);
		std::fs::create_dir_all(&log_dir).expect("failed to create logs directory");
		log_dir
	});

	if args.local_only {
		println!("running in local mode, no remotes will be pupdated");
	} else {
		let remotes = args.remotes.unwrap_or(config.remotes);
		let len = remotes.len();
		let mut failed = Vec::new();

		if len != 0 {
			println!("pupdating {} remotes", len);
			let progress = MultiProgress::new();
			let overall = progress.add(ProgressBar::new(len as u64));
			let spinner_style =
				ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")?
					.tick_chars(SPINNER_STYLE);
			let finished_style = ProgressStyle::with_template("{prefix:.bold.dim} {wide_msg}")?;
			let start = OffsetDateTime::now_utc();

			let mut tasks = Vec::with_capacity(len);
			for remote in remotes {
				let pb = progress.insert_before(&overall, ProgressBar::new_spinner());
				pb.set_prefix(remote.clone());
				pb.set_style(spinner_style.clone());
				pb.enable_steady_tick(Duration::from_millis(SPINNER_TIME_MILLIS));
				tasks.push(tokio::spawn(pupdate_remote(
					remote,
					log_dir.clone(),
					pb,
					finished_style.clone(),
					overall.clone(),
				)));
			}
			overall.tick();

			for task in tasks {
				let (remote, success) = task.await??;
				if !success {
					failed.push(remote);
				}
			}

			let end = OffsetDateTime::now_utc();
			let duration = end - start;

			overall.finish_and_clear();

			println!(
				"{}/{len} remotes pupdated successfully in {} seconds",
				len - failed.len(),
				duration.whole_seconds()
			);
			if !failed.is_empty() {
				println!("the following remotes failed to pupdate:");
				for failed in failed {
					println!("{failed}");
				}
			}
		}
	}

	if !args.skip_local {
		println!("running local pupdates, you may be pawmpted for your password");

		if config.package_managers.is_empty() {
			eprintln!("no package managers defined in config, skipping local pupdates");
		}

		for package_manager in config.package_managers {
			println!("pupdating with {}", package_manager.get_name());
			let start = OffsetDateTime::now_utc();
			package_manager.pupdate(log_dir.clone()).await?;
			let end = OffsetDateTime::now_utc();
			let duration = end - start;

			println!(
				"successfully pupdated the local system in {} seconds",
				duration.whole_seconds()
			);
		}
	}

	Ok(())
}
