//! Capability-manifest generation and validation.
//!
//! `docs/capability-inventory.md` is the command-level source of truth.
//! This module parses it and emits `schemas/capability-manifest.json`
//! with owner, priority, risk, status, test id, and skill reference for
//! every capability. `agentgo manifest validate` fails CI when the
//! manifest is stale, tampered, or inconsistent — keeping implementation,
//! schemas, tests, documentation, manifest, and skill synchronized is a
//! hard rule of this repository.

use std::collections::BTreeSet;
use std::path::Path;

use agentgo_artifacts::sha256_hex;
use agentgo_policy::{ActionRisk, RiskClassifier};
use agentgo_protocol::{PROTOCOL_VERSION, SKILL_VERSION};
use agentgo_shell_bridge::{MethodStatus, ShellMethodRegistry};
use serde::{Deserialize, Serialize};

/// Failures of parsing/generation/validation.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The inventory markdown is structurally broken.
    #[error("inventory parse error at line {line}: {detail}")]
    Parse {
        /// Line number (1-based).
        line: usize,
        /// What went wrong.
        detail: String,
    },
    /// The manifest on disk is out of sync with the inventory.
    #[error("manifest is stale: regenerate with `agentgo manifest generate` ({detail})")]
    Stale {
        /// Why it is stale.
        detail: String,
    },
    /// The manifest is internally inconsistent.
    #[error("manifest invalid: {0}")]
    Invalid(String),
}

/// Ownership values (inventory ownership legend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Owner {
    /// CEF/Chromium or its process/lifecycle layer.
    Runtime,
    /// Rust/C++ host, Windows APIs, filesystem, OS UI, devices.
    Native,
    /// AgentGo semantic model, workflows, skills, policy, token control.
    Agent,
    /// AgentGo's HTML/CSS browser UI.
    Shell,
}

/// Priority values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    /// Required for the first useful browser.
    P0,
    /// Human parity / daily use.
    P1,
    /// Advanced.
    P2,
}

impl Priority {
    /// Parses the inventory's `### P0` headers, including descriptive
    /// variants such as `### P0 mouse/pointer`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.split_whitespace().next() {
            Some("P0") => Some(Self::P0),
            Some("P1") => Some(Self::P1),
            Some("P2") => Some(Self::P2),
            _ => None,
        }
    }
}

/// One capability entry in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    /// Inventory capability id, e.g. `browser.launch`.
    pub id: String,
    /// Description from the inventory (trimmed, one line).
    pub description: String,
    /// P0/P1/P2 from the inventory section.
    pub priority: Priority,
    /// Owning layer.
    pub owner: Owner,
    /// Risk classification (policy crate).
    pub risk: ActionRisk,
    /// Honest status: `planned` until implemented and tested.
    pub status: String,
    /// Test id once a test exists; null while planned.
    pub test_id: Option<String>,
    /// Reference into the bundled skill.
    pub skill_ref: String,
    /// Primary target milestone.
    pub milestone: u8,
}

/// One inventory section and its capabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSection {
    /// Section number from the inventory.
    pub number: u32,
    /// Section title.
    pub title: String,
    /// Owning layer for the section.
    pub owner: Owner,
    /// Primary milestone for the section.
    pub milestone: u8,
    /// Capabilities in inventory order.
    pub capabilities: Vec<ManifestEntry>,
}

/// The generated manifest document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Manifest shape version.
    pub manifest_version: u32,
    /// Source inventory path (repo-relative).
    pub source: String,
    /// SHA-256 of the source inventory at generation time.
    pub source_sha256: String,
    /// AgentGo version.
    pub agentgo_version: String,
    /// Protocol version (must match agentgo-protocol).
    pub protocol_version: String,
    /// Skill version (must match agentgo-protocol).
    pub skill_version: String,
    /// Capability totals by priority.
    pub counts: Counts,
    /// Sections in inventory order.
    pub sections: Vec<ManifestSection>,
}

/// Capability counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Total capabilities.
    pub total: usize,
    /// P0 capabilities.
    pub p0: usize,
    /// P1 capabilities.
    pub p1: usize,
    /// P2 capabilities.
    pub p2: usize,
}

