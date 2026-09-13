'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')
const { test } = require('node:test')

const script = path.join(__dirname, '..', 'scripts', 'check-input-guard-sampling.sh')

// Negative runner tests only. Fake Cargo must never supply acceptance evidence.
for (const [name, output, exitCode] of [
    ['no tests selected', 'test result: ok. 0 passed; 0 failed;', 0],
    ['marker without a passing test', 'CMCP_SAMPLING_CONTRACT_V1_PASS', 0],
    ['passing test without marker', 'test result: ok. 1 passed; 0 failed;', 0],
    ['nonzero exit despite success-looking output', 'CMCP_SAMPLING_CONTRACT_V1_PASS\ntest result: ok. 1 passed; 0 failed;', 1]
]) {
    test(name, () => {
        const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-sampling-runner-test-'))
        try {
            fs.writeFileSync(path.join(temporary, 'cargo'),
                `#!/bin/sh\nprintf '%s\\n' '${output}'\nexit ${exitCode}\n`, { mode: 0o700 })
            const result = spawnSync('bash', [script], {
                encoding: 'utf8',
                env: { ...process.env, PATH: `${temporary}${path.delimiter}${process.env.PATH}` }
            })
            assert.notEqual(result.status, 0)
            assert.equal(result.stdout, '', 'failed checks must not emit a passed report')
        } finally {
            fs.rmSync(temporary, { recursive: true, force: true })
        }
    })
}

test('does not accept a supplied report or test selector', () => {
    const result = spawnSync('bash', [script, 'passed.json'], { encoding: 'utf8' })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /usage:/)
    assert.equal(result.stdout, '')
})
