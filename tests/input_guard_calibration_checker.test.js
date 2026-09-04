'use strict'

const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

const repositoryRoot = path.join(__dirname, '..')
const checkerPath = path.join(repositoryRoot, 'scripts', 'check-input-guard-calibration.sh')
const prefix = 'checker-test'
const guardBinary = process.execPath
const captureApplication = 'Finder'
const launchApplicationPath = '/System/Library/CoreServices/Finder.app'
const guardClockBase = Date.UTC(2026, 7, 31, 12, 0, 0, 0)
const minimalJpeg = Buffer.from(
    [
        '/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkS',
        'Ew8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJ',
        'CQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIy',
        'MjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEA',
        'AAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIh',
        'MUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6',
        'Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZ',
        'mqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx',
        '8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA',
        'AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAV',
        'YnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hp',
        'anN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPE',
        'xcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD3',
        '+iiigD//2Q=='
    ].join(''),
    'base64'
)
const counterNames = [
    'flags_changed',
    'key_down',
    'key_up',
    'left_mouse_down',
    'left_mouse_dragged',
    'left_mouse_up',
    'mouse_moved',
    'other_mouse_down',
    'other_mouse_dragged',
    'other_mouse_up',
    'right_mouse_down',
    'right_mouse_dragged',
    'right_mouse_up',
    'scroll_wheel',
    'tablet_pointer',
    'tablet_proximity'
]

function sha256Bytes(bytes) {
    return crypto.createHash('sha256').update(bytes).digest('hex')
}

function sha256File(file) {
    return sha256Bytes(fs.readFileSync(file))
}

function writeJsonl(file, records) {
    fs.writeFileSync(file, `${records.map((record) => JSON.stringify(record)).join('\n')}\n`)
}

function readJsonl(file) {
    return fs.readFileSync(file, 'utf8').trimEnd().split('\n').map(JSON.parse)
}

function timestamp(milliseconds) {
    return new Date(guardClockBase + milliseconds).toISOString()
}

function counters(overrides = {}) {
    return Object.fromEntries(counterNames.map((name) => [name, overrides[name] || 0]))
}

function identity(index) {
    return {
        guard_session_id: sha256Bytes(Buffer.from(`guard-session-${index}`)),
        guard_process_id: 41000 + index,
        guard_started_at_unix_ms: guardClockBase + index
    }
}

function guardRecord(processIdentity, sequence, record) {
    return {
        version: 5,
        coverage: 'action_windows',
        policy: 'consequential_input_only',
        ...processIdentity,
        record_sequence: sequence,
        recorded_at_unix_ms: guardClockBase + sequence * 40,
        ...record
    }
}

function successfulSampleTiming(sequence, phase) {
    const ordinal = phase === 'arm' ? sequence / 2 : (sequence - 1) / 2
    const offset = ordinal * 40
    const phaseOffset = phase === 'arm' ? 2 : 32
    return {
        sample_started_at_unix_ms: guardClockBase + offset + phaseOffset,
        sample_completed_at_unix_ms: guardClockBase + offset + phaseOffset + 1,
        recorded_at_unix_ms: guardClockBase + offset + phaseOffset + 2
    }
}

function rejectionSampleTiming() {
    return {
        sample_started_at_unix_ms: guardClockBase + 55,
        sample_completed_at_unix_ms: guardClockBase + 65,
        recorded_at_unix_ms: guardClockBase + 66
    }
}

function ready(processIdentity, sequence = 1) {
    return guardRecord(processIdentity, sequence, {
        type: 'ready',
        source: 'hid_system_state',
        privacy: 'counts_and_held_state_boolean'
    })
}

function armed(processIdentity, sequence, actionId) {
    return guardRecord(processIdentity, sequence, {
        type: 'armed',
        source: 'hid_system_state',
        action_id: actionId,
        ...successfulSampleTiming(sequence, 'arm')
    })
}

function result(processIdentity, sequence, actionId, deltas, interferenceDetected) {
    return guardRecord(processIdentity, sequence, {
        type: 'result',
        source: 'hid_system_state',
        action_id: actionId,
        deltas,
        interference_detected: interferenceDetected,
        ...successfulSampleTiming(sequence, 'check')
    })
}

function guardError(processIdentity, sequence, code) {
    return guardRecord(processIdentity, sequence, {
        type: 'error',
        error: { code, message: code }
    })
}

function samplingError(processIdentity, sequence, command, actionId, code) {
    return guardRecord(processIdentity, sequence, {
        type: 'error',
        command,
        action_id: actionId,
        error: { code, message: code },
        ...rejectionSampleTiming()
    })
}