/// Section-number -> primary owner. Recorded from the inventory's
/// ownership legend; adjustments must be justified in the commit that
/// changes them.
pub const OWNER_BY_SECTION: &[(u32, Owner)] = &[
    (1, Owner::Runtime),  // Browser runtime and process lifecycle
    (2, Owner::Native),   // Profiles, user contexts, session state
    (3, Owner::Native),   // Windows, browser windows, viewport control
    (4, Owner::Runtime),  // Tabs, frames, browsing contexts, popups
    (5, Owner::Runtime),  // Navigation, loading, synchronization
    (6, Owner::Agent),    // Mouse, pointer, touch, gesture input
    (7, Owner::Agent),    // Clipboard, selection, focus, editing
    (8, Owner::Agent),    // Semantic observation and visual grounding
    (9, Owner::Agent),    // Locators, grounding, target resolution
    (10, Owner::Agent),   // Page actions and form operations
    (11, Owner::Runtime), // JavaScript, DOM, page-runtime control
    (12, Owner::Native),  // Files, native dialogs, downloads, printing
    (13, Owner::Runtime), // Browser dialogs, menus, prompts, permissions
    (14, Owner::Native),  // Storage, cookies, credentials, site data
    (15, Owner::Runtime), // Network, requests, responses, events
    (16, Owner::Runtime), // Media, camera, microphone, sensors, devices
    (17, Owner::Shell),   // Human browser chrome and product features
    (18, Owner::Runtime), // Extensions, scripts, plugins
    (19, Owner::Agent),   // One-call execution and workflow control
    (20, Owner::Agent),   // Batch execution, synchronization, recovery
    (21, Owner::Agent),   // Research, extraction, data workflows
    (22, Owner::Agent),   // Skills, memory, replay, domain adapters
    (23, Owner::Agent),   // Approval modes, risk policy, human handoff
    (24, Owner::Shell),   // Human handoff UI and shell functions
    (25, Owner::Agent),   // Artifacts, evidence, context control
    (26, Owner::Native),  // Native Windows control lane
    (27, Owner::Agent),   // Accessibility and inclusive interaction
    (28, Owner::Runtime), // Debugging, inspection, diagnostics
    (29, Owner::Agent),   // Metrics and evaluation
    (30, Owner::Agent),   // Agent control protocol and integration surface
    (31, Owner::Agent),   // Security and trust boundaries
    (32, Owner::Agent),   // Suggested first implementation slices
];

/// Section-number -> primary milestone (from the master plan milestones).
pub const PRIORITY_MILESTONE_BY_SECTION: &[(u32, u8)] = &[
    (1, 1),
    (2, 2),
    (3, 2),
    (4, 2),
    (5, 2),
    (6, 4),
    (7, 4),
    (8, 5),
    (9, 5),
    (10, 4),
    (11, 4),
    (12, 8),
    (13, 4),
    (14, 7),
    (15, 5),
    (16, 4),
    (17, 2),
    (18, 10),
    (19, 6),
    (20, 6),
    (21, 6),
    (22, 9),
    (23, 7),
    (24, 2),
    (25, 5),
    (26, 8),
    (27, 5),
    (28, 10),
    (29, 11),
    (30, 3),
    (31, 7),
    (32, 2),
];

fn owner_for_section(number: u32) -> Result<Owner, ManifestError> {
    OWNER_BY_SECTION
        .iter()
        .find(|(n, _)| *n == number)
        .map(|(_, o)| *o)
        .ok_or_else(|| ManifestError::Invalid(format!("no owner mapping for section {number}")))
}

fn milestone_for_section(number: u32) -> Result<u8, ManifestError> {
    PRIORITY_MILESTONE_BY_SECTION
        .iter()
        .find(|(n, _)| *n == number)
        .map(|(_, m)| *m)
        .ok_or_else(|| ManifestError::Invalid(format!("no milestone mapping for section {number}")))
}

