//! Runtime UTF-8 documents, separately packaged for post-install viewing.
use crate::{ConsentRecord, DataCategory, DocumentReceipt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct License {
    pub name: String,
    pub version: Option<String>,
    pub license: String,
    pub copyright_file: String,
    pub body_file: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosureField {
    pub key: String,
    pub title: String,
    pub file: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Disclosure {
    pub summary_file: String,
    pub fields: Vec<DisclosureField>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataPolicy {
    pub category: DataCategory,
    pub version: String,
    pub text: BTreeMap<String, Disclosure>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub document_version: String,
    pub licenses: Vec<License>,
    pub data_policies: Vec<DataPolicy>,
}
#[derive(Clone, Debug)]
pub struct Documents {
    pub catalog: Catalog,
    pub files: BTreeMap<String, String>,
}
impl Documents {
    /// Hash the catalog and every exact document byte, including disclosures.
    /// Callers bind approval to this hash so an OS update cannot replace the
    /// displayed policy between review and persistence.
    pub fn bundle_hash(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"mochiOS device privacy bundle v1\0");
        hash.update(serde_json::to_vec(&self.catalog).expect("serializable document catalog"));
        for (name, body) in &self.files {
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((body.len() as u64).to_le_bytes());
            hash.update(body.as_bytes());
        }
        hash.finalize().into()
    }
    /// Prepare explicitly confirmed device choices while keeping unrelated
    /// permissions and their revisions intact. Missing legal receipts require
    /// explicit confirmation of the current displayed legal documents.
    /// Persistence and administrator authorization remain the caller's duties.
    pub fn confirmed_choices(
        &self,
        mut record: ConsentRecord,
        choices: [bool; 3],
        legal_confirmed: bool,
        now: u64,
    ) -> io::Result<ConsentRecord> {
        if now < 1_577_836_800
            || record.schema_version != crate::SCHEMA_VERSION
            || DataCategory::ALL
                .into_iter()
                .any(|c| self.policy(c).version != c.policy_version())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Privacy policy or clock is not current",
            ));
        }
        if !self.legal_matches(&record) {
            if !legal_confirmed {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Confirm the installed legal documents first",
                ));
            }
            record.terms = Some(self.receipt("terms.txt", now));
            record.privacy = Some(self.receipt("privacy.txt", now));
            record.open_source_licenses = Some(DocumentReceipt::new(
                &self.catalog.document_version,
                &self.license_bytes(),
                now,
            ));
        }
        for category in DataCategory::ALL {
            let choice = choices[category.index()];
            if record.permission(category).is_none_or(|p| {
                p.agreed() != choice || p.policy_version != category.policy_version()
            }) {
                record.set_permission(category, choice, category.policy_version(), now);
            }
        }
        if !record.complete() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Device choices are incomplete",
            ));
        }
        Ok(record)
    }
    /// Load the bounded UTF-8 document bundle and validate every policy category.
    pub fn load(root: &Path) -> io::Result<Self> {
        let catalog: Catalog =
            serde_json::from_str(&read(root, "catalog.json")?).map_err(io::Error::other)?;
        if catalog.schema_version != 1
            || catalog.document_version.is_empty()
            || catalog.licenses.is_empty()
            || catalog.licenses.len() > 256
        {
            return Err(io::Error::other("Unsupported legal document catalog"));
        }
        let mut files = BTreeMap::new();
        for name in ["terms.txt", "privacy.txt"] {
            insert(&mut files, root, name)?;
        }
        for license in &catalog.licenses {
            for name in [&license.copyright_file, &license.body_file] {
                insert(&mut files, root, name)?;
            }
        }
        if catalog.data_policies.len() != 3 {
            return Err(io::Error::other(
                "Each data category requires its own policy",
            ));
        }
        let mut categories = std::collections::BTreeSet::new();
        for policy in &catalog.data_policies {
            if !categories.insert(policy.category.index())
                || policy.version.is_empty()
                || policy.text.len() > 8
                || !policy.text.contains_key("en")
            {
                return Err(io::Error::other("Invalid data policy catalog"));
            }
            for disclosure in policy.text.values() {
                insert(&mut files, root, &disclosure.summary_file)?;
                let required: std::collections::BTreeSet<_> = [
                    "purpose",
                    "data",
                    "destination",
                    "retention",
                    "exclusions",
                    "settings",
                ]
                .into_iter()
                .collect();
                let keys: std::collections::BTreeSet<_> = disclosure
                    .fields
                    .iter()
                    .map(|field| field.key.as_str())
                    .collect();
                if disclosure.fields.len() != 6 || keys != required {
                    return Err(io::Error::other("Incomplete data policy disclosure"));
                }
                for field in &disclosure.fields {
                    insert(&mut files, root, &field.file)?;
                }
            }
        }
        Ok(Self { catalog, files })
    }
    /// Return the validated disclosure policy for a data category.
    pub fn policy(&self, category: DataCategory) -> &DataPolicy {
        self.catalog
            .data_policies
            .iter()
            .find(|p| p.category == category)
            .expect("validated policy catalog")
    }
    /// Return localized disclosure metadata, falling back to English.
    pub fn disclosure(&self, category: DataCategory, locale: &str) -> &Disclosure {
        let policy = self.policy(category);
        policy.text.get(locale).unwrap_or(&policy.text["en"])
    }
    /// Return a previously loaded document without filesystem access.
    pub fn text(&self, name: &str) -> &str {
        self.files.get(name).map_or("", String::as_str)
    }
    /// Bind the OSS receipt to the catalog and exact copyright/body bytes.
    pub fn license_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for license in &self.catalog.licenses {
            for text in [
                license.name.as_str(),
                license.version.as_deref().unwrap_or(""),
                license.license.as_str(),
                self.text(&license.copyright_file),
                self.text(&license.body_file),
            ] {
                bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
                bytes.extend_from_slice(text.as_bytes());
            }
        }
        bytes
    }
    /// Bind acknowledgement to the exact displayed document bytes.
    pub fn receipt(&self, name: &str, at: u64) -> DocumentReceipt {
        DocumentReceipt::new(
            &self.catalog.document_version,
            self.text(name).as_bytes(),
            at,
        )
    }
    /// Check that stored receipts match the installed legal bundle.
    pub fn legal_matches(&self, record: &ConsentRecord) -> bool {
        record.terms.as_ref().is_some_and(|r| {
            r.matches(
                &self.catalog.document_version,
                self.text("terms.txt").as_bytes(),
            )
        }) && record.privacy.as_ref().is_some_and(|r| {
            r.matches(
                &self.catalog.document_version,
                self.text("privacy.txt").as_bytes(),
            )
        }) && record
            .open_source_licenses
            .as_ref()
            .is_some_and(|r| r.matches(&self.catalog.document_version, &self.license_bytes()))
    }
}
fn insert(files: &mut BTreeMap<String, String>, root: &Path, name: &str) -> io::Result<()> {
    if !files.contains_key(name) {
        let body = read(root, name)?;
        if files.values().map(String::len).sum::<usize>() + body.len() > 16 * 1024 * 1024 {
            return Err(io::Error::other("Legal bundle exceeds its size limit"));
        }
        files.insert(name.into(), body);
    }
    Ok(())
}
fn read(root: &Path, name: &str) -> io::Result<String> {
    if Path::new(name).components().count() != 1 || name.starts_with('.') {
        return Err(io::Error::other("Invalid legal document path"));
    }
    let mut bytes = Vec::new();
    fs::File::open(root.join(name))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::other("Legal document is too large"));
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    fn documents() -> Documents {
        let policies: Vec<_> = DataCategory::ALL.into_iter().map(|c| serde_json::json!({
            "category": c.key(), "version": c.policy_version(), "text": {"en": {"summary_file": "summary.txt", "fields": []}}
        })).collect();
        let catalog = serde_json::from_value(serde_json::json!({"schema_version": 1, "document_version": "legal-1", "licenses": [{"name": "Example", "version": null, "license": "MIT", "copyright_file": "license.txt", "body_file": "license.txt"}], "data_policies": policies})).unwrap();
        Documents {
            catalog,
            files: ["terms.txt", "privacy.txt", "license.txt", "summary.txt"]
                .into_iter()
                .map(|name| (name.into(), "abcdef".into()))
                .collect(),
        }
    }
    #[test]
    fn initial_choices_require_confirmation_of_exact_legal_documents() {
        let docs = documents();
        assert!(
            docs.confirmed_choices(ConsentRecord::default(), [true; 3], false, 1_800_000_000)
                .is_err()
        );
        let record = docs
            .confirmed_choices(
                ConsentRecord::default(),
                [true, false, false],
                true,
                1_800_000_000,
            )
            .unwrap();
        assert!(record.complete());
        assert!(docs.legal_matches(&record));
        assert!(record.allows(DataCategory::UsageDiagnostics, "placeholder-2"));
        assert!(!record.allows(DataCategory::UpdateResults, "placeholder-2"));
        assert!(docs.confirmed_choices(record, [true; 3], false, 0).is_err());
    }
    #[test]
    fn changing_one_category_does_not_invalidate_other_categories_queued_data() {
        let docs = documents();
        let record = docs
            .confirmed_choices(ConsentRecord::default(), [true; 3], true, 1_800_000_000)
            .unwrap();
        let updates = record.update_results.clone();
        let crashes = record.crash_reports.clone();
        let terms = record.terms.clone();
        let changed = docs
            .confirmed_choices(record.clone(), [false, true, true], false, 1_800_000_000)
            .unwrap();
        assert_eq!(changed.update_results, updates);
        assert_eq!(changed.crash_reports, crashes);
        assert_eq!(changed.terms, terms);
        assert_eq!(
            changed.usage_diagnostics.as_ref().unwrap().revision,
            record.usage_diagnostics.as_ref().unwrap().revision + 1
        );
        assert_eq!(
            docs.confirmed_choices(changed.clone(), [false, true, true], false, 1_800_000_001)
                .unwrap(),
            changed
        );
    }
    #[test]
    fn document_and_disclosure_changes_are_bound_to_review() {
        let mut docs = documents();
        let original = docs.bundle_hash();
        let record = docs
            .confirmed_choices(ConsentRecord::default(), [false; 3], true, 1_800_000_000)
            .unwrap();
        docs.files.insert("terms.txt".into(), "changed".into());
        assert_ne!(docs.bundle_hash(), original);
        assert!(
            docs.confirmed_choices(record.clone(), [true; 3], false, 1_800_000_001)
                .is_err()
        );
        let updated = docs
            .confirmed_choices(record, [false; 3], true, 1_800_000_001)
            .unwrap();
        assert!(docs.legal_matches(&updated));
        let hash = docs.bundle_hash();
        docs.catalog.data_policies[0].version = "future-policy".into();
        assert_ne!(docs.bundle_hash(), hash);
        assert!(
            docs.confirmed_choices(updated, [true; 3], true, 1_800_000_001)
                .is_err()
        );
    }
}