function finished(processIdentity, sequence) {
    return guardRecord(processIdentity, sequence, {
        type: 'finished',
        source: 'hid_system_state',
        interference_detected: false
    })
}

function safeName(value) {
    return value.replace(/[^A-Za-z0-9._-]/g, '-')
}

function syntheticJpeg(comment) {
    const payload = Buffer.from(comment, 'utf8')
    assert.ok(payload.length <= 65533, 'JPEG COM payload is too large')
    const length = payload.length + 2
    const commentHeader = Buffer.from([0xff, 0xfe, length >> 8, length & 0xff])
    return Buffer.concat([
        minimalJpeg.subarray(0, 2),
        commentHeader,
        payload,
        minimalJpeg.subarray(2)
    ])
}

function makeCapture(fixtureDirectory, actionId, phase, capturedAt, expectedCondition) {
    const basename = `${safeName(actionId)}-${phase}`
    const screenshotRelative = `screenshots/${basename}.jpg`
    const stateRelative = `states/${basename}.json`
    const screenshotBytes = syntheticJpeg(`synthetic calibration screenshot ${actionId} ${phase}`)
    const state = {
        capture_format_version: 1,
        captured_at: capturedAt,
        app: captureApplication,
        text: `synthetic state for ${actionId} ${phase}`
    }
    const stateBytes = Buffer.from(`${JSON.stringify(state)}\n`, 'utf8')

    fs.writeFileSync(path.join(fixtureDirectory, screenshotRelative), screenshotBytes)
    fs.writeFileSync(path.join(fixtureDirectory, stateRelative), stateBytes)

    return {
        fresh: true,
        expected_condition_confirmed: expectedCondition,
        captured_at: capturedAt,
        app: captureApplication,
        screenshot_path: screenshotRelative,
        screenshot_sha256: sha256Bytes(screenshotBytes),
        state_path: stateRelative,
        state_sha256: sha256Bytes(stateBytes)
    }
}

function actionTimes(ordinal) {
    const offset = ordinal * 40
    return {
        pre: timestamp(offset + 10),
        callStarted: timestamp(offset + 20),
        callEnded: timestamp(offset + 21),
        post: timestamp(offset + 30)
    }
}

function actionBase(fixtureDirectory, sessionId, actionId, ordinal, postExpected = true) {
    const times = actionTimes(ordinal)
    return {
        calibration_trace_version: 1,
        record_type: 'action',
        session_id: sessionId,
        action_id: actionId,
        ordinal,
        timestamp: times.callStarted,
        call_started_at: times.callStarted,
        call_ended_at: times.callEnded,
        pre_state: makeCapture(fixtureDirectory, actionId, 'pre', times.pre, true),
        post_state: makeCapture(fixtureDirectory, actionId, 'post', times.post, postExpected)
    }
}

function samplingRejectionActionBase(fixtureDirectory, sessionId, actionId) {
    const action = actionBase(fixtureDirectory, sessionId, actionId, 1)
    action.timestamp = timestamp(54)
    action.call_started_at = timestamp(54)
    action.call_ended_at = timestamp(66)
    return action
}

function sessionHeader(control, sessionId, processIdentity, guardSha, startedAt) {
    return {
        calibration_trace_version: 1,
        record_type: 'session',
        control,
        session_id: sessionId,
        started_at: startedAt,
        fresh_guard_process: true,
        guard_binary_sha256: guardSha,
        ...processIdentity
    }
}

function sessionEnd(sessionId, endedAt, exitStatus, terminal = {}) {
    return {
        calibration_trace_version: 1,
        record_type: 'session_end',
        session_id: sessionId,
        ended_at: endedAt,
        guard_process_exit_status: exitStatus,
        no_retry_within_process: true,
        ...terminal
    }
}

function writeProcessFiles(fixtureDirectory, name, guardRecords, traceRecords) {
    writeJsonl(path.join(fixtureDirectory, `${prefix}-${name}.jsonl`), guardRecords)
    fs.writeFileSync(path.join(fixtureDirectory, `${prefix}-${name}.stderr`), '')
    writeJsonl(path.join(fixtureDirectory, `${prefix}-${name}-trace.jsonl`), traceRecords)
}

