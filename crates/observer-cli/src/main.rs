#![forbid(unsafe_code)]

use std::{
    io::{IsTerminal as _, Write as _},
    path::PathBuf,
};

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
    /// Guided end-to-end collection. Asks for what it needs and needs no other command.
    Run(Box<RunCommand>),
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
    /// Print the exact canonical bytes instead of the reviewable form.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct RunCommand {
    /// Participant home directory. Omitted, the guided flow proposes one for confirmation.
    #[arg(long)]
    home: Option<PathBuf>,
    /// IANA timezone. Omitted, the guided flow proposes the system zone for confirmation.
    #[arg(long)]
    timezone: Option<String>,
    /// Explicit .git directory to inspect for linked-worktree metadata. May be repeated.
    #[arg(long = "git-dir")]
    git_dirs: Vec<PathBuf>,
    #[arg(long, default_value_t = 30)]
    expires_in_days: i64,
    #[arg(long, default_value_os_t = default_consent_path())]
    consent: PathBuf,
    #[arg(long, default_value_os_t = default_pending_path())]
    pending: PathBuf,
    #[arg(long, default_value_os_t = default_identity_path())]
    identity: PathBuf,
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
        Command::Run(command) => guided_run(*command),
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
    let pending = load_pending(&command.pending)?;
    if command.json {
        std::io::stdout().write_all(&pending.canonical_bytes()?)?;
        return Ok(());
    }
    render_export(&pending);
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

// ---------------------------------------------------------------------------
// Guided run
//
// The flag-based subcommands stay the participant-supplied contract: they infer
// nothing. `run` is a terminal front end over exactly those steps, so a
// participant needs no agent and no memorised flags. It differs from them in one
// documented way: it may read the environment to *propose* a home directory and
// timezone, which the participant must then confirm or replace. A proposal that
// requires confirmation is not inference.
// ---------------------------------------------------------------------------

fn guided_run(command: RunCommand) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "`run` is interactive and needs a terminal. Use the individual subcommands \
             (discover, consent init, collect, preview, export, verify) for scripted use."
        );
    }

    rule();
    println!("Second Observer {}", env!("CARGO_PKG_VERSION"));
    println!(
        "This reads local filesystem metadata and counts from coding tools you approve.\n\
         It never reads prompts, commands, transcripts, or file contents, and it never\n\
         uploads. You review the exact payload before anything is written to an export."
    );
    rule();

    // 1. Home directory ----------------------------------------------------
    let home = resolve_home(command.home)?;

    // 2. Timezone ----------------------------------------------------------
    let timezone = resolve_timezone(command.timezone)?;

    // 3. Discovery ---------------------------------------------------------
    println!("\nChecking which tools are present. Nothing is read or executed yet.\n");
    let coverage = discover_coverage(&home, &command.git_dirs)?;
    print_coverage_table(&coverage);

    let observed = coverage
        .iter()
        .filter(|entry| entry.status == observer_domain::CoverageStatus::Observed)
        .map(|entry| entry.adapter_id.clone())
        .collect::<Vec<_>>();

    if observed.is_empty() {
        anyhow::bail!(
            "No adapter returned `observed`, so there is nothing to measure on this machine."
        );
    }

    // 4. Adapter selection -------------------------------------------------
    // Comparison requires every approved adapter to be observed in both phases,
    // so approving a tool the participant does not use guarantees INCOMPARABLE
    // on every later pair. Defaulting to the observed set is the whole reason
    // this flow exists.
    let adapters = choose_adapters(&coverage, &observed)?;

    // 5. Phase, window, and content analysis --------------------------------
    let (phase, window_days, content_analysis) = choose_phase_window_content()?;

    // 6. Consent -----------------------------------------------------------
    let now = Utc::now();
    let manifest = build_consent(now, command.expires_in_days, &adapters, phase, content_analysis)?;
    show_consent(&manifest, phase, window_days, &home, &timezone);
    if !confirm("Collect with exactly this consent?")? {
        println!("Stopped. Nothing was collected and no state was written.");
        return Ok(());
    }
    write_consent(&command.consent, &manifest)?;

    // 7. Collect -----------------------------------------------------------
    println!("\nCollecting.");
    let export = run_collection(
        &manifest,
        now,
        &home,
        &timezone,
        &command.git_dirs,
        &command.identity,
        phase,
        window_days,
    )?;
    write_pending(&command.pending, &export)?;

    // 9. Review ------------------------------------------------------------
    println!();
    render_export(&export);
    println!(
        "\nThis is the entire payload. `preview --json` prints the exact bytes if you want to\n\
         diff or archive them."
    );

    if !confirm("\nWrite this to an export file?")? {
        println!(
            "Stopped. The collection stays in {} and nothing was exported.",
            command.pending.display()
        );
        return Ok(());
    }

    // 10. Export and verify -------------------------------------------------
    let output = PathBuf::from("exports").join(format!("{}.study-export", export.study.run_id));
    let digest = export_finalized(&command.pending, &output)?;
    verify_file(&output)?;
    rule();
    println!("Export written and verified.");
    println!("  file    {}", output.display());
    println!("  sha256  {digest}");
    println!("  keep    {} (the post collection needs it)", command.identity.display());
    println!(
        "\nNothing has been uploaded. To send it to a study coordinator, and only if you\n\
         decide to, use `second-observer-upload send` with a study code."
    );
    rule();
    Ok(())
}

