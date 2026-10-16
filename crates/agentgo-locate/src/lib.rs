//! Locator strategies (inventory section: Locators, grounding, target
//! resolution).
//!
//! Milestone 0 scope: the strategy taxonomy and its stable textual
//! encoding. Resolution against live pages (semantic first, coordinates
//! with fresh hit-test fallback, ambiguity stops) lands in Milestone 5.

use serde::{Deserialize, Serialize};

/// Locator strategy kinds (inventory `locate.*` family).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Locator {
    /// `role=button`
    Role(String),
    /// Accessible name.
    Name(String),
    /// Label text.
    Label(String),
    /// Placeholder text.
    Placeholder(String),
    /// Exact text.
    Text(String),
    /// Text pattern (regex, bounded).
    TextPattern(String),
    /// Link text.
    LinkText(String),
    /// data-testid attribute.
    TestId(String),
    /// CSS selector (bounded complexity).
    Css(String),
    /// XPath expression (bounded complexity).
    XPath(String),
    /// Opaque stable element reference from an observation.
    ElementRef(String),
    /// Absolute page coordinates (requires fresh hit-test).
    Coordinate { x: i32, y: i32 },
}

/// Errors of locator parsing/validation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LocateError {
    /// The textual locator is malformed.
    #[error("malformed locator: {0}")]
    Malformed(String),
}

impl Locator {
    /// Parses the compact textual form `kind:value` (and `kind:x,y` for
    /// coordinates) used by the CLI and skill examples.
    pub fn parse(text: &str) -> Result<Self, LocateError> {
        let Some((kind, value)) = text.split_once(':') else {
            return Err(LocateError::Malformed(format!(
                "expected `kind:value`, got {text:?}"
            )));
        };
        Ok(match kind {
            "role" => Self::Role(value.into()),
            "name" => Self::Name(value.into()),
            "label" => Self::Label(value.into()),
            "placeholder" => Self::Placeholder(value.into()),
            "text" => Self::Text(value.into()),
            "textPattern" => Self::TextPattern(value.into()),
            "link" => Self::LinkText(value.into()),
            "testId" => Self::TestId(value.into()),
            "css" => Self::Css(value.into()),
            "xpath" => Self::XPath(value.into()),
            "ref" => Self::ElementRef(value.into()),
            "coord" => {
                let (x, y) = value.split_once(',').ok_or_else(|| {
                    LocateError::Malformed(format!("coord expects `x,y`, got {value:?}"))
                })?;
                Self::Coordinate {
                    x: x.trim()
                        .parse()
                        .map_err(|_| LocateError::Malformed(format!("bad x in {value:?}")))?,
                    y: y.trim()
                        .parse()
                        .map_err(|_| LocateError::Malformed(format!("bad y in {value:?}")))?,
                }
            }
            other => return Err(LocateError::Malformed(format!("unknown kind {other:?}"))),
        })
    }

    /// Emits the compact textual form.
    pub fn to_text(&self) -> String {
        match self {
            Self::Role(v) => format!("role:{v}"),
            Self::Name(v) => format!("name:{v}"),
            Self::Label(v) => format!("label:{v}"),
            Self::Placeholder(v) => format!("placeholder:{v}"),
            Self::Text(v) => format!("text:{v}"),
            Self::TextPattern(v) => format!("textPattern:{v}"),
            Self::LinkText(v) => format!("link:{v}"),
            Self::TestId(v) => format!("testId:{v}"),
            Self::Css(v) => format!("css:{v}"),
            Self::XPath(v) => format!("xpath:{v}"),
            Self::ElementRef(v) => format!("ref:{v}"),
            Self::Coordinate { x, y } => format!("coord:{x},{y}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_emit_roundtrip() {
        for l in [
            Locator::Role("button".into()),
            Locator::Name("Sign in".into()),
            Locator::TextPattern("^Add to cart".into()),
            Locator::TestId("checkout-button".into()),
            Locator::Css("#submit".into()),
            Locator::ElementRef("e-4f2a-12".into()),
            Locator::Coordinate { x: 12, y: -40 },
        ] {
            let text = l.to_text();
            let back = Locator::parse(&text).unwrap();
            assert_eq!(l, back, "roundtrip failed for {text}");
        }
    }

    #[test]
    fn malformed_inputs_rejected() {
        assert!(matches!(
            Locator::parse("nope"),
            Err(LocateError::Malformed(_))
        ));
        assert!(matches!(
            Locator::parse("bogus:x"),
            Err(LocateError::Malformed(_))
        ));
        assert!(matches!(
            Locator::parse("coord:x,y"),
            Err(LocateError::Malformed(_))
        ));
        assert!(matches!(
            Locator::parse("coord:5"),
            Err(LocateError::Malformed(_))
        ));
    }
}
