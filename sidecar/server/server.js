'use strict'

const http = require('http')
const { spawn } = require('child_process')
const { randomUUID } = require('crypto')
const fs = require('fs')
const path = require('path')

const port = Number(process.env.SIDECAR_PORT || process.env.PORT || 8080)
const authToken = process.env.SIDECAR_AUTH_TOKEN || ''
const workspaceRoot = process.env.AGENT_WORKSPACE_ROOT || '/home/agent/workspace'
const childUid = Number(process.env.AGENT_SUBPROCESS_UID || 1000)
const childGid = Number(process.env.AGENT_SUBPROCESS_GID || 1000)
const terminalShell = process.env.SIDECAR_TERMINAL_SHELL || '/bin/bash'
const terminalSessions = new Map()

const agents = [
  {
    identifier: 'default',
    displayName: 'Default',
    description: 'Uses the first configured local coding harness.',
  },
  {
    identifier: 'codex',
    displayName: 'Codex',
    description: 'Runs the Codex CLI.',
  },
  {
    identifier: 'claude',
    displayName: 'Claude Code',
    description: 'Runs Claude Code.',
  },
  {
    identifier: 'gemini',
    displayName: 'Gemini',
    description: 'Runs Gemini CLI.',
  },
  {
    identifier: 'opencode',
    displayName: 'OpenCode',
    description: 'Runs OpenCode.',
  },
  {
    identifier: 'kimi',
    displayName: 'Kimi',
    description: 'Runs Kimi CLI.',
  },
]

fs.mkdirSync(workspaceRoot, { recursive: true })

function sendJson(res, status, value) {
  const body = JSON.stringify(value)
  res.writeHead(status, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(body),
    'cache-control': 'no-store',
  })
  res.end(body)
}

function sendError(res, status, code, message) {
  sendJson(res, status, {
    success: false,
    error: { code, message },
  })
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let body = ''
    req.setEncoding('utf8')
    req.on('data', (chunk) => {
      body += chunk
      if (body.length > 1024 * 1024) {
        reject(new Error('request body too large'))
        req.destroy()
      }
    })
    req.on('end', () => {
      if (!body.trim()) {
        resolve({})
        return
      }
      try {
        resolve(JSON.parse(body))
      } catch (err) {
        reject(err)
      }
    })
    req.on('error', reject)
  })
}

function requireAuth(req, res) {
  if (!authToken) return true
  const expected = `Bearer ${authToken}`
  if (req.headers.authorization === expected) return true
  sendError(res, 401, 'UNAUTHORIZED', 'Invalid sidecar auth token')
  return false
}

function parseEnv(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const out = {}
  for (const [key, value] of Object.entries(raw)) {
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) out[key] = String(value)
  }
  return out
}

function runProcess(command, args, options = {}) {
  const cwd = options.cwd && path.isAbsolute(options.cwd) ? options.cwd : workspaceRoot
  const timeout = Number(options.timeout || 0)
  const childEnv = {
    ...process.env,
    HOME: process.env.AGENT_HOME || '/home/agent',
    ...parseEnv(options.env),
  }

  return new Promise((resolve) => {
    let stdout = ''
    let stderr = ''
    let timedOut = false
    const child = spawn(command, args, {
      cwd,
      env: childEnv,
      shell: false,
      stdio: ['ignore', 'pipe', 'pipe'],
      uid: process.getuid && process.getuid() === 0 ? childUid : undefined,
      gid: process.getuid && process.getuid() === 0 ? childGid : undefined,
    })

    const timer = timeout > 0
      ? setTimeout(() => {
        timedOut = true
        child.kill('SIGTERM')
        setTimeout(() => child.kill('SIGKILL'), 2000).unref()
      }, timeout)
      : null

    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (err) => {
      if (timer) clearTimeout(timer)
      resolve({ exitCode: 127, stdout, stderr: stderr + err.message })
    })
    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer)
      resolve({
        exitCode: timedOut ? 124 : (code ?? 1),
        stdout,
        stderr: timedOut ? `${stderr}\nprocess timed out`.trim() : stderr,
        signal,
      })
    })
  })
}

