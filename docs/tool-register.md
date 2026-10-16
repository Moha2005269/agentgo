# Named-tool register (copy)

Copied from Appendix A of the graduation roadmap so the list the browser is measured against lives with the code. `scripts/dev/coverage.js` reads this file.

<!-- TOOL_REGISTER_START -->

Inventory snapshot: **1039 unique named tools** across **30 domains**. Source SHA-256: `E1B9C7A6B30F39AF7064669243E3D8482C9B5721C42346BDAB61C4488052C8D1`. Completed tools at roadmap creation: **0 verified here**; this is not a claim that no implementation exists elsewhere.

### 1. Browser runtime and process lifecycle

Source category: **P0**

- [ ] `browser.launch`
- [ ] `browser.attach`
- [ ] `browser.detach`
- [ ] `browser.shutdown`
- [ ] `browser.restart`
- [ ] `browser.health`
- [ ] `browser.version`
- [ ] `browser.capabilities`
- [ ] `browser.feature_flags`
- [ ] `browser.command_line`
- [ ] `browser.processes`
- [ ] `browser.child_process_status`
- [ ] `browser.message_loop_status`
- [ ] `browser.crash_report`
- [ ] `browser.recover`

Source category: **P1**

- [ ] `browser.suspend`
- [ ] `browser.resume`
- [ ] `browser.low_power_mode`
- [ ] `browser.resource_budget`
- [ ] `browser.runtime_metrics`
- [ ] `browser.engine_matrix`

Source category: **P2**

- [ ] `browser.engine_switch`
- [ ] `browser.remote_instance`
- [ ] `browser.update_check`
- [ ] `browser.update_stage`
- [ ] `browser.update_rollback`

### 2. Profiles, user contexts, and session state

Source category: **P0**

- [ ] `profile.create`
- [ ] `profile.open`
- [ ] `profile.close`
- [ ] `profile.list`
- [ ] `profile.select`
- [ ] `profile.status`
- [ ] `profile.lock`
- [ ] `profile.unlock`
- [ ] `profile.clone`
- [ ] `profile.ephemeral`
- [ ] `profile.incognito`
- [ ] `profile.delete`
- [ ] `profile.reset_site`
- [ ] `profile.export_state`
- [ ] `profile.import_state`
- [ ] `profile.checkpoint`
- [ ] `profile.restore_checkpoint`
- [ ] `profile.session_list`
- [ ] `profile.session_save`
- [ ] `profile.session_resume`

Source category: **P1**

- [ ] `profile.cookie_policy`
- [ ] `profile.storage_partition`
- [ ] `profile.locale`
- [ ] `profile.timezone`
- [ ] `profile.accessibility_preferences`
- [ ] `profile.download_directory`
- [ ] `profile.proxy`
- [ ] `profile.permission_defaults`
- [ ] `profile.autofill_policy`
- [ ] `profile.password_vault_binding`

### 3. Windows, browser windows, and viewport control

Source category: **P0**

- [ ] `window.create`
- [ ] `window.close`
- [ ] `window.focus`
- [ ] `window.activate`
- [ ] `window.minimize`
- [ ] `window.maximize`
- [ ] `window.restore`
- [ ] `window.fullscreen`
- [ ] `window.move`
- [ ] `window.resize`
- [ ] `window.bounds`
- [ ] `window.monitor_list`
- [ ] `window.always_on_top`
- [ ] `window.visibility`
- [ ] `window.title`
- [ ] `window.native_handle`
- [ ] `window.capture`
- [ ] `window.set_zoom`
- [ ] `window.reset_zoom`
- [ ] `viewport.size`
- [ ] `viewport.set_size`
- [ ] `viewport.device_scale`
- [ ] `viewport.set_device_scale`

Source category: **P1**

- [ ] `window.split`
- [ ] `window.workspace`
- [ ] `window.detach_tab`
- [ ] `window.attach_tab`
- [ ] `window.presentation_mode`
- [ ] `window.agent_overlay`
- [ ] `window.always_visible_prompt`

### 4. Tabs, frames, browsing contexts, and popups

Source category: **P0**

- [ ] `tab.create`
- [ ] `tab.close`
- [ ] `tab.activate`
- [ ] `tab.list`
- [ ] `tab.status`
- [ ] `tab.navigate`
- [ ] `tab.reload`
- [ ] `tab.stop`
- [ ] `tab.back`
- [ ] `tab.forward`
- [ ] `tab.duplicate`
- [ ] `tab.reopen_closed`
- [ ] `tab.move`
- [ ] `tab.group`
- [ ] `tab.ungroup`
- [ ] `tab.pin`
- [ ] `tab.unpin`
- [ ] `tab.mute`
- [ ] `tab.discard`
- [ ] `tab.restore`
- [ ] `tab.freeze`
- [ ] `tab.unfreeze`
- [ ] `tab.open_popup`
- [ ] `tab.close_popup`
- [ ] `tab.opener`
- [ ] `tab.target`
- [ ] `frame.list`
- [ ] `frame.select`
- [ ] `frame.parent`
- [ ] `frame.children`
- [ ] `frame.focus`
- [ ] `frame.navigate`
- [ ] `frame.wait`
- [ ] `browsing_context.tree`
- [ ] `browsing_context.create`
- [ ] `browsing_context.close`
- [ ] `browsing_context.activate`

