import { Link } from 'react-router';
import type { ReactNode } from 'react';
import { Button } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';

export type ConsoleMetric = {
  label: string;
  value: string;
  detail?: string;
  tone?: 'brand' | 'ready' | 'warn' | 'danger' | 'muted';
};

const metricToneClass: Record<NonNullable<ConsoleMetric['tone']>, string> = {
  brand: 'text-[var(--sandbox-console-brand)]',
  ready: 'text-[var(--sandbox-console-success)]',
  warn: 'text-[var(--sandbox-console-warning)]',
  danger: 'text-[var(--sandbox-console-danger)]',
  muted: 'text-[var(--sandbox-console-muted)]',
};

export function ConsolePage({
  title,
  eyebrow,
  actions,
  children,
  className,
}: {
  title: string;
  eyebrow?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn('flex h-full min-h-0 flex-col overflow-hidden', className)}>
      <div className="flex min-h-15 shrink-0 items-center justify-between gap-4 border-b border-[var(--sandbox-console-border)] px-4 py-3 lg:px-6">
        <div className="min-w-0">
          {eyebrow ? (
            <p className="font-data text-[10px] uppercase tracking-[0.16em] text-[var(--sandbox-console-muted)]">
              {eyebrow}
            </p>
          ) : null}
          <h1 className="truncate font-display text-xl font-semibold leading-tight text-[var(--sandbox-console-text)]">
            {title}
          </h1>
        </div>
        {actions ? <div className="shrink-0">{actions}</div> : null}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4 lg:p-6">
        {children}
      </div>
    </section>
  );
}

export function ConsoleMetricStrip({ metrics }: { metrics: ConsoleMetric[] }) {
  return (
    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
      {metrics.map((metric) => (
        <div key={metric.label} className="sandbox-console-panel rounded-md p-3">
          <p className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
            {metric.label}
          </p>
          <div className="mt-2 flex items-baseline justify-between gap-3">
            <p className={cn('font-data text-2xl font-semibold leading-none', metricToneClass[metric.tone ?? 'muted'])}>
              {metric.value}
            </p>
            {metric.detail ? (
              <span className="truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
                {metric.detail}
              </span>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}

export function ConsoleSection({
  title,
  actionTo,
  actionLabel,
  children,
  className,
}: {
  title: string;
  actionTo?: string;
  actionLabel?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn('sandbox-console-panel overflow-hidden rounded-md', className)}>
      <div className="flex h-11 items-center justify-between border-b border-[var(--sandbox-console-border)] px-3">
        <h2 className="font-data text-[11px] font-semibold uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
          {title}
        </h2>
        {actionTo && actionLabel ? (
          <Link to={actionTo}>
            <Button variant="ghost" size="sm">{actionLabel}</Button>
          </Link>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function EmptyConsoleState({
  icon,
  title,
  detail,
  actionTo,
  actionLabel,
}: {
  icon: string;
  title: string;
  detail?: string;
  actionTo?: string;
  actionLabel?: string;
}) {
  return (
    <div className="flex min-h-52 flex-col items-center justify-center gap-3 p-6 text-center">
      <span className={cn('text-3xl text-[var(--sandbox-console-subtle)]', icon)} />
      <div>
        <p className="font-display text-sm font-medium text-[var(--sandbox-console-text)]">{title}</p>
        {detail ? (
          <p className="mt-1 max-w-md text-sm text-[var(--sandbox-console-muted)]">{detail}</p>
        ) : null}
      </div>
      {actionTo && actionLabel ? (
        <Link to={actionTo}>
          <Button variant="outline" size="sm">{actionLabel}</Button>
        </Link>
      ) : null}
    </div>
  );
}

export function ConsoleChip({
  children,
  tone = 'muted',
}: {
  children: ReactNode;
  tone?: 'brand' | 'ready' | 'warn' | 'danger' | 'muted';
}) {
  return (
    <span
      className={cn(
        'inline-flex h-6 items-center rounded border px-2 font-data text-[10px] uppercase tracking-[0.08em]',
        tone === 'brand' && 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]',
        tone === 'ready' && 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)]',
        tone === 'warn' && 'border-amber-400/20 bg-amber-400/10 text-[var(--sandbox-console-warning)]',
        tone === 'danger' && 'border-red-400/20 bg-red-400/10 text-[var(--sandbox-console-danger)]',
        tone === 'muted' && 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] text-[var(--sandbox-console-muted)]',
      )}
    >
      {children}
    </span>
  );
}
