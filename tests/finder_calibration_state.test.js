'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const { resolveGoToPathField, resolveFixtureTarget, resolveSelectedFixture, requireScratchWindow } =
    require('../scripts/lib/finder-calibration-state')

// Synthetic fixtures model observed structure; no private capture content.
const windowTitle = 'calibration-scratch'
const filename = 'semantic-target.txt'
const expectedUrl = `file:///private/tmp/${windowTitle}/${filename}`
const fixture = { windowTitle, filename, expectedUrl }
const fileNode = `テキストフィールド (settable) URL: ${expectedUrl}, Value: ${filename}, Secondary Actions: Finder項目を開く`
const pathNode = value => value === '' ? 'テキストフィールド (settable) PathTextField' :
    `テキストフィールド (settable) Value: ${value}, ID: PathTextField`
const dialog = (value = '', index = 3) => [
    'Window: "Unknown", App: Finder.',
    '0 シート ID: GoToWindow, Secondary Actions: Raise',
    '\t1 テキスト フォルダへ移動',
    `\t${index} ${pathNode(value)}`,
    '10 menu bar', '\t11 Finder',
    '', `The focused UI element is ${index} ${pathNode(value)}`
].join('\n')
const scratch = (selected = true, index = 8) => [
    `Window: "${windowTitle}", App: Finder.`,
    `0 標準ウインドウ ${windowTitle}, ID: FinderWindow, Secondary Actions: Raise`,
    '\t1 グループを分割',
    '\t\t2 アウトライン サイドバー',
    '\t\t\t3 row (selected) デスクトップ',
    '\t\t4 スクロール領域',
    '\t\t\t5 アウトライン Description: リスト表示, ID: ListView',
    `\t\t\t\t6 row (${selected ? 'selected' : 'selectable'})`,
    '\t\t\t\t\t7 セル', `\t\t\t\t\t\t${index} ${fileNode}`,
    '20 menu bar', '\t21 Finder', '', 'Selected:',
    selected ? '\t6 row (selected)' : '\t3 row (selected) デスクトップ',
    '', `The focused UI element is ${index} ${fileNode}`
].join('\n')

