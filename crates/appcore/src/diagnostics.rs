//! Current-user crash reports, obtained from the authenticated logger service.
//! Report paths and recorded arguments never become execution authority.
use serde_json::Value;
use std::{io, path::Path};

#[derive(Clone, Debug)]
pub struct CrashSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub package_id: Option<String>,
    pub utc_seconds: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct CrashReport {
    pub summary: CrashSummary,
    pub details: String,
    bytes: Vec<u8>,
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid crash report snapshot")
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("crash-")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|s| {
            let b = s.as_bytes();
            b.len() == 37
                && b[16] == b'-'
                && b[..16]
                    .iter()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                && b[17..].iter().all(u8::is_ascii_digit)
        })
}
fn summary(id: &str, record: &Value) -> io::Result<CrashSummary> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let package_id = record["package_id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
        .map(str::to_owned);
    Ok(CrashSummary {
        id: id.into(),
        name: record["package_name"]
            .as_str()
            .unwrap_or("Unknown application")
            .into(),
        version: record["package_version"]
            .as_str()
            .unwrap_or("Unknown version")
            .into(),
        package_id,
        utc_seconds: record["collected_utc_seconds"].as_u64(),
    })
}
/// Read the bounded report catalogue for the current OS account.
pub fn history() -> io::Result<Vec<CrashSummary>> {
    let bytes = native::list()?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if body["schema_version"] != 1 {
        return Err(invalid());
    }
    let records = body["reports"]
        .as_array()
        .filter(|r| r.len() <= 128)
        .ok_or_else(invalid)?;
    records
        .iter()
        .map(|r| summary(r["id"].as_str().ok_or_else(invalid)?, r))
        .collect()
}
/// Fetch exactly one report ID, without accepting a filesystem path.
pub fn report(id: &str) -> io::Result<CrashReport> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let bytes = native::read(id)?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if body["schema_version"] != 1 {
        return Err(invalid());
    }
    let session = body["collector_session"].as_str().ok_or_else(invalid)?;
    let cursor = body["cursor"].as_u64().ok_or_else(invalid)?;
    if format!("crash-{session}-{cursor:020}.json") != id {
        return Err(invalid());
    }
    let mut metadata = body["image"].clone();
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    metadata["collected_utc_seconds"] = body["collected_utc_seconds"].clone();
    let summary = summary(id, &metadata)?;
    let details = serde_json::to_string_pretty(&body).map_err(|_| invalid())?;
    Ok(CrashReport {
        summary,
        details,
        bytes,
    })
}
impl CrashReport {
    /// Export using the caller's existing pathname authority.
    pub fn export(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, &self.bytes)
    }
    /// Export through the exact resource returned by the system save panel.
    #[cfg(feature = "ui")]
    pub fn export_file(&self, file: &crate::DocumentFile) -> io::Result<()> {
        file.replace_contents(&self.bytes)
    }
}
/// Resolve the current installed package through the normal launch broker.
pub fn reopen(report: &CrashReport) -> io::Result<()> {
    native::reopen(report.summary.package_id.as_deref().ok_or_else(invalid)?)
}
#[cfg(feature = "ui")]
pub fn crash_dialog(
    report: &CrashReport,
    count: usize,
    reopen: impl Fn() + 'static,
    details: impl Fn() + 'static,
    close: impl Fn() + Clone + 'static,
) -> Box<dyn viewkit::view::View> {
    use viewkit::prelude::*;
    let dismiss = close.clone();
    Box::new(Dialog::new().accessibility_label("Application crash report").on_dismiss(move || dismiss()).content(
        VStack::new().gap(StackGap::Large)
            .child(Text::styled(format!("{} closed unexpectedly", report.summary.name), TextRole::TitleMedium))
            .child(Text::body(format!("{count} local crash report(s). You can view the details or reopen the application.")))
            .child(HStack::new().gap(StackGap::Medium)
                .child(Button::new("Details").on_click(details))
                .child(Button::new("Reopen").enabled(report.summary.package_id.is_some()).on_click(reopen))
                .child(Button::new("Close").on_click(close)))))
}
#[cfg(target_os = "mochios")]
mod native {
    use super::*;
    fn error(e: mochi_user_platform::syscall::SysError) -> io::Error {
        io::Error::from_raw_os_error(mochios_posix_client::newlib_errno::from_status(e.raw()))
    }
    pub fn list() -> io::Result<Vec<u8>> {
        mochi_user_platform::diagnostic::reports::list().map_err(error)
    }
    pub fn read(id: &str) -> io::Result<Vec<u8>> {
        mochi_user_platform::diagnostic::reports::read(id).map_err(error)
    }
    pub fn reopen(package: &str) -> io::Result<()> {
        mochi_user_platform::workspace::launch_application(package)
            .map(|_| ())
            .map_err(error)
    }
}
#[cfg(not(target_os = "mochios"))]
mod native {
    use super::*;
    pub fn list() -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn read(_: &str) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn reopen(_: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
