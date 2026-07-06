import { AutomationWorkspace } from '~/components/console/ResourceWorkspacePanels';
import type { LocalInstance } from '~/lib/stores/instances';

interface AutomationTabProps {
  inst: LocalInstance;
  workflowCreateHref?: string;
  hasAgent: boolean;
}

export function AutomationTab({ inst, workflowCreateHref, hasAgent }: AutomationTabProps) {
  return (
    <AutomationWorkspace
      createHref={workflowCreateHref}
      scope={inst.teeEnabled ? 'tee-instance' : 'instance'}
      target={inst.id}
      status={inst.status}
      hasAgent={hasAgent}
    />
  );
}
