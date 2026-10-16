//! Lockstep enforcement: implementation, schemas, capability manifest,
//! bundled skill, and pin files must stay synchronized. This test is the
//! repository's guardrail for the master plan's synchronization rule.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop(); // crates/agentgo-app
    dir.pop(); // repo root
    dir
}

#[test]
fn manifest_is_in_sync_with_inventory() {
    let root = repo_root();
    let inv = root.join("docs/capability-inventory.md");
    let man = root.join("schemas/capability-manifest.json");
    assert!(inv.exists(), "inventory missing at {}", inv.display());
    assert!(man.exists(), "manifest missing at {}", man.display());
    match agentgo_app::validate_manifest(&man, &inv) {
        Ok(m) => {
            // The real inventory must be substantial (all 32 sections).
            assert!(
                m.sections.len() >= 32,
                "expected >= 32 sections, got {}",
                m.sections.len()
            );
            assert!(
                m.counts.p0 >= 400,
                "expected a large P0 surface, got {}",
                m.counts.p0
            );
            // Implemented entries are derived from the reviewed runtime
            // registry; every remaining entry stays honestly planned.
            for s in &m.sections {
                for c in &s.capabilities {
                    if c.status == "implemented" {
                        assert_eq!(
                            c.test_id.as_deref(),
                            Some("integration.protocol_smoke"),
                            "{} must point at its integration proof",
                            c.id
                        );
                    } else {
                        assert_eq!(c.status, "planned", "unexpected status on {}", c.id);
                        assert!(c.test_id.is_none(), "{} cannot have a test id yet", c.id);
                    }
                }
            }
        }
        Err(e) => panic!("manifest out of sync: {e}"),
    }
}

#[test]
fn manifest_regenerates_identically() {
    let root = repo_root();
    let inv = root.join("docs/capability-inventory.md");
    let man = root.join("schemas/capability-manifest.json");
    let fresh = agentgo_app::generate_manifest(&inv).expect("inventory parses");
    let on_disk = std::fs::read_to_string(&man).unwrap();
    let fresh_doc: serde_json::Value = serde_json::from_str(&fresh).unwrap();
    let disk_doc: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
    // statuses/test ids may advance over time; everything else must match.
    assert_eq!(fresh_doc["counts"], disk_doc["counts"], "counts drifted");
    assert_eq!(
        fresh_doc["sourceSha256"], disk_doc["sourceSha256"],
        "source hash drifted"
    );
    let strip_status = |mut v: serde_json::Value| -> serde_json::Value {
        for s in v["sections"].as_array_mut().unwrap() {
            for c in s["capabilities"].as_array_mut().unwrap() {
                c["status"] = serde_json::json!("x");
                c["testId"] = serde_json::json!(null);
            }
        }
        v
    };
    assert_eq!(
        strip_status(fresh_doc),
        strip_status(disk_doc),
        "manifest differs from a fresh regeneration beyond status/testId"
    );
}

#[test]
fn pins_json_agrees_with_code_constants() {
    let root = repo_root();
    let pins: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("scripts/pins.json")).unwrap())
            .expect("pins.json parses");
    assert_eq!(pins["cef"]["version"], agentgo_cef::CEF_VERSION);
    assert_eq!(pins["cef"]["chromium"], agentgo_cef::CEF_CHROMIUM_VERSION);
    assert_eq!(pins["cef"]["archiveSha1"], agentgo_cef::CEF_ARCHIVE_SHA1);
    assert_eq!(pins["cef"]["archiveName"], agentgo_cef::CEF_ARCHIVE_NAME);
    assert_eq!(
        pins["cef"]["archiveSizeBytes"].as_u64().unwrap(),
        agentgo_cef::CEF_ARCHIVE_SIZE_BYTES
    );
    assert_eq!(pins["cefRs"]["revision"], agentgo_cef::CEF_RS_REVISION);
    assert_eq!(
        pins["rust"]["toolchain"],
        agentgo_app::Versions::collect().pinned_rust
    );
}

#[test]
fn skill_declares_the_same_versions_and_search_policy() {
    let root = repo_root();
    let skill = std::fs::read_to_string(root.join("skills/agentgo/SKILL.md")).unwrap();
    assert!(
        skill.contains(agentgo_protocol::PROTOCOL_VERSION),
        "skill must declare protocol {}",
        agentgo_protocol::PROTOCOL_VERSION
    );
    assert!(
        skill.contains(agentgo_protocol::SKILL_VERSION),
        "skill must declare its version {}",
        agentgo_protocol::SKILL_VERSION
    );
    assert!(
        skill.contains("https://www.google.com/search"),
        "skill must use Google search"
    );
    assert!(
        !skill.to_lowercase().contains("agentgo search"),
        "no AgentGo Search anywhere"
    );
    // The skill must reference the manifest, keeping the two synchronized.
    assert!(skill.contains("capability-manifest.json"));
}

