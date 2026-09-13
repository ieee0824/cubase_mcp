'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const crypto = require('node:crypto')
const { spawn, spawnSync } = require('node:child_process')
const { test } = require('node:test')

const launcher = path.join(__dirname, '..', 'scripts', 'start-input-guard.sh')
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const digest = filename => crypto.createHash('sha256').update(fs.readFileSync(filename)).digest('hex')

// Fake executables test only launch/stdio ordering. They are not OS/HID evidence.
function fixture(t, body = 'printf "ready\\n"\nIFS= read -r line\nprintf "got:%s\\n" "$line"\n') {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-launcher-test-'))
    const binary = path.join(directory, 'guard with spaces')
    const log = path.join(directory, 'launch.jsonl')
    const marker = path.join(directory, 'executed')
    fs.writeFileSync(binary, '#!/bin/bash\nprintf x >> "$FAKE_EXEC_MARKER"\n' + body, { mode: 0o700 })
    const children = []
    t.after(async () => {
        for (const child of children) {
            if (child.exitCode === null && child.signalCode === null) {
                child.kill('SIGTERM')
                await child.done
            }
        }
        fs.rmSync(directory, { recursive: true, force: true })
    })
    function start(sha = digest(binary), launchLog = log, guard = binary) {
        const child = spawn('bash', [launcher, guard, sha, launchLog], {
            env: { ...process.env, FAKE_EXEC_MARKER: marker }, stdio: ['pipe', 'pipe', 'pipe']
        })
        child.output = ''; child.errors = ''
        child.stdout.on('data', data => { child.output += data })
        child.stderr.on('data', data => { child.errors += data })
        child.stdin.on('error', error => { if (error.code !== 'EPIPE') throw error })
        child.done = new Promise((resolve, reject) => {
            child.on('error', reject)
            child.on('close', (code, signal) => resolve({ code, signal }))
        })
        children.push(child)
        return child
    }
    async function ready(child) {
        const deadline = Date.now() + 3000
        while (Date.now() < deadline) {
            if (fs.existsSync(log) && fs.readFileSync(log, 'utf8').includes('awaiting_start')) return
            if (child.exitCode !== null) throw Error('launcher exited before waiting: ' + child.errors)
            await delay(10)
        }
        throw Error('launcher did not become ready')
    }
    return { directory, binary, log, marker, start, ready }
}

test('no child before explicit start; delay, same PID, clean stdio, and unread commands preserved', { timeout: 12000 }, async t => {
    const f = fixture(t, 'printf "pid:%s\\n" "$$"\nIFS= read -r line\nprintf "got:%s\\n" "$line"\n')
    const child = f.start()
    await f.ready(child)
    await delay(100)
    assert.equal(fs.existsSync(f.marker), false)
    assert.equal(child.output, '')
    assert.equal(child.errors, '')
    child.stdin.end('start\nfinish\n')
    await delay(100)
    assert.equal(fs.existsSync(f.marker), false, 'startup delay remains outside guard sampling')
    assert.deepEqual(await child.done, { code: 0, signal: null })
    const records = fs.readFileSync(f.log, 'utf8').trim().split('\n').map(JSON.parse)
    assert.deepEqual(records.map(r => r.phase), ['awaiting_start', 'start_received', 'launching_guard'])
    assert.equal(child.output, `pid:${records[0].process_id}\ngot:finish\n`)
    assert.equal(records[2].process_id, records[0].process_id)
    assert.equal(child.errors, '')
    assert.equal(fs.readFileSync(f.marker, 'utf8'), 'x')
    assert.equal(fs.statSync(f.log).mode & 0o777, 0o600)
})

for (const [name, input] of [
    ['EOF', ''], ['unterminated start', 'start'], ['early guard command', '{"command":"arm"}\n'],
    ['extra character', 'startX\n'], ['CRLF', 'start\r\n'], ['NUL', 'start\0\n'],
    ['oversize line', 'x'.repeat(8192) + '\n']
]) {
    test(`rejects ${name} without launching`, { timeout: 5000 }, async t => {
        const f = fixture(t)
        const child = f.start()
        await f.ready(child)
        child.stdin.end(input)
        assert.equal((await child.done).code, 1)
        assert.equal(fs.existsSync(f.marker), false)
        assert.equal(child.output, '')
        assert.match(child.errors, /guard was not launched/)
    })
}

test('wrong digest is rejected before readiness', async t => {
    const f = fixture(t)
    const child = f.start('0'.repeat(64))
    child.stdin.end('start\n')
    assert.equal((await child.done).code, 1)
    assert.equal(fs.existsSync(f.log), false)
    assert.equal(fs.existsSync(f.marker), false)
})

test('binary changed while awaiting start is rejected before exec', { timeout: 12000 }, async t => {
    const f = fixture(t)
    const child = f.start()
    await f.ready(child)
    fs.appendFileSync(f.binary, '\n# changed while awaiting start\n')
    child.stdin.end('start\nfinish\n')
    assert.equal((await child.done).code, 1)
    assert.match(child.errors, /guard digest mismatch/)
    assert.equal(fs.existsSync(f.marker), false)
})

test('existing log is preserved', async t => {
    const f = fixture(t)
    fs.writeFileSync(f.log, 'existing evidence\n')
    const child = f.start()
    child.stdin.end('start\n')
    assert.equal((await child.done).code, 1)
    assert.equal(fs.readFileSync(f.log, 'utf8'), 'existing evidence\n')
    assert.equal(fs.existsSync(f.marker), false)
})

test('dangling log symlink is rejected without creating its target', async t => {
    const f = fixture(t)
    const target = path.join(f.directory, 'absent')
    fs.symlinkSync(target, f.log)
    const child = f.start()
    child.stdin.end('start\n')
    assert.equal((await child.done).code, 1)
    assert.equal(fs.existsSync(target), false)
    assert.equal(fs.existsSync(f.marker), false)
})

test('symlink guard is rejected', async t => {
    const f = fixture(t)
    const alias = path.join(f.directory, 'guard-link')
    fs.symlinkSync(f.binary, alias)
    const child = f.start(digest(f.binary), f.log, alias)
    child.stdin.end('start\n')
    assert.equal((await child.done).code, 1)
    assert.equal(fs.existsSync(f.marker), false)
})

test('guard refusal is preserved verbatim and never retried', { timeout: 12000 }, async t => {
    const f = fixture(t, 'printf \'{"type":"error","error":{"code":"KEY_HELD"}}\\n\'\nexit 7\n')
    const child = f.start()
    await f.ready(child)
    child.stdin.end('start\n')
    assert.equal((await child.done).code, 7)
    assert.equal(child.output, '{"type":"error","error":{"code":"KEY_HELD"}}\n')
    assert.equal(child.errors, '')
    assert.equal(fs.readFileSync(f.marker, 'utf8'), 'x')
})

test('termination while awaiting start never launches the guard', { timeout: 5000 }, async t => {
    const f = fixture(t)
    const child = f.start()
    await f.ready(child)
    child.kill('SIGTERM')
    assert.equal((await child.done).signal, 'SIGTERM')
    assert.equal(fs.existsSync(f.marker), false)
})

test('argument contract rejects missing parameters', () => {
    const result = spawnSync('bash', [launcher], { encoding: 'utf8' })
    assert.equal(result.status, 1)
    assert.equal(result.stdout, '')
    assert.match(result.stderr, /usage:/)
})
