#!/usr/bin/env python3
"""Start a disposable vanilla server through the APK's JNI launcher and save it."""
import argparse
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import tempfile
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--launcher', type=Path, required=True)
p.add_argument('--runtime', type=Path, required=True)
p.add_argument('--server-jar', type=Path, required=True)
p.add_argument('--accept-eula', action='store_true')
a = p.parse_args()
if not a.accept_eula:
    p.error('Read https://aka.ms/MinecraftEULA and explicitly pass --accept-eula')
env = {'PATH': '/system/bin', 'MCPANEL_ANDROID_JAVA_HOME': str(a.runtime.resolve())}
launcher = str(a.launcher.resolve())
subprocess.run([launcher, '-version'], env=env, check=True)
with tempfile.TemporaryDirectory(prefix='mcpanel-apk-smoke-') as directory:
    root = Path(directory)
    shutil.copyfile(a.server_jar, root / 'server.jar')
    (root / 'eula.txt').write_text('eula=true\n')
    (root / 'server.properties').write_text('server-ip=127.0.0.1\nserver-port=0\nonline-mode=false\nview-distance=2\nsimulation-distance=2\nmax-players=1\nspawn-protection=0\nlevel-type=minecraft:flat\ngenerate-structures=false\n')
    child = subprocess.Popen([launcher, '-Xms128M', '-Xmx512M', '-XX:ActiveProcessorCount=2', '-jar', str(root / 'server.jar'), 'nogui'],
                             cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ)
    output = bytearray()
    started = False
    deadline = time.monotonic() + 180
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(1):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if chunk:
                    output.extend(chunk)
                    print(chunk.decode(errors="replace"), end="", flush=True)
                    if not started and b'Done (' in output:
                        started = True
                        child.stdin.write(b'say APK launcher smoke test\nstop\n')
                        child.stdin.flush()
                else:
                    selector.unregister(key.fileobj)
            if child.poll() is not None:
                break
        if child.poll() is None:
            raise RuntimeError('Minecraft startup/shutdown timed out')
        if child.returncode or not started or b'Saving worlds' not in output:
            raise RuntimeError(f'Minecraft failed (exit {child.returncode})')
        print('PASS: JNI startup, vanilla Minecraft ready, console command, graceful world save and exit')
    except BaseException:
        print(output.decode(errors='replace'))
        raise
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        selector.close()
