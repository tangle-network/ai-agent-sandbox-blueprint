import { blo, type Address as BloAddress } from 'blo';
import { cn } from '@tangle-network/blueprint-ui';

export type IdentityTone = 'brand' | 'teal' | 'blue' | 'amber' | 'violet' | 'slate' | 'danger';

export type IdentityMeta = {
  label: string;
  mark: string;
  detail?: string;
  icon?: string;
  symbol?: string;
  tone?: IdentityTone;
  image?: 'tangle';
};

const markToneClass: Record<IdentityTone, string> = {
  brand: 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]',
  teal: 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)]',
  blue: 'border-sky-400/30 bg-sky-400/10 text-sky-300 dark:text-sky-300',
  amber: 'border-amber-400/28 bg-amber-400/10 text-amber-300',
  violet: 'border-violet-400/35 bg-violet-400/12 text-violet-300',
  slate: 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)]',
  danger: 'border-red-400/28 bg-red-400/10 text-[var(--sandbox-console-danger)]',
};

const textToneClass: Record<IdentityTone, string> = {
  brand: 'text-[var(--sandbox-console-brand)]',
  teal: 'text-[var(--sandbox-console-success)]',
  blue: 'text-sky-300',
  amber: 'text-amber-300',
  violet: 'text-violet-300',
  slate: 'text-[var(--sandbox-console-secondary)]',
  danger: 'text-[var(--sandbox-console-danger)]',
};

const sizeClass = {
  sm: 'h-7 w-7 text-[10px]',
  md: 'h-9 w-9 text-xs',
  lg: 'h-12 w-12 text-sm',
} as const;

export function IdentityMark({
  identity,
  size = 'md',
  className,
}: {
  identity: IdentityMeta;
  size?: keyof typeof sizeClass;
  className?: string;
}) {
  const tone = identity.tone ?? 'slate';

  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-[5px] border p-1 font-data font-black uppercase tracking-tight shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]',
        sizeClass[size],
        markToneClass[tone],
        className,
      )}
      aria-hidden="true"
      title={identity.label}
    >
      {identity.image === 'tangle' ? (
        <img src="/tangle-mark.svg" alt="" className="h-full w-full object-contain" />
      ) : identity.symbol ? (
        <span className={cn('relative z-10 text-base', size === 'sm' && 'text-sm', size === 'lg' && 'text-lg', identity.symbol)} />
      ) : (
        <span className="relative z-10">{identity.mark}</span>
      )}
    </span>
  );
}

export function IdentityText({
  identity,
  label,
  detail,
  className,
}: {
  identity: IdentityMeta;
  label?: string;
  detail?: string;
  className?: string;
}) {
  return (
    <span className={cn('min-w-0', className)}>
      <span className="block truncate font-display text-[15px] font-bold leading-tight text-[var(--sandbox-console-text)]">
        {label ?? identity.label}
      </span>
      {(detail ?? identity.detail) ? (
        <span className="mt-0.5 block truncate font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">
          {detail ?? identity.detail}
        </span>
      ) : null}
    </span>
  );
}

export function IdentityRow({
  identity,
  label,
  detail,
  size = 'md',
  className,
}: {
  identity: IdentityMeta;
  label?: string;
  detail?: string;
  size?: keyof typeof sizeClass;
  className?: string;
}) {
  return (
    <span className={cn('flex min-w-0 items-center gap-3', className)}>
      <IdentityMark identity={identity} size={size} />
      <IdentityText identity={identity} label={label} detail={detail} />
    </span>
  );
}

export function OperatorIdentity({
  address,
  detail,
  compact = false,
}: {
  address: string;
  detail?: string;
  compact?: boolean;
}) {
  const shortened = shortenAddress(address);
  const normalizedAddress = normalizeOperatorAddress(address);

  return (
    <span className="flex min-w-0 items-center gap-2.5">
      {normalizedAddress ? (
        <OperatorIdenticon address={normalizedAddress} size={compact ? 'sm' : 'md'} />
      ) : (
        <IdentityMark identity={getOperatorIdentity()} size={compact ? 'sm' : 'md'} />
      )}
      <span className="min-w-0">
        <span className="block truncate font-data text-sm font-bold text-[var(--sandbox-console-text)]">
          {shortened}
        </span>
        {detail ? (
          <span className="block truncate font-data text-[11px] font-medium text-[var(--sandbox-console-subtle)]">
            {detail}
          </span>
        ) : null}
      </span>
    </span>
  );
}

