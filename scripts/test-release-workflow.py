#!/usr/bin/env python3
"""Validate the machine-consumed publication graph (requires PyYAML)."""

from pathlib import Path
import re
import unittest
import tomllib

import yaml

ROOT = Path(__file__).resolve().parent.parent


def load(name):
    # BaseLoader preserves GitHub's `on` key instead of YAML 1.1 boolean coercion.
    return yaml.load((ROOT / '.github/workflows' / name).read_text(), Loader=yaml.BaseLoader)


def condition(expression, results):
    expression = expression.removeprefix('${{').removesuffix('}}').strip()
    expression = expression.replace('always()', 'True').replace('&&', 'and').replace('||', 'or')
    expression = re.sub(r'needs\.([\w-]+)\.result',
                        lambda match: repr(results[match[1]]), expression)
    expression = expression.replace('needs.plan.outputs.publishing', "'true'")
    return eval(expression, {'__builtins__': {}}, {})


class ReleaseContract(unittest.TestCase):
    def test_exact_source_and_triggers(self):
        checks, release = load('checks.yml'), load('release.yml')
        self.assertEqual(set(checks['on']), {'schedule', 'workflow_dispatch', 'workflow_call'})
        self.assertEqual(checks['on']['schedule'], [{'cron': '17 3 * * *'}])
        self.assertEqual(set(release['on']), {'push'})
        self.assertEqual(set(release['on']['push']), {'tags'})
        for workflow in (checks, release):
            for job in workflow['jobs'].values():
                for step in job.get('steps', []):
                    if step.get('uses', '').startswith('actions/checkout@'):
                        if 'repository' not in step.get('with', {}):
                            self.assertEqual(step['with']['ref'], '${{ github.sha }}')

    def test_check_python_selection(self):
        steps = load('checks.yml')['jobs']['check']['steps']
        setup = [i for i, step in enumerate(steps)
                 if step.get('uses', '').startswith('actions/setup-python@')]
        self.assertEqual(len(setup), 1)
        selected = steps[setup[0]]
        self.assertEqual(selected['uses'], 'actions/setup-python@v7')
        self.assertEqual(selected['with']['python-version'], '3.14')
        self.assertEqual(selected['with']['check-latest'], 'true')
        self.assertNotIn('if', selected)
        self.assertNotEqual(selected.get('with', {}).get('update-environment'), 'false')
        consumers = [i for i, step in enumerate(steps)
                     if re.search(r'\b(?:python3?|pip3?)\b', step.get('run', ''))
                     or 'bash scripts/check-routine.sh' in step.get('run', '')]
        self.assertTrue(consumers)
        for index in consumers:
            self.assertLess(setup[0], index)
            self.assertNotIn('PATH', steps[index].get('env', {}))
            self.assertNotRegex(steps[index]['run'], r'(?:^|\n)\s*pip3?\s')
        self.assertTrue(any('python3 -m pip install ' in steps[i]['run']
                            for i in consumers))

    def test_repository_toolchain_selection(self):
        toolchain = tomllib.loads((ROOT / 'rust-toolchain.toml').read_text())['toolchain']
        self.assertRegex(toolchain['channel'], r'^\d+\.\d+\.\d+$')
        self.assertEqual(toolchain['profile'], 'minimal')
        self.assertEqual(toolchain['components'], ['clippy', 'rustfmt'])
        self.assertEqual(toolchain['targets'], ['x86_64-unknown-linux-musl'])
        for workflow, job_name in (('checks.yml', 'check'),
                                   ('release.yml', 'build-local-artifacts')):
            job = load(workflow)['jobs'][job_name]
            self.assertNotIn('RUSTUP_TOOLCHAIN', job.get('env', {}))
            steps = job['steps']
            selection = [i for i, step in enumerate(steps)
                         if step.get('run') == 'rustup show active-toolchain']
            self.assertEqual(len(selection), 1)
            self.assertNotIn('if', steps[selection[0]])
            self.assertTrue(any(step.get('uses', '').startswith('actions/checkout@')
                                for step in steps[:selection[0]]))
            build = next(i for i, step in enumerate(steps)
                         if 'dist build ' in step.get('run', '')
                         or 'bash scripts/check-routine.sh' in step.get('run', ''))
            self.assertLess(selection[0], build)
            for step in steps:
                self.assertNotIn('RUSTUP_TOOLCHAIN', step.get('env', {}))
                self.assertFalse(step.get('uses', '').startswith('dtolnay/rust-toolchain@'))
                if 'sh.rustup.rs' in step.get('run', ''):
                    self.assertIn('--default-toolchain none', step['run'])

    def test_fail_closed_publication(self):
        jobs = load('release.yml')['jobs']
        checks, plan, host = (jobs[name] for name in ('custom-checks', 'plan', 'host'))
        self.assertEqual(checks['uses'], './.github/workflows/checks.yml')
        self.assertNotIn('needs', checks)
        self.assertNotIn('if', checks)
        self.assertEqual(plan['needs'], ['custom-checks'])
        self.assertIn('custom-checks', host['needs'])
        for result in ('success', 'failure', 'cancelled', 'skipped'):
            for build in ('success', 'failure', 'cancelled', 'skipped'):
                results = {'custom-checks': result, 'plan': 'success',
                           'build-local-artifacts': build, 'build-global-artifacts': build}
                self.assertEqual(condition(plan['if'], results), result == 'success')
                self.assertEqual(condition(host['if'], results),
                                 result == 'success' and build in ('success', 'skipped'))
        for result in ('failure', 'cancelled', 'skipped'):
            self.assertFalse(condition(host['if'], {'custom-checks': 'success', 'plan': result,
                             'build-local-artifacts': 'skipped', 'build-global-artifacts': 'skipped'}))
        # All public upload/release steps reside behind host. Homebrew's implicit
        # success() requires host; announce explicitly requires host success.
        publishers = [(name, step.get('run', '')) for name, job in jobs.items()
                      for step in job.get('steps', [])
                      if '--steps=upload' in step.get('run', '') or 'gh release ' in step.get('run', '')]
        self.assertTrue(publishers)
        self.assertEqual({name for name, _ in publishers}, {'host'})
        brew = jobs['publish-homebrew-formula']
        self.assertIn('host', brew['needs'])
        self.assertNotRegex(brew['if'], r'always\(|failure\(|cancelled\(')
        self.assertIn("needs.host.result == 'success'", jobs['announce']['if'])


if __name__ == '__main__':
    unittest.main()