function buildAutomation(fixtureDirectory, guardSha, processIdentity) {
    const sessionId = 'automation-session'
    const actionSpecs = [
        ['cal.automation.open', 'exec_command.open', {
            kind: 'os_open',
            resolved_fresh: true,
            exact_application_or_bundle_path: launchApplicationPath,
            absolute_document_path: '/tmp/synthetic-document'
        }],
        ['cal.automation.press-key', 'computer_use.press_key', {
            kind: 'key_chord', resolved_fresh: true, application: captureApplication, window: 'Window', keys: ['CMD', 'A']
        }],
        ['cal.automation.set-value', 'computer_use.set_value', {
            kind: 'semantic_element', resolved_fresh: true, application: captureApplication, window: 'Window',
            element_identity: 'value-field', value_sha256: sha256Bytes(Buffer.from('value'))
        }],
        ['cal.automation.semantic-click', 'computer_use.click.element', {
            kind: 'semantic_element', resolved_fresh: true, application: captureApplication, window: 'Window',
            element_identity: 'semantic-target', click_count: 1
        }],
        ['cal.automation.coordinate-single', 'computer_use.click.coordinate', {
            kind: 'window_coordinate', resolved_fresh: true, application: captureApplication, window: 'Window',
            x: 100, y: 120, click_count: 1
        }],
        ['cal.automation.coordinate-double', 'computer_use.click.coordinate', {
            kind: 'window_coordinate', resolved_fresh: true, application: captureApplication, window: 'Window',
            x: 140, y: 160, click_count: 2
        }]
    ]
    const guard = [ready(processIdentity)]
    const trace = [sessionHeader('automation_negative', sessionId, processIdentity, guardSha, timestamp(0))]

    actionSpecs.forEach(([actionId, api, targetBinding], index) => {
        const ordinal = index + 1
        guard.push(armed(processIdentity, ordinal * 2, actionId))
        guard.push(result(processIdentity, ordinal * 2 + 1, actionId, counters(), false))
        const action = {
            ...actionBase(fixtureDirectory, sessionId, actionId, ordinal),
            api,
            guard: {
                armed_observed: true,
                result_observed: true,
                interference_detected: false,
                consequential_deltas_zero: true
            },
            injected_call_count: 1,
            target_binding: targetBinding,
            call_succeeded: true,
            expected_postcondition: 'synthetic expected state',
            observed_postcondition: 'synthetic expected state observed',
            postcondition_confirmed: true,
            no_retry_within_process: true
        }
        if (targetBinding.kind === 'window_coordinate') {
            action.target_binding.fresh_screenshot_sha256 = action.pre_state.screenshot_sha256
        }
        trace.push(action)
    })

    guard.push(finished(processIdentity, 14))
    trace.push(sessionEnd(sessionId, timestamp(280), 0, {
        finished_observed: true
    }))
    writeProcessFiles(fixtureDirectory, 'automation', guard, trace)
}

function buildMove(fixtureDirectory, guardSha, processIdentity) {
    const sessionId = 'move-session'
    const specs = [
        ['cal.move-only.semantic', 'computer_use.click.element', {
            kind: 'semantic_element', resolved_fresh: true, application: captureApplication, window: 'Window',
            element_identity: 'move-semantic-target', click_count: 1
        }],
        ['cal.move-only.coordinate', 'computer_use.click.coordinate', {
            kind: 'window_coordinate', resolved_fresh: true, application: captureApplication, window: 'Window',
            x: 200, y: 220, click_count: 1
        }]
    ]
    const guard = [ready(processIdentity)]
    const trace = [sessionHeader('move_only_acceptance', sessionId, processIdentity, guardSha, timestamp(0))]

    specs.forEach(([actionId, api, targetBinding], index) => {
        const ordinal = index + 1
        guard.push(armed(processIdentity, ordinal * 2, actionId))
        guard.push(result(processIdentity, ordinal * 2 + 1, actionId, counters({ mouse_moved: ordinal }), false))
        const action = {
            ...actionBase(fixtureDirectory, sessionId, actionId, ordinal),
            api,
            guard: {
                armed_observed: true,
                result_observed: true,
                interference_detected: false,
                mouse_moved_at_least_one: true,
                consequential_deltas_zero: true
            },
            physical_input: {
                kind: 'pointer_move_only',
                operator_attested: true,
                began_after_armed: true,
                ended_before_check: true,
                no_button_key_scroll_or_drag: true
            },
            injected_call_count: 1,
            target_binding: targetBinding,
            call_succeeded: true,
            expected_postcondition: 'target action completed',
            observed_postcondition: 'target action observed',
            postcondition_confirmed: true,
            no_retry_within_process: true
        }
        if (targetBinding.kind === 'window_coordinate') {
            action.target_binding.fresh_screenshot_sha256 = action.pre_state.screenshot_sha256
        }
        trace.push(action)
    })

    guard.push(finished(processIdentity, 6))
    trace.push(sessionEnd(sessionId, timestamp(120), 0, { finished_observed: true }))
    writeProcessFiles(fixtureDirectory, 'move', guard, trace)
}