export function OperatorIdenticon({
  address,
  size = 'md',
  className,
}: {
  address: string;
  size?: keyof typeof sizeClass;
  className?: string;
}) {
  const normalizedAddress = normalizeOperatorAddress(address);
  if (!normalizedAddress) {
    return <IdentityMark identity={getOperatorIdentity()} size={size} className={className} />;
  }

  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 overflow-hidden rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] shadow-[inset_0_1px_0_rgba(255,255,255,0.14)]',
        sizeClass[size],
        className,
      )}
      aria-hidden="true"
      title={shortenAddress(normalizedAddress)}
    >
      <img src={blo(normalizedAddress, 96)} alt="" className="h-full w-full scale-110 object-cover" />
      <span className="pointer-events-none absolute inset-0 rounded-[inherit] ring-1 ring-inset ring-white/10" />
    </span>
  );
}

export function getTextToneClass(identity: IdentityMeta) {
  return textToneClass[identity.tone ?? 'slate'];
}

export function getBlueprintIdentity(blueprintIdOrName?: string): IdentityMeta {
  const value = (blueprintIdOrName ?? '').toLowerCase();
  if (value.includes('tee')) {
    // The blueprint TYPE badge labels the runtime kind, not a verified
    // attestation. Use a plain shield + "confidential runtime", never a
    // shield-CHECK or "attested" (which would imply a completed verdict).
    return { label: 'TEE Instance', mark: 'TEE', detail: 'confidential runtime', icon: 'i-ph:shield', tone: 'amber' };
  }
  if (value.includes('instance')) {
    return { label: 'Instance', mark: 'INS', detail: 'single-tenant service', icon: 'i-ph:cube', tone: 'brand' };
  }
  return { label: 'Sandbox', mark: 'SBX', detail: 'elastic workspace', icon: 'i-ph:cloud', tone: 'teal' };
}

export function getImageIdentity(value: string): IdentityMeta {
  const image = value.toLowerCase();
  if (value === '__custom_image__' || image.length === 0) {
    return { label: 'Custom image', mark: 'IMG', detail: 'operator-compatible image', icon: 'i-ph:code-block', tone: 'violet' };
  }
  if (image.startsWith('ghcr.io/')) {
    return { label: 'Registry image', mark: 'GH', detail: 'ghcr.io/tangle-network', icon: 'i-ph:package', tone: 'blue' };
  }
  if (image.includes('blueprint-sidecar')) {
    return { label: 'Local sidecar', mark: 'LOC', detail: 'local runtime image', icon: 'i-ph:terminal-window', tone: 'teal' };
  }
  return { label: 'Container image', mark: 'IMG', detail: 'custom registry source', icon: 'i-ph:cube', tone: 'slate' };
}

export function getStackIdentity(value: string): IdentityMeta {
  switch (value) {
    case 'python':
      return { label: 'Python', mark: '', detail: 'notebooks, agents, scripts', symbol: 'i-ph:terminal-window', tone: 'blue' };
    case 'nodejs':
      return { label: 'Node.js', mark: '', detail: 'web apps and tool servers', symbol: 'i-ph:graph', tone: 'teal' };
    case 'rust':
      return { label: 'Rust', mark: '', detail: 'systems and operators', symbol: 'i-ph:gear-six', tone: 'amber' };
    default:
      return { label: 'Default', mark: 'TK', detail: 'Tangle sidecar baseline', image: 'tangle', tone: 'brand' };
  }
}

export function getRuntimeIdentity(value: string): IdentityMeta {
  switch (value) {
    case 'firecracker':
      return { label: 'Firecracker', mark: 'uVM', detail: 'microVM isolation', icon: 'i-ph:lightning', tone: 'amber' };
    case 'tee':
      return { label: 'TEE', mark: 'TEE', detail: 'confidential compute', icon: 'i-ph:shield-check', tone: 'amber' };
    default:
      return { label: 'Docker', mark: 'D', detail: 'container runtime', icon: 'i-ph:package', tone: 'teal' };
  }
}

