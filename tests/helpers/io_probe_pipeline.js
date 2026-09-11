'use strict'

// Test-only bridge: execute the actual driver with the same host stub used by
// its unit tests. Raw frames are passed unchanged to Rust; no MIDI ports exist.
const assert = require('node:assert/strict')
const fs = require('node:fs')
const crypto = require('node:crypto')
const readline = require('node:readline')
const path = require('node:path')
const { harness, wire, canary, directory } = require('./io_probe_harness')

if (process.argv[2] === 'driver') {
    const h = harness({ oldApi: process.argv[3] === 'old', autoActivate: false,
        onFrame(frame) { process.stdout.write(JSON.stringify(frame) + '\n') } })
    let started = false
    const input = readline.createInterface({ input: process.stdin })
    input.on('line', line => {
        assert.ok(line.length <= 300000)
        const command = JSON.parse(line)
        if (command.start === true) {
            assert.equal(started, false)
            started = true
            h.activate()
            h.zones[0].slots[0].mOnTitleChange(h.device, h.mapping, canary)
        } else {
            assert.equal(started, true)
            assert.ok(Array.isArray(command.frame))
            const request = wire.decode(command.frame, 4096)
            assert.ok(request)
            if (request.message.method === 'probe.bank.next') {
                assert.equal(request.message.params.config_id, 'IO_INPUT_ALL')
                // A host callback may arrive after the preceding idle but before
                // navigation. Keep it queued: only the real driver may emit it.
                h.zones[0].slots[1].mOnTitleChange(h.device, h.mapping,
                    canary + '-queued-before-navigation')
            }
            h.input.mOnSysex(h.device, command.frame)
        }
        h.idle()
    })
} else if (process.argv[2] === 'audit') {
    let input = ''
    process.stdin.setEncoding('utf8')
    process.stdin.on('data', part => {
        input += part
        assert.ok(input.length <= 4 * 1024 * 1024)
    })
    process.stdin.on('end', () => {
        const request = JSON.parse(input)
        const records = request.raw.trimEnd().split('\n').map(line => JSON.parse(line))
        const expected_probe_files = Object.fromEntries(
            ['CubaseMCPIOProbe_CubaseMCPIOProbe.js', 'io-profile.js', 'wire.js'].map(file =>
                [file, crypto.createHash('sha256').update(fs.readFileSync(path.join(directory, file))).digest('hex')]))
        const manifest = { version: 1, profile: 'io-existing-v1',
            expected_collector_sha256: request.collector_sha256, expected_probe_files,
            host: request.old_api ? { cubase_version: '13.0.30', api_version: '1.1' } :
                { cubase_version: '15.0.30', api_version: '1.3' }, ui_review: 'pending' }
        const report = require('../../scripts/audit-io-probe').audit(records, manifest)
        assert.equal(report.status, 'structurally_valid')
        assert.equal(report.runtime_acceptance, 'pending_ui_review')
        assert.equal(report.complete, false)
        assert.ok(!request.raw.includes(canary))
        process.stdout.write(JSON.stringify(report) + '\n')
    })
} else {
    throw new Error('Test helper requires driver or audit mode')
}
