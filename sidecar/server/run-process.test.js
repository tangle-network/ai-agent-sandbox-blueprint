'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { runProcess } = require('./run-process')

const sh = (script, options) => runProcess('/bin/sh', ['-c', script], options)

function alive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

async function settle(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

test('captures output and exit code', async () => {
  const result = await sh('echo out; echo err >&2; exit 3')
  assert.equal(result.exitCode, 3)
  assert.equal(result.stdout, 'out\n')
  assert.equal(result.stderr, 'err\n')
})

test('child is the leader of its own process group', async () => {
  const result = await sh('ps -o pgid= -p $$; echo $$')
  assert.equal(result.exitCode, 0)
  const [pgid, pid] = result.stdout.trim().split('\n').map((line) => line.trim())
  assert.equal(pgid, pid)
  assert.notEqual(Number(pgid), process.pid)
})

test('timeout kills the whole process group, including grandchildren', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'run-process-'))
  const pidFile = path.join(dir, 'grandchild.pid')
  const result = await sh(`sleep 30 & echo $! > ${pidFile}; wait`, { timeout: 200 })
  assert.equal(result.exitCode, 124)
  assert.match(result.stderr, /process timed out/)
  const grandchild = Number(fs.readFileSync(pidFile, 'utf8').trim())
  await settle(50)
  assert.equal(alive(grandchild), false, 'grandchild sleep survived the group kill')
  fs.rmSync(dir, { recursive: true, force: true })
})

test('escalates to SIGKILL when the group ignores SIGTERM', async () => {
  const started = Date.now()
  const result = await sh("trap '' TERM; sleep 30", { timeout: 100, killGraceMs: 200 })
  assert.equal(result.exitCode, 124)
  assert.equal(result.signal, 'SIGKILL')
  assert.ok(Date.now() - started < 5000)
})

test('a prompt exit after SIGTERM leaves no pending escalation', async () => {
  const result = await sh('sleep 30', { timeout: 100, killGraceMs: 60_000 })
  assert.equal(result.exitCode, 124)
  assert.equal(result.signal, 'SIGTERM')
  // With the escalation timer still armed the process would stay alive for
  // killGraceMs; node exits promptly only when nothing is pending.
  const handles = process.getActiveResourcesInfo().filter((name) => name === 'Timeout')
  assert.equal(handles.length, 0)
})

test('spawn failure resolves with exit code 127', async () => {
  const result = await runProcess('/nonexistent/binary', [])
  assert.equal(result.exitCode, 127)
  assert.match(result.stderr, /ENOENT/)
})