/// Renders an enum using its serde name. `{:?}` lowercased silently drops the
/// underscore, so `DetectedUnmeasured` printed as `detectedunmeasured` and no
/// longer matched the legend or the documented status vocabulary.
fn serde_name<T: serde::Serialize + std::fmt::Debug>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{value:?}"))
}

/// The flag-based subcommands take an explicit `--home` and never consult the
/// environment. The guided flow may *propose* one, which the participant then
/// confirms or replaces. A proposal awaiting confirmation is not inference.
fn resolve_home(supplied: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let home = if let Some(home) = supplied {
        home
    } else {
        let proposed = std::env::var_os("HOME").map(PathBuf::from);
        PathBuf::from(prompt_with_default(
            "Which home directory should be measured?",
            proposed.as_ref().and_then(|path| path.to_str()).unwrap_or(""),
        )?)
    };
    if !home.is_dir() {
        anyhow::bail!("{} is not a directory", home.display());
    }
    Ok(home)
}

/// An abbreviation such as `EST` parses but pins a fixed offset with no daylight
/// saving, so every local day boundary shifts for half the year. Require an IANA
/// name rather than silently accepting a zone that moves the window.
fn resolve_timezone(supplied: Option<String>) -> anyhow::Result<String> {
    if let Some(timezone) = supplied {
        return Ok(timezone);
    }
    loop {
        let answer = prompt_with_default(
            "IANA timezone for local day boundaries (e.g. America/Toronto)?",
            &detected_timezone().unwrap_or_default(),
        )?;
        if !answer.contains('/') {
            println!(
                "  `{answer}` is not an IANA name. An abbreviation pins a fixed offset with no"
            );
            println!(
                "  daylight-saving transition, which shifts every day boundary for half the year."
            );
            println!("  Use a Region/City name.");
            continue;
        }
        if answer.parse::<chrono_tz::Tz>().is_err() {
            println!("  `{answer}` is not in the timezone database.");
            continue;
        }
        return Ok(answer);
    }
}

/// Comparison requires every approved adapter to be observed in both phases, so
/// approving a tool the participant does not use guarantees INCOMPARABLE on every
/// later pair. Defaulting to the observed set is the reason this flow exists.
fn choose_adapters(
    coverage: &[observer_domain::Coverage],
    observed: &[String],
) -> anyhow::Result<Vec<String>> {
    println!("\nApprove only tools you actually use. Comparison later requires every approved");
    println!("adapter to be observed in both phases, so approving an unused tool makes every");
    println!("comparison INCOMPARABLE.");
    loop {
        let answer =
            prompt_with_default("Adapters to approve (comma-separated)", &observed.join(", "))?;
        let chosen = answer
            .split(',')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let known = coverage.iter().map(|entry| entry.adapter_id.as_str()).collect::<Vec<_>>();
        let unknown =
            chosen.iter().filter(|id| !known.contains(&id.as_str())).cloned().collect::<Vec<_>>();
        if !unknown.is_empty() {
            println!("  Not registered adapters: {}", unknown.join(", "));
            continue;
        }
        if chosen.is_empty() {
            println!("  Approve at least one adapter.");
            continue;
        }
        let unobserved =
            chosen.iter().filter(|id| !observed.contains(id)).cloned().collect::<Vec<_>>();
        if !unobserved.is_empty()
            && !confirm(&format!(
                "  {} did not return `observed`. Approving them will make every comparison \
                 INCOMPARABLE. Continue anyway?",
                unobserved.join(", ")
            ))?
        {
            continue;
        }
        return Ok(chosen);
    }
}

