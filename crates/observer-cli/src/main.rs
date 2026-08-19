#![forbid(unsafe_code)]

use std::{io::Write as _, path::PathBuf};

use anyhow::Context as _;
use chrono::{Duration, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use observer_adapters::{AdapterLocations, CollectionPermission, adapters_for_locations};
use observer_core::{
    adapter_registry, collect_measurements_phase, compare, export_finalized, load_consent,
    load_or_create_study, load_pending, verify_file, write_consent, write_pending,
};
use observer_domain::{ConsentManifest, WindowKind};

const DEFAULT_STATE_DIRECTORY: &str = ".second-observer";

#[derive(Debug, Parser)]
#[command(name = "second-observer", about = "Deterministic local workflow measurement collector")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect the embedded registry without reading sources or executing applications.
    Discover(DiscoverCommand),
    /// Create a reviewable consent manifest.
    Consent(ConsentCommand),
    /// Create a local aggregate collection from the approved adapters.
    Collect(CollectCommand),
    /// Show the exact pending aggregate payload.
    Preview(StateCommand),
    /// Copy the exact pending payload into an owner-only finalized export file.
    Export(ExportCommand),
    /// Verify an export's canonical form, contract constants, and digest.
    Verify { export: PathBuf },
    /// Evaluate descriptive comparability between a baseline and post export.
    Compare { baseline: PathBuf, post: PathBuf },
}

