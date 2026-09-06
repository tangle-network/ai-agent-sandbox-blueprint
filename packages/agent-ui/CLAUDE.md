# Agent UI

This is a reusable library for agent-facing interfaces, not a consuming application.
Keep chat and session rendering, tool previews, sidecar authentication, and terminal helpers here.
Chain, contract, and provisioning behavior belongs in the shared Blueprint UI package.
Product routes, copy, and workflow logic belong in each application.
Do not import consuming application source.

Read [package.json](package.json) and the exported entrypoints before changing the public API.
Keep exported types explicit and provide a migration when changing a public contract.
Share code when consumers have the same behavior and ownership boundary; line count alone does not justify extraction.
Keep runtime dependencies justified and framework dependencies shared where supported.

Before merging, verify intentional exports, package boundaries, and affected consumers' typechecks and builds.
