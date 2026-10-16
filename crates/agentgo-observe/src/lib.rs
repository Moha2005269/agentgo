//! Compact semantic observation model (master plan section 11).
//!
//! One internal page model emits **views** instead of repeated dumps.
//! Milestone 0 scope: the view taxonomy, page outline record, and the
//! stable element-reference format with revision counters. CEF/CDP/AX
//! extraction lands in Milestone 5.

use serde::{Deserialize, Serialize};

/// The observation views an agent can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewKind {
    /// URL, title, state, major regions, headings, dialogs, frames.
    Outline,
    /// Roles, names, labels, states, bounds, allowed actions, refs.
    Actionables,
    /// Cleaned article/list/table/result records.
    Content,
    /// Fields, requirements, values, validation, submit targets.
    Forms,
    /// Only revisions since a supplied baseline.
    ChangeSet,
    /// Screenshot or targeted crop (only when semantics are insufficient).
    Visual,
    /// Bounded, explicitly requested DOM/HTML.
    RawDom,
    /// Network-derived structured data (permitted and redacted only).
    NetworkDerived,
}

/// Stable page outline record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageOutline {
    /// Final URL after redirects.
    pub url: String,
    /// Document title.
    pub title: String,
    /// Coarse page lifecycle state.
    pub state: PageState,
    /// Heading outline (level, text).
    pub headings: Vec<Heading>,
    /// Open dialogs/prompts detected on the page.
    pub dialogs: Vec<String>,
    /// Frame count (main + child frames).
    pub frame_count: u32,
    /// Monotonic document generation (bump on navigation).
    pub document_generation: u64,
}

/// Coarse page state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageState {
    /// Navigation in flight.
    Loading,
    /// DOM ready, content settling.
    Interactive,
    /// Stable enough for actions.
    Stable,
    /// Frame crashed or unreachable.
    Crashed,
}

/// One heading entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heading {
    /// 1-6.
    pub level: u8,
    /// Heading text (bounded).
    pub text: String,
}

/// A stable element reference.
///
/// Opaque to callers; internally backed by frame id, document generation,
/// backend DOM/AX identity, semantic fingerprint, and last-known bounds.
/// Validate immediately before action; if stale, re-resolve by
/// fingerprint and report confidence (Milestone 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementRef {
    /// Opaque id, e.g. `e-4f2a-12`.
    pub id: String,
    /// Owning frame id.
    pub frame_id: String,
    /// Document generation the reference was created against.
    pub document_generation: u64,
    /// Semantic fingerprint (role/name/hash).
    pub fingerprint: u64,
}

impl ElementRef {
    /// Whether this reference is still valid for the current document
    /// generation of its frame.
    pub fn is_current_for(&self, frame_id: &str, document_generation: u64) -> bool {
        self.frame_id == frame_id && self.document_generation == document_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_kinds_serde_roundtrip() {
        for k in [
            ViewKind::Outline,
            ViewKind::Actionables,
            ViewKind::Content,
            ViewKind::Forms,
            ViewKind::ChangeSet,
            ViewKind::Visual,
            ViewKind::RawDom,
            ViewKind::NetworkDerived,
        ] {
            let text = serde_json::to_string(&k).unwrap();
            let back: ViewKind = serde_json::from_str(&text).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn outline_roundtrip() {
        let o = PageOutline {
            url: "https://www.google.com/search?q=agentgo".into(),
            title: "agentgo - Google Search".into(),
            state: PageState::Stable,
            headings: vec![Heading {
                level: 1,
                text: "Search results".into(),
            }],
            dialogs: vec![],
            frame_count: 1,
            document_generation: 7,
        };
        let back: PageOutline = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(o, back);
    }

    #[test]
    fn element_ref_validity_tracks_document_generation() {
        let r = ElementRef {
            id: "e-4f2a-12".into(),
            frame_id: "f-1".into(),
            document_generation: 7,
            fingerprint: 0xdead_beef,
        };
        assert!(r.is_current_for("f-1", 7));
        assert!(
            !r.is_current_for("f-1", 8),
            "navigated documents invalidate refs"
        );
        assert!(!r.is_current_for("f-2", 7), "wrong frame invalidates refs");
    }
}
