# Horizon migration (in progress)

The panel supports the native Horizon 26.3 server. A new release executable
contains the pinned Horizon programs and the resource preparer. An administrator
creates a Horizon server, accepts the Minecraft EULA and selects **Start**. On
first start, the panel installs Horizon, downloads the official Minecraft files
and verifies their pinned checksums, then prepares and validates the shared game
resources. Progress appears in the panel. Later starts reuse the installation
and verified files.

The pinned runtime is a vendored fork (`hernan-lc/horizon`, branch
`vendor/gaysermc`), because the original upstream repository is no longer
reachable; the fork's base and its extension points are documented in
`vendor/horizon/VENDOR.md`.

The first start needs internet access, several hundred megabytes of downloads,
free disk space for the downloaded and generated files, and time to prepare
them. It uses Java 25 only to generate the resources: Horizon runs natively.
The panel installs a managed Java 25 where supported; on Termux it installs the
`openjdk-25` package when needed. Prepared Minecraft files stay on the host and
are never included in a release or committed to Git.

From a recursive source checkout, a panel running in that checkout compiles the
exact pinned, clean Horizon submodule on first start when no runtime is present.
This requires Git, Rust and a working native compiler, and can take considerably
longer than starting a release binary. The first installation and preparation
must be started by an administrator. Existing installations and custom resource
paths are not overwritten. Older releases or individual binaries built without
an embedded runtime need a current release or a source checkout.

To install or import manually instead, an administrator can still run:

```sh
./scripts/install-horizon.sh --data-dir ./data --resources /path/to/prepared/26.3
./scripts/install-horizon.sh --data-dir ./data --import-resources /path/to/prepared/26.3
```

The second command adds resources to an existing runtime. An individual server
can also provide its own prepared `resources/` directory or explicit resource
path. Horizon validates imports; incomplete or existing destinations are not
replaced. Back up a world before converting it. Java plugins and mods cannot run
in Horizon.

Horizon and version 26.3 are selected by default when creating a server; Java
providers remain available. The creation form reports when the Horizon programs
are missing, although their presence alone does not validate the game files.
Select Horizon and version 26.3 when changing an existing server's version.
The server directory is `<data-dir>/servers/<id>`. The API also accepts
`core: "horizon"`, `version: "26.3"`, no build, and an explicit EULA decision.
The panel's port is applied to `server.toml` on each start, Horizon's own panel
is disabled, and Horizon's internal crash restart is disabled so Guardian owns
restart policy. Other Horizon settings are retained. Existing Java heap and JVM
arguments do not limit or configure Horizon's native memory usage.

### Migrating a Paper or Spigot world

Paper and Spigot may store world data in a different layout from vanilla.
Horizon preparation stops when it detects the layouts below. To make a
converted copy, stop the Java server, create a new Horizon server in the panel
without starting it, and run:

```sh
python3 scripts/migrate-paper-world.py --source /path/to/java/server \
  --destination /path/to/panel/data/servers/NEW_SERVER_ID --world world
```

Use the exact `level-name` from `server.properties` for `--world`. The
destination must be new or empty. Keep both servers stopped throughout the copy;
the original remains intact. The script refuses symbolic links and existing
destination data, and leaves a marker that prevents Horizon from starting if a
copy fails. Inspect a partial destination before retrying. It preserves other
files from the old split-dimension directories under `.mcpanel-paper-originals`
in the new server. The copy also records the selected world in
`.mcpanel-migrated-world.json`; Horizon preparation checks that this matches
`server.toml` and that its `level.dat` is still present. Correct the imported
settings if they do not match. To deliberately switch to a different world later,
back up the worlds and remove that record after updating the settings. Java
plugins and mods are copied but cannot run in Horizon.

For a manual conversion, stop the Java server and make a full backup before
changing anything. Substitute the configured `level-name` for `world` below.

For older Paper/Spigot worlds, copy `world_nether/DIM-1` to `world/DIM-1` and
`world_the_end/DIM1` to `world/DIM1`. If either destination exists, compare it
with the source and resolve the conflict using your backup; do not overwrite
it blindly. Preserve the original `world_nether` and `world_the_end` folders
outside the server directory after copying so Horizon can start.

For newer Paper worlds, move the following files, when present, from
`world/dimensions/minecraft/overworld/data/minecraft/` to
`world/data/minecraft/`: `game_rules.dat`, `scheduled_events.dat`,
`wandering_trader.dat`, `weather.dat`, and `world_gen_settings.dat`. Resolve
any existing destination files against the backup before replacing anything.
The panel does not validate plugin data or perform a complete compatibility
conversion. Refer to [Paper's migration guide](https://docs.papermc.io/paper/migration/)
for the source layouts and additional limitations.

When the operating system exposes the relevant processes, the panel reports
CPU and the summed resident memory of the supervisor, engine and descendants.
Shared pages may be counted more than once. Metrics are unavailable when the
engine cannot be identified; CPU can also be unavailable separately.
On Android, when host CPU counters are restricted, CPU is estimated from the
visible processes' tick deltas over elapsed time. A new or changed process tree
needs another sample before its CPU usage can be shown.

Horizon is started as `horizon-server run` and receives console commands and
`stop` over stdin. Linux and macOS use their platform sandbox when available;
Android and platforms without a supported sandbox require the existing explicit
`--allow-unsandboxed-servers` setting. Horizon validates generated resources
before publication. Enforced native resource limits and cross-platform game
end-to-end validation remain unfinished. Horizon advertises a force-stop control
protocol (`[horizon] force-stop-v1`): the supervisor kills and reaps its engine
before exiting. The panel also accepts the older `[mcpanel] force-stop-v1`
banner from pre-upstream forks still installed on a host. Binaries without
either protocol cannot be force-stopped by the panel; use Horizon or host tools
if their graceful stop times out.

To run the isolated native integration test with locally built programs and
your prepared game files, set `HORIZON_SMOKE_SERVER`, `HORIZON_SMOKE_ENGINE`,
and `HORIZON_SMOKE_RESOURCES` to their absolute paths, then run:

```sh
HORIZON_SMOKE_FORCE_STOP=1 cargo test -p guardian --test supervisor \
  real_horizon_starts_and_stops -- --ignored --nocapture
```

The test creates a temporary server world and checks the force-stop protocol;
on Linux and Android it also checks that the engine process was reaped. Omit
`HORIZON_SMOKE_FORCE_STOP` to test a graceful stop instead.

To exercise the converter and start its output with the real engine, use the
same paths and command with `HORIZON_SMOKE_SHARED=1 HORIZON_SMOKE_PAPER_COPY=1`
instead. This generates an isolated Horizon world, arranges an empty region in
the older Paper directory layout, converts a copy and starts it. It checks the
original files remain unchanged; it is not a compatibility test of a world
generated by Paper or of plugin data.