function shellCommand(command, payload) {
  return runProcess('/bin/sh', ['-lc', command], {
    cwd: typeof payload.cwd === 'string' ? payload.cwd : workspaceRoot,
    timeout: Number(payload.timeout || payload.timeout_ms || 0),
    env: payload.env,
  })
}

function terminalSummary(session) {
  return {
    sessionId: session.id,
    session_id: session.id,
    title: session.title,
    streamPath: `/terminals/${session.id}/stream`,
    stream_path: `/terminals/${session.id}/stream`,
    cwd: session.cwd,
    cols: session.cols,
    rows: session.rows,
    running: session.running,
    createdAt: session.createdAt,
    updatedAt: session.updatedAt,
  }
}

function createTerminalSession(payload = {}) {
  const cwd = typeof payload.cwd === 'string' && path.isAbsolute(payload.cwd)
    ? payload.cwd
    : workspaceRoot
  const id = randomUUID()
  const childEnv = {
    ...process.env,
    HOME: process.env.AGENT_HOME || '/home/agent',
    TERM: process.env.TERM || 'xterm-256color',
    PS1: '\\u@\\h:\\w\\$ ',
    ...parseEnv(payload.env),
  }
  const child = spawn(terminalShell, ['-i'], {
    cwd,
    env: childEnv,
    shell: false,
    stdio: ['pipe', 'pipe', 'pipe'],
    uid: process.getuid && process.getuid() === 0 ? childUid : undefined,
    gid: process.getuid && process.getuid() === 0 ? childGid : undefined,
  })
  const session = {
    id,
    title: typeof payload.title === 'string' && payload.title.trim() ? payload.title.trim() : 'Shell',
    cwd,
    cols: Number(payload.cols || 80),
    rows: Number(payload.rows || 24),
    child,
    running: true,
    subscribers: new Set(),
    backlog: [],
    createdAt: Date.now(),
    updatedAt: Date.now(),
  }

  const publish = (event, data) => {
    session.updatedAt = Date.now()
    const frame = { sessionId: id, session_id: id, ...data }
    session.backlog.push({ event, data: frame })
    if (session.backlog.length > 200) session.backlog.shift()
    for (const subscriber of session.subscribers) {
      writeSse(subscriber, event, frame)
    }
  }

  child.stdout.on('data', (chunk) => {
    publish('output', { data: chunk.toString() })
  })
  child.stderr.on('data', (chunk) => {
    publish('output', { data: chunk.toString() })
  })
  child.on('error', (err) => {
    publish('error', { message: err.message })
  })
  child.on('close', (code, signal) => {
    session.running = false
    publish('exit', { code, signal })
    for (const subscriber of session.subscribers) subscriber.end()
    session.subscribers.clear()
  })

  terminalSessions.set(id, session)
  return session
}

function getTerminalSession(id, res) {
  const session = terminalSessions.get(id)
  if (!session) {
    sendError(res, 404, 'TERMINAL_NOT_FOUND', 'Terminal session not found')
    return null
  }
  return session
}

function closeTerminalSession(session) {
  terminalSessions.delete(session.id)
  session.running = false
  if (!session.child.killed) {
    session.child.kill('SIGTERM')
    setTimeout(() => {
      if (!session.child.killed) session.child.kill('SIGKILL')
    }, 2000).unref()
  }
  for (const subscriber of session.subscribers) subscriber.end()
  session.subscribers.clear()
}

function selectHarness(identifier, backend) {
  const requested = (identifier || backend?.type || '').trim().toLowerCase()
  if (requested && requested !== 'default') return requested
  if (process.env.SIDECAR_DEFAULT_HARNESS) return process.env.SIDECAR_DEFAULT_HARNESS
  if (process.env.OPENAI_API_KEY) return 'codex'
  if (process.env.ANTHROPIC_API_KEY) return 'claude'
  if (process.env.GEMINI_API_KEY || process.env.GOOGLE_API_KEY) return 'gemini'
  return 'opencode'
}

