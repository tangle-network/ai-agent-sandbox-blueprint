import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const BLUEPRINT_RELEASE_SCHEMA = 'tangle-blueprint-release/v1'

const MAX_UINT64 = (2n ** 64n) - 1n
const ADDRESS_PATTERN = /^0x[0-9a-fA-F]{40}$/
const BINARY_PATTERN = /^[a-z0-9][a-z0-9-]{0,127}$/
const NETWORK_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/
const UINT64_PATTERN = /^(?:0|[1-9][0-9]*)$/
const CONFIG_KEYS = ['schema', 'repository', 'network', 'chainId', 'tangleCore', 'blueprints']
const BLUEPRINT_KEYS = ['binary', 'id', 'bsmAddress']
const REGISTRY_COLUMNS = ['repo', 'blueprint_id', 'bsm_address', 'status', 'note']
const ZERO_ADDRESS = `0x${'0'.repeat(40)}`
const REPOSITORY_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/

export class BlueprintReleaseConfigError extends Error {
  constructor(message) {
    super(message)
    this.name = 'BlueprintReleaseConfigError'
  }
}

function fail(path, message) {
  throw new BlueprintReleaseConfigError(`${path}: ${message}`)
}

function assertObject(value, path) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    fail(path, 'must be an object')
  }
}

function assertExactKeys(value, expected, path) {
  const actual = Object.keys(value).sort()
  const allowed = [...expected].sort()
  const missing = allowed.filter((key) => !Object.hasOwn(value, key))
  const unknown = actual.filter((key) => !allowed.includes(key))
  if (missing.length > 0 || unknown.length > 0) {
    const details = []
    if (missing.length > 0) details.push(`missing ${missing.join(', ')}`)
    if (unknown.length > 0) details.push(`unknown ${unknown.join(', ')}`)
    fail(path, details.join('; '))
  }
}

function normalizeUint64(value, path) {
  let text
  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value) || value < 0) {
      fail(path, 'must be a non-negative safe integer or a decimal uint64 string')
    }
    text = String(value)
  } else if (typeof value === 'string' && UINT64_PATTERN.test(value)) {
    text = value
  } else {
    fail(path, 'must be a non-negative safe integer or a decimal uint64 string')
  }

  const number = BigInt(text)
  if (number > MAX_UINT64) fail(path, 'exceeds uint64 range')
  return number.toString()
}

function normalizeExpectedUint64(value, path) {
  return normalizeUint64(value, path)
}

function normalizeAddress(value, path) {
  if (typeof value !== 'string' || !ADDRESS_PATTERN.test(value)) {
    fail(path, 'must be a 20-byte hexadecimal address')
  }
  return value.toLowerCase()
}

function normalizeRepository(value, path) {
  if (typeof value !== 'string' || !REPOSITORY_PATTERN.test(value)) {
    fail(path, 'must be an owner/repository name')
  }
  return value.toLowerCase()
}

