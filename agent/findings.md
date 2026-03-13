# Findings

Updated: 2026-03-13

## High

1. Subscription diffing and event emission are centralized in one manual dispatcher.
   Files:
   - `crates/nekoland-ipc/src/subscribe.rs`
   Why it matters:
   - Topic names, event names, snapshot diffing, and payload shaping are all duplicated in one place.
   - Adding a new subscription event requires coordinated edits across constants, dispatch logic, and diff helpers.

2. Several pending-event resources are still stringly typed.
   Files:
   - `crates/nekoland-ecs/src/resources/pending_events.rs`
   Why it matters:
   - Fields such as `edges: String`, `source: String`, `detail: String`, and `change: String` make protocol and logging semantics depend on formatting conventions instead of types.
   - This will become increasingly brittle as more producers and consumers appear.

## Medium

3. Keyboard modifier tracking depends on hardcoded numeric keycodes.
   Files:
   - `crates/nekoland-input/src/keyboard.rs`
   Why it matters:
   - The mapping is not self-describing and assumes the current backend/keymap translation.
   - It will be difficult to validate once layouts or input sources diverge.

4. Seat management is a minimal placeholder rather than a real seat lifecycle model.
   Files:
   - `crates/nekoland-input/src/seat_manager.rs`
   Why it matters:
   - The resource is only `Vec<String>`, and the system opportunistically inserts `seat0`.
   - It does not yet model capabilities, focus ownership, device membership, or removal.

5. Gesture recognition is still a coarse pointer-position heuristic.
   Files:
   - `crates/nekoland-input/src/gestures.rs`
   Why it matters:
   - A three-finger swipe is inferred from horizontal pointer buckets instead of real gesture input.
   - This is likely to produce surprising behavior if it becomes user-visible.

6. DRM device discovery falls back to the first `/dev/dri/card*` node when udev seat matching does
   not produce a primary GPU.
   Files:
   - `crates/nekoland-backend/src/drm/device.rs`
   Why it matters:
   - This is pragmatic for bring-up, but it makes multi-GPU and multi-seat setups depend on host
     enumeration order instead of explicit policy.
   - When the fallback picks the wrong card, later GBM or connector failures will look indirect and
     be harder to diagnose.

7. DRM input bounds are derived from the maximum output width and height instead of a true output
   layout extent.
   Files:
   - `crates/nekoland-backend/src/drm/input.rs`
   Why it matters:
   - The current helper treats pointer space as one rectangle bounded by the largest width and
     largest height it sees.
   - That is enough for a single output, but side-by-side or otherwise arranged outputs will need a
     real layout-space model rather than this approximation.

## Low

8. `WorkArea` has a hardcoded `1280x720` default.
   Files:
   - `crates/nekoland-ecs/src/resources/work_area.rs`
   Why it matters:
   - This is convenient for tests, but it can hide startup or sync bugs by providing a plausible-looking fallback geometry.

9. Several integration and benchmark targets are still placeholders.
   Files:
   - `tests/integration/focus_follows_mouse.rs`
   - `tests/integration/ipc_commands.rs`
   - `tests/integration/layout_tiling.rs`
   - `tests/integration/window_lifecycle.rs`
   - `tests/protocol_conformance/wlcs.rs`
   - `benches/ecs_overhead_bench.rs`
   - `benches/layout_bench.rs`
   - `benches/render_bench.rs`
   Why it matters:
   - The repository layout suggests coverage and performance scaffolding in these areas, but the current files are placeholders.
   - This can make the project look more validated than it is when reading the workspace structure alone.

10. The CLI parser/executor in `nekoland-msg/src/main.rs` still concentrates a large amount of
   unrelated responsibility in one file.
   Files:
   - `nekoland-msg/src/main.rs`
   Why it matters:
   - Argument schema, compatibility aliases, help rendering, completion generation, subscription
     formatting, and request/response execution are all maintained together.
   - The file is still workable, but extension cost keeps rising because every new CLI feature
     lands in the same module.

11. Several Wayland-client integration tests duplicate substantial client state-machine and
   dispatch-loop logic.
   Files:
   - `nekoland/tests/e2e_wayland_client.rs`
   - `nekoland/tests/inprocess_keybindings.rs`
   - `nekoland/tests/inprocess_keyboard_repeat.rs`
   - `nekoland/tests/inprocess_layer_shell.rs`
   - `nekoland/tests/inprocess_frame_callbacks.rs`
   - `nekoland/tests/inprocess_presentation_feedback.rs`
   Why it matters:
   - Each test reimplements its own small Wayland client, read loop, timeout handling, and global
     binding flow.
   - The tests are still valuable, but shared client-harness utilities would reduce drift and make
     new protocol scenarios cheaper to add.

12. Many integration tests depend on fixed sleeps and polling deadlines.
    Files:
    - `nekoland/tests/common/mod.rs`
    - `nekoland/tests/e2e_wayland_client.rs`
    - `nekoland/tests/inprocess_clipboard_selection.rs`
    - `nekoland/tests/inprocess_clipboard_transfer.rs`
    - `nekoland/tests/inprocess_frame_callbacks.rs`
    - `nekoland/tests/inprocess_presentation_feedback.rs`
    - `nekoland/tests/ipc_*`
    Why it matters:
    - The tests are pragmatic, but many scenarios rely on short wall-clock sleeps, retry loops, and
      hardcoded two-second deadlines.
    - This raises the risk of environment-sensitive flakes as the suite grows or runs under slower
      CI conditions.

