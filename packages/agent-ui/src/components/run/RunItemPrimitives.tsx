import { useEffect, useState } from 'react';
import { formatDuration } from '~/utils/format';

export function LiveDuration({ startTime }: { startTime: number }) {
  const [elapsed, setElapsed] = useState(Date.now() - startTime);

  useEffect(() => {
    const id = setInterval(() => setElapsed(Date.now() - startTime), 100);
    return () => clearInterval(id);
  }, [startTime]);

  return (
    <span className="text-xs font-mono text-neutral-400 dark:text-neutral-500 tabular-nums">
      {formatDuration(elapsed)}
    </span>
  );
}
