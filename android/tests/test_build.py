import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('apk_build', Path(__file__).parents[1] / 'build.py')
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class PackagingTests(unittest.TestCase):
    def test_runtime_preserves_modules_and_notices_without_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / 'jdk'
            files = {'release': b'JAVA_VERSION="21"', 'lib/modules': b'modules',
                     'lib/server/libjvm.so': b'ELF', 'legal/java.base/LICENSE': b'notice',
                     'bin/java': b'cannot execute from writable storage'}
            for name, data in files.items():
                path = home / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            with patch.object(build, 'BUILD', root), patch.object(build, 'run') as relocate:
                build.pack_runtime(home, root / 'runtime.zip', 'patchelf')
            with zipfile.ZipFile(root / 'runtime.zip') as archive:
                self.assertEqual(archive.read('legal/java.base/LICENSE'), b'notice')
                self.assertEqual(archive.read('lib/modules'), b'modules')
                self.assertNotIn('bin/java', archive.namelist())
            relocate.assert_called_once()
            self.assertIn('$ORIGIN/..', relocate.call_args.args[2])

    def test_runtime_rejects_links_outside_its_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('release', 'lib/modules', 'lib/server/libjvm.so'):
                path = root / 'jdk' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'JAVA_VERSION="21"' if name == 'release' else b'data')
            (root / 'jdk/legal').mkdir()
            (root / 'secret').write_bytes(b'not part of runtime')
            (root / 'jdk/lib/escape').symlink_to(root / 'secret')
            with self.assertRaisesRegex(ValueError, 'escapes'), patch.object(build, 'BUILD', root), patch.object(build, 'run'):
                build.pack_runtime(root / 'jdk', root / 'runtime.zip', 'patchelf')

    def test_resources_are_stored_and_aligned_and_native_code_is_extracted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with zipfile.ZipFile(root / 'input.apk', 'w') as archive:
                archive.writestr('AndroidManifest.xml', b'manifest')
                archive.writestr('resources.arsc', b'resources')
            build.aligned_apk(root / 'input.apk', root / 'output.apk',
                              [('lib/arm64-v8a/libmcpanel.so', b'native'), ('classes.dex', b'dex')])
            with zipfile.ZipFile(root / 'output.apk') as archive, open(root / 'output.apk', 'rb') as raw:
                item = archive.getinfo('resources.arsc')
                raw.seek(item.header_offset)
                header = raw.read(30)
                name, extra = struct.unpack_from('<HH', header, 26)
                self.assertEqual((item.header_offset + 30 + name + extra) % 4, 0)
                self.assertEqual(item.compress_type, zipfile.ZIP_STORED)
                self.assertEqual(archive.getinfo('lib/arm64-v8a/libmcpanel.so').compress_type, zipfile.ZIP_DEFLATED)
                self.assertIsNone(archive.testzip())


if __name__ == '__main__':
    unittest.main()
