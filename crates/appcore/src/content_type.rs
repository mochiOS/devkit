//! Typed document content identifiers and filename inference.

use std::fmt;
use std::path::Path;

use crate::{Error, Result};

pub const DATA: &str = "application/octet-stream";
pub const PLAIN_TEXT: &str = "text/plain";
pub const UTF8_PLAIN_TEXT: &str = "text/plain;charset=utf-8";
pub const JSON: &str = "application/json";
pub const XML: &str = "application/xml";
pub const HTML: &str = "text/html";
pub const CSS: &str = "text/css";
pub const JAVASCRIPT: &str = "text/javascript";
pub const MARKDOWN: &str = "text/markdown";
pub const PNG: &str = "image/png";
pub const JPEG: &str = "image/jpeg";
pub const SVG: &str = "image/svg+xml";
pub const PDF: &str = "application/pdf";

/// A validated, canonical MIME-style content identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentType(String);

impl ContentType {
    pub fn parse(identifier: impl AsRef<str>) -> Result<Self> {
        let identifier = identifier.as_ref();
        if !valid_identifier(identifier) {
            return Err(Error::InvalidArgument);
        }
        Ok(Self(identifier.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.0
    }

    /// Resolves common document types from a filename extension. Unknown
    /// extensions remain generic data instead of being guessed from contents.
    #[must_use]
    pub fn for_path(path: impl AsRef<Path>) -> Self {
        let extension = path
            .as_ref()
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        Self::from_extension(&extension)
    }

    #[must_use]
    pub fn from_extension(extension: &str) -> Self {
        let identifier = match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "txt" | "text" | "log" => UTF8_PLAIN_TEXT,
            "json" => JSON,
            "xml" => XML,
            "html" | "htm" => HTML,
            "css" => CSS,
            "js" | "mjs" | "cjs" => JAVASCRIPT,
            "md" | "markdown" => MARKDOWN,
            "png" => PNG,
            "jpg" | "jpeg" => JPEG,
            "svg" => SVG,
            "pdf" => PDF,
            _ => DATA,
        };
        Self(identifier.to_owned())
    }

    /// Tests the built-in content hierarchy used for broad document filters.
    #[must_use]
    pub fn conforms_to(&self, parent: &ContentType) -> bool {
        if self == parent || parent.identifier() == DATA {
            return true;
        }
        parent.identifier() == PLAIN_TEXT
            && (self.identifier() == UTF8_PLAIN_TEXT
                || self.identifier().starts_with("text/plain;"))
    }

    #[must_use]
    pub fn preferred_extension(&self) -> Option<&'static str> {
        match self.identifier() {
            PLAIN_TEXT | UTF8_PLAIN_TEXT => Some("txt"),
            JSON => Some("json"),
            XML => Some("xml"),
            HTML => Some("html"),
            CSS => Some("css"),
            JAVASCRIPT => Some("js"),
            MARKDOWN => Some("md"),
            PNG => Some("png"),
            JPEG => Some("jpg"),
            SVG => Some("svg"),
            PDF => Some("pdf"),
            _ => None,
        }
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.identifier())
    }
}

impl AsRef<str> for ContentType {
    fn as_ref(&self) -> &str {
        self.identifier()
    }
}

fn valid_identifier(value: &str) -> bool {
    value.is_ascii()
        && !value.is_empty()
        && value.contains('/')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'+' | b'-' | b'.' | b';' | b'=')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_validates_identifiers() {
        assert_eq!(
            ContentType::parse("Text/Plain").unwrap().identifier(),
            PLAIN_TEXT
        );
        assert_eq!(
            ContentType::parse("plain-text"),
            Err(Error::InvalidArgument)
        );
    }

    #[test]
    fn infers_common_editor_formats() {
        assert_eq!(ContentType::for_path("notes.md").identifier(), MARKDOWN);
        assert_eq!(ContentType::for_path("unknown.bin").identifier(), DATA);
    }

    #[test]
    fn utf8_text_conforms_to_plain_text_and_data() {
        let utf8 = ContentType::parse(UTF8_PLAIN_TEXT).unwrap();
        assert!(utf8.conforms_to(&ContentType::parse(PLAIN_TEXT).unwrap()));
        assert!(utf8.conforms_to(&ContentType::parse(DATA).unwrap()));
    }
}
