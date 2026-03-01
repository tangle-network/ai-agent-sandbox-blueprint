import { useState, useEffect, useRef, useCallback } from 'react';
import { cn } from '@tangle/blueprint-ui';
import {
  useProvisionProgress,
  getPhaseLabel,
  isTerminalPhase,
  type ProvisionPhase,
} from '@tangle/blueprint-ui';

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

/** Timeout after 5 minutes of polling with no terminal result. */
const PROVISION_TIMEOUT_MS = 300_000;

function phaseIndex(phase: ProvisionPhase): number {
  const idx = PHASE_ORDER.indexOf(phase);
  return idx >= 0 ? idx : -1;
}

/** Format elapsed milliseconds as "Xm Ys". */
function formatElapsed(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  if (min === 0) return `${sec}s`;
  return `${min}m ${sec.toString().padStart(2, '0')}s`;
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

  // ── Timeout tracking ──
  const [elapsedMs, setElapsedMs] = useState(0);
  const [isTimedOut, setIsTimedOut] = useState(false);
  const [retryCount, setRetryCount] = useState(0);
  const startTimeRef = useRef<number>(Date.now());
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const firedRef = useRef(false);

  const isTerminal = isReady || isFailed || isTimedOut;

  // Start / stop the elapsed-time ticker.
  useEffect(() => {
    if (!callId || isTerminal) {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      return;
    }

    startTimeRef.current = Date.now();
    timerRef.current = setInterval(() => {
      const elapsed = Date.now() - startTimeRef.current;
      setElapsedMs(elapsed);
      if (elapsed >= PROVISION_TIMEOUT_MS) {
        setIsTimedOut(true);
      }
    }, 1000);

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [callId, isTerminal, retryCount]);

  // Reset timeout state when callId changes
  useEffect(() => {
    setElapsedMs(0);
    setIsTimedOut(false);
    firedRef.current = false;
    startTimeRef.current = Date.now();
  }, [callId]);

  const handleRetry = useCallback(() => {
    setElapsedMs(0);
    setIsTimedOut(false);
    firedRef.current = false;
    startTimeRef.current = Date.now();
    setRetryCount(c => c + 1);
  }, []);

  // Notify parent when ready (fire only once)
  useEffect(() => {
    if (isReady && sandboxId && sidecarUrl && onReady && !firedRef.current) {
      firedRef.current = true;
      onReady(sandboxId, sidecarUrl);
    }
  }, [isReady, sandboxId, sidecarUrl, onReady]);

  if (!callId) return null;

  const currentIdx = phase ? phaseIndex(phase) : -1;
  const showTimeout = isTimedOut && !isReady && !isFailed;

  return (
    <div className={cn('rounded-xl border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 p-4', className)}>
      {/* Header */}
      <div className="flex items-center gap-2 mb-3">
        {showTimeout ? (
          <div className="i-ph:warning-circle w-5 h-5 text-amber-400" />
        ) : isFailed ? (
          <div className="i-ph:warning-circle w-5 h-5 text-crimson-400" />
        ) : isReady ? (
          <div className="i-ph:check-circle w-5 h-5 text-teal-400" />
        ) : (
          <div className="i-ph:circle-notch w-5 h-5 text-violet-400 animate-spin" />
        )}
        <span className="text-sm font-display font-medium text-cloud-elements-textPrimary">
          {showTimeout
            ? 'Provisioning timed out'
            : phase ? getPhaseLabel(phase) : 'Waiting...'}
        </span>
        {!isTerminal && (
          <span className="text-xs text-cloud-elements-textTertiary ml-auto font-data">
            Waiting for operator... ({formatElapsed(elapsedMs)})
          </span>
        )}
        {!showTimeout && message && phase !== 'ready' && phase !== 'failed' && isTerminal && (
          <span className="text-xs text-cloud-elements-textTertiary ml-auto">{message}</span>
        )}
      </div>

      {/* Progress bar */}
      <div className="h-1.5 rounded-full bg-cloud-elements-background-depth-3 overflow-hidden mb-3">
        <div
          className={cn(
            'h-full rounded-full transition-all duration-500',
            showTimeout ? 'bg-amber-500' : isFailed ? 'bg-crimson-500' : isReady ? 'bg-teal-500' : 'bg-violet-500',
          )}
          style={{ width: `${showTimeout ? 100 : progressPct}%` }}
        />
      </div>

      {/* Phase steps */}
      <div className="flex items-center justify-between">
        {PHASE_ORDER.map((p, i) => {
          const isActive = i === currentIdx;
          const isDone = currentIdx > i || (isReady && i <= currentIdx);
          const isFail = (isFailed || showTimeout) && i === currentIdx;

          return (
            <div key={p} className="flex flex-col items-center gap-1 flex-1">
              <div
                className={cn(
                  'w-2 h-2 rounded-full transition-colors',
                  isDone && 'bg-teal-500',
                  isActive && !isFail && 'bg-violet-400 animate-pulse',
                  isFail && (showTimeout ? 'bg-amber-400' : 'bg-crimson-400'),
                  !isDone && !isActive && 'bg-cloud-elements-background-depth-4',
                )}
              />
              <span
                className={cn(
                  'text-[10px] leading-tight text-center',
                  isActive ? 'text-cloud-elements-textSecondary' : 'text-cloud-elements-textTertiary',
                )}
              >
                {getPhaseLabel(p)}
              </span>
            </div>
          );
        })}
      </div>

      {/* Timeout message */}
      {showTimeout && (
        <div className="mt-3 p-2.5 rounded-lg bg-amber-500/5 border border-amber-500/20">
          <div className="flex items-center justify-between">
            <p className="text-xs text-amber-400">
              Provisioning timed out. The operator may still be working — click Retry to keep waiting.
            </p>
            <button
              onClick={handleRetry}
              className="ml-3 shrink-0 px-3 py-1 text-xs font-display font-medium text-amber-400 bg-amber-500/10 hover:bg-amber-500/20 border border-amber-500/30 rounded-lg transition-colors"
            >
              Retry
            </button>
          </div>
        </div>
      )}

      {/* Error message */}
      {isFailed && !showTimeout && message && (
        <div className="mt-3 p-2.5 rounded-lg bg-crimson-500/5 border border-crimson-500/20">
          <p className="text-xs text-crimson-400">{message}</p>
        </div>
      )}

      {/* Success */}
      {isReady && sandboxId && (
        <div className="mt-3 p-2.5 rounded-lg bg-teal-500/5 border border-teal-500/20">
          <p className="text-xs text-teal-400">
            Sandbox <code className="font-data">{sandboxId}</code> is ready
          </p>
        </div>
      )}
    </div>
  );
}