fn choose_phase_window_content() -> anyhow::Result<(Phase, u16, bool)> {
    let phase = if confirm("\nIs this your first (baseline) collection?")? {
        Phase::Baseline
    } else {
        Phase::Post
    };
    if phase == Phase::Post {
        println!(
            "  A post collection must use the same window length and the same adapters as its"
        );
        println!("  baseline, and the same .second-observer/study-identity.json.");
    }
    let window_days = loop {
        let answer = prompt_with_default(
            "Window length in days (baseline and post must match)",
            &observer_core::DEFAULT_WINDOW_DAYS.to_string(),
        )?;
        match answer.parse::<u16>() {
            Ok(value) if observer_core::WINDOW_DAYS_RANGE.contains(&value) => break value,
            _ => println!(
                "  Enter a whole number between {} and {}.",
                observer_core::WINDOW_DAYS_RANGE.start(),
                observer_core::WINDOW_DAYS_RANGE.end()
            ),
        }
    };
    println!("\nOptional local content analysis derives relay, routing, and correction heuristics");
    println!("from message and command text on this machine. No text is stored or exported; only");
    println!("aggregate numbers are, and they are labelled `local_content_heuristic`.");
    let content_analysis = confirm("Enable local content analysis?")?;
    Ok((phase, window_days, content_analysis))
}

fn build_consent(
    now: chrono::DateTime<Utc>,
    expires_in_days: i64,
    adapters: &[String],
    phase: Phase,
    content_analysis: bool,
) -> anyhow::Result<ConsentManifest> {
    let mut manifest = ConsentManifest::metadata_first(
        now,
        now + Duration::days(expires_in_days),
        adapters.to_vec(),
    );
    if content_analysis {
        manifest.content_analysis = true;
        manifest.approved_field_classes.extend([
            observer_domain::FieldClass::MessageText,
            observer_domain::FieldClass::CommandText,
        ]);
        manifest.approved_field_classes.sort();
        manifest.approved_field_classes.dedup();
    }
    manifest.windows = vec![WindowKind::RetainedHistory, phase.window()];
    manifest.validate(now)?;
    Ok(manifest)
}

fn show_consent(
    manifest: &ConsentManifest,
    phase: Phase,
    window_days: u16,
    home: &std::path::Path,
    timezone: &str,
) {
    rule();
    println!("Consent manifest");
    println!("  phase              {phase:?}");
    println!("  window             {window_days} days");
    println!("  home               {}", home.display());
    println!("  timezone           {timezone}");
    println!("  adapters           {}", manifest.approved_adapters.join(", "));
    println!("  field classes      {}", field_class_list(manifest));
    println!("  content analysis   {}", if manifest.content_analysis { "yes" } else { "no" });
    println!("  never collected    {}", manifest.prohibited_field_classes.join(", "));
    println!("  expires            {}", manifest.expires_at.format("%Y-%m-%d"));
    rule();
}

#[allow(clippy::too_many_arguments)]
fn run_collection(
    manifest: &ConsentManifest,
    now: chrono::DateTime<Utc>,
    home: &std::path::Path,
    timezone: &str,
    git_dirs: &[PathBuf],
    identity: &std::path::Path,
    phase: Phase,
    window_days: u16,
) -> anyhow::Result<observer_domain::StudyExport> {
    let executable = std::env::current_exe().context("resolve collector executable")?;
    let binary_sha256 = observer_domain::sha256_hex(
        &std::fs::read(&executable).context("read collector executable")?,
    );
    let mut locations = AdapterLocations::from_home(home);
    locations.git_metadata_roots = git_dirs.to_vec();
    let approved = manifest.approved_adapters.iter().collect::<std::collections::BTreeSet<_>>();
    let measurements = adapters_for_locations(&adapter_registry()?.adapters, &locations)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|adapter| {
            let approved = approved.contains(&adapter.definition().id);
            adapter.observe(CollectionPermission::from_consent(manifest, approved))
        })
        .collect::<Vec<_>>();
    Ok(collect_measurements_phase(
        manifest,
        now,
        timezone,
        env!("CARGO_PKG_VERSION"),
        &binary_sha256,
        load_or_create_study(identity)?,
        &measurements,
        phase.window(),
        window_days,
    )?)
}

fn rule() {
    println!("{}", "-".repeat(76));
}

fn field_class_list(manifest: &ConsentManifest) -> String {
    manifest.approved_field_classes.iter().map(serde_name).collect::<Vec<_>>().join(", ")
}

fn read_line() -> anyhow::Result<String> {
    let mut buffer = String::new();
    if std::io::stdin().read_line(&mut buffer)? == 0 {
        anyhow::bail!("input ended before the question was answered");
    }
    Ok(buffer.trim().to_owned())
}

