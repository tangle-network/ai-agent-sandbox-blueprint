import { Card, CardContent, CardHeader, CardTitle } from '@tangle-network/blueprint-ui/components';
import { CopyButton } from './CopyButton';

interface ExposedPort {
  container_port: number;
  host_port: number;
  protocol: string;
}

interface ExposedPortsCardProps {
  ports: ExposedPort[];
  proxyBaseUrl: string;
  className?: string;
}

export function ExposedPortsCard({ ports, proxyBaseUrl, className }: ExposedPortsCardProps) {
  if (!ports.length) return null;

  return (
    <Card className={className}>
      <CardHeader>
        <CardTitle className="text-sm">Exposed Ports</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {ports.map((p) => {
            const proxyUrl = `${proxyBaseUrl}${p.container_port}/`;
            return (
              <div
                key={`${p.container_port}-${p.protocol}`}
                className="flex items-center gap-2 px-3 py-2 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor"
              >
                <div className="i-ph:globe text-sm text-teal-400 shrink-0" />
                <div className="min-w-0 flex-1">
                  <span className="text-xs font-data font-medium text-cloud-elements-textPrimary">
                    :{p.container_port}
                  </span>
                  <span className="text-[10px] text-cloud-elements-textTertiary ml-1">
                    &rarr; {p.host_port}
                  </span>
                  <span className="text-[10px] text-cloud-elements-textTertiary ml-1.5">
                    {p.protocol}
                  </span>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <CopyButton value={proxyUrl} />
                  <a
                    href={proxyUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="p-0.5 rounded text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary transition-colors"
                    title="Open in new tab"
                  >
                    <div className="i-ph:arrow-square-out text-xs" />
                  </a>
                </div>
              </div>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}