Source category: **P1**

- [ ] `tab.search`
- [ ] `tab.sort`
- [ ] `tab.save_session`
- [ ] `tab.restore_session`
- [ ] `tab.freeze_all_background`
- [ ] `tab.close_others`
- [ ] `tab.close_to_right`
- [ ] `tab.suspend_duplicates`
- [ ] `frame.snapshot`

### 5. Navigation, loading, and synchronization

Source category: **P0**

- [ ] `navigate.url`
- [ ] `navigate.search`
- [ ] `navigate.link`
- [ ] `navigate.external`
- [ ] `navigate.history`
- [ ] `navigate.history_back`
- [ ] `navigate.history_forward`
- [ ] `navigate.reload`
- [ ] `navigate.hard_reload`
- [ ] `navigate.stop`
- [ ] `navigate.cancel`
- [ ] `navigate.redirect_chain`
- [ ] `navigate.current`
- [ ] `wait.load`
- [ ] `wait.dom_ready`
- [ ] `wait.network_idle`
- [ ] `wait.selector`
- [ ] `wait.text`
- [ ] `wait.role`
- [ ] `wait.visible`
- [ ] `wait.hidden`
- [ ] `wait.enabled`
- [ ] `wait.stable`
- [ ] `wait.download`
- [ ] `wait.popup`
- [ ] `wait.dialog`
- [ ] `wait.media`
- [ ] `wait.condition`
- [ ] `wait.settle`

Source category: **P1**

- [ ] `navigate.prerender_status`
- [ ] `navigate.service_worker_status`
- [ ] `navigate.offline`
- [ ] `navigate.throttle`
- [ ] `navigate.retry`
- [ ] `navigate.resume_after_interruption`

### 6. Human-equivalent mouse, pointer, touch, and gesture input

Source category: **P0 mouse/pointer**

- [ ] `pointer.move`
- [ ] `pointer.down`
- [ ] `pointer.up`
- [ ] `pointer.click`
- [ ] `pointer.double_click`
- [ ] `pointer.triple_click`
- [ ] `pointer.right_click`
- [ ] `pointer.middle_click`
- [ ] `pointer.hover`
- [ ] `pointer.drag_start`
- [ ] `pointer.drag_move`
- [ ] `pointer.drag_end`
- [ ] `pointer.drag_to`
- [ ] `pointer.scroll`
- [ ] `pointer.scroll_to`
- [ ] `pointer.scroll_into_view`
- [ ] `pointer.wheel`
- [ ] `pointer.position`
- [ ] `pointer.button_state`
- [ ] `pointer.modifiers`
- [ ] `pointer.hit_test`
- [ ] `pointer.capture`

Source category: **P0 keyboard**

- [ ] `keyboard.key_down`
- [ ] `keyboard.key_up`
- [ ] `keyboard.press`
- [ ] `keyboard.type`
- [ ] `keyboard.insert_text`
- [ ] `keyboard.shortcut`
- [ ] `keyboard.chord`
- [ ] `keyboard.tab`
- [ ] `keyboard.shift_tab`
- [ ] `keyboard.enter`
- [ ] `keyboard.escape`
- [ ] `keyboard.home`
- [ ] `keyboard.end`
- [ ] `keyboard.page_up`
- [ ] `keyboard.page_down`
- [ ] `keyboard.arrow`
- [ ] `keyboard.delete`
- [ ] `keyboard.backspace`
- [ ] `keyboard.select_all`
- [ ] `keyboard.undo`
- [ ] `keyboard.redo`
- [ ] `keyboard.composition_start`
- [ ] `keyboard.composition_update`
- [ ] `keyboard.composition_end`
- [ ] `keyboard.layout`
- [ ] `keyboard.focused_element`

Source category: **P1 touch/pen/gesture**

- [ ] `touch.start`
- [ ] `touch.move`
- [ ] `touch.end`
- [ ] `touch.cancel`
- [ ] `touch.tap`
- [ ] `touch.long_press`
- [ ] `touch.swipe`
- [ ] `touch.pinch`
- [ ] `touch.rotate`
- [ ] `pen.move`
- [ ] `pen.down`
- [ ] `pen.up`
- [ ] `pen.pressure`
- [ ] `gesture.back`
- [ ] `gesture.forward`

### 7. Clipboard, selection, focus, and editing

Source category: **P0**

- [ ] `focus.element`
- [ ] `focus.window`
- [ ] `focus.frame`
- [ ] `focus.read`
- [ ] `focus.blur`
- [ ] `selection.read`
- [ ] `selection.set`
- [ ] `selection.extend`
- [ ] `selection.clear`
- [ ] `clipboard.read_text`
- [ ] `clipboard.write_text`
- [ ] `clipboard.read_html`
- [ ] `clipboard.write_html`
- [ ] `clipboard.read_image`
- [ ] `clipboard.write_image`
- [ ] `clipboard.clear`
- [ ] `edit.clear`
- [ ] `edit.fill`
- [ ] `edit.append`
- [ ] `edit.replace_selection`
- [ ] `edit.undo`
- [ ] `edit.redo`
- [ ] `edit.copy`
- [ ] `edit.cut`
- [ ] `edit.paste`
- [ ] `edit.contenteditable`

