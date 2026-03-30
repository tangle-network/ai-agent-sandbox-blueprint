import { useState, useCallback, useEffect } from 'react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';

interface SnapshotParams {
  destination: string;
  include_workspace: boolean;
  include_state: boolean;
}

interface SnapshotDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (params: SnapshotParams) => Promise<void>;
}

export function SnapshotDialog({ open, onOpenChange, onConfirm }: SnapshotDialogProps) {
  const [destination, setDestination] = useState('');
  const [includeWorkspace, setIncludeWorkspace] = useState(true);
  const [includeState, setIncludeState] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset form when dialog closes
  useEffect(() => {
    if (!open) {
      setDestination('');
      setIncludeWorkspace(true);
      setIncludeState(true);
      setBusy(false);
      setError(null);
    }
  }, [open]);

  const handleConfirm = useCallback(async () => {
    const trimmed = destination.trim();
    if (!trimmed.startsWith('https://') && !trimmed.startsWith('s3://')) {
      setError('Destination must start with https:// or s3://');
      return;
    }
    setError(null);
    setBusy(true);
    try {
      await onConfirm({
        destination: trimmed,
        include_workspace: includeWorkspace,
        include_state: includeState,
      });
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Snapshot failed');
    } finally {
      setBusy(false);
    }
  }, [destination, includeWorkspace, includeState, onConfirm, onOpenChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="font-display flex items-center gap-2">
            <div className="i-ph:camera text-lg text-cloud-elements-textTertiary" />
            Take Snapshot
          </DialogTitle>
          <DialogDescription className="text-sm text-cloud-elements-textSecondary">
            Save a snapshot of this sandbox to a remote destination.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 mt-2">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-cloud-elements-textSecondary">
              Destination URL
            </label>
            <Input
              value={destination}
              onChange={(e) => {
                setDestination(e.target.value);
                if (error) setError(null);
              }}
              placeholder="https://storage.example.com/snapshots/ or s3://bucket/path"
              className="font-data text-sm"
              disabled={busy}
            />
            {error && <p className="text-xs text-red-400">{error}</p>}
          </div>

          <div className="space-y-2">
            <label className="text-xs font-medium text-cloud-elements-textSecondary">
              Include in snapshot
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={includeWorkspace}
                onChange={(e) => setIncludeWorkspace(e.target.checked)}
                disabled={busy}
                className="accent-teal-500"
              />
              <span className="text-sm text-cloud-elements-textPrimary">Workspace files</span>
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={includeState}
                onChange={(e) => setIncludeState(e.target.checked)}
                disabled={busy}
                className="accent-teal-500"
              />
              <span className="text-sm text-cloud-elements-textPrimary">Runtime state</span>
            </label>
          </div>
        </div>

        <div className="flex justify-end gap-2 mt-4">
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button
            onClick={handleConfirm}
            disabled={busy || !destination.trim()}
          >
            {busy ? 'Snapshotting...' : 'Take Snapshot'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