function parseJsonFile(file, pathLabel) {
  const source = readTextFile(file, pathLabel)
  try {
    return JSON.parse(source)
  } catch (error) {
    throw new BlueprintReleaseConfigError(
      `${pathLabel}: invalid JSON in ${file}: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

function readTextFile(file, pathLabel) {
  let source
  try {
    source = readFileSync(file, 'utf8')
  } catch (error) {
    throw new BlueprintReleaseConfigError(
      `${pathLabel}: cannot read ${file}: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
  return source
}

function deploymentExpectations(file) {
  const manifest = parseJsonFile(file, 'deployment manifest')
  assertObject(manifest, 'deployment manifest')
  for (const key of ['network', 'chainId', 'tangle']) {
    if (!Object.hasOwn(manifest, key)) fail('deployment manifest', `missing ${key}`)
  }
  return {
    network: manifest.network,
    chainId: manifest.chainId,
    tangleCore: manifest.tangle,
  }
}

/**
 * Parse tnt-core's tab-separated registration ledger.
 *
 * The ledger stores the binary name in the note column because one repository
 * can register several blueprint variants.
 */
export function parseTangleBlueprintRegistry(source, path = 'tnt-core registry') {
  if (typeof source !== 'string' || source.length === 0) fail(path, 'must contain TSV text')
  const lines = source.split(/\r?\n/).filter((line) => line.trim().length > 0)
  if (lines.length < 2) fail(path, 'must contain a header and at least one registration')

  const header = lines[0].split('\t')
  if (new Set(header).size !== header.length) fail(path, 'header contains duplicate columns')
  for (const column of REGISTRY_COLUMNS) {
    if (!header.includes(column)) fail(path, `missing ${column} column`)
  }
  const indexes = Object.fromEntries(REGISTRY_COLUMNS.map((column) => [column, header.indexOf(column)]))

  return Object.freeze(lines.slice(1).map((line, lineIndex) => {
    const fields = line.split('\t')
    const rowPath = `${path}:${lineIndex + 2}`
    if (fields.length < header.length) fail(rowPath, 'has fewer fields than the header')

    const note = fields[indexes.note]
    const binaryMatch = /(?:^|[;\s])binary=([a-z0-9][a-z0-9-]{0,127})(?:;|\s|$)/.exec(note)
    if (!binaryMatch) return null

    const bsmAddress = normalizeAddress(fields[indexes.bsm_address], `${rowPath}.bsm_address`)
    if (bsmAddress === ZERO_ADDRESS) fail(rowPath, 'registered blueprint has a zero BSM address')
    return Object.freeze({
      binary: binaryMatch[1],
      id: normalizeUint64(fields[indexes.blueprint_id], `${rowPath}.blueprint_id`),
      bsmAddress,
      status: fields[indexes.status],
      repo: fields[indexes.repo],
    })
  }).filter((entry) => entry !== null))
}

/**
 * Bind every configured binary to exactly one active tnt-core registration.
 */
export function validateBlueprintRegistrations(config, registrations) {
  if (!Array.isArray(registrations)) fail('tnt-core registry', 'must contain parsed registrations')
  for (const blueprint of config.blueprints) {
    const matches = registrations.filter(({ binary }) => binary === blueprint.binary)
    if (matches.length !== 1) {
      fail(
        `config.blueprints[${config.blueprints.indexOf(blueprint)}]`,
        `requires exactly one tnt-core registration for ${blueprint.binary}, found ${matches.length}`,
      )
    }
    const [registration] = matches
    if (registration.status !== 'registered') {
      fail(`tnt-core registry ${blueprint.binary}`, `status is ${registration.status}, not registered`)
    }
    if (typeof registration.repo !== 'string' || registration.repo.toLowerCase() !== config.repository.split('/').at(-1)) {
      fail(
        `tnt-core registry ${blueprint.binary}`,
        `repository ${registration.repo} does not match configured ${config.repository}`,
      )
    }
    if (registration.id !== blueprint.id) {
      fail(
        `tnt-core registry ${blueprint.binary}`,
        `id ${registration.id} does not match configured id ${blueprint.id}`,
      )
    }
    if (registration.bsmAddress !== blueprint.bsmAddress) {
      fail(
        `tnt-core registry ${blueprint.binary}`,
        `BSM ${registration.bsmAddress} does not match configured BSM ${blueprint.bsmAddress}`,
      )
    }
  }
  return config
}

/**
 * Validate and normalize the checked-in release-to-blueprint mapping.
 *
 * The returned object contains only immutable, normalized values. Callers use
 * the same result for release packaging, workflow dispatch, and source updates.
 */
export function validateBlueprintReleaseConfig(input, expectations = {}) {
  assertObject(input, 'config')
  assertExactKeys(input, CONFIG_KEYS, 'config')

  if (input.schema !== BLUEPRINT_RELEASE_SCHEMA) {
    fail('config.schema', `must equal ${BLUEPRINT_RELEASE_SCHEMA}`)
  }
  const repository = normalizeRepository(input.repository, 'config.repository')
  if (expectations.repository !== undefined) {
    const expectedRepository = normalizeRepository(expectations.repository, 'expected repository')
    if (repository !== expectedRepository) {
      fail('config.repository', `does not match expected ${expectedRepository}`)
    }
  }
  if (typeof input.network !== 'string' || !NETWORK_PATTERN.test(input.network)) {
    fail('config.network', 'must be a lowercase network name')
  }

  const network = input.network
  const chainId = normalizeUint64(input.chainId, 'config.chainId')
  const tangleCore = normalizeAddress(input.tangleCore, 'config.tangleCore')

  if (expectations.network !== undefined && expectations.network !== network) {
    fail('config.network', `does not match expected ${expectations.network}`)
  }
  if (expectations.chainId !== undefined) {
    const expectedChainId = normalizeExpectedUint64(expectations.chainId, 'expected chainId')
    if (chainId !== expectedChainId) {
      fail('config.chainId', `does not match expected ${expectedChainId}`)
    }
  }
  if (expectations.tangleCore !== undefined) {
    const expectedTangleCore = normalizeAddress(expectations.tangleCore, 'expected tangleCore')
    if (tangleCore !== expectedTangleCore) {
      fail('config.tangleCore', `does not match expected ${expectedTangleCore}`)
    }
  }

  if (!Array.isArray(input.blueprints) || input.blueprints.length === 0) {
    fail('config.blueprints', 'must contain at least one blueprint')
  }
  if (input.blueprints.length > 64) fail('config.blueprints', 'contains more than 64 entries')

  const binaries = new Set()
  const ids = new Set()
  const blueprints = input.blueprints.map((entry, index) => {
    const path = `config.blueprints[${index}]`
    assertObject(entry, path)
    assertExactKeys(entry, BLUEPRINT_KEYS, path)

    if (typeof entry.binary !== 'string' || !BINARY_PATTERN.test(entry.binary)) {
      fail(`${path}.binary`, 'must be a lowercase executable name without a path')
    }
    if (binaries.has(entry.binary)) fail(`${path}.binary`, `duplicates ${entry.binary}`)
    binaries.add(entry.binary)

    const id = normalizeUint64(entry.id, `${path}.id`)
    if (ids.has(id)) fail(`${path}.id`, `duplicates blueprint id ${id}`)
    ids.add(id)
    const bsmAddress = normalizeAddress(entry.bsmAddress, `${path}.bsmAddress`)
    if (bsmAddress === ZERO_ADDRESS) fail(`${path}.bsmAddress`, 'must not be the zero address')
    return Object.freeze({ binary: entry.binary, id, bsmAddress })
  })

  return Object.freeze({
    schema: BLUEPRINT_RELEASE_SCHEMA,
    repository,
    network,
    chainId,
    tangleCore,
    blueprints: Object.freeze(blueprints),
  })
}

/**
 * Read a release config and optionally bind it to a deployment manifest.
 */
export function readBlueprintReleaseConfig(file, options = {}) {
  const input = parseJsonFile(file, 'release config')
  const expectations = {
    repository: options.repository,
    network: options.network,
    chainId: options.chainId,
    tangleCore: options.tangleCore,
  }
  if (options.deploymentManifest !== undefined) {
    const manifestExpectations = deploymentExpectations(options.deploymentManifest)
    for (const key of ['network', 'chainId', 'tangleCore']) {
      if (expectations[key] === undefined) continue
      const expected = key === 'network'
        ? expectations[key]
        : key === 'chainId'
          ? normalizeExpectedUint64(expectations[key], `expected ${key}`)
          : normalizeAddress(expectations[key], `expected ${key}`)
      const fromManifest = key === 'network'
        ? manifestExpectations[key]
        : key === 'chainId'
          ? normalizeExpectedUint64(manifestExpectations[key], `deployment manifest.${key}`)
          : normalizeAddress(manifestExpectations[key], `deployment manifest.${key}`)
      if (expected !== fromManifest) {
        fail(`expected ${key}`, `does not match deployment manifest (${fromManifest})`)
      }
    }
    Object.assign(expectations, manifestExpectations)
  }
  const config = validateBlueprintReleaseConfig(input, expectations)
  if (options.registrationFile !== undefined) {
    const source = readTextFile(options.registrationFile, 'tnt-core registry')
    validateBlueprintRegistrations(config, parseTangleBlueprintRegistry(source, 'tnt-core registry'))
  }
  return config
}

export function formatBlueprintReleaseConfig(config, format) {
  switch (format) {
    case 'entries':
      return config.blueprints.map(({ binary, id }) => `${binary}:${id}`).join('\n')
    case 'details':
      return config.blueprints.map(({ binary, id, bsmAddress }) => `${binary}:${id}:${bsmAddress}`).join('\n')
    case 'binaries':
      return config.blueprints.map(({ binary }) => binary).join('\n')
    case 'space':
      return config.blueprints.map(({ binary }) => binary).join(' ')
    case 'json':
      return JSON.stringify(config, null, 2)
    case 'network':
      return config.network
    case 'repository':
      return config.repository
    case 'chain-id':
      return config.chainId
    case 'tangle-core':
      return config.tangleCore
    default:
      throw new BlueprintReleaseConfigError(
        `format: unknown format ${format}; use entries, details, binaries, space, json, repository, network, chain-id, or tangle-core`,
      )
  }
}

function cliOptionKey(name) {
  switch (name) {
    case 'deployment-manifest':
      return 'deploymentManifest'
    case 'registration-file':
      return 'registrationFile'
    case 'chain-id':
      return 'chainId'
    case 'tangle-core':
      return 'tangleCore'
    default:
      return name
  }
}

function parseCli(argv) {
  const moduleRoot = dirname(fileURLToPath(import.meta.url))
  const options = {
    file: resolve(moduleRoot, '../deploy/manifests/base-sepolia/blueprints.json'),
    format: 'entries',
  }
  const optionNames = new Set(['file', 'deployment-manifest', 'registration-file', 'repository', 'network', 'chain-id', 'tangle-core', 'format'])

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (!argument.startsWith('--')) throw new BlueprintReleaseConfigError(`arguments: unexpected ${argument}`)
    const separator = argument.indexOf('=')
    const name = separator === -1 ? argument.slice(2) : argument.slice(2, separator)
    if (!optionNames.has(name)) throw new BlueprintReleaseConfigError(`arguments: unknown option --${name}`)
    const value = separator === -1 ? argv[++index] : argument.slice(separator + 1)
    if (!value) throw new BlueprintReleaseConfigError(`arguments: --${name} requires a value`)
    options[cliOptionKey(name)] = value
  }
  return options
}

function main() {
  try {
    const options = parseCli(process.argv.slice(2))
    const config = readBlueprintReleaseConfig(options.file, options)
    process.stdout.write(`${formatBlueprintReleaseConfig(config, options.format)}\n`)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    process.stderr.write(`error: ${message}\n`)
    process.exitCode = 1
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main()