13. Decoration policy currently branches on a raw layout-name string.
    Files:
    - `crates/nekoland-shell/src/decorations.rs`
    Why it matters:
    - Border width currently depends on whether `config.default_layout == "tiling"`.
    - This keeps the code short, but it couples visual policy to string literals instead of an
      explicit layout enum or decoration policy object.

14. Config hot-reload tests synchronize on repeated schedule ticks and short sleeps instead of an
    explicit reload acknowledgement.
    Files:
    - `nekoland/tests/config_runtime.rs`
    - `nekoland/tests/ipc_config_subscription.rs`
    Why it matters:
    - The tests currently wait for hot reload by running `ExtractSchedule` multiple times or by
      sleeping briefly before rewriting the config file.
    - This keeps the tests simple, but it makes them sensitive to polling cadence and filesystem
      timestamp timing.

15. Clipboard and primary-selection IPC tests are near-structural copies of each other.
    Files:
    - `nekoland/tests/ipc_clipboard_state.rs`
    - `nekoland/tests/ipc_primary_selection_state.rs`
    Why it matters:
    - The protocol objects differ, but the test flow, helper client, retry loops, and snapshot
      assertions are almost identical.
    - This duplication is manageable today, but future changes to the selection test harness will
      likely require keeping both files in sync by hand.

16. Clipboard and primary-selection transfer tests are also near-structural copies of each other.
    Files:
    - `nekoland/tests/inprocess_clipboard_transfer.rs`
    - `nekoland/tests/inprocess_primary_selection_transfer.rs`
    Why it matters:
    - Source/target client state, transfer pumps, receive loops, and persistence assertions follow
      almost the same structure with protocol-specific type substitutions.
    - The duplicated logic works, but extending the real-client transfer harness now requires
      editing both files in parallel.

17. The DnD transfer test identifies source and target roles partly through window-title strings
    and positional fallbacks.
    Files:
    - `nekoland/tests/inprocess_dnd_transfer.rs`
    Why it matters:
    - The synthetic pump first looks for windows titled `dnd-source` and `dnd-target`, then falls
      back to first/last window ordering if those titles are not found.
    - That keeps the test pragmatic, but it hides an implicit contract between helper clients and
      the pump logic that can drift as the scenario evolves.

18. The XWayland smoke test classifies several startup and connection paths through substring
    matching on error messages.
    Files:
    - `nekoland/tests/xwayland_smoke.rs`
    Why it matters:
    - Helpers such as `x11_connect_error_is_retryable` and
      `xwayland_startup_error_is_skippable` depend on free-form error text like
      `"Connection refused"` and `"Operation not permitted"`.
    - This is workable for a smoke test, but it is inherently brittle across library, locale, or
      platform changes.

19. The end-to-end socket discovery helper assumes the first runtime-directory entry is the
    compositor socket.
    Files:
    - `nekoland/tests/e2e_wayland_client.rs`
    Why it matters:
    - `discover_socket` currently returns the first directory entry without filtering by socket
      type or expected name pattern.
    - That keeps the e2e setup minimal, but it makes the test sensitive to unrelated files that may
      appear in the runtime directory.

20. The subscribe-CLI integration helper emits IPC workspace mutations without checking whether any
    individual request succeeded.
    Files:
    - `nekoland-msg/tests/subscribe_cli.rs`
    Why it matters:
    - `emit_workspace_events` repeatedly sends create/switch requests and ignores all reply values
      and request errors.
    - The test still exercises streaming behavior, but helper-side failures are easy to miss and
      can make CLI regressions harder to diagnose.

21. Backend selection silently maps unknown `NEKOLAND_BACKEND` values to `winit`.
    Files:
    - `crates/nekoland-backend/src/manager.rs`
    Why it matters:
    - `requested_backend_kinds` treats any unrecognized token as `BackendKind::Winit`.
    - This keeps startup resilient, but it also hides typos and misconfiguration that could be
      surfaced more explicitly.

22. The virtual backend currently behaves like a single-output backend even when config lists
    multiple enabled outputs.
    Files:
    - `crates/nekoland-backend/src/virtual_output.rs`
    Why it matters:
    - `desired_output_name` picks the first enabled configured output, and `present` only captures
      against the first owned output snapshot.
    - That is fine for today's offscreen test path, but multi-output virtual capture would require
      a less implicit model.

23. X11 interactive-resize requests still encode resize edges as a raw string.
    Files:
    - `crates/nekoland-ecs/src/resources/x11_requests.rs`
    Why it matters:
    - `X11LifecycleAction::InteractiveResize` currently carries `edges: String`.
    - That keeps the protocol bridge simple, but it repeats the project's broader stringly-typed
      pattern in a place where a small enum would be easier to validate and refactor.
