# Termux (Android)

The panel runs natively in Termux without root or a Linux proot distribution.
It uses the Android browser for its UI and the Termux OpenJDK packages for
Minecraft. The ARM64 release target is `aarch64-linux-android` (Android 7/API 24
or newer, subject to your Termux/JDK package requirements). This is a command
line application, not an APK or an Android foreground service.

## Install a release

Use a current Termux installation and keep its packages up to date. On an
ARM64 device (`uname -m` reports `aarch64`):

```sh
pkg update
pkg upgrade
pkg install gh tar openjdk-21
mkdir -p "$HOME/mcpanel"
cd "$HOME/mcpanel"
gh release download --repo nglmercer/minecraft-server-rs --pattern '*-android-arm64.tar.gz'
tar -xzf mcpanel-*-android-arm64.tar.gz --strip-components=1
chmod +x mcpanel
./mcpanel --data-dir "$HOME/.local/share/mcpanel" --bind 127.0.0.1:8080 --allow-unsandboxed-servers
```

Choose a release containing the Android asset; releases made before Android
support was added have none. Linux/glibc and Linux/musl binaries are not Android
binaries, even when their architecture matches. Other device architectures
can build from source using Termux's native Rust compiler.

Keep both the executable and panel data inside Termux's private `$HOME`.
Do not run from `/sdcard` or shared storage: those filesystems have different
permission, execution, and symlink behavior. No shared-storage permission is
needed. The explicit unsandboxed flag acknowledges that every Minecraft JVM
can access files owned by Termux; Android's app sandbox does not separate the
servers from one another.

Open the printed loopback URL in the Android browser and create the first
administrator at `/setup`. The panel also tries `termux-open-url` automatically;
if the helper is unavailable, opening the URL manually works. There is no
system tray. For password recovery use the CLI described in
[Security](security.md). Keep the default loopback bind; remote HTTP and TLS
configuration follow the same rules as other platforms.

## Build on the device

```sh
pkg install git rust clang make pkg-config nodejs-lts openjdk-21
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
CARGO_BUILD_JOBS=2 ./build.sh
./target/release/mcpanel --data-dir "$HOME/.local/share/mcpanel" --bind 127.0.0.1:8080 --allow-unsandboxed-servers
```

`build.sh` builds and embeds the frontend before compiling Rust. Set
`MCPANEL_FAST=1` for a debug build (`target/debug/mcpanel`) while developing.
Limit build jobs on memory-constrained devices. The default build has no GTK,
X11, or desktop dependencies; do not enable `linux-tray`.

If Node fails with a missing C++ library symbol, check for an injected
`LD_LIBRARY_PATH` before changing packages. For example, an agent or launcher
may prepend its own incompatible runtime libraries. With normal Termux
packages installed, `env -u LD_LIBRARY_PATH node --version` can confirm this;
run the build with that same environment if needed. Do not change the panel's
sanitized JVM environment to forward launcher credentials or arbitrary library
preloads.

Alternatively build `web/dist` on another computer, copy it into this checkout,
and run `cargo build --release --locked -p panel --bin mcpanel` in Termux.

## Java versions

Install the Java major required by your selected Minecraft version. For
example, [Termux's OpenJDK 21](https://github.com/termux/termux-packages/blob/master/packages/openjdk-21/build.sh)
provides a Bionic-compatible JVM:

```sh
pkg search openjdk
pkg install openjdk-21
java -version
```

Discovery checks `$JAVA_HOME`, `$PATH`, older `$PREFIX/opt` layouts, and every
versioned JDK under `$PREFIX/lib/jvm`, so multiple installed Java majors can be
used by different servers. If a JDK is installed while the panel is running,
restart the panel or allow its 60-second Java discovery cache to expire. A
missing major produces package-install guidance in the server console; the
panel never downloads a glibc Temurin JDK on Android. Some older majors may
not be packaged by Termux; use a server version compatible with an available
JDK, or supply a trusted Android-compatible JDK through `JAVA_HOME`.

## Background operation and resources

For an interactive session you can keep the terminal open, or use `tmux`:

```sh
pkg install tmux
termux-wake-lock
tmux new -s mcpanel
# Run mcpanel in this session. Detach with Ctrl-B then D.
# Reattach later with: tmux attach -t mcpanel
```

Use `termux-wake-unlock` after stopping the panel. Configure Android battery
settings to allow Termux to run in the background. A wake lock helps with CPU
sleep but cannot prevent Android from killing the app or its child processes.
Stop servers through the panel and shut down the panel with Ctrl-C or SIGTERM
so worlds are saved. Start the panel again after reboot; automatic boot
startup can be managed separately with Termux:Boot.

Keep JVM heaps and player counts appropriate to the phone's RAM, thermal
limits, and storage. Start with a small heap and low view distance. Host CPU
and other process statistics may be restricted by Android. The API returns
`null` for unavailable CPU readings, the UI displays unknown, and readable
process RSS remains visible. Unavailable readings are not spare capacity. Back up worlds outside the device.
Embedded Playit works through the panel's web UI; internet access, account
claiming, and tunnel availability still depend on the Playit service.

## Cross-compilation

On a Linux x86_64 host with Rust and an Android NDK installed:

```sh
rustup target add aarch64-linux-android
# Build web/dist first (cd web && npm ci && npm run build).
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cargo-android.sh
```

The wrapper selects the NDK's API 24 ARM64 compiler for both Rust linking and
C dependencies such as ring. Explicit linker/compiler overrides are preserved.
CI checks Android code and tests with Clippy, builds an executable with the UI
embedded, and uploads it as an artifact. The release workflow packages the
optimized executable as `mcpanel-<version>-android-arm64.tar.gz`.

## Validation

Run on an actual Termux device:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
(cd web && npm test && npm run build)
MCPANEL_FAST=1 ./build.sh
```

Then check `/setup`, login, Java discovery, server provisioning, online console,
stop/restart, file access, backup/restore, and panel shutdown. Use a disposable
server and data directory. CI cross-compilation checks compilation but cannot
replace these Android lifecycle checks.

For a repeatable real-server check (Python 3 and Java 21 required):

```sh
pkg install python
python scripts/smoke-termux.py --binary target/debug/mcpanel --work-dir "$HOME/mcpanel-smoke" --accept-eula
```

Read the [Minecraft EULA](https://aka.ms/MinecraftEULA) before using
`--accept-eula`. The work directory must be empty. The check downloads a vanilla
server, uses random loopback ports and temporary local credentials, and checks
setup/login, embedded UI, Java discovery, provisioning, console, restart/stop,
file editing, backup/restore, and graceful panel shutdown. Data and logs remain
in that directory for inspection. It does not claim a Playit account or create
tunnels. Minecraft's optional hardware probes may log JNA/glibc warnings on
Android; verify the server reaches Online rather than treating a hardware probe
warning as a server crash.
