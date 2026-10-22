//! Registration shared by system and application status items.
//!
//! Applications declare persistent entries in `application.control_center_items`
//! and publish their live card through `ControlCenterItem`. The shell registers
//! both those declarations and its system providers in this registry. Placement
//! belongs to the user: a provider cannot override an existing preference.
use crate::{Error, Result};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusItemPlacement {
    Hidden,
    #[default]
    Menu,
    Pinned,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemStatusItem {
    InputMethod,
    Notifications,
    Battery,
    Network,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationStatusAction {
    OpenApplication,
    ShowCard,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusItemAction {
    System(SystemStatusItem),
    OpenApplication { bundle_id: String },
    ShowCard { bundle_id: String, item_id: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    id: String,
    title: String,
    symbol: String,
    source: String,
    action: StatusItemAction,
    default_placement: StatusItemPlacement,
}
impl StatusItem {
    pub fn application(
        bundle_id: &str,
        item_id: &str,
        title: &str,
        symbol: &str,
        action: ApplicationStatusAction,
    ) -> Result<Self> {
        if bundle_id.is_empty()
            || bundle_id.len() > 255
            || !bundle_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            || item_id.is_empty()
            || item_id.len() > 64
            || !item_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::InvalidArgument);
        }
        let action = match action {
            ApplicationStatusAction::OpenApplication => StatusItemAction::OpenApplication {
                bundle_id: bundle_id.into(),
            },
            ApplicationStatusAction::ShowCard => StatusItemAction::ShowCard {
                bundle_id: bundle_id.into(),
                item_id: item_id.into(),
            },
        };
        let item = Self {
            id: format!("app.{bundle_id}:{item_id}"),
            title: title.into(),
            symbol: symbol.into(),
            source: bundle_id.into(),
            action,
            default_placement: StatusItemPlacement::Menu,
        };
        item.validate()?;
        Ok(item)
    }
    /// Registers shell-owned metadata; this does not grant a system capability.
    pub fn system(kind: SystemStatusItem, title: &str, symbol: &str) -> Result<Self> {
        let name = match kind {
            SystemStatusItem::InputMethod => "input",
            SystemStatusItem::Notifications => "notifications",
            SystemStatusItem::Battery => "battery",
            SystemStatusItem::Network => "network",
        };
        let item = Self {
            id: format!("builtin.{name}"),
            title: title.into(),
            symbol: symbol.into(),
            source: "mochiOS".into(),
            action: StatusItemAction::System(kind),
            default_placement: StatusItemPlacement::Pinned,
        };
        item.validate()?;
        Ok(item)
    }
    fn validate(&self) -> Result<()> {
        if self.title.is_empty()
            || self.title.len() > 96
            || self.title.chars().any(char::is_control)
            || self.symbol.is_empty()
            || self.symbol.len() > 96
            || !self
                .symbol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(Error::InvalidArgument);
        }
        Ok(())
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    /// A VK Symbols asset name; the renderer resolves the installed asset.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn action(&self) -> &StatusItemAction {
        &self.action
    }
    pub fn default_placement(&self) -> StatusItemPlacement {
        self.default_placement
    }
}
#[derive(Default)]
pub struct StatusItemRegistry {
    items: BTreeMap<String, StatusItem>,
    order: Vec<String>,
}
impl StatusItemRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    /// Updates metadata under a stable ID without changing registration order.
    pub fn register(&mut self, item: StatusItem) -> Result<()> {
        item.validate()?;
        if !self.items.contains_key(item.id()) {
            if self.order.len() >= 256 {
                return Err(Error::InvalidArgument);
            }
            self.order.push(item.id.clone());
        }
        self.items.insert(item.id.clone(), item);
        Ok(())
    }
    pub fn items(&self) -> impl Iterator<Item = &StatusItem> {
        self.order.iter().filter_map(|id| self.items.get(id))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_and_application_items_share_registration_without_id_collisions() {
        let mut registry = StatusItemRegistry::new();
        registry
            .register(StatusItem::system(SystemStatusItem::Network, "Wi-Fi", "wifi").unwrap())
            .unwrap();
        let app = || {
            StatusItem::application(
                "org.example.app",
                "network",
                "Network",
                "wifi",
                ApplicationStatusAction::ShowCard,
            )
            .unwrap()
        };
        registry.register(app()).unwrap();
        registry.register(app()).unwrap();
        let items = registry.items().collect::<Vec<_>>();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id(), "builtin.network");
        assert_eq!(items[0].default_placement(), StatusItemPlacement::Pinned);
        assert_eq!(items[1].id(), "app.org.example.app:network");
        assert_eq!(items[1].default_placement(), StatusItemPlacement::Menu);
    }
    #[test]
    fn invalid_registration_is_rejected() {
        for id in ["", "../escape", "UPPER"] {
            assert!(
                StatusItem::application(
                    "org.example",
                    id,
                    "Status",
                    "info",
                    ApplicationStatusAction::ShowCard
                )
                .is_err()
            );
        }
        assert!(
            StatusItem::application(
                "org.example",
                "status",
                "Bad\nTitle",
                "info",
                ApplicationStatusAction::ShowCard
            )
            .is_err()
        );
        assert!(StatusItem::system(SystemStatusItem::Network, "Wi-Fi", "../wifi").is_err());
    }
}
