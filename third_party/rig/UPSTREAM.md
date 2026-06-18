Source: https://github.com/0xPlaygrounds/rig.git
Revision: 128a96e61fcd1c942466c225ad6ccb25e5364d18

This workspace snapshot is intentionally minimal. It contains the upstream root
manifest plus the packages that `rust-agent-fw` consumes or that those packages
reference via local path:

- `rig/rig-core`
- `rig/rig-derive`
- `rig-integrations/rig-bedrock`

Local patches in this checkout add prompt-caching breakpoints for Anthropic tool
definitions and AWS Bedrock cache points for system prompts, tool definitions,
and the last message block.
