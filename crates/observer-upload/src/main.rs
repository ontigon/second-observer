#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use observer_domain::{parse_and_verify_export, sha256_hex};
use reqwest::{
    blocking::Client,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const DEFAULT_INTAKE_ENDPOINT: &str = "https://intake.second.ontigon.ai/";
const MAX_FINALIZED_EXPORT_BYTES: u64 = 256 * 1024;

#[derive(Debug, Parser)]
#[command(name = "second-observer-upload", about = "Isolated Second Observer upload client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Upload one already-finalized, locally verified study export.
    Send(SendCommand),
    /// Retrieve the current intake status for one receipt.
    Status(ReceiptCommand),
    /// Revoke one receipt after explicit confirmation.
    Revoke(RevokeCommand),
}

#[derive(Debug, Args)]
struct SendCommand {
    export: PathBuf,
    #[arg(long)]
    study_code: String,
    /// Required acknowledgement that this exact export is approved for upload.
    #[arg(long)]
    confirm: bool,
    #[arg(long, default_value = DEFAULT_INTAKE_ENDPOINT)]
    endpoint: Url,
}

#[derive(Debug, Args)]
struct ReceiptCommand {
    receipt: String,
    #[arg(long)]
    study_code: String,
    #[arg(long, default_value = DEFAULT_INTAKE_ENDPOINT)]
    endpoint: Url,
}

#[derive(Debug, Args)]
struct RevokeCommand {
    receipt: String,
    #[arg(long)]
    study_code: String,
    /// Required acknowledgement that this receipt should be revoked.
    #[arg(long)]
    confirm: bool,
    #[arg(long, default_value = DEFAULT_INTAKE_ENDPOINT)]
    endpoint: Url,
}