function harnessCommand(harness, payload) {
  const message = String(payload.message || '')
  const model = payload.backend?.model ? String(payload.backend.model) : ''
  const timeout = Number(payload.timeout || 0)

  if (process.env.SIDECAR_AGENT_COMMAND) {
    return {
      command: '/bin/sh',
      args: ['-lc', process.env.SIDECAR_AGENT_COMMAND],
      env: { SIDECAR_AGENT_MESSAGE: message, SIDECAR_AGENT_MODEL: model },
      timeout,
    }
  }

  switch (harness) {
    case 'codex':
      return {
        command: 'codex',
        args: ['exec', '--skip-git-repo-check', '--dangerously-bypass-approvals-and-sandbox', message],
        timeout,
      }
    case 'claude':
      return {
        command: 'claude',
        args: ['-p', message, '--dangerously-skip-permissions'],
        timeout,
      }
    case 'gemini':
      return {
        command: 'gemini',
        args: model
          ? ['--skip-trust', '--yolo', '-m', model, '-p', message]
          : ['--skip-trust', '--yolo', '-p', message],
        timeout,
      }
    case 'kimi':
      return {
        command: 'kimi',
        args: ['-p', message],
        timeout,
      }
    case 'opencode':
      return {
        command: 'opencode',
        args: ['run', message],
        timeout,
      }
    default:
      return null
  }
}

async function runAgent(payload) {
  const identifier = String(payload.identifier || 'default')
  const known = agents.some((agent) => agent.identifier === identifier)
  if (!known) {
    return {
      success: false,
      status: 400,
      error: `No factory registered for agent identifier ${identifier}`,
    }
  }

  const harness = selectHarness(identifier, payload.backend)
  const spec = harnessCommand(harness, payload)
  if (!spec) {
    return {
      success: false,
      status: 400,
      error: `No command registered for harness ${harness}`,
    }
  }

  const result = await runProcess(spec.command, spec.args, {
    cwd: workspaceRoot,
    timeout: spec.timeout,
    env: spec.env,
  })
  const response = result.stdout.trim() || result.stderr.trim()
  return {
    success: result.exitCode === 0,
    status: result.exitCode === 0 ? 200 : 502,
    response,
    stderr: result.stderr,
    exitCode: result.exitCode,
    harness,
  }
}

function writeSse(res, event, data) {
  res.write(`event: ${event}\n`)
  res.write(`data: ${JSON.stringify(data)}\n\n`)
}

