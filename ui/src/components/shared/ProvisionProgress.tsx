import { cn } from '~/lib/utils';
import {
  useProvisionProgress,
  getPhaseLabel,
  isTerminalPhase,
  type ProvisionPhase,
} from '~/lib/hooks/useProvisionProgress';

// ---------------------------------------------------------------------------
// Phase steps for visual indicator
// ---------------------------------------------------------------------------

const PHASE_ORDER: ProvisionPhase[] = [
  'queued',
  'image_pull',
  'container_create',
  'container_start',
  'health_check',
  'ready',
];

function phaseIndex(phase: ProvisionPhase): number {
  const idx = PHASE_ORDER.indexOf(phase);
  return idx >= 0 ? idx : -1;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface ProvisionProgressProps {
  callId: number | null;
  apiUrl?: string;
  className?: string;
  onReady?: (sandboxId: string, sidecarUrl: string) => void;
}

export function ProvisionProgress({ callId, apiUrl, className, onReady }: ProvisionProgressProps) {
  const { status, phase, progressPct, message, isReady, isFailed, sandboxId, sidecarUrl } =
    useProvisionProgress({ callId, apiUrl });

  // Notify parent when ready
  if (isReady && sandboxId && sidecarUrl && onReady) {
    onReady(sandboxId, sidecarUrl);
  }

  if (!callId) return null;

  const currentIdx = phase ? phaseIndex(phase) : -1;

  return (
    <div className={cn('rounded-xl border border-neutral-800 bg-neutral-900/60 p-4', className)}>
      {/* Header */}
      <div className="flex items-center gap-2 mb-3">
        {isFailed ? (
          <div className="i-ph:warning-circle w-5 h-5 text-red-400" />
        ) : isReady ? (
          <div className="i-ph:check-circle w-5 h-5 text-green-400" />
        ) : (
          <div className="i-ph:circle-notch w-5 h-5 text-blue-400 animate-spin" />
        )}
        <span className="text-sm font-medium text-neutral-200">
          {phase ? getPhaseLabel(phase) : 'Waiting...'}
        </span>
        {message && phase !== 'ready' && phase !== 'failed' && (
          <span className="text-xs text-neutral-500 ml-auto">{message}</span>
        )}
      </div>

      {/* Progress bar */}
      <div className="h-1.5 rounded-full bg-neutral-800 overflow-hidden mb-3">
        <div
          className={cn(
            'h-full rounded-full transition-all duration-500',
            isFailed ? 'bg-red-500' : isReady ? 'bg-green-500' : 'bg-blue-500',
          )}
          style={{ width: `${progressPct}%` }}
        />
      </div>

      {/* Phase steps */}
      <div className="flex items-center justify-between">
        {PHASE_ORDER.map((p, i) => {
          const isActive = i === currentIdx;
          const isDone = currentIdx > i || (isReady && i <= currentIdx);
          const isFail = isFailed && i === currentIdx;

          return (
            <div key={p} className="flex flex-col items-center gap-1 flex-1">
              <div
                className={cn(
                  'w-2 h-2 rounded-full transition-colors',
                  isDone && 'bg-green-500',
                  isActive && !isFail && 'bg-blue-400 animate-pulse',
                  isFail && 'bg-red-400',
                  !isDone && !isActive && 'bg-neutral-700',
                )}
              />
              <span
                className={cn(
                  'text-[10px] leading-tight text-center',
                  isActive ? 'text-neutral-300' : 'text-neutral-600',
                )}
              >
                {getPhaseLabel(p)}
              </span>
            </div>
          );
        })}
      </div>

      {/* Error message */}
      {isFailed && message && (
        <div className="mt-3 p-2 rounded bg-red-900/20 border border-red-900/40">
          <p className="text-xs text-red-400">{message}</p>
        </div>
      )}

      {/* Success */}
      {isReady && sandboxId && (
        <div className="mt-3 p-2 rounded bg-green-900/20 border border-green-900/40">
          <p className="text-xs text-green-400">
            Sandbox <code className="font-mono">{sandboxId}</code> is ready
          </p>
        </div>
      )}
    </div>
  );
}
