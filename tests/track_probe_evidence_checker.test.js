'use strict'

const assert = require('node:assert/strict')
const childProcess = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const checker = fs.readFileSync(
    path.join(__dirname, '..', 'scripts', 'check-track-probe-evidence.sh'),
    'utf8'
)

function numberAssignment(name) {
    const match = checker.match(new RegExp(`^${name}=([0-9]+)$`, 'm'))
    assert.ok(match, `missing numeric assignment: ${name}`)
    return Number(match[1])
}

function jsonAssignment(name) {
    const match = checker.match(new RegExp(`^${name}='([^']+)'$`, 'm'))
    assert.ok(match, `missing JSON assignment: ${name}`)
    return JSON.parse(match[1])
}

function stringAssignment(name) {
    const match = checker.match(new RegExp(`^${name}='([^']+)'$`, 'm'))
    assert.ok(match, `missing string assignment: ${name}`)
    return match[1]
}

const actionCount = numberAssignment('EXPECTED_ACTION_COUNT')
const checkpointCount = numberAssignment('EXPECTED_CHECKPOINT_COUNT')
const clickActionCount = numberAssignment('EXPECTED_CLICK_ACTION_COUNT')
const activationActionCount = numberAssignment('EXPECTED_ACTIVATION_ACTION_COUNT')
const nonCaptureIndexEntryCount = numberAssignment('EXPECTED_NON_CAPTURE_INDEX_ENTRY_COUNT')
const actionIds = jsonAssignment('EXPECTED_ACTION_IDS')
const checkpoints = jsonAssignment('EXPECTED_ACTION_CHECKPOINTS')
const apis = jsonAssignment('EXPECTED_ACTION_APIS')
const clickCounts = jsonAssignment('EXPECTED_ACTION_CLICK_COUNTS')
const activationOpenActionIds = jsonAssignment('EXPECTED_ACTIVATION_OPEN_ACTION_IDS')
const activationActionIds = jsonAssignment('EXPECTED_ACTIVATION_ACTION_IDS')
const activationDialogText = stringAssignment('EXPECTED_ACTIVATION_DIALOG_TEXT')
const affirmativeIdentity = stringAssignment('EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY')
const negativeIdentity = stringAssignment('EXPECTED_ACTIVATION_NEGATIVE_IDENTITY')

assert.equal(actionCount, 63)
assert.equal(checkpointCount, 44)
assert.equal(actionIds.length, actionCount)
assert.equal(checkpoints.length, actionCount)
assert.equal(apis.length, actionCount)
assert.equal(clickCounts.length, actionCount)

const activatedOpenActions = new Map([
    ['E0.open-project', 'E0.activate-project'],
    ['E1.open-project', 'E1.activate-project'],
    ['E8.open-project', 'E8.activate-project'],
    ['C1.open-project', 'C1.activate-project'],
    ['S9-empty.open-project', 'S9-empty.activate-project'],
    ['S9-mutation.open-project', 'S9-mutation.activate-project'],
    ['S9-baseline.open-project', 'S9-baseline.activate-project']
])

assert.deepEqual(
    actionIds.filter((actionId) => actionId.endsWith('.activate-project')),
    [...activatedOpenActions.values()]
)
assert.deepEqual(activationOpenActionIds, [...activatedOpenActions.keys()])
assert.deepEqual(activationActionIds, [...activatedOpenActions.values()])
assert.equal(activationActionCount, activatedOpenActions.size)

for (const [openAction, activateAction] of activatedOpenActions) {
    const openIndex = actionIds.indexOf(openAction)
    assert.notEqual(openIndex, -1, `missing open action: ${openAction}`)
    const activateIndex = openIndex + 1
    assert.equal(actionIds[activateIndex], activateAction)
    assert.equal(checkpoints[activateIndex], checkpoints[openIndex])
    assert.equal(apis[activateIndex], 'computer_use.click.element')
    assert.equal(clickCounts[activateIndex], 1)
}