test('empty Go To field omits ID; numbered field wins over focused summary', () => {
    assert.equal(resolveGoToPathField(dialog(), ''), 3)
    assert.throws(() => resolveGoToPathField(dialog(), '/private/tmp/'))
})
test('exact path value and a freshly resolved index are required', () => {
    assert.equal(resolveGoToPathField(dialog('/private/tmp/', 4), '/private/tmp/'), 4)
    assert.equal(resolveGoToPathField(dialog('/private/tmp/', 6), '/private/tmp/'), 6)
    assert.throws(() => resolveGoToPathField(dialog('/private/tmp/'), '/private/tmp'))
    assert.throws(() => resolveGoToPathField(dialog().replace('PathTextField', 'PathTextFieldOther'), ''))
})
test('duplicate field, duplicate index and field outside dialog fail', () => {
    assert.throws(() => resolveGoToPathField(dialog().replace('10 menu bar', `\t4 ${pathNode('')}\n10 menu bar`), ''))
    assert.throws(() => resolveGoToPathField(dialog().replace('10 menu bar', '3 menu bar'), ''))
    assert.throws(() => resolveGoToPathField(dialog().replace('\t3 ', '3 '), ''))
})
test('Japanese surrounding labels and sidebar selection do not replace exact fixture selection', () => {
    assert.equal(requireScratchWindow(scratch(), windowTitle), 0)
    assert.equal(resolveSelectedFixture(scratch(), fixture), 8)
    assert.equal(resolveSelectedFixture(scratch(true, 12), fixture), 12)
    assert.throws(() => resolveSelectedFixture(scratch(false), fixture))
    assert.throws(() => resolveSelectedFixture(scratch().replace(expectedUrl, `${expectedUrl}.wrong`), fixture))
    assert.throws(() => resolveSelectedFixture(scratch().replace(`Value: ${filename}`, 'Value: wrong.txt'), fixture))
})
test('file must be under selected content ancestor, not the sidebar or a selected field alone', () => {
    assert.throws(() => resolveSelectedFixture(scratch().replace('ID: ListView', 'ID: Sidebar'), fixture))
    assert.throws(() => resolveSelectedFixture(scratch(false).replace('(settable) URL:', '(selected, settable) URL:'), fixture))
    assert.throws(() => resolveSelectedFixture(scratch().replace('\t\t\t\t\t\t8 ', '\t\t\t8 '), fixture))
})
test('pre-click resolver accepts unselected content, with the same exact target checks', () => {
    assert.equal(resolveFixtureTarget(scratch(false), fixture), 8)
    assert.equal(resolveFixtureTarget(scratch(false, 12), fixture), 12)
    assert.throws(() => resolveFixtureTarget(scratch(false).replace('ID: ListView', 'ID: ListViewOther'), fixture))
    assert.throws(() => resolveFixtureTarget(scratch(false).replace('アウトライン Description: リスト表示', 'テキスト'), fixture))
    assert.throws(() => resolveFixtureTarget(scratch(false).replace(expectedUrl, `${expectedUrl}.wrong`), fixture))
    assert.throws(() => resolveFixtureTarget(scratch(false).replace('\t\t\t\t\t\t8 ', '\t\t\t8 '), fixture))
})
test('duplicate exact files are ambiguous even when only one is selected', () => {
    const duplicate = scratch().replace('20 menu bar', `\t\t\t\t9 row (selectable)\n\t\t\t\t\t10 ${fileNode}\n20 menu bar`)
    assert.throws(() => resolveSelectedFixture(duplicate, fixture))
})
test('residual GoToWindow, modal, another window and unexpected header fail closed', () => {
    for (const root of ['22 シート ID: GoToWindow', '22 ダイアログ 確認',
        '22 標準ウインドウ other, ID: FinderWindow', '22 modal']) {
        assert.throws(() => requireScratchWindow(scratch().replace('20 menu bar', `${root}\n20 menu bar`), windowTitle))
    }
    assert.throws(() => requireScratchWindow(scratch().replace(`Window: "${windowTitle}"`, 'Window: "other"'), windowTitle))
    assert.throws(() => requireScratchWindow(`${scratch()}\nWindow: "other", App: Finder.`, windowTitle))
    assert.throws(() => requireScratchWindow(dialog(), windowTitle))
})
test('a matching summary cannot substitute for a missing structural field', () => {
    assert.throws(() => resolveGoToPathField(dialog().replace(`\t3 ${pathNode('')}\n`, ''), ''))
    assert.throws(() => resolveFixtureTarget(scratch().replace(`\t\t\t\t\t\t8 ${fileNode}\n`, ''), fixture))
})
test('blank lines cannot hide new roots, unknown indices or inconsistent summaries', () => {
    for (const suffix of ['\n\n22 modal', '\n\n22 シート ID: GoToWindow',
        '\n\nSelected:\n\t999 row (selected)', '\n\nThe focused UI element is 999 modal']) {
        assert.throws(() => requireScratchWindow(`${scratch()}${suffix}`, windowTitle))
    }
    assert.throws(() => requireScratchWindow(scratch().replace('Selected:\n\t6 row (selected)',
        'Selected:\n\t6 row (selectable)'), windowTitle))
    assert.throws(() => requireScratchWindow(scratch().replace(`The focused UI element is 8 ${fileNode}`,
        'The focused UI element is 8 modal'), windowTitle))
})
test('known selected-text fences are accepted but extra content after them is rejected', () => {
    const text = `${dialog('/private/tmp/')}\n\nSelected text: \`\`\`\n/private/tmp/\n\`\`\``
    assert.equal(resolveGoToPathField(text, '/private/tmp/'), 3)
    assert.throws(() => resolveGoToPathField(`${text}\n22 modal`, '/private/tmp/'))
    assert.throws(() => resolveGoToPathField(text.slice(0, -3), '/private/tmp/'))
})
test('folder fixture URLs preserve the exact trailing slash', () => {
    const folder = { windowTitle, filename: 'double-target',
        expectedUrl: `file:///private/tmp/${windowTitle}/double-target/` }
    const text = scratch(false).replaceAll(expectedUrl, folder.expectedUrl).replaceAll(filename, folder.filename)
    assert.equal(resolveFixtureTarget(text, folder), 8)
    assert.throws(() => resolveFixtureTarget(text, { ...folder, expectedUrl: folder.expectedUrl.slice(0, -1) }))
    assert.throws(() => resolveFixtureTarget(text, { ...folder, expectedUrl: `${folder.expectedUrl}/` }))
})
test('focused summary may omit the observed pagination suffix only', () => {
    const text = scratch().replace('ID: ListView\n', 'ID: ListView (showing 0-19 of 100 items)\n')
        .replace(`The focused UI element is 8 ${fileNode}`, 'The focused UI element is 5 アウトライン Description: リスト表示, ID: ListView')
    assert.equal(resolveFixtureTarget(text, fixture), 8)
    assert.throws(() => resolveFixtureTarget(text.replace('showing 0-19 of 100 items', 'unknown metadata'), fixture))
})
