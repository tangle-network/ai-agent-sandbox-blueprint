import { AutomationWorkspace } from '~/components/console/ResourceWorkspacePanels';
import type { LocalSandbox } from '~/lib/stores/sandboxes';

interface AutomationTabProps {
  sb: LocalSandbox;
  workflowCreateHref?: string;
  hasAgent: boolean;
}

export function AutomationTab({ sb, workflowCreateHref, hasAgent }: AutomationTabProps) {
  return (
    <AutomationWorkspace
      createHref={workflowCreateHref}
      scope="sandbox"
      target={sb.sandboxId ?? sb.localId}
      status={sb.status}
      hasAgent={hasAgent}
    />
  );
}
