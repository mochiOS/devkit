//! Document opening and default-application associations.
//!
//! Association reads/writes require `file-association.read` and
//! `file-association.write`. Opening is delegated to `workspace.service`, which
//! validates the selected installed application and its capabilities.

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct AssociationRoles(u16);

impl AssociationRoles {
    pub const VIEW: Self = Self(1 << 0);
    pub const EDIT: Self = Self(1 << 1);
    pub const ALL: Self = Self(Self::VIEW.0 | Self::EDIT.0);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits != 0 && bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssociationHandler {
    pub bundle_id: String,
    pub name: String,
}

fn validate_key(extension: &str, content_type: &str) -> Result<()> {
    if extension.is_empty() && content_type.is_empty() {
        Err(Error::InvalidArgument)
    } else {
        Ok(())
    }
}

pub fn set_default(
    extension: &str,
    content_type: &str,
    bundle_id: &str,
    roles: AssociationRoles,
) -> Result<()> {
    validate_key(extension, content_type)?;
    if bundle_id.is_empty() {
        return Err(Error::InvalidArgument);
    }
    platform::set_default(extension, content_type, bundle_id, roles.bits())
}

pub fn remove_default(extension: &str, content_type: &str, roles: AssociationRoles) -> Result<()> {
    validate_key(extension, content_type)?;
    platform::remove_default(extension, content_type, roles.bits())
}

pub fn resolve_default(
    extension: &str,
    content_type: &str,
    roles: AssociationRoles,
) -> Result<String> {
    validate_key(extension, content_type)?;
    platform::resolve_default(extension, content_type, roles.bits())
}

pub fn handlers(
    extension: &str,
    content_type: &str,
    roles: AssociationRoles,
) -> Result<Vec<AssociationHandler>> {
    validate_key(extension, content_type)?;
    platform::handlers(extension, content_type, roles.bits())
}

pub fn open(path: &str, content_type: &str, roles: AssociationRoles) -> Result<u64> {
    if path.is_empty() || content_type.is_empty() {
        return Err(Error::InvalidArgument);
    }
    platform::open(path, content_type, "", roles.bits())
}

pub fn open_with(
    path: &str,
    content_type: &str,
    bundle_id: &str,
    roles: AssociationRoles,
) -> Result<u64> {
    if path.is_empty() || content_type.is_empty() || bundle_id.is_empty() {
        return Err(Error::InvalidArgument);
    }
    platform::open(path, content_type, bundle_id, roles.bits())
}

#[cfg(target_os = "mochios")]
mod platform {
    use super::AssociationHandler;
    use crate::Result;

    pub fn set_default(
        extension: &str,
        content_type: &str,
        bundle_id: &str,
        roles: u16,
    ) -> Result<()> {
        mochi_user_platform::workspace::set_association(extension, content_type, bundle_id, roles)
            .map_err(Into::into)
    }
    pub fn remove_default(extension: &str, content_type: &str, roles: u16) -> Result<()> {
        mochi_user_platform::workspace::remove_association(extension, content_type, roles)
            .map_err(Into::into)
    }
    pub fn resolve_default(extension: &str, content_type: &str, roles: u16) -> Result<String> {
        mochi_user_platform::workspace::resolve_association(extension, content_type, roles)
            .map_err(Into::into)
    }
    pub fn handlers(
        extension: &str,
        content_type: &str,
        roles: u16,
    ) -> Result<Vec<AssociationHandler>> {
        mochi_user_platform::workspace::association_handlers(extension, content_type, roles)
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| AssociationHandler {
                        bundle_id: item.bundle_id,
                        name: item.name,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }
    pub fn open(path: &str, content_type: &str, bundle_id: &str, roles: u16) -> Result<u64> {
        mochi_user_platform::workspace::open_document_with(path, content_type, bundle_id, roles)
            .map_err(Into::into)
    }
}

#[cfg(not(target_os = "mochios"))]
mod platform {
    use super::AssociationHandler;
    use crate::{Error, Result};

    pub fn set_default(_: &str, _: &str, _: &str, _: u16) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn remove_default(_: &str, _: &str, _: u16) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn resolve_default(_: &str, _: &str, _: u16) -> Result<String> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn handlers(_: &str, _: &str, _: u16) -> Result<Vec<AssociationHandler>> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn open(_: &str, _: &str, _: &str, _: u16) -> Result<u64> {
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn association_roles_reject_unknown_and_empty_bits() {
        assert_eq!(AssociationRoles::from_bits(0), None);
        assert_eq!(AssociationRoles::from_bits(4), None);
        assert_eq!(AssociationRoles::from_bits(3), Some(AssociationRoles::ALL));
    }

    #[test]
    fn association_key_requires_extension_or_content_type() {
        assert_eq!(
            resolve_default("", "", AssociationRoles::VIEW),
            Err(Error::InvalidArgument)
        );
    }
}
