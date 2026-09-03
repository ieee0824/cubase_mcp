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
assert.equal(nonCaptureIndexEntryCount + actionCount * 4, 346)

console.log('track probe evidence checker contract tests passed')
