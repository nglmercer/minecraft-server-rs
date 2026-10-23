# Standalone Android APK

This experimental ARM64 app runs MCPANEL and a bundled Android Java 21 runtime
without Termux installed. The existing web panel runs inside a local WebView;
Minecraft runs as a child process of the native Rust backend. Minimum Android
version is 13 (API 33); target SDK is 35. This initial build targets devices with
4 KB memory pages. A runtime with 16 KB compatible ELF libraries is required
before supporting 16 KB devices.

The APK keeps worlds and accounts in its own private storage. It creates no
preset account: finish the normal first-run setup to choose an administrator
password. Existing Termux worlds/accounts are not imported or modified. Close
the screen to leave the host running; use **Stop host** in the notification to
save worlds and stop the backend. Force-stopping/uninstalling the app does not
provide a graceful world save. Uninstalling deletes private app data; export
backups from the panel first.

## Build on Termux

Install Python 3.10+, Node/npm, Rust, OpenJDK 21, clang, binutils, patchelf,
`aapt2`, `d8`, and `apksigner`. Use the Google Android SDK Platform 35's
`android.jar` (`sdkmanager 'platforms;android-35'` on a supported SDK host), and
copy that platform JAR to the phone. SDK platform files are architecture
independent; SDK Linux native executables are not compatible with Termux.

```sh
pkg install python nodejs-lts rust openjdk-21 clang binutils patchelf aapt2 d8 apksigner
export JAVA_HOME="$PREFIX/lib/jvm/java-21-openjdk"
# Set this to the downloaded/copied SDK platform JAR.
export ANDROID_JAR="$HOME/android-sdk/platforms/android-35/android.jar"

npm --prefix web ci
npm --prefix web run build
cargo build --locked --profile apk -p panel --bin mcpanel

python android/prepare-runtime.py \
  --java-home "$JAVA_HOME" --library-dir "$PREFIX/lib" \
  --notices "$PREFIX/share/doc" \
  --output "$HOME/.cache/mcpanel-apk/runtime"

python android/build.py \
  --runtime "$HOME/.cache/mcpanel-apk/runtime" \
  --panel target/apk/mcpanel --android-jar "$ANDROID_JAR" \
  --java-home "$JAVA_HOME" \
  --d8-jar "$PREFIX/share/java/d8.jar" \
  --apksigner-jar "$PREFIX/share/java/apksigner.jar"
```

Some terminal wrappers inject a conflicting `LD_LIBRARY_PATH`. If tools fail
with missing C++ symbols, run the commands with `env -u LD_LIBRARY_PATH`.
`prepare-runtime.py` requires a new output directory; preserve the previous
runtime while a host is running. Install all OpenJDK package dependencies, or
supply additional extracted package directories with repeated `--library-dir`.

The output is `android/mcpanel-arm64-debug.apk`, signed with a local debug key
in `~/.cache/mcpanel-apk/debug.keystore`. Keep this key to install upgrades over
your previous debug builds. Build files, APKs, runtime files and signing keys
are not committed. Install the resulting APK through Android's installer, or
`adb install -r android/mcpanel-arm64-debug.apk` when a debugging device is
connected.

## Build on Linux

Use a host JDK 21, Python, Node/npm, the Android SDK platform 35/build tools
(aapt2, d8.jar and apksigner.jar), and an ARM64 Android NDK compiler. Build the
web bundle, then cross-compile the panel with the existing NDK wrapper:

```sh
rustup target add aarch64-linux-android
npm --prefix web ci
npm --prefix web run build
scripts/cargo-android.sh build --locked --profile apk -p panel --bin mcpanel

python android/build.py \
  --runtime /path/to/prepared-android-java21-runtime \
  --panel target/aarch64-linux-android/apk/mcpanel \
  --android-jar "$ANDROID_HOME/platforms/android-35/android.jar" \
  --java-home "$JAVA_HOME" \
  --cc "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android33-clang" \
  --aapt2 "$ANDROID_HOME/build-tools/35.0.0/aapt2" \
  --d8-jar "$ANDROID_HOME/build-tools/35.0.0/lib/d8.jar" \
  --apksigner-jar "$ANDROID_HOME/build-tools/35.0.0/lib/apksigner.jar"
```

The runtime must be an **Android/Bionic ARM64** build, with `release`,
`lib/modules`, `lib/server/libjvm.so`, all native dependencies and legal notices.
A desktop Linux/glibc JRE will not work. The preparer also runs on Linux with
an unpacked Termux JDK and unpacked dependency packages. Its shared libraries
are relocated to use `$ORIGIN`; the JNI launcher provides the runtime home.
Use a maintained Java 21 release. No third-party runtime is downloaded
implicitly or pinned to an obsolete portable Java build.

For release signing, add `--keystore /private/release.jks --key-alias YOUR_ALIAS`
and provide `MCPANEL_KEYSTORE_PASSWORD` in the environment. Release keys must
never be committed. The script verifies the APK signature after signing.

## Validation

```sh
python -m unittest discover -s android/tests -p 'test_*.py'
cargo test -p guardian
# Explicit acceptance of the Minecraft EULA is required for this smoke test.
python android/tests/smoke-launcher.py \
  --launcher android/build/native/libmcjava.so \
  --runtime /path/to/prepared-android-runtime \
  --server-jar /path/to/vanilla-1.21.8-server.jar --accept-eula
```

The development build was checked on an ARM64 Android 16 device with 4 KB
pages and Java 21.0.13. APK signing/integrity checks passed. The APK-extracted
backend passed setup/login, Java discovery, real vanilla 1.21.8 provisioning,
console commands, restart/stop, file editing, backup/restore and panel shutdown
with a running JVM. Guardian unit/integration/doc tests (105), clippy and three
packaging tests passed. These checks ran from Termux; the APK's Android activity
and foreground service have not yet been installed/tested in their own app UID.

The launcher smoke test uses a disposable world and an ephemeral loopback game
port; it verifies startup, console input, graceful stop and world save. It does
not install the APK or validate Android service lifecycle behavior. On-device
acceptance must additionally check first-run setup, provisioning and starting
a server, closing/reopening the activity, notification stop, file upload and
backup export. Test APK installation on a clean Android profile without Termux
before calling standalone device support stable.

## Implementation

Android blocks execution of writable app-home files for modern targets. The
Rust executable and JNI Java launcher are installed as extracted native APK
libraries (`libmcpanel.so`, `libmcjava.so`). Java modules/configuration/shared
libraries are unpacked into private runtime storage, preserving HotSpot's
expected layout. `bin/java` links to the installed launcher. The guardian
compares canonical launcher paths, rejects server-owned runtime homes and
forwards the app's runtime setting only to its matching launcher. General
credential and preload environment filtering remains active.

The foreground service declares connected-device use for Minecraft network
clients and holds a partial wake lock. It binds the panel to a random loopback
port. WebView navigation/resources are restricted to that exact origin; HTTPS
links open outside the app. No JavaScript-to-native bridge or file URL access
is enabled. Document pickers handle uploads and downloads without broad storage
permissions. Minecraft mods/plugins share the app's Android UID and must be
trusted; there is no OS isolation between servers inside the app.

Sources: [Android execution restriction](https://developer.android.com/about/versions/10/behavior-changes-10#execute-permission),
[foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types),
[16 KB page sizes](https://developer.android.com/guide/practices/page-sizes),
[Termux packages](https://github.com/termux/termux-packages).