for (const openAction of ['INIT.launch-bootstrap', 'R2.launch-baseline']) {
    const openIndex = actionIds.indexOf(openAction)
    assert.notEqual(openIndex, -1, `missing non-activation open action: ${openAction}`)
    assert.notEqual(actionIds[openIndex + 1], `${checkpoints[openIndex]}.activate-project`)
}

assert.equal(
    apis.filter((api) => api === 'computer_use.click.element' || api === 'computer_use.click.coordinate').length,
    clickActionCount
)
assert.equal(clickActionCount, 46)

// Save As is one checkpoint, but three independently guarded UI calls.
const saveAsActionIds = ['S0.save-as-shortcut', 'S0.save-as-name', 'S0.save-as-confirm']
const saveAsIndices = checkpoints.flatMap((checkpoint, index) => checkpoint === 'S0' ? [index] : [])
assert.deepEqual(saveAsIndices.map(index => actionIds[index]), saveAsActionIds)
assert.deepEqual(saveAsIndices.map(index => apis[index]), [
    'computer_use.press_key', 'computer_use.set_value', 'computer_use.click.element'
])
const fixtureDocument = fs.readFileSync(
    path.join(__dirname, '..', 'docs', 'track-api-fixture.md'), 'utf8'
)
const mutationProcedure = fixtureDocument.split('## Case M1: mutation sequence')[1].split('\n## ')[0]
for (const actionId of saveAsActionIds) {
    assert.ok(mutationProcedure.includes(actionId), `M1 procedure must document guarded action ${actionId}`)
}
for (const [checkpoint, expectedIds, expectedApis] of [
    ['R1', ['R1.studio-menu', 'R1.midi-remote-manager', 'R1.scripts-tab', 'R1.reload-scripts'],
        ['computer_use.click.element', 'computer_use.click.element', 'computer_use.click.coordinate', 'computer_use.click.coordinate']],
    ['R2', ['R2.cubase-menu', 'R2.quit-item', 'R2.launch-baseline'],
        ['computer_use.click.element', 'computer_use.click.element', 'exec_command.open']]
]) {
    const indices = checkpoints.flatMap((value, index) => value === checkpoint ? [index] : [])
    assert.deepEqual(indices.map(index => actionIds[index]), expectedIds)
    assert.deepEqual(indices.map(index => apis[index]), expectedApis)
    const procedureRow = fixtureDocument.split('\n').find(line => line.startsWith(`| ${checkpoint} |`))
    assert.ok(procedureRow, `missing ${checkpoint} procedure`)
    for (const actionId of expectedIds) {
        assert.ok(procedureRow.includes(actionId), `${checkpoint} procedure must document guarded action ${actionId}`)
    }
}

assert.equal(activationDialogText, 'プロジェクトをアクティブにしますか？')
assert.equal(affirmativeIdentity, 'ボタン 有効化, ID: action-button-1')
assert.equal(negativeIdentity, 'ボタン いいえ, ID: action-button-2')
assert.notEqual(affirmativeIdentity, negativeIdentity)
const checkerAcceptsActivationTarget = (identity) => identity === affirmativeIdentity
assert.equal(checkerAcceptsActivationTarget(affirmativeIdentity), true)
assert.equal(checkerAcceptsActivationTarget(negativeIdentity), false)
assert.match(
    checker,
    /\.target_binding\.element_identity == \$activation_identity/
)
assert.match(
    checker,
    /validate_activation_dialog_state "\$EVIDENCE_DIRECTORY\/\$open_post"/
)
assert.match(
    checker,
    /validate_activation_dialog_state "\$EVIDENCE_DIRECTORY\/\$activation_pre"/
)
assert.match(
    checker,
    /validate_no_activation_dialog_state "\$EVIDENCE_DIRECTORY\/\$activation_post"/
)
assert.match(
    checker,
    /activation_transition_count" -eq "\$EXPECTED_ACTIVATION_ACTION_COUNT"/
)

const activationValidationStart = checker.indexOf('validate_activation_dialog_state() {')
const activationValidationEnd = checker.indexOf('\nactivation_transition_count=0')
assert.notEqual(activationValidationStart, -1)
assert.ok(activationValidationEnd > activationValidationStart)
const activationValidationFunctions = checker.slice(
    activationValidationStart,
    activationValidationEnd
)

