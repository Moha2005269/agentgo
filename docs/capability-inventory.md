# Agentgo Browser Capability Inventory

Version: brainstorming baseline 0.1  
Date: 2026-09-07

## Purpose

Agentgo should let an agent operate a browser with the same practical reach as a human: keyboard, mouse, touch-like input, tabs, windows, forms, files, dialogs, media, navigation, settings, downloads, and page understanding. The agent should also get capabilities that humans do not have to use manually: compact semantic state, batching, verification, replay, structured extraction, memory, policy enforcement, and recovery.

This is a requirements inventory, not an implementation plan. Every function should eventually have:

1. An implementation.
2. A machine-readable schema.
3. An entry in the bundled Agentgo skill.
4. Tests and an example.
5. A defined permission/risk level.

## Agent-agnostic ownership model

Agentgo is a browser runtime and universal control surface, not an AI service.

- The external agent owns the model, planning, conversation, identity, and task goal.
- The external agent launches Agentgo and sends commands directly to the browser.
- Agentgo owns browser execution, page observation, human-equivalent input, structured state, permissions, artifacts, verification, and recovery.
- Agentgo must not require an OpenAI, Anthropic, Claude, Codex, OpenCode, or other model-provider API key.
- Agentgo must not select a model or make hidden model calls.
- Codex, Claude Code, OpenCode, and other agent hosts must receive the same browser capabilities and the same control level.
- Differences between callers must come only from the user-selected Agentgo policy/profile, never from vendor-specific browser features.
- The browser UI remains useful for humans, but all browser functions must also be available through the universal control surface.

### Universal control surface

An agent needs a transport to control a process, but this should be a local, open, vendor-neutral **control protocol**, not a hosted product API. Agentgo should have one canonical command/schema contract with thin adapters for:

- command-line and stdin/stdout JSON for any agent that can launch a process;
- a local Windows named pipe or loopback socket for long-lived sessions;
- an optional MCP adapter for hosts that speak MCP;
- optional WebDriver BiDi/CDP compatibility adapters where useful;
- Rust, TypeScript, and Python client libraries generated from the same schema.

All adapters must map to the same underlying Agentgo commands, observations, permissions, and results. No adapter may silently remove control capabilities or add privileged capabilities.

## Decisions already made

- Browser runtime direction: CEF/Chromium, with a native Agentgo control core.
- Platform direction: Windows first only.
- UI direction: HTML/CSS browser shell rendered inside the browser runtime.
- The Agentgo skill ships with the browser and is developed with the functions.
- Agentgo is agent-agnostic: Codex, Claude Code, OpenCode, and other agents launch the executable and control it through the same vendor-neutral local control surface.
- A user may launch the executable directly and then instruct an external agent to connect to it.
- Every agent receives an isolated Agentgo executable/session. Instances use unique random instance identifiers and IPC/session credentials so agents cannot cross-control one another.
- The agent can control the whole browser product, not only tabs: browser chrome, windows, profiles, settings, downloads, dialogs, pages, native file pickers, and permitted device flows.
- Approval Mode pauses at configured sensitive gates.
- Full-Access Mode continues through normal browser workflows, including submissions, messages, account actions, and login when permitted by the selected profile/settings.
- In Approval Mode, login pauses only when credentials are not already stored/available through the selected browser vault/profile.
- Purchases, subscriptions, transfers, donations, bids, and final checkout always require explicit user approval, even in Full-Access Mode.
- CAPTCHA and anti-bot challenges always pause for a human handoff. Agentgo must not solve or bypass them.
- CAPTCHA handoff v1 is a smooth Windows notification; phone relay and richer handoff surfaces can come later.
- The active engine project is CEF/Chromium. Servo is removed from the active Agentgo plan; previously downloaded Servo material is historical research only.
- CEF is used as a source of useful primitives, but Agentgo's browser shell, agent control, state model, permissions, and workflows are custom from the beginning.
- A site may be opened in normal Chrome/Edge as an optional compatibility fallback when the user allows it and Agentgo cannot complete a workflow.
- The agent must have both low-level human-equivalent controls and higher-level one-call workflows.
- The user can always see that an agent is active and can use a visible `Pause` or `Stop Agent` control.
- `Pause` leaves the request resumable so the user can fix something and tell the agent to continue. `Stop Agent` is a red destructive control that cancels the current request completely.
- The user can enable session restoration, replay, and other persistence features from settings.
- Users can modify the open-source browser and share/export profiles, workflows, and skills when they choose.
- Agentgo should provide the agent with DOM, HTML, and page images/screenshots as needed. The observation system chooses the cheapest useful representation, while the agent may request a richer representation.
- Users can open the visible browser while background work is running to inspect what the agent is doing.
- Page content is untrusted input and must never silently change Agentgo permissions or policies.
- The user’s profile, cookies, credentials, downloads, and files are sensitive and should remain local by default.

## Priority legend

- **P0** — required for the first genuinely useful Agentgo browser.
- **P1** — required for human parity and serious daily use.
- **P2** — advanced capability, optimization, or ecosystem feature.
- **Safety** — must be enforced regardless of priority.

## Ownership legend

- **Runtime** — CEF/Chromium or its process/lifecycle layer.
- **Native** — Rust/C++ host, Windows APIs, filesystem, OS UI, or device integration.
- **Agent** — Agentgo semantic model, workflow engine, skills, memory, policy, or token control.
- **Shell** — Agentgo’s HTML/CSS browser UI.

---

## 1. Browser runtime and process lifecycle

### P0

- `browser.launch` — start the browser runtime with profile, window, locale, proxy, and policy settings.
- `browser.attach` — attach to an already-running Agentgo instance.
- `browser.detach` — disconnect a client without closing the browser.
- `browser.shutdown` — close the browser cleanly and flush artifacts.
- `browser.restart` — restart after a crash or engine-level failure while preserving resumable state.
- `browser.health` — report process, renderer, GPU, network, profile, and agent-controller health.
- `browser.version` — return Agentgo, CEF, Chromium, protocol, skill, and schema versions.
- `browser.capabilities` — return supported runtime capabilities and feature limitations.
- `browser.feature_flags` — read and change explicitly allowlisted Agentgo feature flags.
- `browser.command_line` — inspect effective non-secret runtime options.
- `browser.processes` — list browser, renderer, GPU, utility, and agent processes without exposing secrets.
- `browser.child_process_status` — inspect startup, shutdown, crash, and restart state.
- `browser.message_loop_status` — inspect and control external message-pump integration.
- `browser.crash_report` — collect a local crash artifact with redaction.
- `browser.recover` — attempt bounded recovery after a renderer, tab, or connection failure.

### P1

- `browser.suspend` — pause background work while preserving sessions.
- `browser.resume` — resume paused work.
- `browser.low_power_mode` — reduce background timers, media, rendering, and observation frequency.
- `browser.resource_budget` — set CPU, memory, tab, network, and disk budgets.
- `browser.runtime_metrics` — collect CPU, memory, GPU, network, and page-count metrics.
- `browser.engine_matrix` — report which functions are supported by the active engine build.

### P2

- `browser.engine_switch` — move a session to another supported engine runtime through the engine adapter.
- `browser.remote_instance` — connect to a permitted remote Agentgo runtime.
- `browser.update_check` — check signed Agentgo/CEF runtime updates.
- `browser.update_stage` — stage an update without replacing the active runtime.
- `browser.update_rollback` — return to the previous verified runtime.

---

## 2. Profiles, user contexts, and session state

### P0

- `profile.create` — create a named persistent profile.
- `profile.open` — open a profile with an explicit user confirmation when needed.
- `profile.close` — close a profile and flush cookies, storage, downloads, and checkpoints.
- `profile.list` — list profiles without exposing credentials or cookie contents.
- `profile.select` — select the profile for a workflow.
- `profile.status` — report locked, open, logged-in, challenge-paused, or corrupted state.
- `profile.lock` — lock a profile against concurrent control.
- `profile.unlock` — unlock after a verified local permission check.
- `profile.clone` — clone a profile into a separate working copy.
- `profile.ephemeral` — create a temporary isolated profile.
- `profile.incognito` — create a private context with explicit retention behavior.
- `profile.delete` — delete a profile only after an explicit, separate confirmation.
- `profile.reset_site` — clear one site’s state without clearing the whole profile.
- `profile.export_state` — export an encrypted, scoped session state.
- `profile.import_state` — import an encrypted state after validation.
- `profile.checkpoint` — save resumable browser/agent state.
- `profile.restore_checkpoint` — restore a named checkpoint.
- `profile.session_list` — list named sessions and active tabs.
- `profile.session_save` — persist session metadata and workflow position.
- `profile.session_resume` — resume a paused workflow in the same session.

### P1