fn prompt_with_default(question: &str, default: &str) -> anyhow::Result<String> {
    loop {
        if default.is_empty() {
            print!("{question}\n> ");
        } else {
            print!("{question}\n[{default}]\n> ");
        }
        std::io::stdout().flush()?;
        let answer = read_line()?;
        if !answer.is_empty() {
            return Ok(answer);
        }
        if !default.is_empty() {
            return Ok(default.to_owned());
        }
        println!("  An answer is required.");
    }
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    loop {
        print!("{question} [y/n]\n> ");
        std::io::stdout().flush()?;
        match read_line()?.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("  Answer y or n."),
        }
    }
}

/// Reads the system timezone to *propose* a value. macOS and most Linux systems
/// symlink `/etc/localtime` into the zoneinfo tree, whose tail is the IANA name.
fn detected_timezone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let text = target.to_str()?;
    let (_, name) = text.split_once("zoneinfo/")?;
    let name = name.trim_start_matches("posix/").trim_start_matches("right/");
    name.parse::<chrono_tz::Tz>().ok().map(|_| name.to_owned())
}

fn discover_coverage(
    home: &PathBuf,
    git_dirs: &[PathBuf],
) -> anyhow::Result<Vec<observer_domain::Coverage>> {
    let mut locations = AdapterLocations::from_home(home);
    locations.git_metadata_roots = git_dirs.to_vec();
    Ok(adapters_for_locations(&adapter_registry()?.adapters, &locations)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|adapter| adapter.observe(CollectionPermission::metadata_only(true)).coverage)
        .collect())
}

fn print_coverage_table(coverage: &[observer_domain::Coverage]) {
    let width = coverage.iter().map(|entry| entry.adapter_id.len()).max().unwrap_or(10).max(7);
    println!("  {:<width$}  {:<20}  records", "adapter", "status", width = width);
    for entry in coverage {
        println!(
            "  {:<width$}  {:<20}  {}",
            entry.adapter_id,
            serde_name(&entry.status),
            entry.observed_records,
            width = width
        );
    }
    println!("\n  observed = has readable retained history. detected_unmeasured = present but not");
    println!("  measurable. missing = not found. disabled = excluded by consent.");
}

/// The reviewable form of the payload. `preview` previously wrote canonical
/// single-line JSON, which is the exact bytes but is not something a participant
/// can read, and this step is the consent boundary.
fn render_export(export: &observer_domain::StudyExport) {
    rule();
    println!("Collected payload for review");
    rule();
    println!("Windows");
    for window in &export.windows {
        let span = match window.start {
            Some(start) => {
                format!("{} to {}", start.format("%Y-%m-%d"), window.end.format("%Y-%m-%d"))
            }
            None => format!("everything retained, as of {}", window.end.format("%Y-%m-%d")),
        };
        println!("  {:<16} {span}", window.id);
    }

    println!("\nMeasurements");
    let mut adapters = export.metrics.iter().map(|m| m.adapter_id.as_str()).collect::<Vec<_>>();
    adapters.sort_unstable();
    adapters.dedup();
    for adapter in adapters {
        for window in &export.windows {
            let rows = export
                .metrics
                .iter()
                .filter(|m| m.adapter_id == adapter && m.window_id == window.id)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                continue;
            }
            println!("  {adapter} / {}", window.id);
            for metric in rows {
                let value = match metric.value {
                    Some(value) => format!("{value}"),
                    // A missing value is a coverage fact. Printing 0 here would
                    // read as "this happened zero times".
                    None => "not measured".to_owned(),
                };
                println!("    {:<28} {:>14} {}", metric.metric_id, value, metric.unit);
            }
        }
    }

    println!("\nCoverage");
    for entry in &export.coverage {
        println!(
            "  {:<18} {:<20} {} records",
            entry.adapter_id,
            serde_name(&entry.status),
            entry.observed_records
        );
    }

    println!("\nExcluded by construction");
    println!("  raw prompts, commands, transcripts, tool output, file contents, paths,");
    println!("  repository names, remotes, URLs, stable machine identifiers");
    println!(
        "  content persisted: {}   content exported: {}   forbidden fields absent: {}",
        export.privacy.content_persisted,
        export.privacy.content_exported,
        export.privacy.forbidden_fields_absent
    );

    println!("\nThis export does not claim");
    for nonclaim in &export.nonclaims {
        println!("  {nonclaim}");
    }
    rule();
}