const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-activation-contract-'))
const goodDialogState = path.join(temporaryDirectory, 'good-dialog.json')
const negativeOnlyState = path.join(temporaryDirectory, 'negative-only.json')
const extraButtonState = path.join(temporaryDirectory, 'extra-button.json')
const extraDialogState = path.join(temporaryDirectory, 'extra-dialog.json')
const unrelatedDialogState = path.join(temporaryDirectory, 'unrelated-dialog.json')
const closedDialogState = path.join(temporaryDirectory, 'closed-dialog.json')

fs.writeFileSync(goodDialogState, JSON.stringify({
    text: `0 ダイアログ\n\t2 text ${activationDialogText}\n\t3 ${negativeIdentity}\n\t4 ${affirmativeIdentity}`
}))
fs.writeFileSync(negativeOnlyState, JSON.stringify({
    text: `0 ダイアログ\n\t2 text ${activationDialogText}\n\t3 ${negativeIdentity}`
}))
fs.writeFileSync(extraButtonState, JSON.stringify({
    text: `0 ダイアログ\n\t2 text ${activationDialogText}\n\t3 ${negativeIdentity}\n\t4 ${affirmativeIdentity}\n\t5 ボタン Cancel, ID: action-button-3`
}))
fs.writeFileSync(extraDialogState, JSON.stringify({
    text: `0 ダイアログ Description: 通知\n\t2 text ${activationDialogText}\n\t3 ${negativeIdentity}\n\t4 ${affirmativeIdentity}\n5 ダイアログ Description: Missing Content`
}))
fs.writeFileSync(unrelatedDialogState, JSON.stringify({
    text: '0 ダイアログ Description: Missing Content\n\t1 ボタン OK, ID: action-button-1'
}))
fs.writeFileSync(closedDialogState, JSON.stringify({text: 'Window: CMCP_TrackFixture_Empty'}))

function runActivationStateValidator(functionName, statePath) {
    const script = `
set -euo pipefail
EXPECTED_ACTIVATION_DIALOG_TEXT='${activationDialogText}'
EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY='${affirmativeIdentity}'
EXPECTED_ACTIVATION_NEGATIVE_IDENTITY='${negativeIdentity}'
EVIDENCE_DIRECTORY='${temporaryDirectory}'
die() { return 1; }
${activationValidationFunctions}
${functionName} "$1"
`
    return childProcess.spawnSync('bash', ['-c', script, 'bash', statePath], {
        encoding: 'utf8'
    }).status
}

try {
    assert.equal(runActivationStateValidator('validate_activation_dialog_state', goodDialogState), 0)
    assert.notEqual(runActivationStateValidator('validate_activation_dialog_state', negativeOnlyState), 0)
    assert.notEqual(runActivationStateValidator('validate_activation_dialog_state', extraButtonState), 0)
    assert.notEqual(runActivationStateValidator('validate_activation_dialog_state', extraDialogState), 0)
    assert.equal(runActivationStateValidator('validate_no_activation_dialog_state', closedDialogState), 0)
    assert.notEqual(runActivationStateValidator('validate_no_activation_dialog_state', goodDialogState), 0)
    assert.notEqual(runActivationStateValidator('validate_no_activation_dialog_state', unrelatedDialogState), 0)
} finally {
    fs.rmSync(temporaryDirectory, {recursive: true, force: true})
}