- `profile.cookie_policy` — configure per-profile cookie behavior.
- `profile.storage_partition` — create and inspect isolated storage partitions.
- `profile.locale` — set language/locale for the profile.
- `profile.timezone` — set timezone for legitimate testing or user preference.
- `profile.accessibility_preferences` — configure reduced motion, contrast, font scaling, and screen-reader preferences.
- `profile.download_directory` — set a scoped download directory.
- `profile.proxy` — configure an explicit user-approved proxy; never rotate proxies for challenge avoidance.
- `profile.permission_defaults` — configure default site permission behavior.
- `profile.autofill_policy` — configure whether browser-managed autofill may run.
- `profile.password_vault_binding` — bind to an OS/browser password vault without returning plaintext secrets to the model.

### Safety

- Profiles must never be implicitly shared between agents.
- Cookies, passwords, tokens, and private downloads must be redacted from model output by default; explicit user permission may enable credential visibility to the agent under the rules below.
- Full-Access Mode may expose plaintext credentials to the agent if the user explicitly enables that capability. Credential visibility is local-only, never sent to a hosted model by Agentgo, and must be redacted from logs, screenshots, traces, artifacts, and telemetry. Browser/OS autofill remains available as a separate capability.
- Cloud execution must never receive profile credentials or cookies unless the user explicitly chooses it.
- Full-Access Mode must still use scoped profiles, domains, files, and accounts.

---

## 3. Windows, browser windows, and viewport control

### P0

- `window.create` — create a browser window.
- `window.close` — close a window after checking pending work.
- `window.focus` — focus a window.
- `window.activate` — bring a window to the foreground.
- `window.minimize` — minimize a window.
- `window.maximize` — maximize a window.
- `window.restore` — restore a minimized/maximized window.
- `window.fullscreen` — enter or leave fullscreen.
- `window.move` — move a window to a screen position.
- `window.resize` — resize a window.
- `window.bounds` — read window bounds, scale, monitor, and work area.
- `window.monitor_list` — list connected monitors and scale factors.
- `window.always_on_top` — set always-on-top with policy control.
- `window.visibility` — show, hide, or minimize a window.
- `window.title` — read or set the Agentgo window title.
- `window.native_handle` — return a scoped native handle to the host, not the model.
- `window.capture` — capture a window image or video stream.
- `window.set_zoom` — set window/page zoom.
- `window.reset_zoom` — reset zoom.
- `viewport.size` — read the content viewport dimensions.
- `viewport.set_size` — set viewport dimensions.
- `viewport.device_scale` — read device scale factor.
- `viewport.set_device_scale` — set scale for testing or layout control.

### P1

- `window.split` — arrange two or more tabs/windows in a workspace.
- `window.workspace` — create, select, rename, and close a workspace.
- `window.detach_tab` — detach a tab into a window.
- `window.attach_tab` — attach a tab to a window.
- `window.presentation_mode` — hide browser chrome for focused work.
- `window.agent_overlay` — show or hide the Agentgo activity overlay.
- `window.always_visible_prompt` — show a clear indicator when the agent is active.

---

## 4. Tabs, frames, browsing contexts, and popups

### P0

- `tab.create` — create a new tab.
- `tab.close` — close a tab.
- `tab.activate` — activate a tab.
- `tab.list` — list tabs with compact metadata.
- `tab.status` — report loading, idle, crashed, paused, media-playing, or challenge state.
- `tab.navigate` — navigate a tab.
- `tab.reload` — reload a tab.
- `tab.stop` — stop loading.
- `tab.back` — navigate back.
- `tab.forward` — navigate forward.
- `tab.duplicate` — duplicate a tab with selected state.
- `tab.reopen_closed` — reopen the most recently closed tab.
- `tab.move` — reorder a tab.
- `tab.group` — create or assign a tab group.
- `tab.ungroup` — remove a tab from a group.
- `tab.pin` — pin a tab.
- `tab.unpin` — unpin a tab.
- `tab.mute` — mute or unmute a tab.
- `tab.discard` — suspend a background tab.
- `tab.restore` — restore a discarded tab.
- `tab.freeze` — freeze background work where supported.
- `tab.unfreeze` — resume a frozen tab.
- `tab.open_popup` — handle a popup as a new context according to policy.
- `tab.close_popup` — close a popup.
- `tab.opener` — inspect the opener relationship.
- `tab.target` — inspect target IDs and child contexts.
- `frame.list` — list frames and nested browsing contexts.
- `frame.select` — select a frame for an action or observation.
- `frame.parent` — return a frame’s parent.
- `frame.children` — return child frames.
- `frame.focus` — focus a frame.
- `frame.navigate` — navigate a selected frame where permitted.
- `frame.wait` — wait for a frame to appear or become ready.
- `browsing_context.tree` — return the compact browsing-context tree.
- `browsing_context.create` — create a tab/window browsing context.
- `browsing_context.close` — close a browsing context.
- `browsing_context.activate` — activate a top-level browsing context.

### P1

- `tab.search` — search open tabs.
- `tab.sort` — sort tabs by title, domain, activity, or group.
- `tab.save_session` — save a tab/workspace session.
- `tab.restore_session` — restore a saved session.
- `tab.freeze_all_background` — freeze background tabs.
- `tab.close_others` — close other tabs after confirmation if needed.
- `tab.close_to_right` — close tabs to the right.
- `tab.suspend_duplicates` — suspend duplicate pages while preserving the active one.
- `frame.snapshot` — capture a frame-specific semantic snapshot.

---

## 5. Navigation, loading, and synchronization

### P0

- `navigate.url` — open a URL in a selected context.
- `navigate.search` — resolve a search query through a configured search provider.
- `navigate.link` — open a discovered link.
- `navigate.external` — request the OS to open an external URL under policy.
- `navigate.history` — inspect scoped history.
- `navigate.history_back` — go back a controlled number of entries.
- `navigate.history_forward` — go forward.
- `navigate.reload` — reload with selected cache behavior.
- `navigate.hard_reload` — reload while bypassing cache where supported.
- `navigate.stop` — stop navigation.
- `navigate.cancel` — cancel a pending navigation or workflow.
- `navigate.redirect_chain` — inspect redirects.
- `navigate.current` — read current URL/title/origin/ready state.
- `wait.load` — wait for a load readiness state.
- `wait.dom_ready` — wait for DOM readiness.
- `wait.network_idle` — wait for bounded network quiet.
- `wait.selector` — wait for a semantic/CSS/XPath selector.
- `wait.text` — wait for text or a text pattern.
- `wait.role` — wait for an accessibility role/name.
- `wait.visible` — wait for visibility.
- `wait.hidden` — wait for disappearance.
- `wait.enabled` — wait for an enabled control.
- `wait.stable` — wait for layout/action stability.
- `wait.download` — wait for a download event.
- `wait.popup` — wait for a popup/context.
- `wait.dialog` — wait for a user prompt/dialog.
- `wait.media` — wait for media readiness.
- `wait.condition` — wait for a bounded page/runtime condition.
- `wait.settle` — perform a general action-settle cycle.

### P1

- `navigate.prerender_status` — inspect prerender state.
- `navigate.service_worker_status` — inspect service-worker navigation influence.
- `navigate.offline` — emulate or enter an offline state for testing.
- `navigate.throttle` — apply a user-approved network profile for testing.
- `navigate.retry` — retry a failed navigation with a bounded policy.
- `navigate.resume_after_interruption` — resume after a handoff or pause.

---

## 6. Human-equivalent mouse, pointer, touch, and gesture input

### P0 mouse/pointer

- `pointer.move` — move the pointer to coordinates or a semantic target.
- `pointer.down` — press a mouse button.
- `pointer.up` — release a mouse button.
- `pointer.click` — click once.
- `pointer.double_click` — double-click.
- `pointer.triple_click` — triple-click/select paragraph-like content.
- `pointer.right_click` — open a context menu.
- `pointer.middle_click` — open a link in a new tab or invoke middle-click behavior.
- `pointer.hover` — hover without clicking.
- `pointer.drag_start` — begin a drag.
- `pointer.drag_move` — move during a drag.
- `pointer.drag_end` — release a drag.
- `pointer.drag_to` — perform a verified drag-and-drop.
- `pointer.scroll` — scroll by delta.
- `pointer.scroll_to` — scroll to a target/position.
- `pointer.scroll_into_view` — bring a target into view.
- `pointer.wheel` — send wheel input.
- `pointer.position` — read the current pointer position.
- `pointer.button_state` — read button state.
- `pointer.modifiers` — apply Ctrl/Alt/Shift/Meta modifiers.
- `pointer.hit_test` — resolve which element is under coordinates.
- `pointer.capture` — inspect pointer capture ownership.

### P0 keyboard

