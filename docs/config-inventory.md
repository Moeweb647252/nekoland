# Configuration Inventory

This document inventories the current runtime-configurable surface of the project.

Scope:

- non-test runtime configuration
- environment variables used by production code
- IPC and CLI control-plane options that affect runtime behavior
- test-only opt-in flags called out separately at the end

Primary source files:

- `config/default.toml`
- `crates/nekoland-config/src/schema.rs`
- `crates/nekoland-config/src/action_config.rs`
- `crates/nekoland-config/src/resources/compositor_config.rs`
- `crates/nekoland-input/src/keybindings.rs`
- `nekoland/src/lib.rs`
- `nekoland-msg/src/main.rs`

## Config Loading

- Default config path: `config/default.toml`
- Override env var: `NEKOLAND_CONFIG`
- Supported on-disk formats: `.toml`, `.ron`
- Startup behavior:
  - if the configured file is missing or invalid, the compositor falls back to built-in defaults
  - config hot reload is enabled
  - config reload can also be forced through IPC with `nekoland-msg action reload-config`
  - the FPS HUD can be temporarily overridden through `nekoland-msg action fps-hud <on|off|toggle>`

## Disk Config Schema

Top-level keys:

- `default_layout`
- `[theme]`
- `[debug]`
- `[input]`
- `[[window_rules]]`
- `[ipc]`
- `[startup]`
- `[xwayland]`
- `[[outputs]]`
- `[keybinds.bindings]`

### Top Level

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `default_layout` | string | `floating` | Allowed: `floating`, `maximized`, `fullscreen`, `tiling`, `stacking` |

Notes:

- `stacking` currently normalizes to floating layout with normal mode.

### `[theme]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `theme.name` | string | `catppuccin-latte` | Theme metadata exposed through IPC/state snapshots |
| `theme.cursor_theme` | string | `default` | Cursor theme name |
| `theme.border_color` | string | `#5c7cfa` | Hex color string |
| `theme.background_color` | string | `#f5f7ff` | Hex color string |

### `[debug]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `debug.fps_hud` | bool | `false` | Default visibility for the compositor-owned FPS HUD on the current primary output |

### `[input]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `input.focus_follows_mouse` | bool | `true` | Current semantics are effectively on/off |
| `input.repeat_rate` | integer | `30` | Keyboard repeat rate passed into protocol bootstrap |

### `[input.keyboard]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `input.keyboard.current` | string or omitted | first layout name | Must match one configured layout name |
| `input.keyboard.layouts` | array | one implicit `us` layout | If omitted or empty, defaults to a single `us` layout |

Per layout entry in `[[input.keyboard.layouts]]`:

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `name` | string or omitted | same as `layout` | Must be unique and non-empty after normalization |
| `layout` | string | `us` | Must be non-empty |
| `rules` | string | `""` | XKB rules |
| `model` | string | `""` | XKB model |
| `variant` | string | `""` | XKB variant |
| `options` | string | `""` | XKB options |

Validation:

- layout names must be unique
- `current` must exist in `layouts`
- if `current` is omitted, the first layout becomes active

### `[[window_rules]]`

Each window rule can contain:

| Key | Type | Notes |
| --- | --- | --- |
| `app_id` | string | Exact match |
| `title` | string | Exact match |
| `layout` | string | Allowed: `tiled`, `floating` |
| `mode` | string | Allowed: `normal`, `maximized`, `fullscreen`, `hidden` |
| `background` | string | Output name to pin the matching window as background |

Notes:

- matching is exact equality, not regex or glob
- rules layer on top of the global `default_layout`
- multiple matching rules can contribute fields cumulatively

### `[ipc]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `ipc.command_history_limit` | integer | `64` | Runtime ring-buffer size for command history; `0` clears and disables history retention |

### `[startup]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `startup.actions` | array of actions | `[]` | Run once after the nested Wayland socket is ready; waits for XWayland readiness when enabled |

Notes:

- startup actions support the same action shapes as normal action lists
- `viewport_pan_mode` is not valid here

### `[xwayland]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `xwayland.enabled` | bool | `true` | Enables or disables XWayland startup |

### `[[outputs]]`

Each configured output contains:

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `name` | string | `eDP-1` | Output name |
| `mode` | string | `1920x1080@60` | Free-form mode string passed through backend control paths |
| `scale` | integer | `1` | Normalized to `>= 1` |
| `enabled` | bool | `true` | Initial enabled state |

### `[keybinds.bindings]`

This section is a map of:

```toml
[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"Super+Shift+Return" = [
  { workspace = 1 },
  { exec = ["foot"] },
]
```

Representation notes:

- key: binding string
- value: one action object or an array of action objects
- compiled keybindings hot-reload when `config.keybindings` changes

## Action Shapes

The following action objects are accepted in `startup.actions` and regular keybinding action lists unless noted otherwise.