/// Parses the inventory markdown into a [`Manifest`].
pub fn parse_inventory(inventory_text: &str) -> Result<Manifest, ManifestError> {
    let classifier = RiskClassifier;
    let registry = ShellMethodRegistry::new();
    let mut sections: Vec<ManifestSection> = Vec::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();
    let mut current_section: Option<ManifestSection> = None;
    let mut current_priority: Option<Priority> = None;

    for (idx, raw) in inventory_text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("## ") {
            let rest = rest.trim();
            // Numbered sections start a manifest section; unnumbered
            // headers (Purpose, legends, decisions) are document metadata
            // and are skipped.
            if let Some((number, title)) = rest.split_once(". ") {
                if let Ok(number) = number.parse::<u32>() {
                    if let Some(section) = current_section.take() {
                        sections.push(section);
                    }
                    current_section = Some(ManifestSection {
                        number,
                        title: title.trim().to_string(),
                        owner: owner_for_section(number)?,
                        milestone: milestone_for_section(number)?,
                        capabilities: Vec::new(),
                    });
                    current_priority = None;
                }
            }
        } else if let Some(rest) = line.strip_prefix("### ") {
            let rest = rest.trim();
            // Only P0/P1/P2/Safety groups matter; other subheaders
            // (e.g. "Universal control surface") are document metadata.
            if let Some(p) = Priority::parse(rest) {
                current_priority = Some(p);
            } else if rest == "Safety" {
                current_priority = None;
            }
        } else if let Some(rest) = line.strip_prefix("- `") {
            // Bullets with backticked words appear in the document's
            // metadata ("Decisions already made" etc.). Genuine capability
            // entries always sit inside a numbered section under a
            // P0/P1/P2 group; anything else is prose and is skipped.
            let Some((id, after_id)) = rest.split_once('`') else {
                continue;
            };
            let (Some(priority), Some(section)) = (current_priority, current_section.as_mut())
            else {
                continue;
            };
            let description = after_id
                .trim_start()
                .trim_start_matches('\u{2014}')
                .trim_start_matches('-')
                .trim()
                .to_string();
            if !seen_ids.insert(id.to_string()) {
                return Err(ManifestError::Parse {
                    line: line_no,
                    detail: format!("duplicate capability id `{id}`"),
                });
            }
            let owner = section.owner;
            let milestone = section.milestone;
            let (status, test_id) = match registry.lookup(id) {
                Some(method) if method.status == MethodStatus::Implemented => {
                    ("implemented".to_string(), Some(method.test.to_string()))
                }
                _ => ("planned".to_string(), None),
            };
            section.capabilities.push(ManifestEntry {
                id: id.to_string(),
                description,
                priority,
                owner,
                risk: classifier.classify(id),
                status,
                test_id,
                skill_ref: format!("skills/agentgo/SKILL.md#capability-{id}"),
                milestone,
            });
        }
    }
    if let Some(section) = current_section.take() {
        sections.push(section);
    }
    if sections.is_empty() {
        return Err(ManifestError::Parse {
            line: 1,
            detail: "no sections found".into(),
        });
    }

    let mut counts = Counts {
        total: 0,
        p0: 0,
        p1: 0,
        p2: 0,
    };
    for s in &sections {
        for c in &s.capabilities {
            counts.total += 1;
            match c.priority {
                Priority::P0 => counts.p0 += 1,
                Priority::P1 => counts.p1 += 1,
                Priority::P2 => counts.p2 += 1,
            }
        }
    }

    Ok(Manifest {
        manifest_version: 1,
        source: "docs/capability-inventory.md".into(),
        // Hashed with LF line endings, so a Windows checkout (autocrlf)
        // and a Unix one agree.
        source_sha256: sha256_hex(inventory_text.replace("\r\n", "\n").as_bytes()),
        agentgo_version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: PROTOCOL_VERSION.into(),
        skill_version: SKILL_VERSION.into(),
        counts,
        sections,
    })
}

/// Generates the manifest JSON from the inventory at `inventory_path`.
pub fn generate_manifest(inventory_path: &Path) -> Result<String, ManifestError> {
    let text = std::fs::read_to_string(inventory_path)?;
    let manifest = parse_inventory(&text)?;
    serde_json::to_string_pretty(&manifest)
        .map_err(|e| ManifestError::Invalid(format!("serialization failed: {e}")))
}

