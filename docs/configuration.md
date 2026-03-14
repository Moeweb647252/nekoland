# Configuration

Configuration files live under `config/` and map to `nekoland_config::schema::NekolandConfigFile`.

Supported formats:

- `.toml`
- `.ron`

Runtime behavior:

- The compositor loads the configured file at startup.
- `nekoland-config` hot-reloads the file on Linux with `inotify`.
- Invalid reloads are rejected and leave the last good runtime config in place.

Important top-level fields:

- `default_layout`
- `[theme]`
- `[input]`
- `[ipc]`
- `[startup]`
- `[xwayland]`
- `[[outputs]]`
- `[keybinds.bindings]`

`[ipc]` fields:

- `command_history_limit`

`command_history_limit` controls how many external command launch/failure records are retained for
`query commands` / `get_commands`. Setting it to `0` disables command history retention.

`[input]` fields:

- `focus_follows_mouse`
- `repeat_rate`

`[startup]` fields:

- `actions`

`actions` is a list of startup actions applied once after the Wayland socket is ready. External
commands use argv arrays via `{ exec = [...] }` and inherit the compositor's nested Wayland
environment, so GUI apps connect to the compositor they were started from instead of the host
session.

`[xwayland]` fields:

- `enabled`

`xwayland.enabled` is applied at startup. Disabling it removes XWayland support for that run.

`[[outputs]]` fields:

- `name`
- `mode`
- `scale`
- `enabled`

Configured outputs are applied at startup and re-applied when the config file changes.

Supported keybinding actions:

- `{ close = true }`
- `{ move = [x, y] }`
- `{ resize = [width, height] }`
- `{ split = "horizontal" | "vertical" }`
- `{ background = "OUTPUT" }`
- `{ clear_background = true }`
- `{ workspace = 1 | "name" }`
- `{ workspace_create = 1 | "name" }`
- `{ workspace_destroy = 1 | "name" | "active" }`
- `{ output_enable = "OUTPUT" }`
- `{ output_disable = "OUTPUT" }`
- `{ output_configure = { output = "OUTPUT", mode = "1920x1080@60", scale = 2 } }`
- `{ viewport_pan = [dx, dy] }`
- `{ viewport_move = [x, y] }`
- `{ viewport_center = true }`
- `{ viewport_pan_mode = true }`

Keybinding actions are configured as short inline tables. For example:

```toml
[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"Super+Space" = { exec = ["wofi", "--show", "drun"] }
"Super+Alt" = { viewport_pan_mode = true }
"Super+Shift+Q" = { close = true }
"Super+1" = { workspace = 1 }
"Super+Alt+H" = { viewport_pan = [-200, 0] }
```

Key names use the XKB/X11-style names already used elsewhere in the project, for example:

- `Super+Q`
- `Super+Shift+Q`
- `Super+Return`
- `Super+1`

`viewport_pan_mode` is special: the binding must contain modifiers only, for example
`"Super+Alt"` or `"Ctrl+Shift"`. While those modifiers are held, pointer motion is consumed by
viewport panning instead of being forwarded to client hover handling.
