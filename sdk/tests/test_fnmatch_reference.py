"""Compare the selected pinned newlib matcher with an independent host libc."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

@unittest.skipUnless(os.environ.get('MOCHIOS_NEWLIB_SOURCE'),
                     'Set MOCHIOS_NEWLIB_SOURCE to the verified pinned newlib checkout')
class FnmatchReferenceTests(unittest.TestCase):
    def test_selected_source_matches_reference_for_ascii_filename_patterns(self):
        source = Path(os.environ['MOCHIOS_NEWLIB_SOURCE'])/'newlib/libc/posix/fnmatch.c'
        self.assertTrue(source.is_file())
        compiler = shutil.which('cc')
        self.assertIsNotNone(compiler)
        with tempfile.TemporaryDirectory(prefix='mochios-fnmatch-') as tmp:
            tmp = Path(tmp)
            subprocess.run([compiler, '-D_GNU_SOURCE=1', '-Dfnmatch=mochi_fnmatch',
                '-include', ROOT/'newlib-fnmatch-collate.h', '-c', source,
                '-o', tmp/'matcher.o'], check=True, capture_output=True)
            subprocess.run([compiler, ROOT/'tests/fnmatch-reference.c', tmp/'matcher.o',
                '-o', tmp/'reference'], check=True, capture_output=True)
            result = subprocess.run([tmp/'reference'], check=True, capture_output=True, text=True)
            self.assertIn('720 filename matching cases agree', result.stdout)