assert.match(
    checker,
    /^EXPECTED_TRACE_RECORD_COUNT=\$\(\(EXPECTED_ACTION_COUNT \+ 2\)\)$/m
)
assert.match(
    checker,
    /^EXPECTED_CAPTURE_COUNT=\$\(\(EXPECTED_ACTION_COUNT \* 2\)\)$/m
)
assert.match(
    checker,
    /^EXPECTED_DETACHED_INDEX_COUNT=\$\(\(EXPECTED_NON_CAPTURE_INDEX_ENTRY_COUNT \+ EXPECTED_ACTION_COUNT \* 4\)\)$/m
)
assert.match(
    checker,
    /select\(\.action_id == "INIT\.launch-bootstrap" or \.action_id == "R2\.launch-baseline"\)/
)
assert.match(
    checker,
    /noninteractive_launch_count" -eq 2/
)
assert.equal(actionCount + 2, 65)
assert.equal(actionCount * 2, 126)
assert.equal(nonCaptureIndexEntryCount, 14 + 8 * 3 + 14 * 4)
assert.equal(nonCaptureIndexEntryCount + actionCount * 4, 346)
const calibrationIndexLoop = checker.match(/for relative_path in calibration\/[\s\S]*?; do/)[0]
assert.equal((calibrationIndexLoop.match(/\$CALIBRATION_PREFIX/g) || []).length, 24)
assert.match(checker, /held-state-rejection\.jsonl/)
assert.match(checker, /deterministic_sampling_contract/)
assert.match(checker, /\.calibration_report_version == 4/)
assert.match(checker, /\.sampling_contract\.source_sha256 == \$sampling_source_sha/)
assert.match(checker, /\.sampling_contract\.runtime_physical_race_reproduced == false/)
assert.match(checker, /CARGO_TARGET_DIR="\$REPRO_BUILD_DIRECTORY" bash "\$TRUSTED_CALIBRATION_CHECKER"/)
assert.doesNotMatch(checker, /sample-race-rejection\.jsonl/)
assert.doesNotMatch(checker, /-sample-rejection\.(?:jsonl|stderr)/)

// Execute the actual final-report filter, including the new evidence-domain
// boundary. A physical-only report or a stale/self-described test cannot pass.
const samplingReportFilter = checker.match(
    /--arg sampling_checker_sha [^\n]* '\n([\s\S]*?)\n  ' "\$CALIBRATION_TMP"/
)
assert.ok(samplingReportFilter, 'missing final sampling report predicate')
const samplingHash = 'a'.repeat(64)
const samplingNames = ['automation', 'move', 'positive-click', 'positive-key',
    'positive-scroll', 'positive-drag', 'wrong-target', 'held-state-rejection']
const samplingReportFixture = {
    calibration_report_version: 4, status: 'valid', file_prefix: 'cmcp-calibration',
    guard_contract: {version: 5, source: 'hid_system_state', coverage: 'action_windows',
        privacy: 'counts_and_held_state_boolean', policy: 'consequential_input_only', binary_sha256: samplingHash},
    checker_sha256: samplingHash, all_pre_post_artifact_digests_recomputed: true,
    controls: {
        automation_negative: ['exec_command.open', 'computer_use.press_key', 'computer_use.set_value',
            'computer_use.click.element', 'computer_use.click.coordinate.single', 'computer_use.click.coordinate.double'],
        move_only_acceptance: ['semantic_target_binding', 'coordinate_target_binding'],
        consequential_positive: ['physical_click', 'physical_key', 'physical_scroll', 'physical_drag'],
        target_binding_rejection: 'wrong_valid_coordinate_rejected_after_clean_result',
        held_state_rejection: {mode: 'held_state', error_code: 'KEY_HELD', physical_input_kind: 'keyboard_key_held'},
        sampling_contract: {mode: 'deterministic_os_read_substitution', error_code: 'INPUT_DURING_SAMPLE', runtime_physical_race_reproduced: false}
    },
    sampling_contract: {
        sampling_contract_report_version: 1, status: 'passed', mode: 'deterministic_os_read_substitution',
        test: 'tests::deterministic_sampling_contract', cases: 12, runtime_physical_race_reproduced: false,
        source_sha256: samplingHash, cargo_lock_sha256: samplingHash, checker_sha256: samplingHash
    },
    fresh_guard_identity_count: 8,
    guard_sessions: Object.fromEntries(samplingNames.map(name => [name, {}])),
    evidence_sha256: Object.fromEntries(samplingNames.map(name => [name, {
        guard_jsonl: samplingHash, guard_stderr: samplingHash, operator_trace_jsonl: samplingHash
    }]))
}
function acceptsSamplingReport(report) {
    const args = ['-s', '-e']
    for (const key of ['guard_sha', 'checker_sha', 'sampling_source_sha', 'sampling_lock_sha', 'sampling_checker_sha']) {
        args.push('--arg', key, samplingHash)
    }
    args.push(samplingReportFilter[1])
    return childProcess.spawnSync('jq', args, {input: JSON.stringify(report), encoding: 'utf8'}).status === 0
}
assert.equal(acceptsSamplingReport(samplingReportFixture), true)
for (const mutate of [
    report => { report.calibration_report_version = 3 },
    report => { delete report.sampling_contract },
    report => { report.sampling_contract.status = 'not_run' },
    report => { report.sampling_contract.cases = 0 },
    report => { report.sampling_contract.runtime_physical_race_reproduced = true },
    report => { report.sampling_contract.source_sha256 = 'b'.repeat(64) },
    report => { report.sampling_contract.cargo_lock_sha256 = 'b'.repeat(64) },
    report => { report.sampling_contract.checker_sha256 = 'b'.repeat(64) },
    report => { report.fresh_guard_identity_count = 9 }
]) {
    const changed = structuredClone(samplingReportFixture)
    mutate(changed)
    assert.equal(acceptsSamplingReport(changed), false)
}