fn read(root: &std::path::Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

#[test]
fn new_tab_page_has_no_simulated_behavior() {
    let root = repo_root();
    let page = read(&root, "ui/index.html");
    let script = read(&root, "ui/newtab.js");
    let both = format!("{page}{script}");
    // Leftovers of the prototype that faked browser behavior.
    for forbidden in [
        "AgentGo Search",
        "agentgoHost",
        "agentgo.dev/search",
        "view-generic",
        "id=\"loadbar\"",
        "statusbar",
        "btnDelegate",
        "POOL",
        "v1.0.4",
        "localStorage",
    ] {
        assert!(
            !both.contains(forbidden),
            "New Tab page still contains {forbidden}"
        );
    }
    // Search goes to Google, addresses open directly, and live data comes
    // from the read-only bridge method.
    assert!(script.contains("https://www.google.com/search?q="));
    assert!(script.contains("location.assign(url)"));
    assert!(script.contains("method=newtab.state"));
    // No inline script: the page runs under the strict shell CSP.
    assert!(!page.contains("<script>"));
    assert!(page.contains("<script src=\"newtab.js\"></script>"));
}

#[test]
fn landing_shortcuts_are_real_https_navigations() {
    let root = repo_root();
    let script = read(&root, "ui/newtab.js");
    for url in [
        "https://github.com/",
        "https://mail.google.com/",
        "https://www.figma.com/files/",
        "https://maps.google.com/",
        "https://www.youtube.com/",
        "https://news.ycombinator.com/",
        "https://arxiv.org/",
        "https://docs.google.com/",
    ] {
        assert!(script.contains(url), "shortcut missing {url}");
    }
    assert!(script.contains("a.className='scut'; a.href=url;"));
}

#[test]
fn trusted_chrome_and_tools_call_real_commands() {
    let root = repo_root();
    let chrome = read(&root, "ui/chrome.html");
    let chrome_js = read(&root, "ui/chrome.js");
    let tools_js = read(&root, "ui/tools.js");
    for control in [
        "openHistory",
        "openDownloads",
        "openSettings",
        "agentPill",
        "agentPause",
        "agentStop",
        "star",
    ] {
        assert!(
            chrome.contains(&format!("id=\"{control}\"")),
            "chrome must expose {control}"
        );
    }
    // Every command the UI issues exists in the registry as implemented
    // (ui.* are the two private panel commands).
    let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
    for source in [&chrome_js, &tools_js] {
        for call in source.split(['\'', '`']).filter(|part| {
            part.split('.').count() >= 2
                && part.split('.').all(|segment| !segment.is_empty())
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
        }) {
            if call.starts_with("ui.") || call.ends_with(".js") || call.ends_with(".html") {
                continue;
            }
            let method = registry
                .lookup(call)
                .unwrap_or_else(|| panic!("UI calls unregistered command {call}"));
            assert_eq!(
                method.status,
                agentgo_shell_bridge::MethodStatus::Implemented,
                "UI calls planned command {call}"
            );
        }
    }
    // Unknown results are shown as errors, never as fake success on a timer.
    // The only timers are the address bar's suggestion debounce and the delay
    // that lets a click in the suggestion popover land before it hides. Timers\r\n    // tagged ui-timer only clean up an animation or debounce a refresh.
    for line in chrome_js.lines().filter(|line| line.contains("setTimeout")) {
        assert!(
            line.contains("uggest") || line.contains("ui-timer"),
            "chrome.js uses a timer outside the suggestions: {line}"
        );
    }
}

#[test]
fn every_implemented_command_names_an_existing_test() {
    let root = repo_root();
    let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
    for method in registry.implemented() {
        let file = method
            .test
            .strip_prefix("integration.")
            .unwrap_or_else(|| panic!("{} has a malformed test id {}", method.name, method.test));
        let path = root.join("tests/integration").join(format!("{file}.js"));
        assert!(
            path.exists(),
            "{} claims test {} which does not exist",
            method.name,
            method.test
        );
    }
}

#[test]
fn content_tabs_are_stacked_below_the_trusted_chrome() {
    let root = repo_root();
    let app = read(&root, "crates/agentgo-cef/src/runtime/app.rs");
    let state = read(&root, "crates/agentgo-cef/src/runtime/state.rs");
    let scheme = read(&root, "crates/agentgo-cef/src/runtime/scheme.rs");
    assert!(app.contains("window.set_to_box_layout"));
    assert!(app.contains("window.add_child_view(Some(&mut shell_as_view))"));
    assert!(app.contains("window.add_overlay_view(Some(&mut tools_as_view)"));
    // Tabs are stacked, full-size children of one panel below the chrome.
    assert!(app.contains("layout.set_flex_for_view(Some(&mut host_as_view), 1)"));
    assert!(app.contains("host.reorder_child_view(Some(&mut as_view), -1)"));
    assert!(state.contains("host.add_child_view(Some(&mut as_view))"));
    assert!(state.contains("host.remove_child_view(Some(&mut as_view))"));
    assert!(state.contains("pub fn restack_tool_overlay()"));
    // One dispatcher for every transport; the bridge has no handlers of its own.
    assert!(scheme.contains("dispatch::dispatch(&Caller::shell(), &method, params)"));
    assert!(!scheme.contains("KEEPALIVE"));
}

#[test]
fn shell_bridge_contract_matches_registry_invariants() {
    let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
    assert!(!registry.all().is_empty());
    // Implemented methods are reviewed and covered by the runtime smoke
    // path; unrelated contract entries remain planned.
    assert!(!registry.implemented().is_empty());
    // Trust boundary invariants.
    assert!(agentgo_shell_bridge::is_trusted_shell_url(
        "agentgo://shell/index.html"
    ));
    assert!(!agentgo_shell_bridge::is_trusted_shell_url(
        "https://evil.example"
    ));
}