export function getAgentIdentity(value: string): IdentityMeta {
  switch (value) {
    case 'batch':
      return { label: 'Batch', mark: 'BAT', detail: 'queued multi-run agent', icon: 'i-ph:stack', tone: 'blue' };
    case 'default':
      return { label: 'Default', mark: 'AI', detail: 'interactive sidecar agent', icon: 'i-ph:robot', tone: 'brand' };
    case '__none__':
    case '':
      return { label: 'Compute only', mark: 'CPU', detail: 'no bundled agent', icon: 'i-ph:cpu', tone: 'slate' };
    default:
      return { label: value, mark: 'AI', detail: 'custom agent identifier', icon: 'i-ph:robot', tone: 'violet' };
  }
}

export function getCapabilityIdentity(value: 'harness' | 'computer-use' | 'ssh'): IdentityMeta {
  if (value === 'computer-use') {
    return { label: 'Computer Use', mark: 'UI', detail: 'browser and visual tools', icon: 'i-ph:monitor', tone: 'blue' };
  }
  if (value === 'ssh') {
    return { label: 'SSH', mark: 'SSH', detail: 'operator entrypoint', icon: 'i-ph:terminal', tone: 'amber' };
  }
  return { label: 'All-Harness', mark: 'HX', detail: 'Claude, Codex, opencode, Kimi, Gemini', icon: 'i-ph:circles-three-plus', tone: 'brand' };
}

export function getResourceIdentity(value: 'cpu' | 'memory' | 'disk' | 'network'): IdentityMeta {
  if (value === 'memory') return { label: 'Memory', mark: 'RAM', detail: 'working set', icon: 'i-ph:memory', tone: 'blue' };
  if (value === 'disk') return { label: 'Disk', mark: 'DSK', detail: 'persistent storage', icon: 'i-ph:hard-drive', tone: 'amber' };
  if (value === 'network') return { label: 'Network', mark: 'NET', detail: 'operator proxy', icon: 'i-ph:globe', tone: 'teal' };
  return { label: 'CPU', mark: 'CPU', detail: 'compute cores', icon: 'i-ph:cpu', tone: 'brand' };
}

export function getStatusIdentity(status: string): IdentityMeta {
  const normalized = status.toLowerCase();
  if (normalized === 'running' || normalized === 'ready' || normalized === 'active') {
    return { label: 'Running', mark: 'ON', detail: 'operator ready', icon: 'i-ph:check-circle-fill', tone: 'teal' };
  }
  if (normalized === 'creating' || normalized === 'processing' || normalized === 'provisioning') {
    return { label: 'Provisioning', mark: 'PRV', detail: 'operator pending', icon: 'i-ph:pulse', tone: 'brand' };
  }
  if (normalized === 'stopped' || normalized === 'warm' || normalized === 'cold' || normalized === 'timed-out') {
    return { label: 'Paused', mark: 'IDL', detail: 'resume path', icon: 'i-ph:pause-circle', tone: 'amber' };
  }
  if (normalized === 'error' || normalized === 'blocked' || normalized === 'failed') {
    return { label: 'Attention', mark: 'ERR', detail: 'needs action', icon: 'i-ph:warning-circle', tone: 'danger' };
  }
  return { label: 'Unknown', mark: 'UNK', detail: 'unresolved state', icon: 'i-ph:circle-dashed', tone: 'slate' };
}

