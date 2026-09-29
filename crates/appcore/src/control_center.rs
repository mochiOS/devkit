//! Declarative cards rendered inside the system Control Center.
//!
//! Applications publish data, not executable UI code, so a faulty provider
//! cannot crash Binder. Binder renders every card with the standard ViewKit
//! controls and applies the user's ordering and visibility preferences.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlCenterCardRow {
    label: String,
    value: String,
}

impl ControlCenterCardRow {
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlCenterCard {
    title: String,
    rows: Vec<ControlCenterCardRow>,
}

impl ControlCenterCard {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    #[must_use]
    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.rows.push(ControlCenterCardRow::new(label, value));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlCenterItem {
    bundle_id: String,
    item_id: String,
    card: Option<ControlCenterCard>,
}

impl ControlCenterItem {
    /// Begins runtime registration for an item declared in `manifest.toml`.
    #[must_use]
    pub fn register(bundle_id: impl Into<String>, item_id: impl Into<String>) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            item_id: item_id.into(),
            card: None,
        }
    }

    #[must_use]
    pub fn card(mut self, card: ControlCenterCard) -> Self {
        self.card = Some(card);
        self
    }

    /// Publishes or replaces the card. Re-register to update its values.
    pub fn publish(self) -> Result<()> {
        let Some(card) = self.card else {
            return Err(Error::InvalidArgument);
        };
        if self.bundle_id.is_empty()
            || self.item_id.is_empty()
            || card.title.is_empty()
            || self.bundle_id.len() > 255
            || !self.bundle_id.is_ascii()
            || !self
                .bundle_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || self.item_id.len() > 64
            || !self
                .item_id
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
            || card.title.chars().any(char::is_control)
            || card.title.len() > 96
            || card.rows.is_empty()
            || card.rows.len() > 4
            || card
                .rows
                .iter()
                .map(|row| row.label.len() + 1 + row.value.len())
                .sum::<usize>()
                .saturating_add(card.rows.len().saturating_sub(1))
                > 2048
            || card.rows.iter().any(|row| {
                row.label.is_empty()
                    || row.value.is_empty()
                    || row.label.chars().any(char::is_control)
                    || row.value.chars().any(char::is_control)
            })
        {
            return Err(Error::InvalidArgument);
        }

        #[cfg(target_os = "mochios")]
        {
            let card = mochi_user_platform::workspace::ControlCenterCard {
                bundle_id: self.bundle_id,
                item_id: self.item_id,
                title: card.title,
                rows: card
                    .rows
                    .into_iter()
                    .map(|row| mochi_user_platform::workspace::ControlCenterCardRow {
                        label: row.label,
                        value: row.value,
                    })
                    .collect(),
            };
            mochi_user_platform::workspace::register_control_center_card(&card)?;
            Ok(())
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_requires_a_non_empty_card() {
        assert_eq!(
            ControlCenterItem::register("org.mochios.example", "status").publish(),
            Err(Error::InvalidArgument)
        );
        assert_eq!(
            ControlCenterItem::register("org.mochios.example", "status")
                .card(ControlCenterCard::new("Status"))
                .publish(),
            Err(Error::InvalidArgument)
        );
    }
}