- `keyboard.key_down` — press a key down.
- `keyboard.key_up` — release a key.
- `keyboard.press` — press a key or key combination.
- `keyboard.type` — type human-readable text with correct input events.
- `keyboard.insert_text` — insert text into the focused control.
- `keyboard.shortcut` — execute a named browser/app shortcut.
- `keyboard.chord` — send a multi-key chord.
- `keyboard.tab` — move focus forward.
- `keyboard.shift_tab` — move focus backward.
- `keyboard.enter` — submit/activate according to focus.
- `keyboard.escape` — cancel/close a menu or dialog.
- `keyboard.home` — move to start.
- `keyboard.end` — move to end.
- `keyboard.page_up` — page upward.
- `keyboard.page_down` — page downward.
- `keyboard.arrow` — send directional navigation.
- `keyboard.delete` — delete selected/following content.
- `keyboard.backspace` — delete preceding content.
- `keyboard.select_all` — select all in the focused context.
- `keyboard.undo` — undo.
- `keyboard.redo` — redo.
- `keyboard.composition_start` — begin IME composition.
- `keyboard.composition_update` — update IME composition.
- `keyboard.composition_end` — commit IME composition.
- `keyboard.layout` — report active keyboard layout.
- `keyboard.focused_element` — report focus target.

### P1 touch/pen/gesture

- `touch.start` — start a touch contact.
- `touch.move` — move a touch contact.
- `touch.end` — end a touch contact.
- `touch.cancel` — cancel a touch contact.
- `touch.tap` — tap a target.
- `touch.long_press` — long-press.
- `touch.swipe` — swipe.
- `touch.pinch` — pinch/zoom gesture.
- `touch.rotate` — rotate gesture where supported.
- `pen.move` — move a pen/stylus pointer.
- `pen.down` — press stylus contact.
- `pen.up` — release stylus contact.
- `pen.pressure` — send pressure where supported.
- `gesture.back` — platform back gesture.
- `gesture.forward` — platform forward gesture.

### Safety

- Every input action must record target, coordinates/semantic reference, modifiers, and result.
- Coordinate actions require a viewport/scale check and a post-action verification.
- Agentgo must distinguish page input from privileged Agentgo-shell input.

---

## 7. Clipboard, selection, focus, and editing

### P0

- `focus.element` — focus a semantic target.
- `focus.window` — focus the browser window.
- `focus.frame` — focus a frame.
- `focus.read` — read current focus metadata.
- `focus.blur` — blur the current element.
- `selection.read` — read selected text and range metadata.
- `selection.set` — set a text selection.
- `selection.extend` — extend a selection.
- `selection.clear` — clear selection.
- `clipboard.read_text` — read text clipboard with permission check.
- `clipboard.write_text` — write text clipboard.
- `clipboard.read_html` — read HTML clipboard with permission check.
- `clipboard.write_html` — write HTML clipboard.
- `clipboard.read_image` — read image clipboard.
- `clipboard.write_image` — write image clipboard.
- `clipboard.clear` — clear the clipboard.
- `edit.clear` — clear a field.
- `edit.fill` — fill a field while dispatching correct input/change events.
- `edit.append` — append text.
- `edit.replace_selection` — replace the current selection.
- `edit.undo` — undo an edit.
- `edit.redo` — redo an edit.
- `edit.copy` — copy selection.
- `edit.cut` — cut selection.
- `edit.paste` — paste from clipboard.
- `edit.contenteditable` — detect and operate rich editable regions.

### P1

- `edit.rich_text` — preserve formatting in contenteditable editors.
- `edit.markdown` — type markdown with editor-aware behavior.
- `edit.code_editor` — interact with Monaco/CodeMirror-like editors.
- `edit.autocomplete` — inspect, choose, or dismiss autocomplete suggestions.
- `edit.spellcheck` — inspect browser/editor spellcheck suggestions.
- `edit.input_method` — control IME-aware text entry.

---

## 8. Semantic page observation and visual grounding

### P0

- `observe.page` — return a compact page summary.
- `observe.outline` — return title, URL, regions, landmarks, dialogs, and major controls.
- `observe.accessibility_tree` — return an accessibility tree with compact stable references.
- `observe.dom_snapshot` — return a filtered DOM snapshot.
- `observe.dom_subtree` — return a selected subtree.
- `observe.layout` — return layout boxes and visibility for selected nodes.
- `observe.text` — return readable visible text.
- `observe.content` — return cleaned article/content text.
- `observe.interactive` — return actionable elements only.
- `observe.forms` — return detected forms and fields.
- `observe.tables` — return detected tables and table metadata.
- `observe.lists` — return detected lists/cards/results.
- `observe.links` — return links with text, URL, region, and state.
- `observe.images` — return image metadata and accessible descriptions.
- `observe.media` — return video/audio/player state.
- `observe.dialogs` — return browser/page/native dialogs.
- `observe.overlays` — return modals, menus, popovers, banners, and blockers.
- `observe.focus` — return focused element and focus context.
- `observe.selection` — return selected content metadata.
- `observe.frames` — return frame tree and origins.
- `observe.shadow_roots` — inspect open shadow roots under policy.
- `observe.iframe_content` — inspect permitted iframe content.
- `observe.page_metadata` — return title, description, canonical URL, language, author, and timestamps.
- `observe.favicon` — return favicon metadata.
- `observe.viewport` — return viewport, scroll, scale, and visible bounds.
- `observe.screenshot` — capture a viewport screenshot.
- `observe.full_page_screenshot` — capture a full-page screenshot.
- `observe.crop` — capture a targeted crop.
- `observe.ocr` — OCR a targeted visual region.
- `observe.hit_test` — map a coordinate to the current target.

### P0 state efficiency

- `state.create_baseline` — create a page-state baseline.
- `state.fingerprint` — fingerprint structure, URL, title, and relevant content.
- `state.diff` — return only changed state.
- `state.delta_after_action` — return changes caused by one action/batch.
- `state.ref_create` — create a semantic reference.
- `state.ref_resolve` — resolve a reference against current state.
- `state.ref_validate` — verify a reference before acting.
- `state.ref_heal` — re-ground a stale reference.
- `state.ref_expire` — expire references after structural changes.
- `state.relevance_filter` — filter state by intent.
- `state.region_focus` — focus observation on a page region.
- `state.content_hash` — hash selected content for change detection.
- `state.page_identity` — verify that the current page is the expected page.
- `state.snapshot_store` — store a compact snapshot outside model context.
- `state.snapshot_load` — load a stored snapshot by handle.

### P1 visual/diagnostic observation

- `observe.cursor` — show/read pointer state.
- `observe.animation` — inspect animations/transitions.
- `observe.canvas` — inspect canvas bounds and capture output.
- `observe.computed_style` — read selected computed styles.
- `observe.paint_regions` — inspect paint/layout regions for grounding.
- `observe.scroll_containers` — identify nested scroll containers.
- `observe.fixed_elements` — identify fixed/sticky overlays.
- `observe.loading_indicators` — identify spinners/skeletons/progress.
- `observe.visual_diff` — compare screenshots or crops.
- `observe.record_video` — record selected browser content.

---

## 9. Locators, grounding, and target resolution

### P0

- `locate.role` — locate by accessibility role.
- `locate.name` — locate by accessible name.
- `locate.label` — locate a form control by label.
- `locate.placeholder` — locate by placeholder.
- `locate.text` — locate by visible text.
- `locate.text_pattern` — locate by bounded text pattern.
- `locate.link` — locate a link by name or URL.
- `locate.button` — locate a button by name.
- `locate.field` — locate a field by semantic metadata.
- `locate.checkbox` — locate a checkbox.
- `locate.radio` — locate a radio control.
- `locate.select` — locate a select control.
- `locate.file_input` — locate a file input.
- `locate.test_id` — locate by explicit test ID.
- `locate.css` — locate by CSS selector under policy.
- `locate.xpath` — locate by XPath under policy.
- `locate.dom_ref` — locate by internal DOM reference.
- `locate.ax_ref` — locate by accessibility reference.
- `locate.coordinate` — locate by viewport coordinates.
- `locate.region` — locate a page region.
- `locate.frame` — locate the correct frame first.
- `locate.shadow` — locate inside an open shadow root.
- `locate.nth` — choose a bounded matching index.
- `locate.filter` — filter matches by state/text/attribute.
- `locate.closest` — resolve the nearest actionable ancestor.
- `locate.descendant` — resolve a descendant.
- `locate.all` — return bounded matches.
- `locate.unique` — require a unique match.
- `locate.explain` — explain why a target was selected.

### P1

- `ground.semantic` — combine role/name/label/text/structure into one target.
- `ground.visual` — ground a target from a visual crop.
- `ground.hybrid` — combine semantic and visual evidence.
- `ground.reobserve` — refresh the relevant region before acting.
- `ground.compare_candidates` — compare ambiguous candidates.
- `ground.confidence` — return target confidence and reasons.
- `ground.stale_recovery` — recover from a stale target.

---

## 10. Page actions and form operations

### P0

