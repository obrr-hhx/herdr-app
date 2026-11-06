import unittest
from types import SimpleNamespace
from local_auth import subscription_environment

FAKE = 'sk-ant-oat01-' + 'synthetic-not-a-real-secret' * 2

class LocalAuthTest(unittest.TestCase):
    def test_inherited_token_bypasses_gateway_without_mutating_input(self):
        source = {'CLAUDE_CODE_OAUTH_TOKEN': FAKE, 'ANTHROPIC_AUTH_TOKEN': 'gateway', 'ANTHROPIC_BASE_URL': 'https://gateway.invalid'}
        env, enabled = subscription_environment(source)
        self.assertTrue(enabled)
        self.assertEqual(env['CLAUDE_CODE_OAUTH_TOKEN'], FAKE)
        self.assertEqual(env['ANTHROPIC_AUTH_TOKEN'], '')
        self.assertEqual(env['ANTHROPIC_BASE_URL'], 'https://api.anthropic.com')
        self.assertEqual(source['ANTHROPIC_AUTH_TOKEN'], 'gateway')

    def test_ssh_source_is_direct_and_secret_never_enters_command_arguments(self):
        def run(args, **kw):
            self.assertIn('fixture-host', args)
            self.assertNotIn(FAKE, str(args))
            self.assertFalse(any(k.lower().endswith('_proxy') for k in kw['env']))
            return SimpleNamespace(returncode=0, stdout=FAKE)
        env, enabled = subscription_environment({'HTTPS_PROXY': 'fixture', 'all_proxy': 'fixture'}, 'fixture-host', run)
        self.assertTrue(enabled)
        self.assertEqual(env['CLAUDE_CODE_OAUTH_TOKEN'], FAKE)

    def test_missing_source_never_falls_back_to_gateway(self):
        for result in [SimpleNamespace(returncode=1, stdout=''), SimpleNamespace(returncode=0, stdout=''), SimpleNamespace(returncode=0, stdout='not-an-oauth-token')]:
            with self.assertRaises(ValueError):
                subscription_environment({}, 'fixture-host', lambda *a, **kw: result)
        with self.assertRaises(ValueError):
            subscription_environment({}, '-oProxyCommand=bad')
        self.assertEqual(subscription_environment({}), ({}, False))

if __name__ == '__main__':
    unittest.main()
