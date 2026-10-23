#!/usr/bin/env python3
"""Relocate a Termux OpenJDK runtime and its native dependencies for an APK.

Inputs can be extracted .deb packages; Termux itself is not required at run time.
The original legal notices are preserved. Supply package notices for copied
native dependencies in --notices. No Minecraft files or credentials are copied.
"""
import argparse
from pathlib import Path
import re
import shutil
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--java-home', type=Path, required=True)
p.add_argument('--library-dir', type=Path, action='append', required=True)
p.add_argument('--notices', type=Path, action='append', default=[])
p.add_argument('--output', type=Path, required=True)
p.add_argument('--patchelf', default='patchelf')
p.add_argument('--readelf', default='readelf')
a = p.parse_args()
home, output = a.java_home.resolve(), a.output.resolve()
if output.exists():
    p.error('Output must not already exist')
if not (home / 'legal').is_dir():
    p.error('JDK legal notices are required')
output.mkdir(parents=True)
for name in ('lib', 'conf', 'legal'):
    shutil.copytree(home / name, output / name, symlinks=False)
shutil.copyfile(home / 'release', output / 'release')
# JNI launcher replaces the writable jspawnhelper/Java executables.
for path in (output / 'lib').rglob('*'):
    if path.name in ('src.zip', 'jspawnhelper', 'jexec') and path.is_file():
        path.unlink()
system = {'libc.so', 'libm.so', 'libdl.so', 'liblog.so', 'libz.so', 'libandroid.so',
          'libjnigraphics.so', 'libnativewindow.so', 'libEGL.so', 'libGLESv2.so', 'libOpenSLES.so'}
queue = list((output / 'lib').rglob('*.so'))
seen = set()
while queue:
    library = queue.pop()
    if library in seen:
        continue
    seen.add(library)
    dynamic = subprocess.check_output([a.readelf, '-d', str(library)], text=True)
    for dependency in re.findall(r'\(NEEDED\).*?\[(.*?)\]', dynamic):
        if dependency in system:
            continue
        candidates = list((output / 'lib').rglob(dependency))
        if candidates:
            continue
        source = next((directory / dependency for directory in a.library_dir if (directory / dependency).is_file()), None)
        if source is None:
            raise SystemExit(f'Missing {dependency}, required by {library.name}; add an extracted package library directory')
        target = output / 'lib' / dependency
        shutil.copyfile(source, target)
        queue.append(target)
    subprocess.run([a.patchelf, '--set-rpath', '$ORIGIN:$ORIGIN/server:$ORIGIN/..', str(library)], check=True)
notices = output / 'legal' / 'native-dependencies'
notices.mkdir()
for index, source in enumerate(a.notices):
    if source.is_dir():
        shutil.copytree(source, notices / f'{index}-{source.name}')
    else:
        shutil.copyfile(source, notices / f'{index}-{source.name}')
print(f'Prepared {output}; audited {len(seen)} native libraries. Include dependency licenses with --notices before redistribution.')