Source category: **P1**

- [ ] `edit.rich_text`
- [ ] `edit.markdown`
- [ ] `edit.code_editor`
- [ ] `edit.autocomplete`
- [ ] `edit.spellcheck`
- [ ] `edit.input_method`

### 8. Semantic page observation and visual grounding

Source category: **P0**

- [ ] `observe.page`
- [ ] `observe.outline`
- [ ] `observe.accessibility_tree`
- [ ] `observe.dom_snapshot`
- [ ] `observe.dom_subtree`
- [ ] `observe.layout`
- [ ] `observe.text`
- [ ] `observe.content`
- [ ] `observe.interactive`
- [ ] `observe.forms`
- [ ] `observe.tables`
- [ ] `observe.lists`
- [ ] `observe.links`
- [ ] `observe.images`
- [ ] `observe.media`
- [ ] `observe.dialogs`
- [ ] `observe.overlays`
- [ ] `observe.focus`
- [ ] `observe.selection`
- [ ] `observe.frames`
- [ ] `observe.shadow_roots`
- [ ] `observe.iframe_content`
- [ ] `observe.page_metadata`
- [ ] `observe.favicon`
- [ ] `observe.viewport`
- [ ] `observe.screenshot`
- [ ] `observe.full_page_screenshot`
- [ ] `observe.crop`
- [ ] `observe.ocr`
- [ ] `observe.hit_test`

Source category: **P0 state efficiency**

- [ ] `state.create_baseline`
- [ ] `state.fingerprint`
- [ ] `state.diff`
- [ ] `state.delta_after_action`
- [ ] `state.ref_create`
- [ ] `state.ref_resolve`
- [ ] `state.ref_validate`
- [ ] `state.ref_heal`
- [ ] `state.ref_expire`
- [ ] `state.relevance_filter`
- [ ] `state.region_focus`
- [ ] `state.content_hash`
- [ ] `state.page_identity`
- [ ] `state.snapshot_store`
- [ ] `state.snapshot_load`

Source category: **P1 visual/diagnostic observation**

- [ ] `observe.cursor`
- [ ] `observe.animation`
- [ ] `observe.canvas`
- [ ] `observe.computed_style`
- [ ] `observe.paint_regions`
- [ ] `observe.scroll_containers`
- [ ] `observe.fixed_elements`
- [ ] `observe.loading_indicators`
- [ ] `observe.visual_diff`
- [ ] `observe.record_video`

### 9. Locators, grounding, and target resolution

Source category: **P0**

- [ ] `locate.role`
- [ ] `locate.name`
- [ ] `locate.label`
- [ ] `locate.placeholder`
- [ ] `locate.text`
- [ ] `locate.text_pattern`
- [ ] `locate.link`
- [ ] `locate.button`
- [ ] `locate.field`
- [ ] `locate.checkbox`
- [ ] `locate.radio`
- [ ] `locate.select`
- [ ] `locate.file_input`
- [ ] `locate.test_id`
- [ ] `locate.css`
- [ ] `locate.xpath`
- [ ] `locate.dom_ref`
- [ ] `locate.ax_ref`
- [ ] `locate.coordinate`
- [ ] `locate.region`
- [ ] `locate.frame`
- [ ] `locate.shadow`
- [ ] `locate.nth`
- [ ] `locate.filter`
- [ ] `locate.closest`
- [ ] `locate.descendant`
- [ ] `locate.all`
- [ ] `locate.unique`
- [ ] `locate.explain`

Source category: **P1**

- [ ] `ground.semantic`
- [ ] `ground.visual`
- [ ] `ground.hybrid`
- [ ] `ground.reobserve`
- [ ] `ground.compare_candidates`
- [ ] `ground.confidence`
- [ ] `ground.stale_recovery`

### 10. Page actions and form operations

Source category: **P0**

- [ ] `act.click`
- [ ] `act.double_click`
- [ ] `act.right_click`
- [ ] `act.hover`
- [ ] `act.focus`
- [ ] `act.type`
- [ ] `act.fill`
- [ ] `act.clear`
- [ ] `act.select_option`
- [ ] `act.check`
- [ ] `act.uncheck`
- [ ] `act.toggle`
- [ ] `act.choose_radio`
- [ ] `act.press`
- [ ] `act.submit`
- [ ] `act.scroll`
- [ ] `act.scroll_page`
- [ ] `act.drag_drop`
- [ ] `act.drag_file`
- [ ] `act.upload`
- [ ] `act.download`
- [ ] `act.open`
- [ ] `act.close`
- [ ] `act.expand`
- [ ] `act.collapse`
- [ ] `act.play`
- [ ] `act.pause`
- [ ] `act.seek`
- [ ] `act.set_value`
- [ ] `act.dispatch_input`
- [ ] `act.wait_and_click`
- [ ] `act.click_and_verify`

