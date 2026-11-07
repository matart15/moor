"""Guard the local signing workflow against regressions to ad-hoc-only signing."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class LocalSigningTests(unittest.TestCase):
    def test_persistent_identity_is_preferred_without_altering_keychain_acl(self):
        script = (ROOT / 'scripts/install-macos-dev.sh').read_text()
        self.assertIn("LOCAL_SIGNING_IDENTITY='Moor Dev Local Signing'", script)
        self.assertIn('security find-identity -v -p codesigning', script)
        self.assertIn('SIGNING_IDENTITY="$LOCAL_SIGNING_IDENTITY"', script)
        self.assertIn('codesign --force --deep --sign "$SIGNING_IDENTITY"', script)
        self.assertIn("SIGNING_IDENTITY='-'", script)
        self.assertIn('codesign --verify --deep --strict', script)
        for prohibited in ('set-key-partition-list', 'set-generic-password-partition-list',
                           'add-trusted-cert', 'set-keychain-settings', 'spctl --master-disable'):
            self.assertNotIn(prohibited, script)


if __name__ == '__main__':
    unittest.main()
