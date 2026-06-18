#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

bun run build:packages
bun run typecheck
python3 ../contracts/harness-studio/v1/validate_contract.py

bun test \
  app/lib/api/schemas.test.ts \
  app/lib/studio-config/flowai-config.test.ts \
  app/lib/studio-nav/studio-module-registry.test.ts \
  app/lib/runtime/flowai-harness-runtime-adapter.test.ts \
  app/lib/runtime/harness-studio-ui.test.ts \
  app/lib/domain/workspace-runtime-scope.test.ts \
  packages/studio-core/src/runtime/adapters.test.ts \
  packages/studio-core/src/stream/sse.test.ts \
  packages/studio-core/src/storage/namespaced-storage.test.ts \
  packages/studio-ui/src/primitives.test.tsx \
  packages/studio-ui/src/page-shell.test.tsx \
  packages/studio-ui/src/studio-shell.test.tsx \
  packages/studio-ui/src/styles-assets.test.ts \
  packages/studio-features-connect/src/connect-dashboard-model.test.ts \
  packages/studio-features-connect/src/connect-discovery-model.test.ts \
  packages/studio-features-connect/src/connect-search-model.test.ts \
  packages/studio-features-connect/src/connect-target-model.test.ts \
  packages/studio-features-connect/src/connect-tools-model.test.ts \
  packages/studio-features-evals/src/eval-case-nav-model.test.ts \
  packages/studio-features-evals/src/eval-case-thread-model.test.ts \
  packages/studio-features-evals/src/eval-matrix-model.test.ts \
  packages/studio-features-evals/src/eval-sidebar-model.test.ts
