# Playit Setup

No separate Playit installation is required. The panel ships an embedded Playit runtime.

## Embedded mode (default)

1. Start `mcpanel` and open the admin-only **Playit** section in the web UI (`#/playit`).
2. Click **Connect**. The panel starts the embedded Playit runtime and stores its dedicated secret under `<data-dir>/playit/secret.toml`.
3. Open the generated claim link and return to the panel when approval is complete.

Once connected, choose a server and create its tunnel. The panel uses TCP to `127.0.0.1:<server-port>`, stores the Playit tunnel id in `panel.json`, and polls the service so provisioning, disabled, drifted, and connected states are visible. The server settings page also exposes the same attach/detach controls. Deleting a server or tunnel removes a panel-managed tunnel first when the service is available.

## External mode (legacy IPC)

Operators who intentionally run a compatible external `playitd` can select the legacy IPC backend explicitly. Keep the listener on loopback, or explicitly acknowledge the risk with `--allow-insecure-http` on an isolated network:

```sh
mcpanel --playit-mode external --data-dir ./data --bind 127.0.0.1:8080
```

The `MCPANEL_PLAYIT_MODE=external` environment variable is equivalent. External mode does not stop the independently managed daemon when the panel exits.

## Credentials and switching modes

Embedded and external modes keep separate Playit credentials. Switching from an external daemon does not import its secret automatically, so embedded mode may require a new claim. Existing panel tunnel bindings are preserved and reconciled against whichever Playit account is active.

## Runtime boundary

```
panel (mcpanel)
    │
    └── PlayitManager
          ├── embedded PlayitRuntime (default)
          └── external playitd IPC (optional)
```

## API

See [API — Playit endpoints](api.md#playit) and [Security](security.md) for the trust implications of exposing a Minecraft port.

## Troubleshooting

- If a tunnel shows `drifted`, the remote Playit state no longer matches `panel.json`; re-attach from the server settings or recreate the tunnel.
- Switching modes requires a claim in the new mode; bindings survive the switch.