- `act.click` — click a target.
- `act.double_click` — double-click a target.
- `act.right_click` — context-click a target.
- `act.hover` — hover a target.
- `act.focus` — focus a target.
- `act.type` — type into a target.
- `act.fill` — replace a target’s value.
- `act.clear` — clear a target.
- `act.select_option` — select one or more options.
- `act.check` — check a checkbox.
- `act.uncheck` — uncheck a checkbox.
- `act.toggle` — toggle a control.
- `act.choose_radio` — choose a radio option.
- `act.press` — press a key while targeting an element.
- `act.submit` — submit a form according to policy.
- `act.scroll` — scroll a target/container.
- `act.scroll_page` — scroll the page.
- `act.drag_drop` — drag one target to another.
- `act.drag_file` — drag a scoped file into a drop zone.
- `act.upload` — upload scoped file(s).
- `act.download` — trigger or collect a download.
- `act.open` — open a menu, disclosure, or link.
- `act.close` — close a menu, disclosure, or modal.
- `act.expand` — expand a collapsed region.
- `act.collapse` — collapse a region.
- `act.play` — play media.
- `act.pause` — pause media.
- `act.seek` — seek media.
- `act.set_value` — set a field value with correct events.
- `act.dispatch_input` — dispatch a controlled input/change event when needed.
- `act.wait_and_click` — wait for target readiness then click.
- `act.click_and_verify` — click and verify a specified postcondition.

### P1 form-specific operations

- `form.discover` — discover fields, labels, types, requirements, and validation.
- `form.map` — map user data to form fields.
- `form.fill` — fill a form with field-level evidence.
- `form.fill_batch` — fill multiple fields atomically where possible.
- `form.validate` — inspect client-side validation.
- `form.invalid_fields` — return invalid fields and messages.
- `form.required_fields` — return required fields.
- `form.submit_draft` — save a draft without final submission.
- `form.submit` — submit a form after policy checks.
- `form.confirm` — handle page confirmation.
- `form.review` — produce a reviewable field/value summary.
- `form.reset` — reset a form.
- `form.autofill` — invoke browser-managed autofill under policy.
- `form.date` — operate date controls.
- `form.time` — operate time controls.
- `form.datetime` — operate date-time controls.
- `form.number` — operate number/range controls.
- `form.color` — operate color controls.
- `form.file` — operate file inputs.
- `form.rich_text` — operate rich text fields.
- `form.multi_select` — operate multiselect widgets.
- `form.combobox` — operate custom comboboxes.
- `form.autocomplete` — operate autocomplete controls.
- `form.slider` — operate sliders with value verification.

### Safety

- `form.submit` and `act.submit` must report the exact destination and side effect.
- Purchase/checkout/payment actions are always approval-gated.
- Authentication and account-security fields are governed by the selected permission mode and secret-vault rules.

---

## 11. JavaScript, DOM, and page-runtime control

### P0

- `runtime.evaluate` — evaluate a bounded script in the page context.
- `runtime.evaluate_isolated` — evaluate in an isolated world.
- `runtime.call_function` — call a page function with structured arguments.
- `runtime.read_property` — read a bounded property.
- `runtime.query` — query DOM/page structures.
- `runtime.query_all` — query bounded collections.
- `runtime.get_attribute` — read an attribute.
- `runtime.get_property` — read a property.
- `runtime.set_property` — set a property only under policy.
- `runtime.get_text` — read text.
- `runtime.get_html` — read bounded HTML.
- `runtime.get_value` — read a control value.
- `runtime.get_rect` — read a layout rectangle.
- `runtime.add_init_script` — add a controlled initialization script.
- `runtime.remove_init_script` — remove an initialization script.
- `runtime.add_binding` — expose a typed native binding.
- `runtime.remove_binding` — remove a binding.
- `runtime.message` — send a structured message between page and host.
- `runtime.promise` — await a bounded page promise.
- `runtime.console` — read console messages.
- `runtime.exception` — read page exceptions.

### Safety

- The model must not receive arbitrary native handles or unrestricted host APIs.
- Page JavaScript cannot modify Agentgo policy, approval state, secrets, or audit history.
- Evaluation must be origin/frame scoped, time-limited, and size-limited.

---

## 12. Files, native dialogs, downloads, and printing

### P0

- `file.choose` — choose a scoped file for an upload.
- `file.choose_multiple` — choose multiple files.
- `file.choose_directory` — choose a directory where supported.
- `file.upload` — set files on a file input.
- `file.drag_to_upload` — drag a file into a drop zone.
- `file.dialog_open` — detect an open native file dialog.
- `file.dialog_save` — detect an open native save dialog.
- `file.dialog_cancel` — cancel a native file dialog.
- `file.dialog_confirm` — confirm a native file dialog after policy checks.
- `file.path_validate` — validate path scope and existence.
- `file.metadata` — return safe file metadata.
- `file.hash` — hash a file for deduplication.
- `file.read_text` — read a scoped text file.
- `file.read_bytes` — read a scoped binary file through an artifact handle.
- `file.write_text` — write a scoped text artifact.
- `file.copy` — copy within a scoped artifact area.
- `file.move` — move within a scoped artifact area.
- `file.rename` — rename within a scoped artifact area.
- `download.start` — initiate a download.
- `download.list` — list active/completed downloads.
- `download.pause` — pause a download.
- `download.resume` — resume a download.
- `download.cancel` — cancel a download.
- `download.open` — open a downloaded artifact under policy.
- `download.show` — reveal a download in the local file manager under policy.
- `download.destination` — choose a scoped destination.
- `download.progress` — report progress.
- `download.verify` — verify size, hash, content type, and completion.
- `print.preview` — create a print preview.
- `print.to_pdf` — save a page as PDF.
- `print.to_printer` — print to a selected printer with approval policy.
- `page.save` — save page/content using a selected format.
- `page.save_snapshot` — save a page snapshot artifact.

### P1

- `file.archive` — package selected artifacts.
- `file.extract_archive` — extract an archive into a scoped directory.
- `download.resume_after_restart` — resume downloads after a restart.
- `download.deduplicate` — identify duplicate downloads.
- `download.quarantine` — quarantine suspicious downloads.
- `print.selection` — print selected content.
- `print.backgrounds` — control print backgrounds.

### Safety

- No arbitrary filesystem access from page content.
- File upload paths must be explicitly scoped.
- Downloaded files should be quarantined or inspected before execution.
- Opening executables or launching external applications requires a separate policy.

---

## 13. Browser dialogs, menus, prompts, and permissions

### P0

- `dialog.list` — list active page/native dialogs.
- `dialog.alert` — observe an alert.
- `dialog.confirm` — accept/deny a confirm dialog.
- `dialog.prompt` — fill/accept/deny a prompt dialog.
- `dialog.before_unload` — handle before-unload prompts.
- `dialog.javascript` — handle JavaScript dialog state.
- `dialog.file` — handle file chooser dialog state.
- `dialog.auth` — handle HTTP authentication request.
- `dialog.permission` — handle a site permission request.
- `dialog.close` — close a dialog where supported.
- `menu.open` — open a browser/page menu.
- `menu.close` — close a menu.
- `menu.select` — select a menu item.
- `context_menu.open` — open a context menu.
- `context_menu.select` — select a context menu action.
- `context_menu.dismiss` — dismiss a context menu.
- `permission.list` — list site permissions.
- `permission.grant` — grant a scoped permission.
- `permission.deny` — deny a permission.
- `permission.reset` — reset a permission.
- `permission.explain` — explain why a permission is requested.

### P1

- `dialog.print` — handle print dialog/preview state.
- `dialog.download` — handle download prompts.
- `dialog.install` — handle extension/app install prompts.
- `dialog.external_protocol` — handle mailto/tel/custom-protocol prompts.
- `dialog.certificate` — handle certificate errors according to strict policy.
- `dialog.fullscreen` — handle fullscreen requests.

### Safety

- Certificate errors must never be silently accepted for normal browsing.
- Permission grants must be origin-scoped, time-scoped, and logged.
- Authentication prompts must not cause the agent to invent credentials.

---

## 14. Storage, cookies, credentials, and site data

### P0

- `storage.cookies_list` — list cookies by scoped origin.
- `storage.cookie_get` — read a cookie only when policy permits.
- `storage.cookie_set` — set a cookie only when policy permits.
- `storage.cookie_delete` — delete a cookie.
- `storage.cookies_clear` — clear scoped cookies.
- `storage.local_get` — read localStorage.
- `storage.local_set` — set localStorage.
- `storage.local_delete` — delete localStorage entry.
- `storage.local_clear` — clear localStorage.
- `storage.session_get` — read sessionStorage.
- `storage.session_set` — set sessionStorage.
- `storage.session_clear` — clear sessionStorage.
- `storage.indexeddb_list` — list IndexedDB databases.
- `storage.indexeddb_export` — export selected IndexedDB data.
- `storage.indexeddb_clear` — clear selected IndexedDB data.
- `storage.cache_list` — list CacheStorage caches.
- `storage.cache_export` — export selected cache entries.
- `storage.cache_clear` — clear selected cache entries.
- `storage.service_workers` — list service workers.
- `storage.service_worker_unregister` — unregister a selected worker.
- `storage.site_data_clear` — clear all selected site data.
- `storage.site_data_export` — export selected site data.
- `storage.permissions` — inspect storage-related permissions.