Source category: **P1 form-specific operations**

- [ ] `form.discover`
- [ ] `form.map`
- [ ] `form.fill`
- [ ] `form.fill_batch`
- [ ] `form.validate`
- [ ] `form.invalid_fields`
- [ ] `form.required_fields`
- [ ] `form.submit_draft`
- [ ] `form.submit`
- [ ] `form.confirm`
- [ ] `form.review`
- [ ] `form.reset`
- [ ] `form.autofill`
- [ ] `form.date`
- [ ] `form.time`
- [ ] `form.datetime`
- [ ] `form.number`
- [ ] `form.color`
- [ ] `form.file`
- [ ] `form.rich_text`
- [ ] `form.multi_select`
- [ ] `form.combobox`
- [ ] `form.autocomplete`
- [ ] `form.slider`

### 11. JavaScript, DOM, and page-runtime control

Source category: **P0**

- [ ] `runtime.evaluate`
- [ ] `runtime.evaluate_isolated`
- [ ] `runtime.call_function`
- [ ] `runtime.read_property`
- [ ] `runtime.query`
- [ ] `runtime.query_all`
- [ ] `runtime.get_attribute`
- [ ] `runtime.get_property`
- [ ] `runtime.set_property`
- [ ] `runtime.get_text`
- [ ] `runtime.get_html`
- [ ] `runtime.get_value`
- [ ] `runtime.get_rect`
- [ ] `runtime.add_init_script`
- [ ] `runtime.remove_init_script`
- [ ] `runtime.add_binding`
- [ ] `runtime.remove_binding`
- [ ] `runtime.message`
- [ ] `runtime.promise`
- [ ] `runtime.console`
- [ ] `runtime.exception`

### 12. Files, native dialogs, downloads, and printing

Source category: **P0**

- [ ] `file.choose`
- [ ] `file.choose_multiple`
- [ ] `file.choose_directory`
- [ ] `file.upload`
- [ ] `file.drag_to_upload`
- [ ] `file.dialog_open`
- [ ] `file.dialog_save`
- [ ] `file.dialog_cancel`
- [ ] `file.dialog_confirm`
- [ ] `file.path_validate`
- [ ] `file.metadata`
- [ ] `file.hash`
- [ ] `file.read_text`
- [ ] `file.read_bytes`
- [ ] `file.write_text`
- [ ] `file.copy`
- [ ] `file.move`
- [ ] `file.rename`
- [ ] `download.start`
- [ ] `download.list`
- [ ] `download.pause`
- [ ] `download.resume`
- [ ] `download.cancel`
- [ ] `download.open`
- [ ] `download.show`
- [ ] `download.destination`
- [ ] `download.progress`
- [ ] `download.verify`
- [ ] `print.preview`
- [ ] `print.to_pdf`
- [ ] `print.to_printer`
- [ ] `page.save`
- [ ] `page.save_snapshot`

Source category: **P1**

- [ ] `file.archive`
- [ ] `file.extract_archive`
- [ ] `download.resume_after_restart`
- [ ] `download.deduplicate`
- [ ] `download.quarantine`
- [ ] `print.selection`
- [ ] `print.backgrounds`

### 13. Browser dialogs, menus, prompts, and permissions

Source category: **P0**

- [ ] `dialog.list`
- [ ] `dialog.alert`
- [ ] `dialog.confirm`
- [ ] `dialog.prompt`
- [ ] `dialog.before_unload`
- [ ] `dialog.javascript`
- [ ] `dialog.file`
- [ ] `dialog.auth`
- [ ] `dialog.permission`
- [ ] `dialog.close`
- [ ] `menu.open`
- [ ] `menu.close`
- [ ] `menu.select`
- [ ] `context_menu.open`
- [ ] `context_menu.select`
- [ ] `context_menu.dismiss`
- [ ] `permission.list`
- [ ] `permission.grant`
- [ ] `permission.deny`
- [ ] `permission.reset`
- [ ] `permission.explain`

Source category: **P1**

- [ ] `dialog.print`
- [ ] `dialog.download`
- [ ] `dialog.install`
- [ ] `dialog.external_protocol`
- [ ] `dialog.certificate`
- [ ] `dialog.fullscreen`

### 14. Storage, cookies, credentials, and site data

Source category: **P0**

- [ ] `storage.cookies_list`
- [ ] `storage.cookie_get`
- [ ] `storage.cookie_set`
- [ ] `storage.cookie_delete`
- [ ] `storage.cookies_clear`
- [ ] `storage.local_get`
- [ ] `storage.local_set`
- [ ] `storage.local_delete`
- [ ] `storage.local_clear`
- [ ] `storage.session_get`
- [ ] `storage.session_set`
- [ ] `storage.session_clear`
- [ ] `storage.indexeddb_list`
- [ ] `storage.indexeddb_export`
- [ ] `storage.indexeddb_clear`
- [ ] `storage.cache_list`
- [ ] `storage.cache_export`
- [ ] `storage.cache_clear`
- [ ] `storage.service_workers`
- [ ] `storage.service_worker_unregister`
- [ ] `storage.site_data_clear`
- [ ] `storage.site_data_export`
- [ ] `storage.permissions`