assert.match(checker, /--bin cubase_track_probe_collector/)
assert.match(checker, /--bin cubase_input_guard/)
assert.match(checker, /--bin cubase_track_probe_audit/)
assert.match(checker, /REBUILT_COLLECTOR=.*cubase_track_probe_collector/)
assert.match(checker, /REBUILT_GUARD=.*cubase_input_guard/)
assert.match(checker, /REBUILT_AUDITOR=.*cubase_track_probe_audit/)
assert.match(checker, /cmp -s "\$COLLECTOR" "\$REBUILT_COLLECTOR"/)
assert.match(checker, /cmp -s "\$GUARD_BINARY" "\$REBUILT_GUARD"/)
assert.match(checker, /cmp -s "\$AUDITOR" "\$REBUILT_AUDITOR"/)
assert.match(checker, /auditor_rebuilt_from_inventory_commit_and_bytes_matched:true/)

const formalGuardMatch = checker.match(
    /jq -s -e --argjson action_count "\$EXPECTED_ACTION_COUNT" '\n([\s\S]*?)\n' "\$GUARD"/
)
assert.ok(formalGuardMatch, 'missing executable formal guard v5 filter')
const formalGuardFilter = formalGuardMatch[1]
const guardSessionFields = {
    version: 5,
    coverage: 'action_windows',
    policy: 'consequential_input_only',
    guard_session_id: 'a'.repeat(64),
    guard_process_id: 42,
    guard_started_at_unix_ms: 1000
}
const zeroDeltas = Object.fromEntries([
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
].map((name) => [name, 0]))
const validFormalGuard = [
    {
        ...guardSessionFields,
        record_sequence: 1,
        recorded_at_unix_ms: 1010,
        type: 'ready',
        source: 'hid_system_state',
        privacy: 'counts_and_held_state_boolean'
    },
    {
        ...guardSessionFields,
        record_sequence: 2,
        recorded_at_unix_ms: 1030,
        type: 'armed',
        source: 'hid_system_state',
        action_id: 'A',
        sample_started_at_unix_ms: 1020,
        sample_completed_at_unix_ms: 1025
    },
    {
        ...guardSessionFields,
        record_sequence: 3,
        recorded_at_unix_ms: 1060,
        type: 'result',
        source: 'hid_system_state',
        action_id: 'A',
        sample_started_at_unix_ms: 1050,
        sample_completed_at_unix_ms: 1055,
        interference_detected: false,
        deltas: zeroDeltas
    },
    {
        ...guardSessionFields,
        record_sequence: 4,
        recorded_at_unix_ms: 1070,
        type: 'finished',
        source: 'hid_system_state',
        interference_detected: false
    }
]

function runFormalGuard(records, expectedActionCount = 1) {
    return childProcess.spawnSync(
        'jq',
        ['-s', '-e', '--argjson', 'action_count', String(expectedActionCount), formalGuardFilter],
        {
            input: `${records.map((record) => JSON.stringify(record)).join('\n')}\n`,
            encoding: 'utf8'
        }
    ).status
}

