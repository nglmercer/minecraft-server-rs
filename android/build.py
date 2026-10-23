#!/usr/bin/env python3
"""Build a signed ARM64 APK using SDK tools; also works natively in Termux.

No Gradle daemon or downloaded build plugins. Run --help for tool overrides.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
BUILD = ROOT / "build"

def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)

def pack_runtime(home, output, patchelf):
    """Package data and shared libraries, excluding writable executables."""
    for required in ("release", "lib/modules", "lib/server/libjvm.so"):
        if not (home / required).is_file():
            raise ValueError(f"Android Java runtime is missing {required}")
    if not (home / "legal").is_dir():
        raise ValueError("Runtime legal notices are required")
    if not re.search(r'^JAVA_VERSION="21[.\"]', (home / "release").read_text(), re.MULTILINE):
        raise ValueError("This APK build requires an Android Java 21 runtime")
    # No scripts/executables are launched from app storage; bin/java is linked
    # to the installed JNI launcher by HostService after extraction.
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(home.rglob("*")):
            relative = path.relative_to(home)
            if relative.parts[0] == "bin" or not path.is_file():
                continue
            if path.is_symlink() and not path.resolve().is_relative_to(home.resolve()):
                raise ValueError(f"Runtime symlink escapes its root: {relative}")
            if path.suffix == ".so" or ".so." in path.name:
                # Android's loader must find dependencies beside each library,
                # including libnio -> libnet before Java explicitly loads it.
                patched = BUILD / "patched-library.so"
                shutil.copyfile(path, patched)
                run(patchelf, "--set-rpath", "$ORIGIN:$ORIGIN/server:$ORIGIN/..", patched)
                archive.write(patched, relative.as_posix())
            else:
                archive.write(path, relative.as_posix())

def aligned_apk(source, destination, additions):
    """Keep resources.arsc uncompressed and 4-byte aligned before signing.

    Native executables are compressed and extracted by Android's installer;
    their ZIP offsets need no mmap alignment.
    """
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination, "w") as output:
        entries = [(item.filename, original.read(item)) for item in original.infolist()]
        entries.extend(additions)
        for name, data in entries:
            info = zipfile.ZipInfo(name, (2024, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED if name == "resources.arsc" else zipfile.ZIP_DEFLATED
            if info.compress_type == zipfile.ZIP_STORED:
                offset = output.fp.tell() + 30 + len(name.encode())
                padding = (-offset) % 4
                if padding:
                    info.extra = struct.pack("<HH", 0xd935, padding) + b"\0" * padding
            output.writestr(info, data)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True, help="unpacked Android/Bionic Java 21 runtime, including legal notices")
    parser.add_argument("--panel", type=Path, required=True, help="ARM64 Android mcpanel built with embedded web assets (release or apk profile)")
    parser.add_argument("--android-jar", type=Path, required=True, help="SDK platforms/android-35/android.jar")
    parser.add_argument("--java-home", type=Path, default=os.environ.get("JAVA_HOME"), help="host JDK 21 for javac, keytool and signing")
    parser.add_argument("--cc", default=os.environ.get("CC", "clang"), help="Android ARM64 compiler (NDK aarch64-linux-android33-clang on Linux)")
    parser.add_argument("--aapt2", default="aapt2")
    parser.add_argument("--patchelf", default="patchelf", help="relocate runtime library dependencies")
    parser.add_argument("--d8-jar", type=Path, required=True)
    parser.add_argument("--apksigner-jar", type=Path, required=True)
    parser.add_argument("--keystore", type=Path, help="release signing key; omit to create/use a private local debug key")
    parser.add_argument("--key-alias", default="androiddebugkey")
    args = parser.parse_args()
    if not args.java_home:
        parser.error("--java-home or JAVA_HOME is required")
    args.java_home = args.java_home.resolve()
    for name in ("runtime", "panel", "android_jar", "d8_jar", "apksigner_jar"):
        setattr(args, name, getattr(args, name).resolve())
    BUILD.mkdir(exist_ok=True)
    for directory in ("assets", "classes", "dex", "native", "boot-stubs"):
        shutil.rmtree(BUILD / directory, ignore_errors=True)
        (BUILD / directory).mkdir()
    assets = BUILD / "assets"
    pack_runtime(args.runtime, assets / "runtime.zip", args.patchelf)
    digest = hashlib.sha256((assets / "runtime.zip").read_bytes()).hexdigest()
    (assets / "runtime.sha256").write_text(digest + "\n")
    java = args.java_home / "bin/java"
    javac = args.java_home / "bin/javac"
    run(args.cc, "--target=aarch64-linux-android33", "-Wall", "-Wextra", "-Werror", "-fPIE", "-pie", "-Wl,-z,max-page-size=16384",
        "-I", args.java_home / "include", "-I", args.java_home / "include/linux",
        ROOT / "native/java.c", "-ldl", "-o", BUILD / "native/libmcjava.so")
    shutil.copyfile(args.panel, BUILD / "native/libmcpanel.so")
    # Native Termux compilers may inject absolute Termux search paths. The
    # installed executables use only Bionic system libraries, so remove them.
    for executable in ("libmcpanel.so", "libmcjava.so"):
        run(args.patchelf, "--remove-rpath", BUILD / "native" / executable)
    (assets / "build-info.json").write_text(json.dumps({
        "abi": "arm64-v8a", "min_sdk": 33, "target_sdk": 35,
        "panel_sha256": hashlib.sha256((BUILD / "native/libmcpanel.so").read_bytes()).hexdigest(),
        "runtime_sha256": digest,
        "java_release": (args.runtime / "release").read_text(),
    }, indent=2) + "\n")
    # Read only ELF header: reject accidental desktop or 32-bit builds.
    for binary in (args.panel, BUILD / "native/libmcjava.so", args.runtime / "lib/server/libjvm.so"):
        header = binary.read_bytes()[:20]
        if header[:5] != b"\x7fELF\x02" or header[18:20] != b"\xb7\x00":
            parser.error(f"Expected ARM64 ELF binary: {binary}")
    run(args.aapt2, "compile", "--dir", ROOT / "res", "-o", BUILD / "resources.zip")
    run(args.aapt2, "link", "-I", args.android_jar, "--manifest", ROOT / "AndroidManifest.xml",
        "-A", assets, "-o", BUILD / "resources.apk", BUILD / "resources.zip")
    # Compile against Android APIs, never the desktop JDK API surface.
    # The SDK normally supplies core-lambda-stubs.jar; this compile-only
    # signature also permits native Termux builds without that SDK JAR.
    run(javac, "-source", "8", "-target", "8", "-Xlint:-options", "-bootclasspath", args.android_jar,
        "-d", BUILD / "boot-stubs", ROOT / "build-support/java/lang/invoke/LambdaMetafactory.java")
    bootclasspath = str(BUILD / "boot-stubs") + os.pathsep + str(args.android_jar)
    sources = sorted((ROOT / "src").rglob("*.java"))
    run(javac, "-source", "8", "-target", "8", "-Xlint:-options", "-bootclasspath", bootclasspath, "-d", BUILD / "classes", *sources)
    # D8 consumes JARs/classes rather than directories.
    with zipfile.ZipFile(BUILD / "classes.jar", "w") as classes:
        for path in sorted((BUILD / "classes").rglob("*.class")):
            classes.write(path, path.relative_to(BUILD / "classes").as_posix())
    run(java, "-cp", args.d8_jar, "com.android.tools.r8.D8", "--min-api", "33", "--lib", args.android_jar,
        "--output", BUILD / "dex", BUILD / "classes.jar")
    additions = [("lib/arm64-v8a/libmcpanel.so", (BUILD / "native/libmcpanel.so").read_bytes()),
                 ("lib/arm64-v8a/libmcjava.so", (BUILD / "native/libmcjava.so").read_bytes())]
    additions.extend((path.name, path.read_bytes()) for path in sorted((BUILD / "dex").glob("*.dex")))
    aligned_apk(BUILD / "resources.apk", BUILD / "unsigned.apk", additions)
    if args.keystore:
        key = args.keystore.resolve()
        password = os.environ.get("MCPANEL_KEYSTORE_PASSWORD")
        if not password:
            parser.error("Set MCPANEL_KEYSTORE_PASSWORD for release signing")
        signing = ["--ks-pass", "env:MCPANEL_KEYSTORE_PASSWORD"]
        variant = "release"
    else:
        key = Path.home() / ".cache/mcpanel-apk/debug.keystore"
        key.parent.mkdir(parents=True, exist_ok=True)
        if not key.exists():
            run(args.java_home / "bin/keytool", "-genkeypair", "-keystore", key, "-storepass", "android",
                "-keypass", "android", "-alias", args.key_alias, "-keyalg", "RSA", "-keysize", "2048",
                "-validity", "10000", "-dname", "CN=Android Debug,O=MCPANEL,C=US")
            key.chmod(0o600)
        signing = ["--ks-pass", "pass:android", "--key-pass", "pass:android"]
        variant = "debug"
    output = ROOT / f"mcpanel-arm64-{variant}.apk"
    run(java, "-jar", args.apksigner_jar, "sign", "--ks", key, "--ks-key-alias", args.key_alias,
        *signing, "--out", output, BUILD / "unsigned.apk")
    run(java, "-jar", args.apksigner_jar, "verify", "--verbose", output)
    print(f"APK: {output}\nSHA256: {hashlib.sha256(output.read_bytes()).hexdigest()}")

if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