#[derive(Clone)]
struct UploadClient {
    client: Client,
    endpoint: Url,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NegotiateRequest<'a> {
    study_code: &'a str,
    export_digest: &'a str,
    size: usize,
    schema: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NegotiateResponse {
    receipt: String,
    #[serde(default)]
    next_action: Option<NextAction>,
    upload: Option<UploadGrant>,
    #[serde(rename = "resultUrl")]
    result_url: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum NextAction {
    Upload,
    Status,
}

#[derive(Debug, Deserialize)]
struct UploadGrant {
    url: Url,
    headers: BTreeMap<String, String>,
    #[serde(rename = "expiresInSeconds")]
    _expires_in_seconds: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizeRequest<'a> {
    study_code: &'a str,
    receipt: &'a str,
    export_digest: &'a str,
}

impl UploadClient {
    fn new(endpoint: Url) -> anyhow::Result<Self> {
        require_secure_url(&endpoint, "intake endpoint")?;
        Ok(Self { client: Client::builder().redirect(Policy::none()).build()?, endpoint })
    }

    fn send(&self, export_path: &Path, study_code: &str) -> anyhow::Result<Value> {
        validate_study_code(study_code)?;
        let bytes = read_finalized_export(export_path)?;
        let export =
            parse_and_verify_export(&bytes).context("verify finalized export before upload")?;
        let digest = sha256_hex(&bytes);
        let negotiate: NegotiateResponse = self
            .client
            .post(self.endpoint.join("v1/negotiate")?)
            .bearer_auth(study_code)
            .json(&NegotiateRequest {
                study_code,
                export_digest: &digest,
                size: bytes.len(),
                schema: &export.contract_version,
            })
            .send()?
            .error_for_status()?
            .json()?;
        validate_receipt(&negotiate.receipt)?;
        let result_url =
            Url::parse(&negotiate.result_url).context("result URL must be absolute")?;
        require_secure_url(&result_url, "result URL")?;
        require_same_origin(&self.endpoint, &result_url)?;

        match negotiate.next_action.unwrap_or(NextAction::Upload) {
            NextAction::Status => {
                if negotiate.upload.is_some() {
                    anyhow::bail!("status negotiation response must not include an upload grant");
                }
                let mut status = self.status(&negotiate.receipt, study_code)?;
                let Value::Object(ref mut object) = status else {
                    anyhow::bail!("status response must be a JSON object");
                };
                object.insert("resultUrl".to_owned(), Value::String(negotiate.result_url));
                return Ok(status);
            }
            NextAction::Upload => {}
        }
        let upload =
            negotiate.upload.context("upload negotiation response is missing an upload grant")?;
        require_secure_url(&upload.url, "upload URL")?;
        require_same_origin(&self.endpoint, &upload.url)?;

        let mut upload_headers = HeaderMap::new();
        upload_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (name, value) in upload.headers {
            upload_headers
                .insert(HeaderName::from_bytes(name.as_bytes())?, HeaderValue::from_str(&value)?);
        }
        self.client
            .put(upload.url)
            .headers(upload_headers)
            .body(bytes)
            .send()?
            .error_for_status()?;

        let mut finalized: Value = self
            .client
            .post(self.endpoint.join("v1/finalize")?)
            .bearer_auth(study_code)
            .json(&FinalizeRequest {
                study_code,
                receipt: &negotiate.receipt,
                export_digest: &digest,
            })
            .send()?
            .error_for_status()?
            .json()?;
        let Value::Object(ref mut object) = finalized else {
            anyhow::bail!("finalize response must be a JSON object");
        };
        object.insert("resultUrl".to_owned(), Value::String(negotiate.result_url));
        Ok(finalized)
    }

    fn status(&self, receipt: &str, study_code: &str) -> anyhow::Result<Value> {
        validate_receipt(receipt)?;
        validate_study_code(study_code)?;
        self.client
            .get(self.endpoint.join(&format!("v1/status/{receipt}"))?)
            .bearer_auth(study_code)
            .header("x-study-code", study_code)
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }

    fn revoke(&self, receipt: &str, study_code: &str) -> anyhow::Result<Value> {
        validate_receipt(receipt)?;
        validate_study_code(study_code)?;
        self.client
            .post(self.endpoint.join(&format!("v1/revoke/{receipt}"))?)
            .bearer_auth(study_code)
            .header("x-study-code", study_code)
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }
}

fn require_secure_url(url: &Url, label: &str) -> anyhow::Result<()> {
    if url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
    {
        return Ok(());
    }
    #[cfg(test)]
    if url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"))
    {
        return Ok(());
    }
    anyhow::bail!("{label} must use HTTPS")
}

fn validate_study_code(study_code: &str) -> anyhow::Result<()> {
    if study_code.is_empty() || study_code.len() > 512 || study_code.chars().any(char::is_control) {
        anyhow::bail!("study code must be non-empty, bounded, and contain no control characters");
    }
    Ok(())
}

fn require_same_origin(intake: &Url, upload: &Url) -> anyhow::Result<()> {
    if intake.scheme() != upload.scheme()
        || intake.host_str() != upload.host_str()
        || intake.port_or_known_default() != upload.port_or_known_default()
    {
        anyhow::bail!("upload URL must use the configured intake origin");
    }
    Ok(())
}

fn validate_receipt(receipt: &str) -> anyhow::Result<()> {
    if receipt.is_empty()
        || receipt.len() > 128
        || !receipt.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        anyhow::bail!("receipt must use only ASCII letters, digits, hyphens, or underscores");
    }
    Ok(())
}

fn read_finalized_export(path: &Path) -> anyhow::Result<Vec<u8>> {
    let initial = fs::symlink_metadata(path).context("inspect finalized export")?;
    require_regular_single_link(&initial)?;
    if initial.len() > MAX_FINALIZED_EXPORT_BYTES {
        anyhow::bail!("finalized export exceeds the 262144-byte upload limit");
    }

    let file = File::open(path).context("open finalized export")?;
    let opened = file.metadata().context("inspect opened finalized export")?;
    require_regular_single_link(&opened)?;
    if opened.len() > MAX_FINALIZED_EXPORT_BYTES {
        anyhow::bail!("finalized export exceeds the 262144-byte upload limit");
    }
    ensure_same_file(&initial, &opened)?;

    let mut bytes =
        Vec::with_capacity(usize::try_from(opened.len()).context("export size overflow")?);
    file.take(MAX_FINALIZED_EXPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("read finalized export")?;
    if bytes.len() > usize::try_from(MAX_FINALIZED_EXPORT_BYTES).expect("constant fits usize") {
        anyhow::bail!("finalized export exceeds the 262144-byte upload limit");
    }
    Ok(bytes)
}

fn require_regular_single_link(metadata: &fs::Metadata) -> anyhow::Result<()> {
    if !metadata.file_type().is_file() {
        anyhow::bail!("finalized export must be a regular file, not a symlink or directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            anyhow::bail!("finalized export must not have hard links");
        }
    }
    Ok(())
}

fn ensure_same_file(initial: &fs::Metadata, opened: &fs::Metadata) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if initial.dev() != opened.dev() || initial.ino() != opened.ino() {
            anyhow::bail!("finalized export changed while opening");
        }
    }
    #[cfg(not(unix))]
    if initial.len() != opened.len() {
        anyhow::bail!("finalized export changed while opening");
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Send(command) => {
            if !command.confirm {
                anyhow::bail!("refusing upload without --confirm");
            }
            print_json(
                &UploadClient::new(command.endpoint)?.send(&command.export, &command.study_code)?,
            )?;
        }
        Command::Status(command) => {
            print_json(
                &UploadClient::new(command.endpoint)?
                    .status(&command.receipt, &command.study_code)?,
            )?;
        }
        Command::Revoke(command) => {
            if !command.confirm {
                anyhow::bail!("refusing revocation without --confirm");
            }
            print_json(
                &UploadClient::new(command.endpoint)?
                    .revoke(&command.receipt, &command.study_code)?,
            )?;
        }
    }
    Ok(())
}