function buildPositive(fixtureDirectory, guardSha, processIdentity, name, kind, actionId, physicalKind, deltas) {
    const sessionId = `${name}-session`
    const guard = [
        ready(processIdentity),
        armed(processIdentity, 2, actionId),
        result(processIdentity, 3, actionId, counters(deltas), true),
        guardError(processIdentity, 4, 'INTERFERENCE_LATCHED')
    ]
    const action = {
        ...actionBase(fixtureDirectory, sessionId, actionId, 1),
        api: 'physical_input',
        injected_call_count: 0,
        physical_input: {
            kind: physicalKind,
            operator_attested: true,
            began_after_armed: true,
            ended_before_check: true,
            no_additional_input_class: true
        },
        guard: {
            armed_observed: true,
            result_observed: true,
            interference_detected: true,
            expected_counter_positive: true
        },
        finish_sent_after_result: true,
        latched_finish_error_confirmed: true,
        continued_after_latch: false,
        no_retry_within_process: true
    }
    const trace = [
        sessionHeader(`consequential_positive_${kind}`, sessionId, processIdentity, guardSha, timestamp(0)),
        action,
        sessionEnd(sessionId, timestamp(80), 1, { terminal_guard_error: 'INTERFERENCE_LATCHED' })
    ]
    writeProcessFiles(fixtureDirectory, name, guard, trace)
}

function buildWrongTarget(fixtureDirectory, guardSha, processIdentity) {
    const sessionId = 'wrong-target-session'
    const actionId = 'cal.wrong-target'
    const guard = [
        ready(processIdentity),
        armed(processIdentity, 2, actionId),
        result(processIdentity, 3, actionId, counters(), false),
        guardRecord(processIdentity, 4, {
            type: 'rejected',
            action_id: actionId,
            reason: 'postcondition_failed',
            after_clean_result: true,
            session_aborted: true
        }),
        guardError(processIdentity, 5, 'SESSION_ABORTED')
    ]
    const action = {
        ...actionBase(fixtureDirectory, sessionId, actionId, 1, false),
        api: 'computer_use.click.coordinate',
        injected_call_count: 1,
        target_binding: {
            kind: 'window_coordinate',
            resolved_fresh: true,
            application: captureApplication,
            window: 'Window',
            x: 300,
            y: 320,
            click_count: 1,
            coordinate_within_current_window: true,
            intentionally_wrong: true
        },
        call_succeeded: true,
        expected_postcondition: 'intended element selected',
        observed_postcondition: 'different element selected',
        postcondition_confirmed: false,
        check_sent: true,
        reject_sent: true,
        cancel_sent: false,
        guard: {
            armed_observed: true,
            result_observed: true,
            interference_detected: false,
            consequential_deltas_zero: true,
            rejected_observed: true,
            reject_reason: 'postcondition_failed',
            reject_after_clean_result: true,
            session_aborted_finish_error_observed: true
        },
        run_rejected: true,
        no_retry_within_process: true
    }
    action.target_binding.fresh_screenshot_sha256 = action.pre_state.screenshot_sha256
    const trace = [
        sessionHeader('target_binding_rejection', sessionId, processIdentity, guardSha, timestamp(0)),
        action,
        sessionEnd(sessionId, timestamp(80), 1, { terminal_guard_error: 'SESSION_ABORTED' })
    ]
    writeProcessFiles(fixtureDirectory, 'wrong-target', guard, trace)
}

function buildHeldStateRejection(fixtureDirectory, guardSha, processIdentity) {
    const sessionId = 'held-state-rejection-session'
    const actionId = 'cal.held-state-rejection'
    const guard = [ready(processIdentity), samplingError(processIdentity, 2, 'arm', actionId, 'KEY_HELD')]
    const action = {
        ...samplingRejectionActionBase(fixtureDirectory, sessionId, actionId),
        api: 'physical_input',
        injected_call_count: 0,
        guard_command_phase: 'arm',
        mode: 'held_state',
        physical_input: {
            kind: 'keyboard_key_held',
            operator_attested: true,
            began_before_arm_command: true,
            held_through_sample: true
        },
        guard: {
            ready_observed: true,
            armed_observed: false,
            observed_error: 'KEY_HELD'
        },
        no_retry_within_process: true
    }
    const trace = [
        sessionHeader('held_state_rejection', sessionId, processIdentity, guardSha, timestamp(0)),
        action,
        sessionEnd(sessionId, timestamp(80), 1, { terminal_guard_error: 'KEY_HELD' })
    ]
    writeProcessFiles(fixtureDirectory, 'held-state-rejection', guard, trace)
}