Source category: **P1**

- [ ] `credential.vault_status`
- [ ] `credential.autofill`
- [ ] `credential.save`
- [ ] `credential.delete`
- [ ] `credential.revoke_session`
- [ ] `credential.redact`

### 15. Network, requests, responses, and web platform events

Source category: **P0**

- [ ] `network.observe_requests`
- [ ] `network.observe_responses`
- [ ] `network.request_body`
- [ ] `network.response_body`
- [ ] `network.headers`
- [ ] `network.timing`
- [ ] `network.failures`
- [ ] `network.redirects`
- [ ] `network.block`
- [ ] `network.allow`
- [ ] `network.continue`
- [ ] `network.fulfill`
- [ ] `network.modify_headers`
- [ ] `network.modify_response`
- [ ] `network.intercept`
- [ ] `network.clear_interceptors`
- [ ] `network.cache_status`
- [ ] `network.clear_cache`
- [ ] `network.offline`
- [ ] `network.online`
- [ ] `network.websocket_list`
- [ ] `network.websocket_messages`
- [ ] `network.sse_events`
- [ ] `network.fetch`
- [ ] `network.download_response`
- [ ] `network.har_start`
- [ ] `network.har_stop`

Source category: **P1**

- [ ] `network.emulate_latency`
- [ ] `network.emulate_bandwidth`
- [ ] `network.emulate_errors`
- [ ] `network.dns_status`
- [ ] `network.connection_status`
- [ ] `network.service_worker_events`
- [ ] `network.background_tasks`
- [ ] `network.replay`

### 16. Media, camera, microphone, sensors, and devices

Source category: **P0**

- [ ] `media.list`
- [ ] `media.play`
- [ ] `media.pause`
- [ ] `media.stop`
- [ ] `media.seek`
- [ ] `media.volume`
- [ ] `media.mute`
- [ ] `media.speed`
- [ ] `media.fullscreen`
- [ ] `media.picture_in_picture`
- [ ] `media.tracks`
- [ ] `media.select_track`
- [ ] `media.capture_frame`
- [ ] `media.capture_stream`
- [ ] `media.status`
- [ ] `media.transcript`

Source category: **P1**

- [ ] `device.camera_list`
- [ ] `device.microphone_list`
- [ ] `device.output_list`
- [ ] `device.camera_select`
- [ ] `device.microphone_select`
- [ ] `device.output_select`
- [ ] `device.geolocation`
- [ ] `device.orientation`
- [ ] `device.bluetooth`
- [ ] `device.usb`
- [ ] `device.serial`
- [ ] `device.hid`
- [ ] `device.nfc`
- [ ] `device.web_authn`

### 17. Human browser chrome and product features

Source category: **P0**

- [ ] `omnibox.focus`
- [ ] `omnibox.read`
- [ ] `omnibox.type`
- [ ] `omnibox.submit`
- [ ] `omnibox.clear`
- [ ] `omnibox.suggestions`
- [ ] `omnibox.select_suggestion`
- [ ] `browser_ui.back`
- [ ] `browser_ui.forward`
- [ ] `browser_ui.reload`
- [ ] `browser_ui.stop`
- [ ] `browser_ui.new_tab`
- [ ] `browser_ui.close_tab`
- [ ] `browser_ui.tab_switch`
- [ ] `browser_ui.menu`
- [ ] `browser_ui.find`
- [ ] `browser_ui.find_next`
- [ ] `browser_ui.find_previous`
- [ ] `browser_ui.zoom_in`
- [ ] `browser_ui.zoom_out`
- [ ] `browser_ui.zoom_reset`
- [ ] `browser_ui.fullscreen`
- [ ] `browser_ui.devtools`
- [ ] `browser_ui.reader_mode`
- [ ] `browser_ui.translate`
- [ ] `browser_ui.view_source`
- [ ] `browser_ui.save_page`
- [ ] `browser_ui.print`
- [ ] `browser_ui.downloads`
- [ ] `browser_ui.history`
- [ ] `browser_ui.bookmarks`

Source category: **P1**

- [ ] `bookmark.add`
- [ ] `bookmark.remove`
- [ ] `bookmark.edit`
- [ ] `bookmark.move`
- [ ] `bookmark.search`
- [ ] `bookmark.open`
- [ ] `history.list`
- [ ] `history.search`
- [ ] `history.delete`
- [ ] `history.clear`
- [ ] `download_manager.open`
- [ ] `download_manager.search`
- [ ] `download_manager.remove`
- [ ] `site_info.open`
- [ ] `site_info.permissions`
- [ ] `browser_settings.open`
- [ ] `browser_settings.read`
- [ ] `browser_settings.update`
- [ ] `browser_shortcut.list`
- [ ] `browser_shortcut.configure`
- [ ] `theme.get`
- [ ] `theme.set`
- [ ] `appearance.fonts`
- [ ] `appearance.contrast`
- [ ] `appearance.motion`

Source category: **P2**