assert.equal(runFormalGuard(validFormalGuard), 0)
const guardWithoutSampleBoundary = clone(validFormalGuard)
delete guardWithoutSampleBoundary[1].sample_started_at_unix_ms
assert.notEqual(runFormalGuard(guardWithoutSampleBoundary), 0)
const guardWithWrongArmSource = clone(validFormalGuard)
guardWithWrongArmSource[1].source = 'untrusted_source'
assert.notEqual(runFormalGuard(guardWithWrongArmSource), 0)
const nonmonotonicGuardRecords = clone(validFormalGuard)
nonmonotonicGuardRecords[2].recorded_at_unix_ms = 1020
assert.notEqual(runFormalGuard(nonmonotonicGuardRecords), 0)

const twoActionFormalGuard = [
    validFormalGuard[0],
    validFormalGuard[1],
    validFormalGuard[2],
    {
        ...guardSessionFields,
        record_sequence: 4,
        recorded_at_unix_ms: 1080,
        type: 'armed',
        source: 'hid_system_state',
        action_id: 'B',
        sample_started_at_unix_ms: 1070,
        sample_completed_at_unix_ms: 1075
    },
    {
        ...guardSessionFields,
        record_sequence: 5,
        recorded_at_unix_ms: 1110,
        type: 'result',
        source: 'hid_system_state',
        action_id: 'B',
        sample_started_at_unix_ms: 1100,
        sample_completed_at_unix_ms: 1105,
        interference_detected: false,
        deltas: zeroDeltas
    },
    {
        ...guardSessionFields,
        record_sequence: 6,
        recorded_at_unix_ms: 1120,
        type: 'finished',
        source: 'hid_system_state',
        interference_detected: false
    }
]
assert.equal(runFormalGuard(twoActionFormalGuard, 2), 0)
const overlappingGuardSamples = clone(twoActionFormalGuard)
overlappingGuardSamples[3].sample_started_at_unix_ms = 1040
overlappingGuardSamples[3].sample_completed_at_unix_ms = 1045
assert.notEqual(runFormalGuard(overlappingGuardSamples, 2), 0)

const timeBindingMatch = checker.match(
    /ACTION_TIME_BINDING_FILTER='([\s\S]*?)'\n\njq -s -e/
)
assert.ok(timeBindingMatch, 'missing executable action/guard/checkpoint time-binding filter')
const timeBindingFilter = timeBindingMatch[1]

function clone(value) {
    return JSON.parse(JSON.stringify(value))
}

const validTimeBinding = {
    raw: [
        {record_type: 'collector_started', timestamp_unix_ms: 1000},
        {
            record_type: 'collector_checkpoint',
            phase: 'begin',
            checkpoint_id: 'C',
            timestamp_unix_ms: 2000
        },
        {
            record_type: 'collector_action',
            phase: 'marked',
            checkpoint_id: 'C',
            timestamp_unix_ms: 2100
        },
        {
            record_type: 'collector_checkpoint',
            phase: 'end',
            checkpoint_id: 'C',
            timestamp_unix_ms: 5000
        },
        {record_type: 'collector_summary', timestamp_unix_ms: 6000}
    ],
    guard: [
        {type: 'ready', recorded_at_unix_ms: 1500},
        {
            type: 'armed',
            action_id: 'A',
            sample_started_at_unix_ms: 2110,
            sample_completed_at_unix_ms: 2120,
            recorded_at_unix_ms: 2130
        },
        {
            type: 'result',
            action_id: 'A',
            sample_started_at_unix_ms: 2510,
            sample_completed_at_unix_ms: 2520,
            recorded_at_unix_ms: 2530
        },
        {type: 'finished', recorded_at_unix_ms: 6100}
    ],
    trace: [
        {
            record_type: 'session',
            started_at: '1970-01-01T09:00:01.200+09:00'
        },
        {
            record_type: 'action',
            action_id: 'A',
            checkpoint_id: 'C',
            pre_state: {captured_at: '1970-01-01T09:00:02.200+09:00'},
            call_started_at: '1970-01-01T09:00:02.300+09:00',
            call_ended_at: '1970-01-01T09:00:02.400+09:00',
            post_state: {captured_at: '1970-01-01T09:00:02.500+09:00'}
        },
        {
            record_type: 'session_end',
            ended_at: '1970-01-01T09:00:07.000+09:00'
        }
    ]
}