/// Validates the manifest at `manifest_path` against the inventory at
/// `inventory_path` plus internal consistency rules.
pub fn validate_manifest(
    manifest_path: &Path,
    inventory_path: &Path,
) -> Result<Manifest, ManifestError> {
    let manifest_text = std::fs::read_to_string(manifest_path)?;
    let inventory_text = std::fs::read_to_string(inventory_path)?;
    let manifest: Manifest = serde_json::from_str(&manifest_text)
        .map_err(|e| ManifestError::Invalid(format!("manifest is not valid JSON: {e}")))?;
    let fresh = parse_inventory(&inventory_text)?;

    // Staleness: the manifest must match a regeneration exactly except
    // for statuses/test ids, which may advance from `planned`/null.
    if manifest.source_sha256 != fresh.source_sha256 {
        return Err(ManifestError::Stale {
            detail: "inventory changed after the manifest was generated".into(),
        });
    }
    if manifest.counts != fresh.counts {
        return Err(ManifestError::Stale {
            detail: format!(
                "counts differ: manifest {:?} vs inventory {:?}",
                manifest.counts, fresh.counts
            ),
        });
    }
    if manifest.protocol_version != fresh.protocol_version {
        return Err(ManifestError::Stale {
            detail: "protocol version drift".into(),
        });
    }
    if manifest.skill_version != fresh.skill_version {
        return Err(ManifestError::Stale {
            detail: "skill version drift".into(),
        });
    }

    // Structural equality of ids/sections.
    let ids = |m: &Manifest| {
        m.sections
            .iter()
            .flat_map(|s| s.capabilities.iter().map(|c| (s.number, c.id.clone())))
            .collect::<Vec<_>>()
    };
    if ids(&manifest) != ids(&fresh) {
        return Err(ManifestError::Stale {
            detail: "capability ids differ from inventory".into(),
        });
    }

    // Internal consistency: entries must carry valid fields.
    let mut seen = BTreeSet::new();
    for s in &manifest.sections {
        for c in &s.capabilities {
            if !seen.insert(c.id.clone()) {
                return Err(ManifestError::Invalid(format!("duplicate id {}", c.id)));
            }
            if !matches!(
                c.status.as_str(),
                "planned" | "implemented" | "partial" | "unsupported"
            ) {
                return Err(ManifestError::Invalid(format!(
                    "bad status {} on {}",
                    c.status, c.id
                )));
            }
            if c.status == "implemented" && c.test_id.is_none() {
                return Err(ManifestError::Invalid(format!(
                    "{} is implemented but has no test id",
                    c.id
                )));
            }
            if c.status == "implemented" && c.risk != RiskClassifier.classify(&c.id) {
                return Err(ManifestError::Invalid(format!(
                    "{} risk drifted from the classifier",
                    c.id
                )));
            }
            if !c.skill_ref.contains(&c.id) {
                return Err(ManifestError::Invalid(format!(
                    "{} skill ref is wrong",
                    c.id
                )));
            }
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
# Fixture inventory

## 1. Browser runtime and process lifecycle

### P0

- `browser.launch` — start the browser runtime.
- `browser.version` — return versions.

### P1

- `browser.suspend` — suspend background work.

## 2. Windows, browser windows, and viewport control

### P0

- `window.create` — create a window.
";

    #[test]
    fn parses_fixture_into_manifest() {
        let m = parse_inventory(FIXTURE).unwrap();
        assert_eq!(m.sections.len(), 2);
        assert_eq!(m.sections[0].capabilities.len(), 3); // 2x P0 + 1x P1
        assert_eq!(m.sections[1].capabilities.len(), 1);
        assert_eq!(
            m.counts,
            Counts {
                total: 4,
                p0: 3,
                p1: 1,
                p2: 0
            }
        );
        assert_eq!(m.sections[0].owner, Owner::Runtime);
        assert_eq!(m.sections[1].owner, Owner::Native);
        assert_eq!(
            m.sections[0].capabilities[0].description,
            "start the browser runtime."
        );
        // The status comes from the live registry: browser.launch is built.
        assert_eq!(m.sections[0].capabilities[0].status, "implemented");
        assert!(m.sections[0].capabilities[0]
            .skill_ref
            .contains("browser.launch"));
    }

    #[test]
    fn risk_comes_from_policy_classifier() {
        let m = parse_inventory(FIXTURE).unwrap();
        assert_eq!(m.sections[0].capabilities[1].risk, ActionRisk::Benign);
        assert_eq!(m.sections[1].capabilities[0].risk, ActionRisk::Sensitive);
    }

    #[test]
    fn parses_descriptive_priority_headers() {
        let inventory = FIXTURE.replace("### P0\n", "### P0 mouse/pointer\n");
        let manifest = parse_inventory(&inventory).unwrap();
        assert_eq!(manifest.counts.total, 4);
        assert_eq!(manifest.counts.p0, 3);
    }

    #[test]
    fn duplicate_ids_rejected() {
        let bad = FIXTURE.to_string() + "\n- `browser.launch` — duplicate.";
        assert!(matches!(
            parse_inventory(&bad),
            Err(ManifestError::Parse { .. })
        ));
    }

    #[test]
    fn capability_before_section_rejected() {
        let bad = "- `orphan.capability` — no section.";
        assert!(matches!(
            parse_inventory(bad),
            Err(ManifestError::Parse { .. })
        ));
    }

    #[test]
    fn generation_is_deterministic() {
        let a = parse_inventory(FIXTURE).unwrap();
        let b = parse_inventory(FIXTURE).unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&a).unwrap(),
            serde_json::to_string_pretty(&b).unwrap()
        );
    }

    #[test]
    fn validate_catches_removal() {
        let dir =
            std::env::temp_dir().join(format!("agentgo-manifest-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inv = dir.join("inv.md");
        let man = dir.join("manifest.json");
        std::fs::write(&inv, FIXTURE).unwrap();
        let generated = generate_manifest(&inv).unwrap();
        std::fs::write(&man, &generated).unwrap();
        validate_manifest(&man, &inv).unwrap();
        // Tamper: drop one capability.
        let mut doc: serde_json::Value = serde_json::from_str(&generated).unwrap();
        doc["sections"][0]["capabilities"]
            .as_array_mut()
            .unwrap()
            .pop();
        std::fs::write(&man, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        match validate_manifest(&man, &inv) {
            Err(ManifestError::Stale { .. }) => {}
            other => panic!("expected Stale, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