#[derive(Debug, Args)]
struct ConsentCommand {
    #[command(subcommand)]
    command: ConsentSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConsentSubcommand {
    /// Write metadata-first consent for every registered adapter. Review it before collection.
    Init(ConsentInitCommand),
}

#[derive(Debug, Args)]
struct ConsentInitCommand {
    #[arg(long, default_value_os_t = default_consent_path())]
    output: PathBuf,
    /// Approve only these adapter IDs. Repeating this option overrides the registry default.
    #[arg(long = "adapter")]
    adapters: Vec<String>,
    #[arg(long, default_value_t = 30)]
    expires_in_days: i64,
    /// Enable locally aggregated content heuristics. Raw content remains prohibited from exports.
    #[arg(long)]
    content_analysis: bool,
    #[arg(long, value_enum, default_value_t = Phase::Baseline)]
    phase: Phase,
}

#[derive(Debug, Args)]
struct DiscoverCommand {
    /// Explicit participant home directory. The collector never reads HOME.
    #[arg(long)]
    home: PathBuf,
    /// Explicit .git directory to inspect for linked-worktree metadata. May be repeated.
    #[arg(long = "git-dir")]
    git_dirs: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct CollectCommand {
    #[arg(long, default_value = "retained-history")]
    profile: String,
    /// Measurement-window length in days. Baseline and post must use the same value; a mismatch
    /// makes the pair INCOMPARABLE rather than silently comparing unequal windows.
    #[arg(long, default_value_t = observer_core::DEFAULT_WINDOW_DAYS)]
    baseline: u16,
    #[arg(long, default_value_os_t = default_consent_path())]
    consent: PathBuf,
    #[arg(long, default_value_os_t = default_pending_path())]
    output: PathBuf,
    /// IANA name or documented fixed offset recorded in the export. The first skeleton defaults to UTC.
    #[arg(long, default_value = "UTC")]
    timezone: String,
    /// Explicit participant home directory. The collector never reads HOME.
    #[arg(long)]
    home: PathBuf,
    /// Explicit .git directory to inspect for linked-worktree metadata. May be repeated.
    #[arg(long = "git-dir")]
    git_dirs: Vec<PathBuf>,
    #[arg(long, value_enum, default_value_t = Phase::Baseline)]
    phase: Phase,
    /// Owner-only stable participant/device identity; run IDs remain fresh for each collection.
    #[arg(long, default_value_os_t = default_identity_path())]
    identity: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Phase {
    Baseline,
    Post,
}

impl Phase {
    const fn window(self) -> WindowKind {
        match self {
            Self::Baseline => WindowKind::Baseline,
            Self::Post => WindowKind::Post,
        }
    }
}

#[derive(Debug, Args)]
struct StateCommand {
    #[arg(long, default_value_os_t = default_pending_path())]
    pending: PathBuf,
}

#[derive(Debug, Args)]
struct ExportCommand {
    #[arg(long, default_value_os_t = default_pending_path())]
    pending: PathBuf,
    /// Output file. The default is exports/<run-id>.study-export.
    #[arg(long)]
    output: Option<PathBuf>,
}

fn default_consent_path() -> PathBuf {
    PathBuf::from(DEFAULT_STATE_DIRECTORY).join("consent.json")
}

fn default_pending_path() -> PathBuf {
    PathBuf::from(DEFAULT_STATE_DIRECTORY).join("pending.study-export")
}

fn default_identity_path() -> PathBuf {
    PathBuf::from(DEFAULT_STATE_DIRECTORY).join("study-identity.json")
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Discover(command) => discover_command(command),
        Command::Consent(ConsentCommand { command: ConsentSubcommand::Init(command) }) => {
            consent_init(command)
        }
        Command::Collect(command) => collect_command(command),
        Command::Preview(command) => preview_command(&command),
        Command::Export(command) => export_command(command),
        Command::Verify { export } => {
            let verified = verify_file(&export)?;
            println!("{}", verified.integrity.payload_sha256);
            Ok(())
        }
        Command::Compare { baseline, post } => {
            let comparison = compare(&verify_file(&baseline)?, &verify_file(&post)?);
            print_json(&comparison)
        }
    }?;
    Ok(())
}

fn consent_init(command: ConsentInitCommand) -> anyhow::Result<()> {
    if command.expires_in_days <= 0 {
        anyhow::bail!("--expires-in-days must be positive");
    }
    let now = Utc::now();
    let adapters = if command.adapters.is_empty() {
        adapter_registry()?.adapters.into_iter().map(|adapter| adapter.id).collect()
    } else {
        command.adapters
    };
    let mut manifest = ConsentManifest::metadata_first(
        now,
        now + Duration::days(command.expires_in_days),
        adapters,
    );
    if command.content_analysis {
        manifest.content_analysis = true;
        manifest.approved_field_classes.extend([
            observer_domain::FieldClass::MessageText,
            observer_domain::FieldClass::CommandText,
        ]);
        manifest.approved_field_classes.sort();
        manifest.approved_field_classes.dedup();
    }
    manifest.windows = vec![WindowKind::RetainedHistory, command.phase.window()];
    manifest.validate(now)?;
    write_consent(&command.output, &manifest)?;
    print_json(&manifest)
}

fn discover_command(command: DiscoverCommand) -> anyhow::Result<()> {
    let mut locations = AdapterLocations::from_home(&command.home);
    locations.git_metadata_roots = command.git_dirs;
    let observations = adapters_for_locations(&adapter_registry()?.adapters, &locations)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|adapter| adapter.observe(CollectionPermission::metadata_only(true)).coverage)
        .collect::<Vec<_>>();
    print_json(&observations)
}

fn collect_command(command: CollectCommand) -> anyhow::Result<()> {
    if command.profile != "retained-history" {
        anyhow::bail!("v1 requires --profile retained-history");
    }
    // The window length is a comparability unit, not a constant. Baseline and post must be equal
    // in length; `compare` enforces that from the recorded bounds and refuses when they differ.
    if !observer_core::WINDOW_DAYS_RANGE.contains(&command.baseline) {
        anyhow::bail!(
            "--baseline must be between {} and {} days",
            observer_core::WINDOW_DAYS_RANGE.start(),
            observer_core::WINDOW_DAYS_RANGE.end()
        );
    }
    let now = Utc::now();
    let consent = load_consent(&command.consent, now)
        .with_context(|| format!("load consent manifest {}", command.consent.display()))?;
    let executable =
        std::env::current_exe().context("resolve collector executable for binary digest")?;
    let binary = std::fs::read(&executable)
        .with_context(|| format!("read collector executable {}", executable.display()))?;
    let binary_sha256 = observer_domain::sha256_hex(&binary);
    let mut locations = AdapterLocations::from_home(&command.home);
    locations.git_metadata_roots = command.git_dirs;
    let definitions = adapter_registry()?.adapters;
    let approved = consent.approved_adapters.iter().collect::<std::collections::BTreeSet<_>>();
    let measurements = adapters_for_locations(&definitions, &locations)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|adapter| {
            let approved = approved.contains(&adapter.definition().id);
            adapter.observe(CollectionPermission::from_consent(&consent, approved))
        })
        .collect::<Vec<_>>();
    let export = collect_measurements_phase(
        &consent,
        now,
        &command.timezone,
        env!("CARGO_PKG_VERSION"),
        &binary_sha256,
        load_or_create_study(&command.identity)?,
        &measurements,
        command.phase.window(),
        command.baseline,
    )?;
    write_pending(&command.output, &export)?;
    println!("{}\t{}", export.integrity.payload_sha256, command.output.display());
    Ok(())
}

fn preview_command(command: &StateCommand) -> anyhow::Result<()> {
    let bytes = load_pending(&command.pending)?.canonical_bytes()?;
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}

fn export_command(command: ExportCommand) -> anyhow::Result<()> {
    let pending = load_pending(&command.pending)?;
    let output = command.output.unwrap_or_else(|| {
        PathBuf::from("exports").join(format!("{}.study-export", pending.study.run_id))
    });
    let digest = export_finalized(&command.pending, &output)?;
    println!("{}\t{}", digest, output.display());
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