export function getSecurityIdentity(value: string): IdentityMeta {
  const normalized = value.toLowerCase();
  // "attested" is reserved for a resource whose server-evaluated attestation
  // verdict is `verified` (shield-CHECK + "hardware trust"). It must never be
  // derived from the `teeEnabled` config flag alone.
  if (normalized.includes('attested')) {
    return { label: 'Attested', mark: 'TEE', detail: 'hardware trust', icon: 'i-ph:shield-check', tone: 'amber' };
  }
  // "tee-enabled" claims a capability (TEE runtime requested), NOT a completed
  // attestation: a plain shield, no check, and a detail that says "runtime" not
  // "trust". This is what list views derive from `teeEnabled` until a per-row
  // verdict is fetched.
  if (normalized.includes('tee')) {
    return { label: 'TEE runtime', mark: 'TEE', detail: 'attestation not verified', icon: 'i-ph:shield', tone: 'amber' };
  }
  if (normalized.includes('secret')) {
    return { label: 'Secrets', mark: 'KEY', detail: 'encrypted material', icon: 'i-ph:lock-key', tone: 'brand' };
  }
  return { label: 'Session', mark: 'SES', detail: 'wallet-scoped auth', icon: 'i-ph:shield', tone: 'slate' };
}

/**
 * Security identity for a sandbox/instance's secret material, driven by the REAL
 * server-evaluated attestation verdict — never by the `teeEnabled` config flag
 * alone.
 *
 * - `verified === true` (server returned verdict `verified`): the workload is
 *   genuinely hardware-attested → the "Attested / hardware trust" identity with
 *   the shield-check.
 * - `teeRequested` but not verified (no attestation fetched yet, or verdict is
 *   `unverified`/`measurement_mismatch`): an honest amber "TEE requested —
 *   unverified" identity with NO shield-check and NO "hardware trust" claim.
 * - non-TEE: ordinary operator-encrypted secret identity.
 *
 * `credentialsMissing` always wins (nothing to protect), matching the prior row.
 */
export function getTeeSecurityIdentity(opts: {
  credentialsMissing: boolean;
  teeRequested: boolean;
  verified: boolean;
}): IdentityMeta {
  if (opts.credentialsMissing) return getSecurityIdentity('session');
  if (opts.teeRequested) {
    if (opts.verified) return getSecurityIdentity('attested');
    return {
      label: 'TEE requested',
      mark: 'TEE',
      detail: 'unverified — verify attestation',
      icon: 'i-ph:shield-warning',
      tone: 'amber',
    };
  }
  return getSecurityIdentity('secrets');
}

export function getStorageIdentity(value: string): IdentityMeta {
  const normalized = value.toLowerCase();
  if (normalized.includes('gone')) {
    return { label: 'Deleted', mark: 'DEL', detail: 'removed volume', icon: 'i-ph:x-circle', tone: 'danger' };
  }
  if (normalized.includes('warm') || normalized.includes('hot')) {
    return { label: 'Warm', mark: 'HOT', detail: 'resume cache', icon: 'i-ph:database', tone: 'amber' };
  }
  if (normalized.includes('cold')) {
    return { label: 'Cold', mark: 'CLD', detail: 'archived volume', icon: 'i-ph:snowflake', tone: 'blue' };
  }
  return { label: 'Ephemeral', mark: 'EFS', detail: 'runtime volume', icon: 'i-ph:hard-drives', tone: 'slate' };
}

export function getOperatorIdentity(address?: string): IdentityMeta {
  if (!address) {
    return { label: 'Operator', mark: 'OP', detail: 'Tangle operator', image: 'tangle', tone: 'brand' };
  }

  return {
    label: 'Operator',
    mark: operatorShard(address),
    detail: shortenAddress(address),
    icon: 'i-ph:hexagon',
    tone: operatorTone(address),
  };
}

function shortenAddress(value: string) {
  if (value.length <= 16) return value;
  return `${value.slice(0, 8)}...${value.slice(-6)}`;
}

function normalizeOperatorAddress(value?: string): BloAddress | undefined {
  if (!value || !/^0x[a-f0-9]{40}$/i.test(value)) return undefined;
  return value as BloAddress;
}

function operatorShard(value: string) {
  const compact = value.replace(/^0x/i, '').replace(/[^a-f0-9]/gi, '').toUpperCase();
  if (compact.length < 2) return 'OP';
  return compact.slice(0, 2);
}

function operatorTone(value: string): IdentityTone {
  const compact = value.replace(/^0x/i, '').replace(/[^a-f0-9]/gi, '');
  const pivot = parseInt(compact.slice(-2) || '0', 16);
  const tones: IdentityTone[] = ['brand', 'teal', 'blue', 'amber', 'violet'];
  return tones[pivot % tones.length];
}
