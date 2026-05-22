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
