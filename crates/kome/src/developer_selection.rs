use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use mochios_certificate::is_valid_developer_id;

use crate::{auth::DeveloperMembership, manifest::KomeManifest, preferences::Preferences};

pub fn resolve(
    manifest: &KomeManifest,
    preferences: &Preferences,
    memberships: &[DeveloperMembership],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<String> {
    if let Some(developer) = &manifest.developer {
        return select_explicit(&developer.id, memberships, "Kome.toml");
    }
    if let Some(developer_id) = &preferences.default_developer {
        return select_explicit(developer_id, memberships, "ユーザー設定");
    }

    let issuable: Vec<&DeveloperMembership> = memberships
        .iter()
        .filter(|membership| membership.can_issue())
        .collect();
    match issuable.as_slice() {
        [] => bail!(
            "利用可能なDeveloperがありません。\n\nConsoleでDeveloperを作成してから、もう一度`kome sign`を実行してください。"
        ),
        [membership] => Ok(membership.developer_id.clone()),
        _ => choose_interactively(&issuable, input, output),
    }
}

fn select_explicit(
    developer_id: &str,
    memberships: &[DeveloperMembership],
    source: &str,
) -> Result<String> {
    if !is_valid_developer_id(developer_id) {
        bail!("{source}のDeveloper IDが正しい形式ではありません");
    }
    let membership = memberships
        .iter()
        .find(|membership| membership.developer_id == developer_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "現在のAccountは指定されたDeveloperのactive Memberではありません。\n\n確認:\n  kome account\n  kome developer list"
            )
        })?;
    if membership.membership_status != "active" {
        bail!(
            "現在のAccountは指定されたDeveloperのactive Memberではありません。\n\n確認:\n  kome account\n  kome developer list"
        );
    }
    if membership.developer_status != "verified" || !membership.certificate_issuable {
        bail!("Developerはまだ確認されていません。\nConsoleで状態を確認してください。");
    }
    Ok(developer_id.to_string())
}

fn choose_interactively(
    memberships: &[&DeveloperMembership],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<String> {
    writeln!(output, "Select a Developer:")?;
    for (index, membership) in memberships.iter().enumerate() {
        writeln!(
            output,
            "  {}. {} {}",
            index + 1,
            membership.developer_id,
            membership.name
        )?;
    }
    write!(output, "> ")?;
    output.flush()?;
    let mut selection = String::new();
    input
        .read_line(&mut selection)
        .context("failed to read Developer selection")?;
    let index = selection
        .trim()
        .parse::<usize>()
        .context("Developer selection must be a number")?;
    let membership = memberships
        .get(
            index
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("Developer selection is out of range"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("Developer selection is out of range"))?;
    Ok(membership.developer_id.clone())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::manifest::KomeManifest;

    fn membership(id: &str) -> DeveloperMembership {
        DeveloperMembership {
            developer_id: id.to_string(),
            name: "Example".to_string(),
            membership_status: "active".to_string(),
            developer_status: "verified".to_string(),
            certificate_issuable: true,
        }
    }

    fn manifest() -> KomeManifest {
        KomeManifest::new_app(
            "Example".to_string(),
            "com.example.app".to_string(),
            "Example Developer".to_string(),
        )
    }

    #[test]
    fn one_developer_is_selected_automatically() {
        let memberships = vec![membership("019f9e5ac6687902b0e72fe53abfbef1")];
        let selected = resolve(
            &manifest(),
            &Preferences::default(),
            &memberships,
            &mut Cursor::new(Vec::<u8>::new()),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(selected, "019f9e5ac6687902b0e72fe53abfbef1");
    }

    #[test]
    fn project_developer_precedes_user_default() {
        let first = "019f9e5ac6687902b0e72fe53abfbef1";
        let second = "019f9e5ac6687902b0e72fe53abfbef2";
        let mut manifest = manifest();
        manifest.developer = Some(crate::manifest::Developer {
            id: first.to_string(),
        });
        let preferences = Preferences {
            default_developer: Some(second.to_string()),
        };
        let selected = resolve(
            &manifest,
            &preferences,
            &[membership(first), membership(second)],
            &mut Cursor::new(Vec::<u8>::new()),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(selected, first);
    }

    #[test]
    fn multiple_developers_are_selected_interactively() {
        let first = "019f9e5ac6687902b0e72fe53abfbef1";
        let second = "019f9e5ac6687902b0e72fe53abfbef2";
        let selected = resolve(
            &manifest(),
            &Preferences::default(),
            &[membership(first), membership(second)],
            &mut Cursor::new(b"2\n"),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(selected, second);
    }

    #[test]
    fn no_developer_has_actionable_error() {
        let error = resolve(
            &manifest(),
            &Preferences::default(),
            &[],
            &mut Cursor::new(Vec::<u8>::new()),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("利用可能なDeveloperがありません"));
    }

    #[test]
    fn interactive_selection_rejects_zero() {
        let first = "019f9e5ac6687902b0e72fe53abfbef1";
        assert!(resolve(
            &manifest(),
            &Preferences::default(),
            &[
                membership(first),
                membership("019f9e5ac6687902b0e72fe53abfbef2")
            ],
            &mut Cursor::new(b"0\n"),
            &mut Vec::new(),
        )
        .is_err());
    }
}
