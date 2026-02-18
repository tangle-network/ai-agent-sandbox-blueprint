import type { JobDefinition } from '~/lib/blueprints/registry';
import { useJobForm } from '~/lib/hooks/useJobForm';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { BlueprintJobForm } from './BlueprintJobForm';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '~/components/ui/dialog';
import { Button } from '~/components/ui/button';
import { cn } from '~/lib/utils';

interface JobExecutionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  job: JobDefinition;
  serviceId: bigint;
  context?: Record<string, unknown>;
  onSuccess?: () => void;
}

export function JobExecutionDialog({
  open,
  onOpenChange,
  job,
  serviceId,
  context,
  onSuccess,
}: JobExecutionDialogProps) {
  const { values, errors, onChange, validate, reset } = useJobForm(job);
  const { submitJob, status, error: txError, txHash, reset: resetTx } = useSubmitJob();

  const hasFields = job.fields.filter((f) => !f.internal).length > 0;

  const handleSubmit = async () => {
    if (hasFields && !validate()) return;
    const args = encodeJobArgs(job, values, context);
    const hash = await submitJob({ serviceId, jobId: job.id, args, label: job.label });
    if (hash) {
      onSuccess?.();
    }
  };

  const handleClose = (nextOpen: boolean) => {
    if (!nextOpen) {
      reset();
      resetTx();
    }
    onOpenChange(nextOpen);
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="font-display flex items-center gap-2">
            <div className={`${job.icon} text-lg`} />
            {job.label}
          </DialogTitle>
          <DialogDescription>{job.description}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 mt-2">
          {job.warning && (
            <div className="glass-card rounded-lg p-3 border-amber-500/30">
              <div className="flex items-center gap-2">
                <div className="i-ph:warning text-sm text-amber-400" />
                <p className="text-xs text-amber-400">{job.warning}</p>
              </div>
            </div>
          )}

          {hasFields && (
            <BlueprintJobForm job={job} values={values} onChange={onChange} errors={errors} />
          )}

          {/* TX Status */}
          {status !== 'idle' && (
            <div
              className={cn(
                'glass-card rounded-lg p-3',
                status === 'confirmed' && 'border-teal-500/30',
                status === 'failed' && 'border-crimson-500/30',
              )}
            >
              <div className="flex items-center gap-3">
                {(status === 'signing' || status === 'pending') && (
                  <div className="i-ph:circle-fill text-sm text-blue-400 animate-pulse" />
                )}
                {status === 'confirmed' && <div className="i-ph:check-circle-fill text-sm text-teal-400" />}
                {status === 'failed' && <div className="i-ph:x-circle-fill text-sm text-crimson-400" />}
                <div>
                  <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                    {status === 'signing' && 'Waiting for wallet signature...'}
                    {status === 'pending' && 'Transaction submitted...'}
                    {status === 'confirmed' && 'Transaction confirmed!'}
                    {status === 'failed' && 'Transaction failed'}
                  </p>
                  {txHash && (
                    <p className="text-xs font-data text-cloud-elements-textTertiary mt-0.5 truncate max-w-xs">
                      TX: {txHash}
                    </p>
                  )}
                  {txError && <p className="text-xs text-crimson-400 mt-0.5">{txError}</p>}
                </div>
              </div>
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="secondary" onClick={() => handleClose(false)}>
              {status === 'confirmed' ? 'Close' : 'Cancel'}
            </Button>
            {status !== 'confirmed' && (
              <Button
                onClick={handleSubmit}
                disabled={status === 'signing' || status === 'pending'}
              >
                {status === 'signing' || status === 'pending' ? (
                  <>
                    <div className="i-ph:circle-fill text-sm animate-pulse" />
                    Submitting...
                  </>
                ) : (
                  'Submit'
                )}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