- [ ] `sync.status`
- [ ] `sync.start`
- [ ] `sync.stop`
- [ ] `web_app.install`
- [ ] `web_app.uninstall`
- [ ] `browser.profile_switcher`
- [ ] `browser.task_manager`

### 18. Extensions, scripts, and Agentgo plugins

Source category: **P1**

- [ ] `extension.list`
- [ ] `extension.install`
- [ ] `extension.uninstall`
- [ ] `extension.enable`
- [ ] `extension.disable`
- [ ] `extension.permissions`
- [ ] `extension.permission_request`
- [ ] `extension.content_scripts`
- [ ] `extension.browser_actions`
- [ ] `extension.context_menu`
- [ ] `extension.storage`
- [ ] `extension.reload`
- [ ] `script.user_list`
- [ ] `script.user_install`
- [ ] `script.user_remove`
- [ ] `script.user_enable`
- [ ] `script.user_disable`
- [ ] `native_messaging.list`
- [ ] `native_messaging.invoke`

### 19. Agent one-call execution and workflow control

Source category: **P0**

- [ ] `agent.run`
- [ ] `agent.plan`
- [ ] `agent.preview`
- [ ] `agent.observe`
- [ ] `agent.act`
- [ ] `agent.batch`
- [ ] `agent.extract`
- [ ] `agent.search_and_extract`
- [ ] `agent.fill_and_review`
- [ ] `agent.navigate_and_wait`
- [ ] `agent.collect`
- [ ] `agent.compare`
- [ ] `agent.monitor`
- [ ] `agent.pause`
- [ ] `agent.resume`
- [ ] `agent.cancel`
- [ ] `agent.checkpoint`
- [ ] `agent.restore`
- [ ] `agent.retry`
- [ ] `agent.replan`
- [ ] `agent.verify`
- [ ] `agent.assert`
- [ ] `agent.progress`
- [ ] `agent.status`
- [ ] `agent.result`
- [ ] `agent.evidence`
- [ ] `agent.explain`
- [ ] `agent.budget`
- [ ] `agent.dry_run`
- [ ] `agent.handoff`
- [ ] `agent.resume_after_handoff`

Source category: **P1**

- [ ] `agent.parallel_tabs`
- [ ] `agent.parallel_profiles`
- [ ] `agent.queue`
- [ ] `agent.priority`
- [ ] `agent.lease`
- [ ] `agent.lock`
- [ ] `agent.unlock`
- [ ] `agent.subtask`
- [ ] `agent.merge_results`
- [ ] `agent.deduplicate_results`
- [ ] `agent.resume_after_crash`

### 20. Batch execution, synchronization, and recovery

Source category: **P0**

- [ ] `batch.begin`
- [ ] `batch.add`
- [ ] `batch.commit`
- [ ] `batch.preview`
- [ ] `batch.stop_on_failure`
- [ ] `batch.continue_on_failure`
- [ ] `batch.transaction_status`
- [ ] `batch.rollback_ui`
- [ ] `batch.wait_between`
- [ ] `batch.wait_until`
- [ ] `batch.verify_each`
- [ ] `batch.verify_end`
- [ ] `batch.reobserve`
- [ ] `batch.replan`
- [ ] `batch.dedupe`
- [ ] `batch.idempotency_key`

Source category: **P1**

- [ ] `recovery.classify_error`
- [ ] `recovery.stale_reference`
- [ ] `recovery.page_changed`
- [ ] `recovery.timeout`
- [ ] `recovery.renderer_crash`
- [ ] `recovery.popup_blocked`
- [ ] `recovery.permission_denied`
- [ ] `recovery.download_failed`
- [ ] `recovery.network_failed`
- [ ] `recovery.ambiguous_target`
- [ ] `recovery.false_completion`
- [ ] `recovery.max_attempts`

### 21. Research, extraction, and data workflows

Source category: **P0**

- [ ] `extract.text`
- [ ] `extract.article`
- [ ] `extract.list`
- [ ] `extract.table`
- [ ] `extract.links`
- [ ] `extract.images`
- [ ] `extract.media`
- [ ] `extract.metadata`
- [ ] `extract.json_ld`
- [ ] `extract.schema`
- [ ] `extract.form_state`
- [ ] `extract.network_data`
- [ ] `extract.accessibility`
- [ ] `extract.selection`
- [ ] `extract.diff`
- [ ] `extract.citations`
- [ ] `collect.paginate`
- [ ] `collect.infinite_scroll`
- [ ] `collect.next_page`
- [ ] `collect.deduplicate`
- [ ] `collect.rank`
- [ ] `collect.filter`
- [ ] `collect.sort`
- [ ] `collect.limit`
- [ ] `collect.resume`
- [ ] `research.search`
- [ ] `research.open_result`
- [ ] `research.compare_sources`
- [ ] `research.save_results`

Source category: **P1**

- [ ] `extract.readability`
- [ ] `extract.transcript`
- [ ] `extract.comments`
- [ ] `research_cache.get`
- [ ] `research_cache.put`
- [ ] `research_cache.invalidate`

### 22. Skills, memory, replay, and domain adapters

Source category: **P0**