| Action | Shape | Notes |
| --- | --- | --- |
| exec | `{ exec = ["program", "arg1"] }` | Must contain at least one argv element; first element must be non-empty |
| close | `{ close = true }` | Boolean must be `true` |
| move | `{ move = [x, y] }` | Signed integers |
| resize | `{ resize = [width, height] }` | Unsigned integers |
| tiling_focus_column | `{ tiling_focus_column = "left" }` | Allowed: `left`, `right` |
| tiling_focus_window | `{ tiling_focus_window = "up" }` | Allowed: `up`, `down` |
| tiling_move_column | `{ tiling_move_column = "left" }` | Allowed: `left`, `right` |
| tiling_move_window | `{ tiling_move_window = "up" }` | Allowed: `up`, `down` |
| tiling_consume | `{ tiling_consume = "left" }` | Allowed: `left`, `right` |
| tiling_expel | `{ tiling_expel = "right" }` | Allowed: `left`, `right` |
| tiling_pan | `{ tiling_pan = "left" }` | Allowed: `left`, `right`, `up`, `down` |
| background | `{ background = "eDP-1" }` | Output name |
| clear_background | `{ clear_background = true }` | Boolean must be `true` |
| workspace | `{ workspace = 1 }` or `{ workspace = "web" }` | Switch/create workspace by id or name |
| workspace_create | `{ workspace_create = 1 }` or `{ workspace_create = "web" }` | Create workspace by id or name |
| workspace_destroy | `{ workspace_destroy = 1 }`, `{ workspace_destroy = "web" }`, `{ workspace_destroy = "active" }` | Destroy by id, name, or active workspace |
| output_enable | `{ output_enable = "eDP-1" }` | Output name |
| output_disable | `{ output_disable = "eDP-1" }` | Output name |
| output_configure | `{ output_configure = { output = "eDP-1", mode = "1920x1080@60", scale = 1 } }` | `scale` is optional |
| viewport_pan | `{ viewport_pan = [dx, dy] }` | Signed integers |
| viewport_move | `{ viewport_move = [x, y] }` | Signed integers |
| viewport_center | `{ viewport_center = true }` | Boolean must be `true` |
| viewport_pan_mode | `{ viewport_pan_mode = true }` | Keybinding-only; must be the only action in that binding |

## Keybinding Syntax

Regular keybindings:

- must contain exactly one non-modifier key
- may contain zero or more modifiers

Accepted modifier tokens:

- `Ctrl`
- `Control`
- `Alt`
- `Shift`
- `Super`
- `Logo`
- `Meta`

Accepted non-modifier keys:

- digits: `0` to `9`
- letters: `a` to `z`
- `Tab`
- `Return` / `Enter`
- `Space`
- `Escape` / `Esc`
- `Backspace`
- `Delete`
- arrows: `Left`, `Right`, `Up`, `Down`
- function keys: `F1` to `F12`

Examples:

- `Super+Return`
- `Super+Shift+1`
- `Ctrl+Alt+Delete`

Special case: viewport pan mode binding

- configured as a normal binding entry with `{ viewport_pan_mode = true }`
- must be defined at most once
- must contain only modifiers
- must include at least one modifier
- the binding string is normalized into `viewport_pan_modifiers`

## Environment Variables

### Production Runtime

| Variable | Default | Effect |
| --- | --- | --- |
| `NEKOLAND_CONFIG` | `config/default.toml` | Override config file path |
| `NEKOLAND_BACKEND` | `winit` | Requested backend list; comma-separated |
| `NEKOLAND_SEAT` | `seat0` | DRM session seat name |
| `NEKOLAND_MAX_FRAMES` | unset | Optional run-loop frame cap; `0` means no cap |
| `NEKOLAND_FRAME_TIMEOUT_MS` | `16` | Run-loop frame timeout in milliseconds |
| `NEKOLAND_IPC_SOCKET` | derived path | Override IPC socket path directly |
| `NEKOLAND_RUNTIME_DIR` | unset | Runtime directory override used by IPC and Wayland bootstrap |
| `XDG_RUNTIME_DIR` | system environment | Fallback runtime dir when `NEKOLAND_RUNTIME_DIR` is unset |
| `NEKOLAND_DISABLE_STARTUP_COMMANDS` | unset | Disable startup actions when set to a non-empty value other than `0` or `false` |

Backend parsing details for `NEKOLAND_BACKEND`:

- `drm` -> DRM backend
- `virtual`, `headless`, `offscreen` -> virtual backend
- `winit`, `x11` -> winit backend
- duplicates are removed while preserving order
- unknown values currently fall back to `winit`

### Logging

| Variable | Default | Effect |
| --- | --- | --- |
| `RUST_LOG` | `info,nekoland=debug` | Inferred from `tracing_subscriber::EnvFilter::try_from_default_env()` in `nekoland` |

Note:

- `RUST_LOG` is not referenced by name in project code, but `EnvFilter::try_from_default_env()` uses the standard tracing env var.

