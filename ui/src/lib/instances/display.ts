import type { LocalInstance } from '~/lib/stores/instances';

type InstanceLifecycleFields = Pick<LocalInstance, 'status' | 'serviceId' | 'sandboxId'>;

export function getInstanceStatusLabel(instance: InstanceLifecycleFields): string {
  if (instance.status !== 'creating') {
    switch (instance.status) {
      case 'running':
        return 'Running';
      case 'stopped':
        return 'Stopped';
      case 'gone':
        return 'Deleted';
      case 'error':
        return 'Error';
      default:
        return 'Creating';
    }
  }

  if (!instance.serviceId) return 'Pending activation';
  if (!instance.sandboxId) return 'Provisioning';
  return 'Provisioning';
}

export function getInstanceServiceDisplayValue(instance: InstanceLifecycleFields): string {
  return instance.serviceId ? `#${instance.serviceId}` : 'Pending activation';
}

export function getInstanceSandboxDisplayValue(instance: InstanceLifecycleFields): string {
  if (instance.sandboxId) return instance.sandboxId;
  return instance.serviceId ? 'Provisioning' : 'Pending activation';
}