- [ ] `skill.discover`
- [ ] `skill.capabilities`
- [ ] `skill.help`
- [ ] `skill.example`
- [ ] `skill.validate`
- [ ] `skill.version`
- [ ] `skill.run`
- [ ] `skill.parameterize`
- [ ] `skill.permission_requirements`
- [ ] `memory.store`
- [ ] `memory.retrieve`
- [ ] `memory.forget`
- [ ] `memory.scope`
- [ ] `memory.redact`
- [ ] `workflow.record`
- [ ] `workflow.replay`
- [ ] `workflow.pause`
- [ ] `workflow.resume`
- [ ] `workflow.diff`
- [ ] `workflow.verify`
- [ ] `workflow.parameterize`
- [ ] `workflow.disable`

Source category: **P1**

- [ ] `skill.install_local`
- [ ] `skill.remove_local`
- [ ] `skill.enable`
- [ ] `skill.disable`
- [ ] `skill.test`
- [ ] `skill.compile`
- [ ] `skill.domain_adapter`
- [ ] `skill.update`
- [ ] `memory.summarize`
- [ ] `memory.import`
- [ ] `memory.export`
- [ ] `workflow.heal`
- [ ] `workflow.approve`

### 23. Approval modes, risk policy, and human handoff

Source category: **P0**

- [ ] `policy.mode_get`
- [ ] `policy.mode_set`
- [ ] `policy.domain_allowlist`
- [ ] `policy.domain_blocklist`
- [ ] `policy.url_scope`
- [ ] `policy.profile_scope`
- [ ] `policy.file_scope`
- [ ] `policy.network_scope`
- [ ] `policy.permission_scope`
- [ ] `policy.action_classify`
- [ ] `policy.preview_side_effect`
- [ ] `policy.require_approval`
- [ ] `policy.approval_request`
- [ ] `policy.approval_grant`
- [ ] `policy.approval_deny`
- [ ] `policy.approval_expire`
- [ ] `policy.audit`
- [ ] `handoff.detect_login`
- [ ] `handoff.detect_2fa`
- [ ] `handoff.detect_captcha`
- [ ] `handoff.detect_ambiguity`
- [ ] `handoff.detect_high_impact`
- [ ] `handoff.pause_same_session`
- [ ] `handoff.show_user_task`
- [ ] `handoff.wait_user`
- [ ] `handoff.verify_completion`
- [ ] `handoff.resume`
- [ ] `handoff.timeout`
- [ ] `handoff.cancel`

### 24. Human handoff UI and Agentgo shell functions

Source category: **P0 Shell functions**

- [ ] `shell.agent_status`
- [ ] `shell.agent_goal`
- [ ] `shell.agent_step`
- [ ] `shell.agent_activity`
- [ ] `shell.agent_pause`
- [ ] `shell.agent_resume`
- [ ] `shell.agent_stop`
- [ ] `shell.agent_approve`
- [ ] `shell.agent_deny`
- [ ] `shell.agent_edit_plan`
- [ ] `shell.agent_handoff`
- [ ] `shell.agent_return_control`
- [ ] `shell.agent_trace`
- [ ] `shell.agent_cost`
- [ ] `shell.agent_result`
- [ ] `shell.agent_artifacts`
- [ ] `shell.agent_error`
- [ ] `shell.agent_retry`
- [ ] `shell.agent_replan`
- [ ] `shell.command_bar`
- [ ] `shell.quick_actions`
- [ ] `shell.tab_strip`
- [ ] `shell.sidebar`
- [ ] `shell.permission_center`
- [ ] `shell.profile_switcher`
- [ ] `shell.download_center`
- [ ] `shell.history_center`
- [ ] `shell.settings`

Source category: **P1**

- [ ] `shell.diff_view`
- [ ] `shell.target_highlight`
- [ ] `shell.action_preview`
- [ ] `shell.page_outline`
- [ ] `shell.page_find`
- [ ] `shell.artifact_preview`
- [ ] `shell.workflow_editor`
- [ ] `shell.skill_browser`
- [ ] `shell.policy_editor`
- [ ] `shell.audit_viewer`

### 25. Artifacts, evidence, and context control

Source category: **P0**

- [ ] `artifact.create`
- [ ] `artifact.store`
- [ ] `artifact.read`
- [ ] `artifact.preview`
- [ ] `artifact.metadata`
- [ ] `artifact.hash`
- [ ] `artifact.link`
- [ ] `artifact.list`
- [ ] `artifact.search`
- [ ] `artifact.download`
- [ ] `artifact.delete`
- [ ] `artifact.redact`
- [ ] `evidence.capture`
- [ ] `evidence.screenshot`
- [ ] `evidence.snapshot`
- [ ] `evidence.network`
- [ ] `evidence.page_identity`
- [ ] `evidence.postcondition`
- [ ] `context.compact`
- [ ] `context.relevance`
- [ ] `context.delta`
- [ ] `context.reference`
- [ ] `context.budget`
- [ ] `context.redact`

Source category: **P1**

- [ ] `artifact.bundle`
- [ ] `artifact.deduplicate`
- [ ] `artifact.retention`
- [ ] `artifact.expire`
- [ ] `evidence.compare`
- [ ] `evidence.citation`
- [ ] `context.cache`
- [ ] `context.invalidate`

