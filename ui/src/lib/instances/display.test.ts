import { describe, expect, it } from 'vitest';
import {
  getInstanceSandboxDisplayValue,
  getInstanceServiceDisplayValue,
  getInstanceStatusLabel,
} from './display';

describe('instance lifecycle display', () => {
  it('shows pending activation before service activation', () => {
    expect(getInstanceStatusLabel({ status: 'creating', serviceId: '', sandboxId: undefined })).toBe('Pending activation');
    expect(getInstanceServiceDisplayValue({ status: 'creating', serviceId: '', sandboxId: undefined })).toBe('Pending activation');
    expect(getInstanceSandboxDisplayValue({ status: 'creating', serviceId: '', sandboxId: undefined })).toBe('Pending activation');
  });

  it('shows provisioning after service activation and before sandbox provisioning', () => {
    expect(getInstanceStatusLabel({ status: 'creating', serviceId: '7', sandboxId: undefined })).toBe('Provisioning');
    expect(getInstanceServiceDisplayValue({ status: 'creating', serviceId: '7', sandboxId: undefined })).toBe('#7');
    expect(getInstanceSandboxDisplayValue({ status: 'creating', serviceId: '7', sandboxId: undefined })).toBe('Provisioning');
  });

  it('preserves terminal status labels', () => {
    expect(getInstanceStatusLabel({ status: 'running', serviceId: '7', sandboxId: 'sandbox-1' })).toBe('Running');
    expect(getInstanceStatusLabel({ status: 'error', serviceId: '', sandboxId: undefined })).toBe('Error');
  });
});