### P1

- `credential.vault_status` — report whether a credential vault is available.
- `credential.autofill` — request vault-managed autofill without returning plaintext.
- `credential.save` — request saving credentials to the OS vault after explicit policy.
- `credential.delete` — delete a saved credential after confirmation.
- `credential.revoke_session` — revoke a selected local session where supported.
- `credential.redact` — redact secrets in traces and artifacts.

### Safety

- Agentgo should prefer browser/OS vault actions over exposing passwords to the model.
- Storage APIs are origin-scoped and must redact tokens, session IDs, and sensitive values.
- Exporting cookies or auth state requires explicit approval.

---

## 15. Network, requests, responses, and web platform events

### P0

- `network.observe_requests` — observe request metadata.
- `network.observe_responses` — observe response metadata.
- `network.request_body` — inspect a bounded request body.
- `network.response_body` — inspect a bounded response body.
- `network.headers` — inspect headers with secret redaction.
- `network.timing` — inspect timing milestones.
- `network.failures` — list failed requests.
- `network.redirects` — inspect redirect chains.
- `network.block` — block a scoped request.
- `network.allow` — allow a scoped request.
- `network.continue` — continue a paused request.
- `network.fulfill` — provide a controlled response for testing/local fixtures.
- `network.modify_headers` — modify headers in a scoped test or app policy.
- `network.modify_response` — modify responses only in explicit local/testing modes.
- `network.intercept` — register an allowlisted interceptor.
- `network.clear_interceptors` — remove interceptors.
- `network.cache_status` — inspect cache behavior.
- `network.clear_cache` — clear selected cache.
- `network.offline` — set offline mode.
- `network.online` — resume online mode.
- `network.websocket_list` — list WebSocket connections.
- `network.websocket_messages` — inspect bounded WebSocket messages.
- `network.sse_events` — inspect server-sent events.
- `network.fetch` — perform a permitted browser-context fetch.
- `network.download_response` — save a response as an artifact.
- `network.har_start` — start a scoped network trace.
- `network.har_stop` — stop and save a network trace.

### P1

- `network.emulate_latency` — emulate latency.
- `network.emulate_bandwidth` — emulate bandwidth.
- `network.emulate_errors` — emulate selected errors.
- `network.dns_status` — inspect DNS resolution metadata.
- `network.connection_status` — inspect connection state.
- `network.service_worker_events` — inspect background fetch/cache events.
- `network.background_tasks` — inspect background network activity.
- `network.replay` — replay a captured local fixture.

### Safety

- Request/response interception is not an anti-bot mechanism.
- Do not rotate identities, spoof fingerprints, or alter traffic to evade challenges.
- Network traces must redact authorization headers, cookies, tokens, and personal data.

---

## 16. Media, camera, microphone, sensors, and devices

### P0

- `media.list` — list media elements and player states.
- `media.play` — play a media element.
- `media.pause` — pause a media element.
- `media.stop` — stop media.
- `media.seek` — seek to a position.
- `media.volume` — set volume.
- `media.mute` — mute/unmute.
- `media.speed` — set playback speed.
- `media.fullscreen` — enter/leave media fullscreen.
- `media.picture_in_picture` — enter/leave picture-in-picture.
- `media.tracks` — inspect audio/text/video tracks.
- `media.select_track` — select a track.
- `media.capture_frame` — capture a video frame.
- `media.capture_stream` — capture a permitted media stream.
- `media.status` — return buffering, playing, paused, ended, and error state.
- `media.transcript` — extract available captions/transcript text.

### P1

- `device.camera_list` — list cameras after permission.
- `device.microphone_list` — list microphones after permission.
- `device.output_list` — list audio outputs.
- `device.camera_select` — choose a camera.
- `device.microphone_select` — choose a microphone.
- `device.output_select` — choose an audio output.
- `device.geolocation` — provide or inspect geolocation under permission.
- `device.orientation` — inspect orientation.
- `device.bluetooth` — handle Bluetooth permission/device flows.
- `device.usb` — handle USB permission/device flows.
- `device.serial` — handle serial permission/device flows.
- `device.hid` — handle HID permission/device flows.
- `device.nfc` — handle NFC permission/device flows where supported.
- `device.web_authn` — handle WebAuthn/passkey flows through explicit policy.

### Safety

- Camera, microphone, location, Bluetooth, USB, serial, HID, NFC, and WebAuthn permissions must be origin-scoped and logged.
- Agentgo must not silently capture audio/video or expose biometric/passkey secrets.

---

## 17. Human browser chrome and product features

### P0

- `omnibox.focus` — focus the address/search bar.
- `omnibox.read` — read the current input.
- `omnibox.type` — type an address/query.
- `omnibox.submit` — navigate/search.
- `omnibox.clear` — clear input.
- `omnibox.suggestions` — return suggestions.
- `omnibox.select_suggestion` — select a suggestion.
- `browser_ui.back` — activate back button.
- `browser_ui.forward` — activate forward button.
- `browser_ui.reload` — activate reload.
- `browser_ui.stop` — activate stop.
- `browser_ui.new_tab` — activate new tab.
- `browser_ui.close_tab` — activate close tab.
- `browser_ui.tab_switch` — switch tab through UI.
- `browser_ui.menu` — open Agentgo browser menu.
- `browser_ui.find` — find text in page.
- `browser_ui.find_next` — find next match.
- `browser_ui.find_previous` — find previous match.
- `browser_ui.zoom_in` — zoom in.
- `browser_ui.zoom_out` — zoom out.
- `browser_ui.zoom_reset` — reset zoom.
- `browser_ui.fullscreen` — browser fullscreen.
- `browser_ui.devtools` — open/close developer tools under policy.
- `browser_ui.reader_mode` — enter reader mode where available.
- `browser_ui.translate` — translate page through a configured provider.
- `browser_ui.view_source` — view source.
- `browser_ui.save_page` — save page.
- `browser_ui.print` — print page.
- `browser_ui.downloads` — open download manager.
- `browser_ui.history` — open history manager.
- `browser_ui.bookmarks` — open bookmark manager.

### P1

- `bookmark.add` — add a bookmark.
- `bookmark.remove` — remove a bookmark.
- `bookmark.edit` — edit bookmark title/folder.
- `bookmark.move` — move bookmark.
- `bookmark.search` — search bookmarks.
- `bookmark.open` — open bookmark.
- `history.list` — list history.
- `history.search` — search history.
- `history.delete` — delete scoped history.
- `history.clear` — clear history after confirmation.
- `download_manager.open` — open download manager.
- `download_manager.search` — search downloads.
- `download_manager.remove` — remove download record.
- `site_info.open` — show origin/site information.
- `site_info.permissions` — show site permissions.
- `browser_settings.open` — open settings.
- `browser_settings.read` — read settings.
- `browser_settings.update` — update settings with policy checks.
- `browser_shortcut.list` — list shortcuts.
- `browser_shortcut.configure` — configure shortcuts.
- `theme.get` — read theme.
- `theme.set` — set theme.
- `appearance.fonts` — configure fonts.
- `appearance.contrast` — configure contrast.
- `appearance.motion` — configure motion.

### P2

- `sync.status` — inspect sync status.
- `sync.start` — start user-approved sync.
- `sync.stop` — stop sync.
- `web_app.install` — install a permitted web app.
- `web_app.uninstall` — uninstall a web app.
- `browser.profile_switcher` — switch profiles through the shell.
- `browser.task_manager` — inspect browser task resource usage.

---

## 18. Extensions, scripts, and Agentgo plugins

### P1

- `extension.list` — list installed extensions.
- `extension.install` — install an allowlisted extension.
- `extension.uninstall` — uninstall an extension.
- `extension.enable` — enable an extension.
- `extension.disable` — disable an extension.
- `extension.permissions` — inspect extension permissions.
- `extension.permission_request` — handle extension permission changes.
- `extension.content_scripts` — inspect content-script registrations.
- `extension.browser_actions` — invoke an extension browser action.
- `extension.context_menu` — invoke an extension context menu item.
- `extension.storage` — inspect extension storage.
- `extension.reload` — reload an extension during development.
- `script.user_list` — list user scripts.
- `script.user_install` — install an allowlisted user script.
- `script.user_remove` — remove a user script.
- `script.user_enable` — enable a user script.
- `script.user_disable` — disable a user script.
- `native_messaging.list` — list native messaging hosts.
- `native_messaging.invoke` — invoke a permitted host with typed input.

