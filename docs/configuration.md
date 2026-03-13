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

`[startup]` fields:

- `commands`

`commands` is a list of shell-style command lines that are split into argv and launched once after
the Wayland socket is ready. These commands inherit the compositor's nested Wayland environment, so
GUI apps connect to the compositor they were started from instead of the host session.

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

- `close-window`
- `window move <x> <y>`
- `window resize <width> <height>`
- `window split <horizontal|vertical>`
- `workspace <name>`
- `workspace switch <name>`
- `workspace create <name>`
- `workspace destroy <name>`
- `output enable <name>`
- `output disable <name>`
- `output configure <name> <mode>`
- `output configure <name> <mode> <scale>`

External commands are configured directly as argv arrays on the right-hand side of a binding. For
example:

```toml
[keybinds.bindings]
"Super+Return" = ["foot"]
"Super+Space" = ["wofi", "--show", "drun"]
"Super+Shift+Q" = "close-window"
```

Key names use the XKB/X11-style names already used elsewhere in the project, for example:

- `Super+Q`
- `Super+Shift+Q`
- `Super+Return`
- `Super+1`