async function handle(req, res) {
  const url = new URL(req.url, `http://${req.headers.host || 'localhost'}`)

  if (req.method === 'GET' && url.pathname === '/health') {
    sendJson(res, 200, { status: 'ok' })
    return
  }

  if (!requireAuth(req, res)) return

  if (req.method === 'GET' && url.pathname === '/agents') {
    sendJson(res, 200, { agents, count: agents.length })
    return
  }

  if (req.method === 'POST' && url.pathname === '/terminals/commands') {
    const payload = await readBody(req)
    if (!payload.command || typeof payload.command !== 'string') {
      sendError(res, 400, 'INVALID_COMMAND', 'command is required')
      return
    }
    const result = await shellCommand(payload.command, payload)
    sendJson(res, 200, {
      result: {
        exitCode: result.exitCode,
        stdout: result.stdout,
        stderr: result.stderr,
      },
    })
    return
  }

  if (req.method === 'GET' && url.pathname === '/terminals') {
    const data = Array.from(terminalSessions.values()).map(terminalSummary)
    sendJson(res, 200, { data, terminals: data })
    return
  }

  if (req.method === 'POST' && url.pathname === '/terminals') {
    const payload = await readBody(req)
    const summary = terminalSummary(createTerminalSession(payload))
    sendJson(res, 200, { data: summary, ...summary })
    return
  }

  const terminalMatch = url.pathname.match(/^\/terminals\/([^/]+)(?:\/([^/]+))?$/)
  if (terminalMatch) {
    const [, sessionId, action] = terminalMatch
    const session = getTerminalSession(sessionId, res)
    if (!session) return

    if (req.method === 'GET' && !action) {
      const summary = terminalSummary(session)
      sendJson(res, 200, { data: summary, ...summary })
      return
    }

    if (req.method === 'GET' && action === 'stream') {
      res.writeHead(200, {
        'content-type': 'text/event-stream',
        'cache-control': 'no-store',
        connection: 'keep-alive',
      })
      session.subscribers.add(res)
      for (const frame of session.backlog.slice(-20)) {
        writeSse(res, frame.event, frame.data)
      }
      req.on('close', () => {
        session.subscribers.delete(res)
      })
      return
    }

    if (req.method === 'POST' && (action === 'input' || action === 'execute')) {
      const payload = await readBody(req)
      const data = typeof payload.data === 'string'
        ? payload.data
        : (typeof payload.command === 'string' ? `${payload.command}\n` : '')
      if (!data) {
        sendError(res, 400, 'INVALID_INPUT', 'data is required')
        return
      }
      if (!session.running || session.child.killed) {
        sendError(res, 409, 'TERMINAL_CLOSED', 'Terminal session is not running')
        return
      }
      session.child.stdin.write(data)
      sendJson(res, 200, { success: true })
      return
    }

    if (req.method === 'PATCH' && !action) {
      const payload = await readBody(req)
      session.cols = Number(payload.cols || session.cols)
      session.rows = Number(payload.rows || session.rows)
      session.updatedAt = Date.now()
      sendJson(res, 200, { success: true, data: terminalSummary(session) })
      return
    }

    if (req.method === 'DELETE' && !action) {
      closeTerminalSession(session)
      sendJson(res, 200, { deleted: true, sessionId, session_id: sessionId })
      return
    }
  }

  if (req.method === 'POST' && url.pathname === '/agents/run') {
    const payload = await readBody(req)
    const result = await runAgent(payload)
    if (!result.success) {
      sendError(res, result.status, 'AGENT_EXECUTION_FAILED', result.error || result.response)
      return
    }
    sendJson(res, 200, {
      success: true,
      response: result.response,
      traceId: randomUUID(),
      sessionId: payload.sessionId || randomUUID(),
      usage: {},
      metadata: { harness: result.harness },
    })
    return
  }

  if (req.method === 'POST' && url.pathname === '/agents/run/stream') {
    const payload = await readBody(req)
    res.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-store',
      connection: 'keep-alive',
    })
    const result = await runAgent(payload)
    if (!result.success) {
      writeSse(res, 'error', {
        code: 'AGENT_EXECUTION_FAILED',
        message: result.error || result.response,
      })
      res.end()
      return
    }
    const text = result.response
    writeSse(res, 'message.part.updated', {
      part: { id: 'part-1', type: 'text', text },
    })
    writeSse(res, 'result', {
      finalText: text,
      metadata: {
        sessionId: payload.sessionId || randomUUID(),
        traceId: randomUUID(),
        harness: result.harness,
      },
      tokenUsage: {},
    })
    res.end()
    return
  }

  if (req.method === 'POST' && url.pathname === '/agents/run/cancel') {
    sendJson(res, 200, { success: true })
    return
  }

  sendError(res, 404, 'NOT_FOUND', 'Unknown sidecar endpoint')
}

const server = http.createServer((req, res) => {
  handle(req, res).catch((err) => {
    sendError(res, 500, 'INTERNAL_ERROR', err.message || String(err))
  })
})

server.listen(port, '0.0.0.0', () => {
  console.log(`blueprint sidecar listening on ${port}`)
})

process.on('exit', () => {
  for (const session of terminalSessions.values()) {
    if (!session.child.killed) session.child.kill('SIGTERM')
  }
})
