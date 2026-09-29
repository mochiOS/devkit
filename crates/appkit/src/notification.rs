//! User notifications delivered by the system notification center.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserNotification {
    bundle_id: String,
    title: String,
    body: String,
}

impl UserNotification {
    #[must_use]
    pub fn new(bundle_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            title: title.into(),
            body: String::new(),
        }
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn deliver(self) -> Result<u64> {
        if self.bundle_id.is_empty()
            || self.bundle_id.len() > 255
            || !self.bundle_id.is_ascii()
            || !self
                .bundle_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || self.title.is_empty()
            || self.title.len() > 128
            || self.title.chars().any(char::is_control)
            || self.body.is_empty()
            || self.body.len() > 1024
            || self
                .body
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(Error::InvalidArgument);
        }

        #[cfg(target_os = "mochios")]
        {
            Ok(mochi_user_platform::workspace::post_notification(
                &self.bundle_id,
                &self.title,
                &self.body,
            )?)
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
    fn rejects_empty_and_control_text() {
        assert_eq!(
            UserNotification::new("org.mochios.example", "")
                .body("Done")
                .deliver(),
            Err(Error::InvalidArgument)
        );
        assert_eq!(
            UserNotification::new("org.mochios.example", "Done")
                .body("bad\0body")
                .deliver(),
            Err(Error::InvalidArgument)
        );
    }
}