### Safety

- Extensions and native messaging hosts require explicit installation and permission review.
- A page cannot install an extension or native host by instructing the agent.

---

## 19. Agent one-call execution and workflow control

### P0

- `agent.run` — execute a goal using observe, act, wait, verify, and recover internally.
- `agent.plan` — create a structured plan without executing it.
- `agent.preview` — show intended side effects and target pages.
- `agent.observe` — request a compact intent-filtered observation.
- `agent.act` — execute one verified action.
- `agent.batch` — execute a batch of deterministic actions.
- `agent.extract` — extract structured results.
- `agent.search_and_extract` — search, paginate, deduplicate, and return structured records.
- `agent.fill_and_review` — fill a form and return a reviewable summary.
- `agent.navigate_and_wait` — navigate and wait for a stable condition.
- `agent.collect` — collect results across pages/scroll/pagination.
- `agent.compare` — compare items/pages/records.
- `agent.monitor` — monitor a page or state change.
- `agent.pause` — pause execution.
- `agent.resume` — resume execution.
- `agent.cancel` — cancel execution.
- `agent.checkpoint` — save workflow state.
- `agent.restore` — restore workflow state.
- `agent.retry` — perform bounded retry.
- `agent.replan` — re-observe and produce a new plan after a verified failure.
- `agent.verify` — evaluate a postcondition.
- `agent.assert` — assert a page or workflow condition.
- `agent.progress` — report progress without dumping raw page state.
- `agent.status` — return compact workflow status.
- `agent.result` — return final structured output.
- `agent.evidence` — return evidence handles supporting the result.
- `agent.explain` — explain decisions using trace references.
- `agent.budget` — set token/time/action/resource budgets.
- `agent.dry_run` — simulate actions without side effects.
- `agent.handoff` — pause and hand control to a human.
- `agent.resume_after_handoff` — resume the same session after human action.

### P1

- `agent.parallel_tabs` — execute independent tasks across tabs.
- `agent.parallel_profiles` — execute isolated tasks across profiles.
- `agent.queue` — queue workflows.
- `agent.priority` — prioritize workflows.
- `agent.lease` — lease a tab/profile to one workflow.
- `agent.lock` — lock a workflow resource.
- `agent.unlock` — release a workflow resource.
- `agent.subtask` — create a bounded subtask.
- `agent.merge_results` — merge structured results.
- `agent.deduplicate_results` — deduplicate records.
- `agent.resume_after_crash` — resume from a safe checkpoint.

### One-call contract

Conceptually:

```json
{
  "goal": "Find the newest matching videos and return title, channel, date, duration, and URL",
  "intent": "search_and_extract",
  "profile": "user-default",
  "policy": "full_access",
  "budget": {"max_model_turns": 2, "max_actions": 80, "max_seconds": 120},
  "output_schema": {
    "items": "array",
    "fields": ["title", "channel", "date", "duration", "url"]
  }
}
```

The model should receive the final structured result, compact progress, and evidence handles—not every intermediate node, screenshot, request, and unchanged DOM subtree.

---

## 20. Batch execution, synchronization, and recovery

### P0

- `batch.begin` — begin a batch.
- `batch.add` — add a typed action.
- `batch.commit` — execute the batch.
- `batch.preview` — preview actions and risk.
- `batch.stop_on_failure` — stop on first failure.
- `batch.continue_on_failure` — continue only for explicitly independent actions.
- `batch.transaction_status` — report action progress.
- `batch.rollback_ui` — return to a known UI state where possible.
- `batch.wait_between` — apply a bounded wait.
- `batch.wait_until` — wait for a condition between actions.
- `batch.verify_each` — verify every action.
- `batch.verify_end` — verify only the final postcondition.
- `batch.reobserve` — refresh state after a structural change.
- `batch.replan` — invoke bounded recovery.
- `batch.dedupe` — avoid duplicate actions.
- `batch.idempotency_key` — prevent accidental repeated side effects.

### P1

- `recovery.classify_error` — classify a failure.
- `recovery.stale_reference` — re-ground a stale reference.
- `recovery.page_changed` — detect unexpected page change.
- `recovery.timeout` — recover from timeout.
- `recovery.renderer_crash` — recover from renderer crash.
- `recovery.popup_blocked` — recover from popup blocking.
- `recovery.permission_denied` — request or report permission.
- `recovery.download_failed` — resume or report a failed download.
- `recovery.network_failed` — recover from network failure.
- `recovery.ambiguous_target` — pause or ask for target clarification.
- `recovery.false_completion` — detect that the task was not actually completed.
- `recovery.max_attempts` — stop after bounded attempts.

---

## 21. Research, extraction, and data workflows

### P0

- `extract.text` — extract clean readable text.
- `extract.article` — extract article title/author/date/body.
- `extract.list` — extract cards/list items.
- `extract.table` — extract tables into rows/columns.
- `extract.links` — extract links with metadata.
- `extract.images` — extract image URLs/alt/captions.
- `extract.media` — extract media URLs/metadata where exposed.
- `extract.metadata` — extract page metadata.
- `extract.json_ld` — extract JSON-LD.
- `extract.schema` — extract using a user-provided output schema.
- `extract.form_state` — extract form values/requirements/validation.
- `extract.network_data` — extract a structured response from permitted network activity.
- `extract.accessibility` — extract accessibility-relevant content.
- `extract.selection` — extract selected content.
- `extract.diff` — extract only changed content.
- `extract.citations` — associate output with page/artifact evidence.
- `collect.paginate` — traverse pagination.
- `collect.infinite_scroll` — collect infinite-scroll content.
- `collect.next_page` — detect and follow next-page controls.
- `collect.deduplicate` — deduplicate items by URL/key/content hash.
- `collect.rank` — rank records using declared criteria.
- `collect.filter` — filter records.
- `collect.sort` — sort records.
- `collect.limit` — enforce a result limit.
- `collect.resume` — resume collection from a checkpoint.
- `research.search` — perform a search workflow.
- `research.open_result` — open a result.
- `research.compare_sources` — compare sources.
- `research.save_results` — save structured results.

### P1

- `extract.readability` — convert a page to reader-friendly content.
- `extract.transcript` — extract captions/transcripts.
- `extract.comments` — extract comments/reviews.
- `extract_products` — extract product data.
- `extract_jobs` — extract job postings.
- `extract_profiles` — extract profile data.
- `extract_search_results` — extract search-result records.
- `research_cache.get` — retrieve cached research.
- `research_cache.put` — store permitted research.
- `research_cache.invalidate` — invalidate changed content.

---

## 22. Skills, memory, replay, and domain adapters

### P0

- `skill.discover` — discover the bundled Agentgo skill and capabilities.
- `skill.capabilities` — return capabilities relevant to an intent.
- `skill.help` — return concise usage help.
- `skill.example` — return a workflow example.
- `skill.validate` — validate a skill against the current API.
- `skill.version` — return skill version.
- `skill.run` — run a named skill.
- `skill.parameterize` — supply typed skill parameters.
- `skill.permission_requirements` — report required permissions.
- `memory.store` — store a permitted workflow fact.
- `memory.retrieve` — retrieve relevant workflow memory.
- `memory.forget` — delete selected memory.
- `memory.scope` — scope memory to profile/domain/user/task.
- `memory.redact` — redact sensitive values.
- `workflow.record` — record a successful workflow.
- `workflow.replay` — replay a verified workflow.
- `workflow.pause` — pause a replay.
- `workflow.resume` — resume a replay.
- `workflow.diff` — compare recorded workflow with current page.
- `workflow.verify` — verify page identity and postconditions before replay.
- `workflow.parameterize` — turn a workflow into a reusable template.
- `workflow.disable` — disable a broken workflow.

### P1

- `skill.install_local` — install a local skill after validation.
- `skill.remove_local` — remove a local skill.
- `skill.enable` — enable a skill.
- `skill.disable` — disable a skill.
- `skill.test` — run skill tests.
- `skill.compile` — compile a skill into deterministic primitives.
- `skill.domain_adapter` — register a domain-specific adapter.
- `skill.update` — update a skill with version checks.
- `memory.summarize` — compress old workflow memory.
- `memory.import` — import scoped memory.
- `memory.export` — export scoped memory.
- `workflow.heal` — repair a replay using current semantics.
- `workflow.approve` — approve a generated workflow for reuse.

### Safety

- A skill must declare permissions, domains, file scopes, side effects, and data handling.
- Page instructions cannot install or modify a skill.
- Replays must verify target identity and postconditions; recorded clicks alone are not trusted.

---

## 23. Approval modes, risk policy, and human handoff

### P0

