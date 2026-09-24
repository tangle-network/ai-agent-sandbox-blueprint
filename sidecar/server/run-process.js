'use strict'

const { spawn } = require('child_process')

const DEFAULT_KILL_GRACE_MS = 2000

// Runs a harness (or shell) command to completion and captures its output.
//
// The child is spawned as its own process-group leader (`detached: true`).
// Harnesses such as opencode spawn tool subprocesses (bash, node, curl) and
// signal their process group on a tool timeout. When the harness shares the
// sidecar's group that signal crosses the session boundary and fails with
// `kill EPERM`, which ended agent runs mid-turn. Owning its group lets the
// harness signal its children, and lets us tear the whole tree down at once
// on the sidecar's own timeout.
function runProcess(command, args, options = {}) {
  const timeout = Number(options.timeout || 0)
  const killGraceMs = Number(options.killGraceMs ?? DEFAULT_KILL_GRACE_MS)

  return new Promise((resolve) => {
    let stdout = ''
    let stderr = ''
    let timedOut = false
    let settled = false
    let timer = null
    let escalation = null

    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      shell: false,
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
      uid: options.uid,
      gid: options.gid,
    })

    // Signal the whole group (negative pid) so the harness and every tool
    // subprocess it spawned die together. A plain child.kill leaves
    // grandchildren orphaned. Never signal after the child has closed: the
    // pid, and so the pgid, can already belong to an unrelated process.
    const killGroup = (signal) => {
      if (settled || !child.pid) return
      try {
        process.kill(-child.pid, signal)
      } catch {
        try { child.kill(signal) } catch { /* already gone */ }
      }
    }

    const clearTimers = () => {
      if (timer) clearTimeout(timer)
      if (escalation) clearTimeout(escalation)
      timer = null
      escalation = null
    }

    if (timeout > 0) {
      timer = setTimeout(() => {
        timedOut = true
        killGroup('SIGTERM')
        escalation = setTimeout(() => killGroup('SIGKILL'), killGraceMs)
        escalation.unref()
      }, timeout)
    }

    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (err) => {
      settled = true
      clearTimers()
      resolve({ exitCode: 127, stdout, stderr: stderr + err.message })
    })
    child.on('close', (code, signal) => {
      settled = true
      clearTimers()
      resolve({
        exitCode: timedOut ? 124 : (code ?? 1),
        stdout,
        stderr: timedOut ? `${stderr}\nprocess timed out`.trim() : stderr,
        signal,
      })
    })
  })
}

module.exports = { runProcess, DEFAULT_KILL_GRACE_MS }