## Runtime Control Plane

The main `nekoland` binary currently has no user-facing CLI flags. Runtime control is mainly exposed through config, env vars, and the `nekoland-msg` IPC client.

### `nekoland-msg` Top-Level Commands

- `query`
- `window`
- `popup`
- `workspace`
- `output`
- `tiling`
- `action`
- `completion`
- `subscribe`
- `help`

Compatibility aliases also exist:

- `get_tree`
- `get_outputs`
- `get_workspaces`
- `get_windows`
- `get_commands`
- `get_config`
- `get_keyboard_layouts`
- `get_clipboard`
- `get_primary_selection`
- `get_present_audit`

### `query`

Supported query targets:

- `tree`
- `outputs`
- `workspaces`
- `windows`
- `keyboard-layouts`
- `commands`
- `config`
- `clipboard`
- `primary-selection`
- `present-audit`

### `window`

Supported actions:

- `focus <surface_id>`
- `close <surface_id>`
- `move <surface_id> <x> <y>`
- `resize <surface_id> <width> <height>`
- `background <surface_id> <output>`
- `clear-background <surface_id>`

### `tiling`

Supported actions:

- `focus-column <left|right>`
- `focus-window <up|down>`
- `move-column <left|right>`
- `move-window <up|down>`
- `consume <left|right>`
- `expel <left|right>`
- `pan <left|right|up|down>`

### `popup`

Supported actions:

- `dismiss <surface_id>`

### `workspace`

Supported actions:

- `switch <workspace>`
- `create <workspace>`
- `destroy <workspace>`

### `output`

Supported actions:

- `enable <output>`
- `disable <output>`
- `configure <output> <mode> [scale]`
- `viewport-move <output> <x> <y>`
- `viewport-pan <output> <dx> <dy>`
- `center-viewport-on-window <output> <surface_id>`

### `action`

Supported actions:

- `focus-workspace <workspace>`
- `focus-window --id <surface_id>`
- `close-window --id <surface_id>`
- `spawn -- <argv...>`
- `switch-keyboard-layout-next`
- `switch-keyboard-layout-prev`
- `switch-keyboard-layout-name <name>`
- `switch-keyboard-layout-index <index>`
- `fps-hud <on|off|toggle>`
- `reload-config`
- `quit`
- `power-off-monitors`
- `power-on-monitors`

### `completion`

Supported shells:

- `bash`
- `zsh`
- `fish`

### `subscribe`

Supported topics:

- `window`
- `popup`
- `workspace`
- `output`
- `command`
- `config`
- `keyboard-layout`
- `clipboard`
- `primary-selection`
- `present-audit`
- `focus`
- `tree`
- `all`

Supported flags:

- `--json`
- `--pretty`
- `--jsonl`
- `--no-payloads`
- `--event <name|prefix*>`

Known event names:

- `window_created`
- `window_closed`
- `window_moved`
- `window_opened_or_changed`
- `windows_changed`
- `window_geometry_changed`
- `window_layouts_changed`
- `window_state_changed`
- `popup_created`
- `popup_dismissed`
- `popup_geometry_changed`
- `popup_grab_changed`
- `output_connected`
- `output_disconnected`
- `outputs_changed`
- `workspaces_changed`
- `workspace_activated`
- `command_launched`
- `command_failed`
- `config_changed`
- `keyboard_layouts_changed`
- `keyboard_layout_switched`
- `clipboard_changed`
- `primary_selection_changed`
- `present_audit_changed`
- `focus_changed`
- `window_focus_changed`
- `tree_changed`

## IPC-Visible Config Snapshot

`nekoland-msg query config` exposes a normalized `ConfigSnapshot`, not a byte-for-byte copy of the source file.

Fields currently included in that snapshot:

- config path
- load/reload state
- theme fields
- `default_layout`
- `focus_follows_mouse`
- `repeat_rate`
- configured keyboard layout and normalized keyboard layouts
- normalized `viewport_pan_modifiers`
- `command_history_limit`
- `startup_actions`
- `xwayland_enabled`
- normalized outputs
- normalized keybindings

Notable omission:

- `window_rules` are configurable on disk but are not currently included in the IPC config snapshot

## Test-Only Opt-In Flags

These are used only by tests and are not part of the normal runtime configuration surface.

| Variable | Scope | Notes |
| --- | --- | --- |
| `NEKOLAND_RUN_REAL_DRM_IMPORT_TEST` | test-only | Enables the real DRM dma-buf import integration test |
| `NEKOLAND_TEST_RENDER_NODE` | test-only | Supplies a render node path to the real DRM import test |

## Example Files

The repository currently ships multiple example-style config files:

- `config/default.toml`
  - the practical default config used by the repository
- `config/example.toml`
  - a shorter alternative example
- `config/full-example.toml`
  - a comment-heavy reference that documents the full on-disk config surface