- `policy.mode_get` — read Approval Mode or Full-Access Mode.
- `policy.mode_set` — change mode with explicit user control.
- `policy.domain_allowlist` — allow specific domains.
- `policy.domain_blocklist` — block domains.
- `policy.url_scope` — scope navigation to URL patterns.
- `policy.profile_scope` — scope access to selected profiles.
- `policy.file_scope` — scope filesystem paths.
- `policy.network_scope` — scope network destinations.
- `policy.permission_scope` — scope browser/device permissions.
- `policy.action_classify` — classify an action’s risk.
- `policy.preview_side_effect` — preview side effects.
- `policy.require_approval` — require approval for a class of action.
- `policy.approval_request` — request user approval.
- `policy.approval_grant` — grant approval for the exact requested scope.
- `policy.approval_deny` — deny an approval.
- `policy.approval_expire` — expire an approval.
- `policy.audit` — record policy decisions.
- `handoff.detect_login` — detect a login/auth gate.
- `handoff.detect_2fa` — detect a two-factor gate.
- `handoff.detect_captcha` — detect CAPTCHA/challenge pages.
- `handoff.detect_ambiguity` — detect an ambiguous identity/decision.
- `handoff.detect_high_impact` — detect a high-impact action.
- `handoff.pause_same_session` — pause the same tab/profile/session.
- `handoff.show_user_task` — show the user exactly what to do.
- `handoff.wait_user` — wait for the user.
- `handoff.verify_completion` — verify that the human step completed.
- `handoff.resume` — resume from the same state.
- `handoff.timeout` — time out a handoff safely.
- `handoff.cancel` — cancel a handoff.

### Required policy behavior

**Approval Mode:** pause for login/credential entry, 2FA, CAPTCHA, purchases, account/security changes, deletion, external messages, and final high-impact submissions unless the user explicitly configures a narrower or broader rule.

**Full-Access Mode:** continue through normal browsing, searching, navigation, form filling, uploads, downloads, and login according to the selected scopes. Still pause for:

- CAPTCHA/anti-bot challenge;
- purchases, subscriptions, transfers, donations, bids, or final checkout;
- any user-configured hard gate;
- a missing credential or 2FA secret that the browser vault cannot handle safely.

### Safety

- No CAPTCHA solver, challenge-answer guessing, proxy rotation, fingerprint spoofing, or anti-detect behavior.
- Human challenge handling must stay in the same browser session/context.
- Approval prompts must identify the origin, action, target, data, and irreversible effect.

---

## 24. Human handoff UI and Agentgo shell functions

### P0 Shell functions

- `shell.agent_status` — show active/paused/completed state.
- `shell.agent_goal` — show current goal.
- `shell.agent_step` — show the current high-level step.
- `shell.agent_activity` — show compact activity.
- `shell.agent_pause` — pause the agent.
- `shell.agent_resume` — resume the agent.
- `shell.agent_stop` — stop the agent.
- `shell.agent_approve` — approve an exact action.
- `shell.agent_deny` — deny an exact action.
- `shell.agent_edit_plan` — edit a proposed plan.
- `shell.agent_handoff` — display a human task.
- `shell.agent_return_control` — return control to the agent.
- `shell.agent_trace` — show action/evidence trace.
- `shell.agent_cost` — show token/time/action cost.
- `shell.agent_result` — show structured result.
- `shell.agent_artifacts` — show related files/screenshots/traces.
- `shell.agent_error` — show a recoverable error.
- `shell.agent_retry` — request retry.
- `shell.agent_replan` — request re-plan.
- `shell.command_bar` — enter an agent goal.
- `shell.quick_actions` — show safe contextual actions.
- `shell.tab_strip` — render tabs/groups/status.
- `shell.sidebar` — render workspaces, skills, artifacts, and activity.
- `shell.permission_center` — manage permissions.
- `shell.profile_switcher` — switch profiles.
- `shell.download_center` — manage downloads.
- `shell.history_center` — manage history.
- `shell.settings` — manage Agentgo settings.

### P1

- `shell.diff_view` — show what changed after an action.
- `shell.target_highlight` — show the target being acted on.
- `shell.action_preview` — show a preview before an action.
- `shell.page_outline` — show compact page structure.
- `shell.page_find` — search page state.
- `shell.artifact_preview` — preview an artifact.
- `shell.workflow_editor` — edit a reusable workflow.
- `shell.skill_browser` — browse skills.
- `shell.policy_editor` — edit policy.
- `shell.audit_viewer` — inspect audit history.

---

## 25. Artifacts, evidence, and context control

### P0

- `artifact.create` — create an artifact handle.
- `artifact.store` — store content outside model context.
- `artifact.read` — read an artifact by handle.
- `artifact.preview` — create a small preview.
- `artifact.metadata` — return size/type/hash/source metadata.
- `artifact.hash` — hash an artifact.
- `artifact.link` — link an artifact to a workflow/result.
- `artifact.list` — list artifacts by session/task.
- `artifact.search` — search artifacts.
- `artifact.download` — export an artifact.
- `artifact.delete` — delete an artifact according to retention policy.
- `artifact.redact` — redact secrets/personal data.
- `evidence.capture` — capture evidence for a completed action.
- `evidence.screenshot` — attach a screenshot/crop.
- `evidence.snapshot` — attach a semantic snapshot.
- `evidence.network` — attach a permitted network trace.
- `evidence.page_identity` — attach page identity evidence.
- `evidence.postcondition` — attach completion evidence.
- `context.compact` — compact old state.
- `context.relevance` — filter state to current intent.
- `context.delta` — return only changes.
- `context.reference` — return a handle instead of raw content.
- `context.budget` — enforce token/output budgets.
- `context.redact` — redact sensitive content.

### P1

- `artifact.bundle` — bundle related artifacts.
- `artifact.deduplicate` — deduplicate identical artifacts.
- `artifact.retention` — configure retention.
- `artifact.expire` — expire old artifacts.
- `evidence.compare` — compare evidence before/after.
- `evidence.citation` — create source citations.
- `context.cache` — cache stable page content.
- `context.invalidate` — invalidate stale cached content.

---

## 26. Native Windows control lane

This lane exists because a web protocol alone cannot operate every Windows UI surface, especially native file pickers and system dialogs.

### P0

- `native.window_list` — list visible/owned windows in the Agentgo scope.
- `native.window_focus` — focus a scoped window.
- `native.window_bounds` — read/set bounds.
- `native.ui_tree` — inspect the Windows UI Automation tree.
- `native.find` — find a native control by role/name/value.
- `native.click` — click a native control.
- `native.double_click` — double-click a native control.
- `native.type` — type into a native control.
- `native.key` — send a native key.
- `native.hotkey` — send a native shortcut.
- `native.read_value` — read a native control value.
- `native.select` — select a native item.
- `native.expand` — expand a native tree/list item.
- `native.collapse` — collapse a native tree/list item.
- `native.scroll` — scroll a native surface.
- `native.drag_drop` — perform native drag/drop.
- `native.clipboard` — use scoped native clipboard operations.
- `native.file_dialog` — detect/interact with file dialogs.
- `native.save_dialog` — detect/interact with save dialogs.
- `native.print_dialog` — detect/interact with print dialogs.
- `native.permission_dialog` — detect/interact with permission dialogs.
- `native.notification` — show an Agentgo notification.
- `native.screenshot` — capture a scoped native window.
- `native.accessibility` — expose native accessibility metadata.

### P1

- `native.system_tray` — manage the Agentgo tray icon.
- `native.global_hotkey` — register scoped global shortcuts.
- `native.taskbar` — manage taskbar state.
- `native.toast` — show a Windows toast.
- `native.open_with` — invoke an OS open-with flow.
- `native.external_app` — request an external app launch with approval.
- `native.window_snap` — use Windows snap layouts.
- `native.multi_monitor` — move windows across monitors.

### Safety

- Native control must remain scoped to Agentgo-owned windows and explicitly selected dialogs.
- Do not allow a page to select arbitrary native windows.
- Native paths and control values must be redacted where sensitive.

---

## 27. Accessibility and inclusive interaction

### P0

- `a11y.tree` — return accessibility tree.
- `a11y.role` — read role.
- `a11y.name` — read accessible name.
- `a11y.description` — read description.
- `a11y.value` — read accessible value.
- `a11y.state` — read checked/disabled/expanded/selected/pressed state.
- `a11y.bounds` — read accessible bounds.
- `a11y.actions` — list supported accessible actions.
- `a11y.perform` — perform an accessible action.
- `a11y.focus` — focus by accessible node.
- `a11y.live_regions` — observe live-region updates.
- `a11y.labels` — inspect labels and relationships.
- `a11y.landmarks` — inspect page landmarks.
- `a11y.heading_outline` — return heading hierarchy.

### P1

