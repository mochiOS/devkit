//! Local crash history and the shared crash presentation. Detailed reports
//! remain local; neither reading nor exporting a report grants upload consent.
use crate::{Error, Result};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct CrashSummary {
    pub id: String,
    pub package_id: Option<String>,
    pub name: String,
    pub version: String,
    pub utc_seconds: Option<u64>,
}

impl CrashSummary {
    fn decode(value: &Value) -> Result<Self> {
        let id = value["id"]
            .as_str()
            .ok_or(Error::InvalidArgument)?
            .to_owned();
        if !valid_report_id(&id) {
            return Err(Error::InvalidArgument);
        }
        Ok(Self {
            id,
            package_id: value["package_id"].as_str().map(str::to_owned),
            name: value["package_name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or("Application")
                .to_owned(),
            version: value["package_version"]
                .as_str()
                .unwrap_or("Unknown")
                .to_owned(),
            utc_seconds: value["collected_utc_seconds"].as_u64(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct CrashReport {
    pub summary: CrashSummary,
    pub details: String,
    bytes: Vec<u8>,
}

pub fn valid_report_id(id: &str) -> bool {
    let Some(body) = id
        .strip_prefix("crash-")
        .and_then(|id| id.strip_suffix(".json"))
    else {
        return false;
    };
    let bytes = body.as_bytes();
    bytes.len() == 37
        && bytes[16] == b'-'
        && bytes[..16]
            .iter()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && bytes[17..].iter().all(u8::is_ascii_digit)
}

pub fn history() -> Result<Vec<CrashSummary>> {
    #[cfg(target_os = "mochios")]
    {
        let bytes = mochi_user_platform::diagnostic::reports::list()?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidArgument)?;
        if value["schema_version"] != 1 {
            return Err(Error::InvalidArgument);
        }
        let reports = value["reports"]
            .as_array()
            .filter(|v| v.len() <= 128)
            .ok_or(Error::InvalidArgument)?;
        reports.iter().rev().map(CrashSummary::decode).collect()
    }
    #[cfg(not(target_os = "mochios"))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

pub fn report(id: &str) -> Result<CrashReport> {
    if !valid_report_id(id) {
        return Err(Error::InvalidArgument);
    }
    #[cfg(target_os = "mochios")]
    {
        let bytes = mochi_user_platform::diagnostic::reports::read(id)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidArgument)?;
        if value["schema_version"] != 1 {
            return Err(Error::InvalidArgument);
        }
        let image = &value["image"];
        let summary = CrashSummary::decode(&serde_json::json!({
            "id":id, "package_id":image["package_id"], "package_name":image["package_name"],
            "package_version":image["package_version"], "collected_utc_seconds":value["collected_utc_seconds"],
        }))?;
        let mut details = format!(
            "Application: {}\nVersion: {}\nPackage: {}\nProcess: {} / Thread: {}\nImage revision: {}\nArchitecture: {}\nException vector: {} / cause: {}\nInstruction: {}\nStack pointer: {}\nFault address: {}\nImage SHA-256: {}\n\n{}\n",
            summary.name,
            summary.version,
            summary.package_id.as_deref().unwrap_or("Unavailable"),
            value["process_id"],
            value["thread_id"],
            value["image_revision"],
            value["architecture"],
            value["exception"]["vector"],
            value["exception"]["cause"],
            value["exception"]["instruction"]
                .as_str()
                .unwrap_or("Unavailable"),
            value["exception"]["stack_pointer"]
                .as_str()
                .unwrap_or("Unavailable"),
            value["exception"]["address"]
                .as_str()
                .unwrap_or("Unavailable"),
            value["image_sha256"].as_str().unwrap_or("Unavailable"),
            value["os_version_toml"]
                .as_str()
                .unwrap_or("OS version unavailable")
        );
        if let Some(modules) = image["modules"].as_array() {
            details.push_str("Loaded images at launch:\n");
            for module in modules.iter().take(128) {
                details.push_str(&format!(
                    "{}\n  SHA-256: {}\n  Load bias: {}\n",
                    module["path"].as_str().unwrap_or("Unavailable"),
                    module["source_sha256"].as_str().unwrap_or("Unavailable"),
                    module["load_bias"]
                ));
            }
        }
        details.push_str("\nLater dynamic loads and full stack traces are not captured.\nThis report has not been sent to mochiOS.");
        Ok(CrashReport {
            summary,
            details,
            bytes,
        })
    }
    #[cfg(not(target_os = "mochios"))]
    {
        Err(Error::UnsupportedPlatform)
    }
}

pub fn reopen(report: &CrashReport) -> Result<()> {
    let package = report
        .summary
        .package_id
        .as_deref()
        .ok_or(Error::InvalidArgument)?;
    #[cfg(target_os = "mochios")]
    {
        mochi_user_platform::workspace::launch_application(package)?;
        Ok(())
    }
    #[cfg(not(target_os = "mochios"))]
    {
        let _ = package;
        Err(Error::UnsupportedPlatform)
    }
}

impl CrashReport {
    /// Export only to the destination explicitly selected by the user.
    pub fn export(&self, destination: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(destination)?;
        file.write_all(&self.bytes)?;
        file.sync_all()
    }
}

/// Standard ViewKit crash prompt shared by system diagnostic readers.
#[cfg(feature = "ui")]
pub fn crash_dialog(
    report: &CrashReport,
    count: usize,
    reopen: impl FnMut() + 'static,
    details: impl FnMut() + 'static,
    close: impl FnMut() + 'static,
) -> Box<dyn viewkit::view::View> {
    use viewkit::prelude::*;
    let message = if count > 1 {
        format!(
            "{count} applications stopped unexpectedly. The latest report is available in Details."
        )
    } else {
        "A local diagnostic report was saved. You can reopen the application or view its details."
            .into()
    };
    Box::new(
        Dialog::new()
            .accessibility_label("Application stopped unexpectedly")
            .content(
                VStack::new()
                    .gap(StackGap::Large)
                    .child(
                        HStack::new().child(Icon::new(SymbolName::Warning)).child(
                            Text::new(format!("{} quit unexpectedly", report.summary.name))
                                .style(TextRole::TitleSmall),
                        ),
                    )
                    .child(Text::new(message))
                    .child(
                        Text::new("This report has not been sent to mochiOS.")
                            .tone(TextTone::Secondary),
                    )
                    .child(Spacer::new())
                    .child(
                        HStack::new()
                            .distribution(StackDistribution::End)
                            .child(Button::new("Close").on_click(close))
                            .child(Button::new("Details").on_click(details))
                            .child(
                                Button::new("Reopen")
                                    .style(ButtonStyle::Primary)
                                    .enabled(report.summary.package_id.is_some())
                                    .on_click(reopen),
                            ),
                    ),
            ),
    )
}
