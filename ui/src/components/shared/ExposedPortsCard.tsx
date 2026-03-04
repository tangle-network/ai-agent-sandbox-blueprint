import { Card, CardContent, CardHeader, CardTitle } from '@tangle-network/blueprint-ui/components';

interface ExposedPort {
  container_port: number;
  host_port: number;
  protocol: string;
}

interface ExposedPortsCardProps {
  ports: ExposedPort[];
  accessPath: string;
  className?: string;
}

export function ExposedPortsCard({ ports, accessPath, className }: ExposedPortsCardProps) {
  if (!ports.length) return null;

  return (
    <Card className={className}>
      <CardHeader>
        <CardTitle className="text-sm">Exposed Ports</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {ports.map((p) => (
            <div
              key={`${p.container_port}-${p.protocol}`}
              className="flex items-center gap-2 px-3 py-2 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor"
            >
              <div className="i-ph:globe text-sm text-teal-400" />
              <div className="min-w-0">
                <span className="text-xs font-data font-medium text-cloud-elements-textPrimary">
                  :{p.container_port}
                </span>
                <span className="text-[10px] text-cloud-elements-textTertiary ml-1.5">
                  {p.protocol}
                </span>
              </div>
            </div>
          ))}
        </div>
        <p className="text-[11px] text-cloud-elements-textTertiary mt-2">
          Access via <span className="font-data">{accessPath}</span>
        </p>
      </CardContent>
    </Card>
  );
}