- `a11y.screen_reader_mode` — optimize output for screen-reader semantics.
- `a11y.high_contrast` — inspect/enable high contrast.
- `a11y.reduced_motion` — inspect/enable reduced motion.
- `a11y.font_scale` — read/set font scaling.
- `a11y.keyboard_only` — run keyboard-only workflows.
- `a11y.focus_order` — inspect tab/focus order.
- `a11y.contrast_check` — check selected contrast ratios.

---

## 28. Debugging, inspection, and diagnostics

### P0

- `debug.console` — read console logs.
- `debug.exceptions` — read page exceptions.
- `debug.network` — inspect network activity.
- `debug.dom` — inspect DOM.
- `debug.accessibility` — inspect accessibility.
- `debug.layout` — inspect layout.
- `debug.screenshot` — capture diagnostic screenshot.
- `debug.trace_start` — start a trace.
- `debug.trace_stop` — stop a trace.
- `debug.trace_export` — export trace.
- `debug.action_log` — read action log.
- `debug.page_events` — inspect page lifecycle events.
- `debug.performance` — inspect timing/CPU/memory metrics.
- `debug.storage` — inspect site storage.
- `debug.permissions` — inspect permissions.
- `debug.downloads` — inspect download state.

### P1

- `debug.devtools_open` — open DevTools.
- `debug.devtools_close` — close DevTools.
- `debug.breakpoint` — set a controlled breakpoint.
- `debug.profile_cpu` — profile CPU.
- `debug.profile_memory` — profile memory.
- `debug.coverage` — collect code coverage in test mode.
- `debug.performance_timeline` — collect performance timeline.
- `debug.replay_trace` — replay a trace against a fixture.

---

## 29. Metrics and evaluation

### P0

- `metrics.success` — record task success/failure.
- `metrics.success_per_token` — measure success divided by model tokens.
- `metrics.model_turns` — count model turns.
- `metrics.browser_actions` — count browser actions.
- `metrics.batch_size` — measure actions per batch.
- `metrics.latency` — measure task and step latency.
- `metrics.recovery_count` — count recovery attempts.
- `metrics.stale_reference_rate` — measure stale references.
- `metrics.false_completion_rate` — measure incorrect success claims.
- `metrics.handoff_rate` — measure human handoffs.
- `metrics.handoff_resolution_time` — measure time to resume.
- `metrics.native_dialog_success` — measure native dialog success.
- `metrics.upload_success` — measure uploads.
- `metrics.download_success` — measure downloads.
- `metrics.extraction_accuracy` — evaluate structured extraction.

### P1

- `benchmark.run` — run a benchmark suite.
- `benchmark.record` — record a benchmark trace.
- `benchmark.compare` — compare engines/runtimes/strategies.
- `benchmark.regression` — detect regressions.
- `benchmark.browsergym` — run BrowserGym-compatible tasks.
- `benchmark.custom_suite` — run Agentgo task suites.
- `benchmark.token_report` — produce a token-efficiency report.
- `benchmark.recovery_report` — produce a recovery report.
- `benchmark.policy_report` — evaluate policy enforcement.

### Required benchmark categories

- Search and structured extraction.
- Multi-page collection and deduplication.
- Complex forms and rich editors.
- Login with a pre-provisioned profile.
- Human handoff for CAPTCHA/2FA.
- Native Windows file picker.
- Downloads and artifact verification.
- Popups, dialogs, iframes, shadow DOM, and overlays.
- Media playback and transcript extraction.
- Stale-reference recovery.
- Page prompt injection resistance.
- Purchase flow stops before money movement.
- Success-per-token and time-to-result.

---

## 30. Agent control protocol and integration surface

These are protocol commands, not model-provider APIs. They exist only so an external agent can launch, observe, and control Agentgo consistently.

### P0

- `api.health` — health endpoint.
- `api.capabilities` — capability discovery.
- `api.schema` — return JSON schemas.
- `api.run` — submit a one-call workflow.
- `api.observe` — request an observation.
- `api.act` — submit an action.
- `api.events` — subscribe to events.
- `api.cancel` — cancel a request.
- `api.pause` — pause a request.
- `api.resume` — resume a request.
- `api.artifact` — retrieve artifact metadata/handles.
- `api.trace` — retrieve trace references.
- `api.policy` — inspect applicable policy.
- `api.version` — return protocol version.
- `api.error` — return typed errors.
- `api.idempotency` — prevent duplicate requests.

### P1

- `sdk.rust` — Rust SDK.
- `sdk.typescript` — TypeScript SDK.
- `sdk.python` — Python SDK.
- `sdk.mcp` — MCP adapter.
- `sdk.websocket` — WebSocket transport.
- `sdk.http` — HTTP transport.
- `sdk.local_ipc` — local IPC transport.
- `sdk.plugin` — plugin SDK.
- `sdk.skill` — skill authoring SDK.
- `sdk.test` — test harness SDK.

### Protocol requirements

- Typed arguments and typed results.
- Stable IDs for tabs, contexts, frames, elements, workflows, artifacts, and approvals.
- Structured errors with recovery hints.
- Event subscriptions instead of polling where possible.
- Output limits and pagination.
- Sensitive-field redaction.
- Version negotiation.
- Explicit capability discovery.
- Idempotency keys for side effects.
- Audit correlation IDs.

---

## 31. Security and trust boundaries

### Safety requirements

- Treat every web page, document, image, PDF, email, and downloaded file as untrusted input.
- Keep Agentgo shell content separate from page content and privileges.
- Enforce origin/frame/profile scopes at the native boundary.
- Use separate processes for browser, renderers, utilities, and the agent controller where practical.
- Keep credentials in the OS/browser vault and expose only typed vault operations.
- Redact secrets from logs, traces, screenshots, snapshots, network data, and model context.
- Require explicit user policy for cloud execution.
- Require approval for money movement regardless of mode.
- Pause for CAPTCHA/anti-bot challenges and hand them to the user.
- Do not implement CAPTCHA solving, anti-detect fingerprints, proxy rotation, or challenge bypass.
- Verify downloads before opening or executing them.
- Require signed updates and dependency review.
- Maintain a local audit log that the page cannot alter.
- Make every privileged operation explainable and attributable to a workflow.
- Make cancellation reliable and visible.
- Prefer bounded retries over infinite recovery loops.

---

## 32. Suggested first implementation slices

This is only a prioritization aid; it does not remove the rest of the inventory.

### Slice A — human parity

Browser lifecycle, profiles, windows, tabs, navigation, mouse, keyboard, clipboard, semantic observation, locators, forms, files, downloads, dialogs, permissions, and native Windows file dialogs.

### Slice B — agent advantage

Compact snapshots, stable references, diffs, intent filtering, batch actions, wait/settle, postconditions, bounded recovery, structured extraction, artifacts, and the bundled skill.

### Slice C — autonomy and safety

Approval modes, purchase hard-stop, CAPTCHA handoff, credential vault integration, audit trace, policy scopes, prompt-injection boundary, and resumable checkpoints.

### Slice D — serious browser product

Bookmarks, history, settings, downloads manager, themes, workspaces, extension permissions, media/device flows, DevTools, performance metrics, and update/rollback.

### Slice E — measurable superiority

BrowserGym/custom benchmarks, success-per-token dashboard, replay suite, real-site compatibility matrix, native-dialog suite, and challenge-handoff metrics.

---

## Research basis

This inventory is based on the downloaded Agentgo source archive plus the following primary protocol/runtime references:

- [CEF source](https://github.com/chromiumembedded/cef)
- [CEF documentation](https://chromiumembedded.github.io/cef/)
- [CEF general usage and application hooks](https://chromiumembedded.github.io/cef/general_usage.html)
- [CEF sample project](https://github.com/chromiumembedded/cef-project)
- [CEF Rust bindings](https://github.com/tauri-apps/cef-rs)
- [Chrome DevTools Protocol domains](https://chromedevtools.github.io/devtools-protocol/)
- [Chrome DevTools Protocol Input domain](https://chromedevtools.github.io/devtools-protocol/tot/Input/)
- [Chrome DevTools Protocol DOMSnapshot domain](https://chromedevtools.github.io/devtools-protocol/tot/DOMSnapshot/)
- [W3C WebDriver](https://www.w3.org/TR/webdriver/)
- [W3C WebDriver BiDi](https://www.w3.org/TR/webdriver-bidi/)
- [Microsoft Playwright browser support](https://playwright.dev/docs/browsers)
- [BrowserGym evaluation framework](https://github.com/ServiceNow/BrowserGym)
- [Windows Harness native Windows control](https://github.com/browser-use/windows-harness)
- [I Am Not A Robot human verification relay](https://github.com/mcpware/i-am-not-a-robot)

## Status

This is the living requirements catalog. New functions, safety rules, and design decisions should be appended or revised here as brainstorming continues. Nothing in this document means the function has already been implemented.

Latest architectural rule: Agentgo is agent-agnostic. External agents launch and control it through one universal local control protocol; Agentgo contains no required model-provider API or hidden AI service.
