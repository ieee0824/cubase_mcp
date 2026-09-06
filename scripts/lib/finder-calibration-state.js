'use strict'

// Pure predicates for the observed Finder AX text format, not a GUI runner.
// Call with a fresh full capture each time. These functions cannot prove capture
// freshness, screenshot agreement, physical-input isolation, or action success.
// Only the observed tab-indented Japanese Finder list-view format is supported;
// unknown/ambiguous structures fail instead of guessing a target.
function requireCondition(condition, message) {
    if (!condition) throw new Error(`Finder calibration: ${message}`)
}

function validateSummaries(lines, nodes) {
    const seen = new Set()
    const note = 'Note: Pay special attention to the content selected by the user. If the user asks a question or refers to ' +
        'the content they are looking at on-screen, they might be referring to the selected content ' +
        "(but they might be referring to something else that's visible, too)."
    const once = name => {
        requireCondition(!seen.has(name), 'duplicate AX summary')
        seen.add(name)
    }
    const verifyNode = match => {
        requireCondition(match && nodes.some(node => node.index === Number(match[1]) &&
            (node.body === match[2] || node.body.replace(/ \(showing \d+-\d+ of \d+ items\)$/, '') === match[2])),
        'summary does not match a structural node') // Observed focused summaries omit pagination.
    }
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i]
        if (!line.trim()) continue
        if (line.startsWith('The focused UI element is ')) {
            once('focused')
            verifyNode(/^The focused UI element is (\d+) (.+)$/.exec(line))
        } else if (line === 'Selected:') {
            once('selected')
            let count = 0
            while (i + 1 < lines.length && /^\t+\d+ /.test(lines[i + 1])) {
                const match = /^\t+(\d+) (.+)$/.exec(lines[++i])
                verifyNode(match)
                requireCondition(/\(selected(?:, |\))/.test(match[2]), 'summary node is not selected')
                count++
            }
            requireCondition(count > 0, 'empty selected-node summary')
        } else if (line === 'Selected text: ```') {
            once('selected-text')
            const end = lines.indexOf('```', i + 1)
            requireCondition(end > i, 'unterminated selected-text summary')
            i = end // The known fenced section contains text, not AX nodes.
        } else {
            requireCondition(line === note, 'unknown trailing AX content')
            once('note')
        }
    }
}

function parse(fullText) {
    requireCondition(typeof fullText === 'string', 'full AX text required')
    const lines = fullText.split(/\r?\n/)
    const header = /^Window: "([^"]*)", App: Finder\.$/.exec(lines[0])
    requireCondition(header && lines.filter(line => /^Window: /.test(line)).length === 1,
        'one unambiguous Finder window header required')
    const nodes = [], stack = [], indices = new Set()
    let lineIndex = 1
    for (; lineIndex < lines.length; lineIndex++) {
        const line = lines[lineIndex]
        if (!line.trim()) break
        const match = /^(\t*)(\d+) (.+)$/.exec(line)
        if (!match) {
            requireCondition(!/^\s*\d+ /.test(line), 'unsupported AX indentation')
            continue // Observed multiline column-heading values have no index.
        }
        const depth = match[1].length, index = Number(match[2])
        requireCondition(Number.isSafeInteger(index) && !indices.has(index), 'duplicate or invalid element index')
        requireCondition(depth <= stack.length, 'incomplete AX ancestry')
        stack.length = depth
        const node = { index, body: match[3], ancestors: [...stack] }
        nodes.push(node)
        stack.push(node)
        indices.add(index)
    }
    requireCondition(nodes.length > 0, 'structural AX nodes required')
    validateSummaries(lines.slice(lineIndex), nodes)
    return { title: header[1], nodes }
}

function hasId(node, id) {
    return new RegExp(`(?:^|[ ,])ID: ${id}(?=, | \\(showing |$)`).test(node.body)
}

function only(nodes, message) {
    requireCondition(nodes.length === 1, message)
    return nodes[0]
}

function scratch(parsed, windowTitle) {
    requireCondition(typeof windowTitle === 'string' && windowTitle.length > 0 &&
        parsed.title === windowTitle, 'unexpected active window')
    const window = only(parsed.nodes.filter(node => hasId(node, 'FinderWindow')),
        'one FinderWindow required')
    requireCondition(window.ancestors.length === 0 &&
        window.body.startsWith(`標準ウインドウ ${windowTitle}, ID: FinderWindow`), 'unexpected Finder window structure')
    requireCondition(!parsed.nodes.some(node => hasId(node, 'GoToWindow') ||
        /^(?:シート|ダイアログ|sheet|dialog|modal)(?: |$)/i.test(node.body)), 'residual dialog or modal')
    requireCondition(parsed.nodes.every(node => node.ancestors.length > 0 ||
        node === window || node.body === 'menu bar'), 'ambiguous additional window or root')
    return window
}