### 26. Native Windows control lane

Source category: **P0**

- [ ] `native.window_list`
- [ ] `native.window_focus`
- [ ] `native.window_bounds`
- [ ] `native.ui_tree`
- [ ] `native.find`
- [ ] `native.click`
- [ ] `native.double_click`
- [ ] `native.type`
- [ ] `native.key`
- [ ] `native.hotkey`
- [ ] `native.read_value`
- [ ] `native.select`
- [ ] `native.expand`
- [ ] `native.collapse`
- [ ] `native.scroll`
- [ ] `native.drag_drop`
- [ ] `native.clipboard`
- [ ] `native.file_dialog`
- [ ] `native.save_dialog`
- [ ] `native.print_dialog`
- [ ] `native.permission_dialog`
- [ ] `native.notification`
- [ ] `native.screenshot`
- [ ] `native.accessibility`

Source category: **P1**

- [ ] `native.system_tray`
- [ ] `native.global_hotkey`
- [ ] `native.taskbar`
- [ ] `native.toast`
- [ ] `native.open_with`
- [ ] `native.external_app`
- [ ] `native.window_snap`
- [ ] `native.multi_monitor`

### 27. Accessibility and inclusive interaction

Source category: **P0**

- [ ] `a11y.tree`
- [ ] `a11y.role`
- [ ] `a11y.name`
- [ ] `a11y.description`
- [ ] `a11y.value`
- [ ] `a11y.state`
- [ ] `a11y.bounds`
- [ ] `a11y.actions`
- [ ] `a11y.perform`
- [ ] `a11y.focus`
- [ ] `a11y.live_regions`
- [ ] `a11y.labels`
- [ ] `a11y.landmarks`
- [ ] `a11y.heading_outline`

Source category: **P1**

- [ ] `a11y.screen_reader_mode`
- [ ] `a11y.high_contrast`
- [ ] `a11y.reduced_motion`
- [ ] `a11y.font_scale`
- [ ] `a11y.keyboard_only`
- [ ] `a11y.focus_order`
- [ ] `a11y.contrast_check`

### 28. Debugging, inspection, and diagnostics

Source category: **P0**

- [ ] `debug.console`
- [ ] `debug.exceptions`
- [ ] `debug.network`
- [ ] `debug.dom`
- [ ] `debug.accessibility`
- [ ] `debug.layout`
- [ ] `debug.screenshot`
- [ ] `debug.trace_start`
- [ ] `debug.trace_stop`
- [ ] `debug.trace_export`
- [ ] `debug.action_log`
- [ ] `debug.page_events`
- [ ] `debug.performance`
- [ ] `debug.storage`
- [ ] `debug.permissions`
- [ ] `debug.downloads`

Source category: **P1**

- [ ] `debug.devtools_open`
- [ ] `debug.devtools_close`
- [ ] `debug.breakpoint`
- [ ] `debug.profile_cpu`
- [ ] `debug.profile_memory`
- [ ] `debug.coverage`
- [ ] `debug.performance_timeline`
- [ ] `debug.replay_trace`

### 29. Metrics and evaluation

Source category: **P0**

- [ ] `metrics.success`
- [ ] `metrics.success_per_token`
- [ ] `metrics.model_turns`
- [ ] `metrics.browser_actions`
- [ ] `metrics.batch_size`
- [ ] `metrics.latency`
- [ ] `metrics.recovery_count`
- [ ] `metrics.stale_reference_rate`
- [ ] `metrics.false_completion_rate`
- [ ] `metrics.handoff_rate`
- [ ] `metrics.handoff_resolution_time`
- [ ] `metrics.native_dialog_success`
- [ ] `metrics.upload_success`
- [ ] `metrics.download_success`
- [ ] `metrics.extraction_accuracy`

Source category: **P1**

- [ ] `benchmark.run`
- [ ] `benchmark.record`
- [ ] `benchmark.compare`
- [ ] `benchmark.regression`
- [ ] `benchmark.browsergym`
- [ ] `benchmark.custom_suite`
- [ ] `benchmark.token_report`
- [ ] `benchmark.recovery_report`
- [ ] `benchmark.policy_report`

### 30. Agent control protocol and integration surface

Source category: **P0**

- [ ] `api.health`
- [ ] `api.capabilities`
- [ ] `api.schema`
- [ ] `api.run`
- [ ] `api.observe`
- [ ] `api.act`
- [ ] `api.events`
- [ ] `api.cancel`
- [ ] `api.pause`
- [ ] `api.resume`
- [ ] `api.artifact`
- [ ] `api.trace`
- [ ] `api.policy`
- [ ] `api.version`
- [ ] `api.error`
- [ ] `api.idempotency`

Source category: **P1**

- [ ] `sdk.rust`
- [ ] `sdk.typescript`
- [ ] `sdk.python`
- [ ] `sdk.mcp`
- [ ] `sdk.websocket`
- [ ] `sdk.http`
- [ ] `sdk.local_ipc`
- [ ] `sdk.plugin`
- [ ] `sdk.skill`
- [ ] `sdk.test`