function buildSampleRaceRejection(fixtureDirectory, guardSha, processIdentity) {
    const sessionId = 'sample-race-rejection-session'
    const actionId = 'cal.sample-race-rejection'
    const guard = [
        ready(processIdentity),
        samplingError(processIdentity, 2, 'arm', actionId, 'INPUT_DURING_SAMPLE')
    ]
    const action = {
        ...samplingRejectionActionBase(fixtureDirectory, sessionId, actionId),
        api: 'physical_input',
        injected_call_count: 0,
        guard_command_phase: 'arm',
        mode: 'sample_race',
        physical_input: {
            kind: 'pointer_move_during_sample',
            operator_attested: true,
            continuous_during_arm_sample: true
        },
        guard: {
            ready_observed: true,
            armed_observed: false,
            observed_error: 'INPUT_DURING_SAMPLE'
        },
        no_retry_within_process: true
    }
    const trace = [
        sessionHeader('sample_race_rejection', sessionId, processIdentity, guardSha, timestamp(0)),
        action,
        sessionEnd(sessionId, timestamp(80), 1, { terminal_guard_error: 'INPUT_DURING_SAMPLE' })
    ]
    writeProcessFiles(fixtureDirectory, 'sample-race-rejection', guard, trace)
}

function buildFixture(fixtureDirectory, guardSha) {
    fs.mkdirSync(fixtureDirectory)
    fs.mkdirSync(path.join(fixtureDirectory, 'screenshots'))
    fs.mkdirSync(path.join(fixtureDirectory, 'states'))

    buildAutomation(fixtureDirectory, guardSha, identity(1))
    buildMove(fixtureDirectory, guardSha, identity(2))
    buildPositive(
        fixtureDirectory, guardSha, identity(3), 'positive-click', 'click',
        'cal.physical-click', 'left_mouse_click', { left_mouse_down: 1, left_mouse_up: 1 }
    )
    buildPositive(
        fixtureDirectory, guardSha, identity(4), 'positive-key', 'key',
        'cal.physical-key', 'keyboard_nonmodifier_tap', { key_down: 1, key_up: 1 }
    )
    buildPositive(
        fixtureDirectory, guardSha, identity(5), 'positive-scroll', 'scroll',
        'cal.physical-scroll', 'scroll', { scroll_wheel: 1 }
    )
    buildPositive(
        fixtureDirectory, guardSha, identity(6), 'positive-drag', 'drag',
        'cal.physical-drag', 'left_mouse_drag', {
            left_mouse_down: 1, left_mouse_dragged: 1, left_mouse_up: 1, mouse_moved: 1
        }
    )
    buildWrongTarget(fixtureDirectory, guardSha, identity(7))
    buildHeldStateRejection(fixtureDirectory, guardSha, identity(8))
    buildSampleRaceRejection(fixtureDirectory, guardSha, identity(9))
}

function runChecker(fixtureDirectory, guardSha) {
    return spawnSync('bash', [checkerPath, fixtureDirectory, prefix, guardBinary, guardSha], {
        cwd: repositoryRoot,
        encoding: 'utf8'
    })
}

function cloneFixture(source, destination) {
    fs.cpSync(source, destination, { recursive: true })
}

function mutateJsonl(file, mutation) {
    const records = readJsonl(file)
    mutation(records)
    writeJsonl(file, records)
}

function setCaptureTimestamp(fixtureDirectory, processName, recordIndex, stateKey, capturedAt) {
    const traceFile = path.join(fixtureDirectory, `${prefix}-${processName}-trace.jsonl`)
    mutateJsonl(traceFile, (records) => {
        const capture = records[recordIndex][stateKey]
        capture.captured_at = capturedAt
        const stateFile = path.join(fixtureDirectory, capture.state_path)
        const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'))
        state.captured_at = capturedAt
        fs.writeFileSync(stateFile, `${JSON.stringify(state)}\n`)
        capture.state_sha256 = sha256File(stateFile)
    })
}

function expectRejected(validFixture, root, guardSha, name, expectedError, mutation) {
    const fixture = path.join(root, name)
    cloneFixture(validFixture, fixture)
    mutation(fixture)
    const result = runChecker(fixture, guardSha)
    assert.notEqual(result.status, 0, `${name} unexpectedly passed:\n${result.stdout}`)
    assert.match(result.stderr, expectedError, `${name} failed for an unexpected reason`)
}