// Return the structural FinderWindow index, or throw. This does not identify a
// filesystem directory: callers must separately check fixture file URLs.
function requireScratchWindow(fullText, windowTitle) {
    return scratch(parse(fullText), windowTitle).index
}

// Return a fresh structural PathTextField index under the sole GoToWindow.
// Finder renders an empty field as bare PathTextField (no Value: or ID:).
function resolveGoToPathField(fullText, expectedValue) {
    requireCondition(typeof expectedValue === 'string' && !/[\r\n]/.test(expectedValue), 'exact path value required')
    const { nodes } = parse(fullText)
    const dialog = only(nodes.filter(node => hasId(node, 'GoToWindow')), 'one GoToWindow required')
    requireCondition(dialog.ancestors.length === 0 && dialog.body.startsWith('シート '), 'unexpected GoToWindow structure')
    requireCondition(nodes.every(node => node.ancestors.length > 0 || node === dialog ||
        node.body === 'menu bar'), 'ambiguous active dialog')
    requireCondition(!nodes.some(node => node !== dialog &&
        /^(?:シート|ダイアログ|sheet|dialog|modal)(?: |$)/i.test(node.body)), 'additional dialog or modal')
    const field = only(nodes.filter(node => hasId(node, 'PathTextField') ||
        /^テキストフィールド \(settable\) PathTextField$/.test(node.body)), 'one PathTextField required')
    const match = /^テキストフィールド \(settable\) (?:Value: (.*), ID: PathTextField|PathTextField)$/.exec(field.body)
    requireCondition(match && field.ancestors.includes(dialog) && (match[1] ?? '') === expectedValue,
        'unexpected path field structure or value')
    return field.index
}

function fixtureTarget(fullText, { windowTitle, filename, expectedUrl }) {
    requireCondition(typeof filename === 'string' && /^[A-Za-z0-9_.-]+$/.test(filename) &&
        filename !== '.' && filename !== '..', 'simple synthetic filename required')
    requireCondition(typeof expectedUrl === 'string' && expectedUrl.startsWith('file:///') &&
        !/[\s,?#]/.test(expectedUrl), 'exact local fixture URL required')
    const url = new URL(expectedUrl)
    requireCondition(decodeURIComponent(url.pathname.replace(/\/$/, '').split('/').pop()) === filename,
        'fixture URL and filename disagree')
    const parsed = parse(fullText), window = scratch(parsed, windowTitle)
    const list = only(parsed.nodes.filter(node => hasId(node, 'ListView') &&
        node.ancestors.includes(window)), 'one content ListView required')
    requireCondition(/^アウトライン Description: リスト表示, ID: ListView(?: \(showing [^)]+\))?$/.test(list.body),
        'unsupported content ListView structure')
    const matching = parsed.nodes.filter(node => {
        const match = /^テキストフィールド \((?:selected, )?settable\) URL: ([^,]+), Value: ([^,]+), Secondary Actions: Finder項目を開く$/.exec(node.body)
        return match && match[1] === expectedUrl && match[2] === filename
    })
    const target = only(matching, 'one exact fixture file required')
    requireCondition(target.ancestors.includes(list), 'fixture is outside content ListView')
    return { target, list }
}

// Resolve an exact content file/folder for a click, whether or not selected.
// Folder URLs may end in one slash; matching never strips or normalizes that URL.
function resolveFixtureTarget(fullText, fixture) {
    return fixtureTarget(fullText, fixture).target.index
}

// Return the exact file text-field index below a selected content row/cell in
// the observed ListView. A selected sidebar or a selected field alone is not proof.
function resolveSelectedFixture(fullText, fixture) {
    const { target, list } = fixtureTarget(fullText, fixture)
    const contentAncestors = target.ancestors.slice(target.ancestors.indexOf(list) + 1)
    requireCondition(contentAncestors.some(node =>
        /^(?:row|セル) \(selected(?:, [^)]+)?\)(?: |$)/.test(node.body)), 'fixture content ancestor is not selected')
    return target.index
}

module.exports = { resolveGoToPathField, resolveFixtureTarget, resolveSelectedFixture, requireScratchWindow }