fn print_json(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread,
    };

    use chrono::{TimeZone as _, Utc};
    use observer_domain::{
        ADAPTER_REGISTRY_VERSION, Collector, Comparability, ComparabilityDisposition,
        EXPORT_CONTRACT_VERSION, ExportConsent, Integrity, METRIC_REGISTRY_VERSION, NONCLAIMS,
        Privacy, Study, StudyExport, ZERO_SHA256,
    };

    use super::*;

    fn sample_export() -> StudyExport {
        let at = Utc.with_ymd_and_hms(2026, 8, 18, 0, 0, 0).single().expect("timestamp");
        let mut export = StudyExport {
            contract_version: EXPORT_CONTRACT_VERSION.to_owned(),
            collector: Collector {
                version: "test".to_owned(),
                binary_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
                configuration_sha256: ZERO_SHA256.to_owned(),
                metric_registry_version: METRIC_REGISTRY_VERSION.to_owned(),
                adapter_registry_version: ADAPTER_REGISTRY_VERSION.to_owned(),
            },
            study: Study {
                participant_id: "participant01".to_owned(),
                device_id: "device000001".to_owned(),
                run_id: "run000000001".to_owned(),
            },
            consent: ExportConsent {
                manifest_sha256: ZERO_SHA256.to_owned(),
                approved_adapters: vec!["codex".to_owned()],
                approved_field_classes: vec![
                    observer_domain::FieldClass::FilesystemMetadata,
                    observer_domain::FieldClass::Timestamps,
                    observer_domain::FieldClass::EventTypes,
                    observer_domain::FieldClass::Counters,
                ],
                content_analysis: false,
                collection_approved_at: at,
            },
            windows: vec![
                observer_domain::CollectionWindow {
                    id: "retained-history".to_owned(),
                    kind: observer_domain::WindowKind::RetainedHistory,
                    start: None,
                    end: at,
                    timezone: "UTC".to_owned(),
                },
                observer_domain::CollectionWindow {
                    id: "baseline-28d".to_owned(),
                    kind: observer_domain::WindowKind::Baseline,
                    start: Some(at - chrono::Duration::days(28)),
                    end: at,
                    timezone: "UTC".to_owned(),
                },
            ],
            coverage: vec![],
            metrics: vec![],
            comparability: Comparability {
                disposition: ComparabilityDisposition::Incomparable,
                blocking_mismatches: vec!["empty".to_owned()],
            },
            privacy: Privacy {
                forbidden_fields_absent: true,
                content_persisted: false,
                content_exported: false,
            },
            integrity: Integrity { payload_sha256: ZERO_SHA256.to_owned() },
            nonclaims: NONCLAIMS.iter().map(|value| (*value).to_owned()).collect(),
        };
        export.finalize().expect("finalize");
        export
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let count = stream.read(&mut chunk).expect("read request");
            bytes.extend_from_slice(&chunk[..count]);
            let text = String::from_utf8_lossy(&bytes);
            if let Some(header_end) = text.find("\r\n\r\n") {
                let length = text[..header_end]
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length: ")
                            .or_else(|| line.strip_prefix("Content-Length: "))
                    })
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + length {
                    return text.into_owned();
                }
            }
        }
    }

    fn reply(stream: &mut TcpStream, body: &str, extra_headers: &str) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{}\r\n{}",
            body.len(),
            extra_headers,
            body
        )
        .expect("write response");
    }

    #[test]
    fn rejects_non_https_endpoints_outside_test_loopback() {
        let public_http = Url::parse("http://intake.example.test/").expect("URL");
        assert!(UploadClient::new(public_http).is_err());
        let credentials = Url::parse("https://code@intake.example.test/").expect("URL");
        assert!(UploadClient::new(credentials).is_err());
        let loopback = Url::parse("http://127.0.0.1:9999/").expect("URL");
        assert!(UploadClient::new(loopback).is_ok());
    }

    #[test]
    fn rejects_cross_origin_upload_grants() {
        let intake = Url::parse("https://intake.example.test/").expect("URL");
        let different_host =
            Url::parse("https://archive.example.test/v1/upload/receipt").expect("URL");
        let different_port =
            Url::parse("https://intake.example.test:8443/v1/upload/receipt").expect("URL");
        assert!(require_same_origin(&intake, &different_host).is_err());
        assert!(require_same_origin(&intake, &different_port).is_err());
    }

    #[test]
    fn finalized_export_reader_rejects_oversized_file_before_parsing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("oversized.study-export");
        fs::write(
            &path,
            vec![b'x'; usize::try_from(MAX_FINALIZED_EXPORT_BYTES + 1).expect("size")],
        )
        .expect("write export");
        assert!(read_finalized_export(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn finalized_export_reader_rejects_links() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("tempdir");
        let regular = directory.path().join("regular.study-export");
        let linked = directory.path().join("linked.study-export");
        let symlinked = directory.path().join("symlinked.study-export");
        fs::write(&regular, sample_export().canonical_bytes().expect("bytes"))
            .expect("write export");
        fs::hard_link(&regular, &linked).expect("hard link");
        symlink(&regular, &symlinked).expect("symlink");
        assert!(read_finalized_export(&linked).is_err());
        assert!(read_finalized_export(&symlinked).is_err());
    }

    #[test]
    fn mocked_negotiate_put_finalize_status_and_revoke_flow() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for _ in 0..5 {
                let (mut stream, _) = listener.accept().expect("accept");
                let request = read_request(&mut stream);
                let first_line = request.lines().next().expect("request line").to_owned();
                captured.lock().expect("lock").push(request);
                if first_line.contains("/v1/negotiate") {
                    reply(
                        &mut stream,
                        &format!(
                            "{{\"receipt\":\"receipt-1\",\"nextAction\":\"upload\",\"upload\":{{\"url\":\"http://{address}/v1/upload/receipt-1\",\"headers\":{{\"authorization\":\"Upload upload-token\",\"content-type\":\"application/json\"}},\"expiresInSeconds\":60}},\"resultUrl\":\"http://{address}/v1/results/receipt-1#cap=result-capability\"}}"
                        ),
                        "",
                    );
                } else if first_line.contains("/v1/upload/receipt-1") {
                    reply(&mut stream, "{}", "");
                } else if first_line.contains("/v1/finalize") {
                    reply(&mut stream, "{\"receipt\":\"receipt-1\"}", "");
                } else if first_line.starts_with("GET /v1/status/receipt-1") {
                    reply(&mut stream, "{\"status\":\"accepted\"}", "");
                } else {
                    reply(&mut stream, "{\"revoked\":true}", "");
                }
            }
        });
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("final.study-export");
        fs::write(&path, sample_export().canonical_bytes().expect("bytes")).expect("write export");
        let endpoint = Url::parse(&format!("http://{address}/")).expect("endpoint");
        let client = UploadClient::new(endpoint).expect("client");
        let result = client.send(&path, "study-code").expect("send");
        assert_eq!(result["receipt"], "receipt-1");
        assert_eq!(
            result["resultUrl"],
            format!("http://{address}/v1/results/receipt-1#cap=result-capability")
        );
        assert_eq!(client.status("receipt-1", "study-code").expect("status")["status"], "accepted");
        assert_eq!(client.revoke("receipt-1", "study-code").expect("revoke")["revoked"], true);
        server.join().expect("server");
        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 5);
        assert!(requests[0].contains("POST /v1/negotiate"));
        assert!(requests[0].to_ascii_lowercase().contains("authorization: bearer study-code"));
        assert!(requests[1].contains("PUT /v1/upload/receipt-1"));
        assert!(requests[1].to_ascii_lowercase().contains("authorization: upload upload-token"));
        assert_eq!(
            requests[1]
                .lines()
                .filter(|line| line.to_ascii_lowercase().starts_with("content-type:"))
                .count(),
            1
        );
        assert!(requests[2].contains("POST /v1/finalize"));
        assert!(requests[3].contains("GET /v1/status/receipt-1"));
        assert!(requests[3].to_ascii_lowercase().contains("x-study-code: study-code"));
        assert!(requests[4].contains("POST /v1/revoke/receipt-1"));
    }

    #[test]
    fn duplicate_status_negotiation_returns_a_recoverable_receipt_without_reuploading() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let request = read_request(&mut stream);
                let first_line = request.lines().next().expect("request line").to_owned();
                captured.lock().expect("lock").push(request);
                if first_line.contains("/v1/negotiate") {
                    reply(
                        &mut stream,
                        &format!(
                            "{{\"receipt\":\"receipt-1\",\"status\":\"queued\",\"duplicate\":true,\"nextAction\":\"status\",\"resultUrl\":\"http://{address}/v1/results/receipt-1#cap=result-capability\"}}"
                        ),
                        "",
                    );
                } else {
                    reply(&mut stream, "{\"receipt\":\"receipt-1\",\"status\":\"queued\"}", "");
                }
            }
        });
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("final.study-export");
        fs::write(&path, sample_export().canonical_bytes().expect("bytes")).expect("write export");
        let endpoint = Url::parse(&format!("http://{address}/")).expect("endpoint");
        let result = UploadClient::new(endpoint)
            .expect("client")
            .send(&path, "study-code")
            .expect("duplicate status recovery");
        assert_eq!(result["status"], "queued");
        assert_eq!(
            result["resultUrl"],
            format!("http://{address}/v1/results/receipt-1#cap=result-capability")
        );
        server.join().expect("server");
        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("POST /v1/negotiate"));
        assert!(requests[1].contains("GET /v1/status/receipt-1"));
        assert!(!requests.iter().any(|request| request.contains("PUT /v1/upload/")));
        assert!(!requests.iter().any(|request| request.contains("POST /v1/finalize")));
    }

    #[test]
    fn uploader_rejects_tampered_export_before_network() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("tampered.study-export");
        let bytes = sample_export().canonical_bytes().expect("bytes");
        let tampered =
            String::from_utf8(bytes).expect("utf8").replace("not productivity", "not productivitz");
        fs::write(&path, tampered).expect("write export");
        let client = UploadClient::new(Url::parse("http://127.0.0.1:9/").expect("endpoint"))
            .expect("client");
        assert!(client.send(&path, "study-code").is_err());
    }
}
