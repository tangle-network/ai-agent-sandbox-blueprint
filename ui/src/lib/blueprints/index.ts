// Auto-register all blueprints on import
import './sandbox-blueprint';
import './instance-blueprint';
import './tee-instance-blueprint';

export { getAllBlueprints, getBlueprint, getBlueprintJobs, getJobById, registerBlueprint } from '@tangle/blueprint-ui';
export type { BlueprintDefinition, JobDefinition, JobFieldDef, JobCategory, AbiContextParam } from '@tangle/blueprint-ui';