function runTimeBinding(input, expectedActionCount = 1, expectedCheckpointCount = 1) {
    return childProcess.spawnSync(
        'jq',
        [
            '-s',
            '-e',
            '--argjson',
            'action_count',
            String(expectedActionCount),
            '--argjson',
            'checkpoint_count',
            String(expectedCheckpointCount),
            '--argjson',
            'guard',
            JSON.stringify(input.guard),
            '--argjson',
            'raw',
            JSON.stringify(input.raw),
            timeBindingFilter
        ],
        {
            input: `${input.trace.map((record) => JSON.stringify(record)).join('\n')}\n`,
            encoding: 'utf8'
        }
    ).status
}

assert.equal(runTimeBinding(validTimeBinding), 0)

const predatedGuardArm = clone(validTimeBinding)
predatedGuardArm.guard[1].recorded_at_unix_ms = 2300
assert.notEqual(runTimeBinding(predatedGuardArm), 0)

const guardClosedBeforePostCapture = clone(validTimeBinding)
guardClosedBeforePostCapture.guard[2].sample_started_at_unix_ms = 2400
assert.notEqual(runTimeBinding(guardClosedBeforePostCapture), 0)

const uiBeforeActionMarker = clone(validTimeBinding)
uiBeforeActionMarker.raw[2].timestamp_unix_ms = 2250
assert.notEqual(runTimeBinding(uiBeforeActionMarker), 0)

const uiOutsideCheckpoint = clone(validTimeBinding)
uiOutsideCheckpoint.raw[3].timestamp_unix_ms = 2525
assert.notEqual(runTimeBinding(uiOutsideCheckpoint), 0)

const staleCheckpointCombinedWithTrace = clone(validTimeBinding)
for (const record of staleCheckpointCombinedWithTrace.raw.slice(1, 4)) {
    record.timestamp_unix_ms += 10000
}
assert.notEqual(runTimeBinding(staleCheckpointCombinedWithTrace), 0)

const wrongCheckpoint = clone(validTimeBinding)
wrongCheckpoint.trace[1].checkpoint_id = 'OTHER'
assert.notEqual(runTimeBinding(wrongCheckpoint), 0)

const sharedMarkerTimeBinding = clone(validTimeBinding)
sharedMarkerTimeBinding.guard.splice(
    3,
    0,
    {
        type: 'armed',
        action_id: 'B',
        sample_started_at_unix_ms: 2600,
        sample_completed_at_unix_ms: 2620,
        recorded_at_unix_ms: 2630
    },
    {
        type: 'result',
        action_id: 'B',
        sample_started_at_unix_ms: 3010,
        sample_completed_at_unix_ms: 3020,
        recorded_at_unix_ms: 3030
    }
)
sharedMarkerTimeBinding.trace.splice(2, 0, {
    record_type: 'action',
    action_id: 'B',
    checkpoint_id: 'C',
    pre_state: {captured_at: '1970-01-01T09:00:02.700+09:00'},
    call_started_at: '1970-01-01T09:00:02.800+09:00',
    call_ended_at: '1970-01-01T09:00:02.900+09:00',
    post_state: {captured_at: '1970-01-01T09:00:03.000+09:00'}
})
assert.equal(runTimeBinding(sharedMarkerTimeBinding, 2, 1), 0)
const secondActionEndsAfterCheckpoint = clone(sharedMarkerTimeBinding)
secondActionEndsAfterCheckpoint.guard[4].recorded_at_unix_ms = 5100
assert.notEqual(runTimeBinding(secondActionEndsAfterCheckpoint, 2, 1), 0)

console.log('track probe evidence checker contract tests passed')