const shasum = spawnSync('shasum', ['-a', '256', guardBinary], { encoding: 'utf8' })
assert.equal(
    shasum.status,
    0,
    `the calibration checker requires shasum on macOS and Linux: ${shasum.stderr || shasum.error || ''}`
)
assert.ok(fs.statSync(guardBinary).isFile(), 'process.execPath must identify a regular guard-binary stand-in')
assert.equal(fs.lstatSync(guardBinary).isSymbolicLink(), false, 'checker rejects a symbolic-link guard binary')
const guardSha = shasum.stdout.trim().split(/\s+/)[0]
assert.equal(guardSha, sha256File(guardBinary), 'Node and shasum must agree on the guard binary digest')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-calibration-checker-test-'))
try {
    const validFixture = path.join(root, 'valid')
    buildFixture(validFixture, guardSha)

    const valid = runChecker(validFixture, guardSha)
    assert.equal(valid.status, 0, `valid fixture rejected:\nstdout: ${valid.stdout}\nstderr: ${valid.stderr}`)
    const report = JSON.parse(valid.stdout)
    assert.equal(report.status, 'valid')
    assert.equal(report.calibration_report_version, 3)
    assert.equal(report.guard_contract.version, 5)
    assert.equal(report.fresh_guard_identity_count, 9)
    assert.deepEqual(Object.keys(report.guard_sessions), [
        'automation',
        'move',
        'positive-click',
        'positive-key',
        'positive-scroll',
        'positive-drag',
        'wrong-target',
        'held-state-rejection',
        'sample-race-rejection'
    ])
    assert.deepEqual(report.controls.held_state_rejection, {
        mode: 'held_state',
        error_code: 'KEY_HELD',
        physical_input_kind: 'keyboard_key_held'
    })
    assert.deepEqual(report.controls.sample_race_rejection, {
        mode: 'sample_race',
        error_code: 'INPUT_DURING_SAMPLE',
        physical_input_kind: 'pointer_move_during_sample'
    })
    assert.equal(Object.hasOwn(report.controls, 'sample_guard_rejection'), false)

    const mouseHeldFixture = path.join(root, 'valid-mouse-held')
    cloneFixture(validFixture, mouseHeldFixture)
    mutateJsonl(path.join(mouseHeldFixture, `${prefix}-held-state-rejection.jsonl`), (records) => {
        records[1].error = { code: 'MOUSE_BUTTON_HELD', message: 'MOUSE_BUTTON_HELD' }
    })
    mutateJsonl(path.join(mouseHeldFixture, `${prefix}-held-state-rejection-trace.jsonl`), (records) => {
        records[1].physical_input = {
            kind: 'mouse_button_held',
            operator_attested: true,
            began_before_arm_command: true,
            held_through_sample: true
        }
        records[1].guard.observed_error = 'MOUSE_BUTTON_HELD'
        records[2].terminal_guard_error = 'MOUSE_BUTTON_HELD'
    })
    const validMouseHeld = runChecker(mouseHeldFixture, guardSha)
    assert.equal(
        validMouseHeld.status,
        0,
        `valid mouse-held fixture rejected:\nstdout: ${validMouseHeld.stdout}\nstderr: ${validMouseHeld.stderr}`
    )
    assert.deepEqual(JSON.parse(validMouseHeld.stdout).controls.held_state_rejection, {
        mode: 'held_state',
        error_code: 'MOUSE_BUTTON_HELD',
        physical_input_kind: 'mouse_button_held'
    })

    expectRejected(validFixture, root, guardSha, 'old-or-false-pass', /held-state rejection guard stream invalid/, (fixture) => {
        const guardFile = path.join(fixture, `${prefix}-held-state-rejection.jsonl`)
        mutateJsonl(guardFile, (records) => {
            records[1].error = { code: 'INPUT_DURING_SAMPLE', message: 'INPUT_DURING_SAMPLE' }
        })
        const traceFile = path.join(fixture, `${prefix}-held-state-rejection-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            records[1].mode = 'sample_race'
            records[1].physical_input = {
                kind: 'pointer_move_during_sample',
                operator_attested: true,
                continuous_during_arm_sample: true
            }
            records[1].guard.observed_error = 'INPUT_DURING_SAMPLE'
            records[2].terminal_guard_error = 'INPUT_DURING_SAMPLE'
        })
    })

    expectRejected(validFixture, root, guardSha, 'held-kind-code-mismatch', /held-state rejection trace invalid/, (fixture) => {
        const traceFile = path.join(fixture, `${prefix}-held-state-rejection-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            records[1].physical_input.kind = 'mouse_button_held'
        })
    })

    expectRejected(validFixture, root, guardSha, 'sample-race-wrong-kind', /sample-race rejection trace invalid/, (fixture) => {
        const traceFile = path.join(fixture, `${prefix}-sample-race-rejection-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            records[1].physical_input.kind = 'pointer_move_only'
        })
    })

    expectRejected(validFixture, root, guardSha, 'missing-independent-sample-race', /27 canonical files/, (fixture) => {
        for (const suffix of ['.jsonl', '.stderr', '-trace.jsonl']) {
            fs.rmSync(path.join(fixture, `${prefix}-sample-race-rejection${suffix}`))
        }
    })

    expectRejected(validFixture, root, guardSha, 'action-before-arm-recorded', /not enclosed by its guard arm\/check sampling window/, (fixture) => {
        setCaptureTimestamp(fixture, 'automation', 1, 'pre_state', timestamp(41))
    })

    expectRejected(validFixture, root, guardSha, 'post-capture-after-check-started', /not enclosed by its guard arm\/check sampling window/, (fixture) => {
        setCaptureTimestamp(fixture, 'automation', 1, 'post_state', timestamp(73))
    })

    expectRejected(validFixture, root, guardSha, 'held-input-starts-after-sample-start', /sample rejection is not time-bound/, (fixture) => {
        const traceFile = path.join(fixture, `${prefix}-held-state-rejection-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            records[1].timestamp = timestamp(56)
            records[1].call_started_at = timestamp(56)
        })
    })

    expectRejected(validFixture, root, guardSha, 'sample-race-input-ends-before-sample-completes', /sample rejection is not time-bound/, (fixture) => {
        const traceFile = path.join(fixture, `${prefix}-sample-race-rejection-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            records[1].call_ended_at = timestamp(64)
        })
    })

    expectRejected(validFixture, root, guardSha, 'extra-file', /unexpected calibration artifact/, (fixture) => {
        fs.writeFileSync(path.join(fixture, 'unreviewed-retry.txt'), 'must not be ignored\n')
    })

    expectRejected(validFixture, root, guardSha, 'screenshot-bytes', /screenshot digest mismatch/, (fixture) => {
        fs.appendFileSync(path.join(fixture, 'screenshots', 'cal.automation.open-pre.jpg'), Buffer.from([0x00]))
    })

    expectRejected(validFixture, root, guardSha, 'undecodable-screenshot', /(decoded screenshot format|screenshot (cannot be decoded|has no positive pixel))/, (fixture) => {
        const relativeScreenshot = 'screenshots/cal.automation.open-pre.jpg'
        const screenshotFile = path.join(fixture, relativeScreenshot)
        fs.writeFileSync(screenshotFile, Buffer.from([0xff, 0xd8, 0xff]))
        const updatedSha = sha256File(screenshotFile)
        const traceFile = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            const action = records.find(
                (record) => record.record_type === 'action' && record.pre_state.screenshot_path === relativeScreenshot
            )
            assert.ok(action)
            action.pre_state.screenshot_sha256 = updatedSha
            action.target_binding.fresh_screenshot_sha256 = updatedSha
        })
    })

    expectRejected(validFixture, root, guardSha, 'trace-identity', /trace session header does not bind/, (fixture) => {
        const trace = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(trace, (records) => {
            records[0].guard_session_id = 'f'.repeat(64)
        })
    })

    expectRejected(validFixture, root, guardSha, 'target-app-mismatch', /automation trace invalid/, (fixture) => {
        const trace = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(trace, (records) => {
            records[2].target_binding.application = '/Applications/Different.app'
        })
    })

    expectRejected(validFixture, root, guardSha, 'old-wrong-target-cancel', /wrong-target guard stream must be/, (fixture) => {
        const file = path.join(fixture, `${prefix}-wrong-target.jsonl`)
        const records = readJsonl(file)
        const cancelled = guardRecord(identity(7), 3, {
            type: 'cancelled',
            action_id: 'cal.wrong-target',
            session_aborted: true
        })
        const terminal = guardError(identity(7), 4, 'SESSION_ABORTED')
        writeJsonl(file, [records[0], records[1], cancelled, terminal])
    })

    expectRejected(validFixture, root, guardSha, 'state-extra-field', /canonical capture schema/, (fixture) => {
        const relativeState = 'states/cal.automation.open-pre.json'
        const stateFile = path.join(fixture, relativeState)
        const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'))
        state.untrusted_extra = true
        fs.writeFileSync(stateFile, `${JSON.stringify(state)}\n`)
        const updatedSha = sha256File(stateFile)
        const traceFile = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            const action = records.find((record) => record.record_type === 'action' && record.pre_state.state_path === relativeState)
            assert.ok(action)
            action.pre_state.state_sha256 = updatedSha
        })
    })

    expectRejected(validFixture, root, guardSha, 'state-multiple-json-values', /canonical capture schema/, (fixture) => {
        const relativeState = 'states/cal.automation.open-pre.json'
        const stateFile = path.join(fixture, relativeState)
        const validState = fs.readFileSync(stateFile, 'utf8')
        fs.writeFileSync(stateFile, `${JSON.stringify({ untrusted: true })}\n${validState}`)
        const updatedSha = sha256File(stateFile)
        const traceFile = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            const action = records.find(
                (record) => record.record_type === 'action' && record.pre_state.state_path === relativeState
            )
            assert.ok(action)
            action.pre_state.state_sha256 = updatedSha
        })
    })

    expectRejected(validFixture, root, guardSha, 'legacy-retry-field', /legacy ambiguous retry fields/, (fixture) => {
        const trace = path.join(fixture, `${prefix}-move-trace.jsonl`)
        mutateJsonl(trace, (records) => {
            records[0].completed_without_retry = true
        })
    })

    expectRejected(validFixture, root, guardSha, 'non-whitespace-stderr', /stderr contains non-whitespace/, (fixture) => {
        fs.writeFileSync(path.join(fixture, `${prefix}-positive-click.stderr`), 'unexpected diagnostic\n')
    })

    expectRejected(validFixture, root, guardSha, 'record-sequence', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-automation.jsonl`)
        mutateJsonl(guard, (records) => {
            records[1].record_sequence = 1
        })
    })

    expectRejected(validFixture, root, guardSha, 'legacy-guard-v4', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-automation.jsonl`)
        mutateJsonl(guard, (records) => {
            records.forEach((record) => { record.version = 4 })
        })
    })

    expectRejected(validFixture, root, guardSha, 'missing-recorded-at', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-automation.jsonl`)
        mutateJsonl(guard, (records) => {
            delete records[0].recorded_at_unix_ms
        })
    })

    expectRejected(validFixture, root, guardSha, 'missing-check-sample-completion', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-automation.jsonl`)
        mutateJsonl(guard, (records) => {
            delete records[2].sample_completed_at_unix_ms
        })
    })

    expectRejected(validFixture, root, guardSha, 'sample-completes-after-record', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-automation.jsonl`)
        mutateJsonl(guard, (records) => {
            records[1].sample_completed_at_unix_ms = records[1].recorded_at_unix_ms + 1
        })
    })

    expectRejected(validFixture, root, guardSha, 'sample-error-command-mismatch', /sample-race rejection guard stream invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-sample-race-rejection.jsonl`)
        mutateJsonl(guard, (records) => {
            records[1].command = 'check'
        })
    })

    expectRejected(validFixture, root, guardSha, 'sample-error-missing-timing-pair', /guard v5 identity, timing, or record sequence invalid/, (fixture) => {
        const guard = path.join(fixture, `${prefix}-sample-race-rejection.jsonl`)
        mutateJsonl(guard, (records) => {
            delete records[1].sample_started_at_unix_ms
            delete records[1].sample_completed_at_unix_ms
        })
    })

    expectRejected(validFixture, root, guardSha, 'duplicate-identity', /nine distinct fresh process identities/, (fixture) => {
        const automation = readJsonl(path.join(fixture, `${prefix}-automation.jsonl`))[0]
        const duplicateFields = {
            guard_session_id: automation.guard_session_id,
            guard_process_id: automation.guard_process_id,
            guard_started_at_unix_ms: automation.guard_started_at_unix_ms
        }
        const moveGuard = path.join(fixture, `${prefix}-move.jsonl`)
        mutateJsonl(moveGuard, (records) => {
            records.forEach((record) => Object.assign(record, duplicateFields))
        })
        const moveTrace = path.join(fixture, `${prefix}-move-trace.jsonl`)
        mutateJsonl(moveTrace, (records) => {
            Object.assign(records[0], duplicateFields)
        })
    })

    expectRejected(validFixture, root, guardSha, 'timestamp-order', /trace timestamps must .* strictly increase/, (fixture) => {
        const trace = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(trace, (records) => {
            records[1].pre_state.captured_at = records[1].call_started_at
        })
    })

    expectRejected(validFixture, root, guardSha, 'invalid-calendar-date', /trace timestamps must .* strictly increase/, (fixture) => {
        const traceFile = path.join(fixture, `${prefix}-automation-trace.jsonl`)
        mutateJsonl(traceFile, (records) => {
            for (const record of records) {
                for (const key of ['started_at', 'timestamp', 'call_started_at', 'call_ended_at', 'ended_at']) {
                    if (typeof record[key] === 'string') record[key] = record[key].replace('2026-08-31', '2026-02-31')
                }
                for (const stateKey of ['pre_state', 'post_state']) {
                    if (!record[stateKey]) continue
                    record[stateKey].captured_at = record[stateKey].captured_at.replace('2026-08-31', '2026-02-31')
                    const stateFile = path.join(fixture, record[stateKey].state_path)
                    const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'))
                    state.captured_at = record[stateKey].captured_at
                    fs.writeFileSync(stateFile, `${JSON.stringify(state)}\n`)
                    record[stateKey].state_sha256 = sha256File(stateFile)
                }
            }
        })
    })
} finally {
    fs.rmSync(root, { recursive: true, force: true })
}

console.log('input guard calibration checker tests passed')
