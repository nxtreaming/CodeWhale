# Plugin bundles

Codewhale v0.9.1 supports a deliberately small plugin-bundle boundary. A
bundle may contribute declarative Skills and MCP server configuration through
Codewhale's existing engines. Discovery alone never executes, enables, trusts,
downloads, updates, or installs anything.

## Discovery and precedence

Codewhale scans only its own roots:

- User: `~/.codewhale/plugins/<name>/plugin.toml`
- Workspace: `<workspace>/.codewhale/plugins/<name>/plugin.toml`

No built-in bundle ships in v0.9.1. The internal precedence order is built-in,
user, then workspace; the first bundle with a given name wins. This prevents a
repository from shadowing an explicitly installed user bundle. Symbolic-link
roots, manifests, component paths, and nested component files fail closed.

New user and workspace bundles are always untrusted and disabled. Discovery is
read-only and does not inspect Claude, Cursor, Codex, Kimi, Grok, or other
tools' extension or credential directories.

Pre-v0.9.1 `overrides.json` enablement is intentionally not imported as trust.
Existing bundles therefore return to disabled until they receive the v1
content and capability review.

## Manifest

Every bundle uses a versioned `plugin.toml` and a semantic version:

```toml
schema_version = 1

[plugin]
name = "example"
version = "0.1.0"
description = "Example instruction and MCP bundle"
author = "Example Author"

[skills]
path = "skills"

[mcp_servers.local]
command = "node"
args = ["server.js"]
cwd = "mcp"

[mcp_servers.remote]
url = "https://example.invalid/mcp"

[when]
os = ["macos", "linux", "windows"]
binaries = ["node"]
```

Component paths must be relative, contained, present, and free of symbolic
links. Remote MCP URLs must be HTTP(S) and cannot embed credentials. Prefer
`env`, `env_headers`, or `bearer_token_env_var` names over literal secrets.

`[skills]` and `[mcp_servers.*]` are the only active component adapters in
v0.9.1. The manifest can inventory the following future surfaces, but a bundle
declaring any of them cannot be enabled yet:

```toml
[commands]
path = "commands"

[agents]
path = "agents"

[hooks]
path = "hooks"

[lsp]
path = "lsp"

[native]
path = "native"

[capabilities]
filesystem_roots = ["workspace"]
network_hosts = ["api.example.invalid"]
lifecycle_mutation = true
```

Remote MCP endpoint hosts are added to the displayed network inventory
automatically. A successful environment or health check is never treated as
trust.

## Review, trust, and enablement

Use the in-session command surface:

```text
/plugin list
/plugin validate example
/plugin show example
/plugin enable example
```

The first `enable` opens a review showing source, component inventory,
requested permissions, sanitized MCP endpoints, full content and capability
hashes, and inactive declarations. It also prints an exact confirmation:

```text
/plugin trust example <content-prefix>.<capability-prefix>
```

Run that exact command only after reviewing the bundle, then run `/plugin
enable example` again. Trust and enablement are separate:

- `/plugin disable example` stops contribution while preserving trust.
- `/plugin revoke example` removes trust while preserving the enablement bit;
  the bundle remains inactive until reviewed again.
- `/plugin reload` rebuilds the read-only registry snapshot. Restart the
  session after a lifecycle change so the model prompt and MCP pool are both
  rebuilt from the same snapshot.

Trust receipts live in `~/.codewhale/plugins/state.json`. Atomic owner-only
writes record the full content hash, capability hash, reviewed capability
inventory, and review time, with the latest 32 reviews retained as a bounded
audit trail. Malformed or unsupported state is not overwritten: all bundles
fail closed until the state file is repaired or moved.

The content hash covers the manifest, complete bundle tree, and relevant file
permission metadata in deterministic path order, including local MCP
entrypoints and companion assets. The capability hash covers the normalized
component and permission inventory. A content edit or capability change
invalidates the receipt deterministically; an already-enabled bundle becomes
inactive until it is reviewed again.

## Runtime behavior

An active bundle must be enabled, trusted for its current hashes, applicable to
the host, free of validation errors, and limited to supported component kinds.

- Skills are snapshotted during registry initialization and exposed only as
  `<plugin>:<skill>`. The model-facing catalogue and `load_skill` use that
  reviewed in-memory snapshot rather than a mutable disk path; plugin companion
  files are not exposed in v0.9.1. Skills disappear when the bundle is
  inactive. `/skills inspect` remains the explicit human provenance view.
- MCP server names are exposed as `<plugin>-<server>`. Disabled or untrusted
  bundles are denied again at the headless MCP adapter.
- Plain launch, resume, fork, exec, and serve initialize the registry before
  constructing Skills or MCP configuration.
- Constitution, repository instructions, permission rules, sandbox policy,
  and MCP tool approval continue to outrank plugin instructions.

`/plugin list`, `show`, and `validate` perform no network requests, process
launches, credential reads, or configuration writes. Legacy executable tools
under `[tools].plugin_dir` remain a distinct system and are listed under
`/plugin tools`.

## Explicit non-goals for v0.9.1

There is no remote marketplace, install/update command, ambient compatibility
discovery, automatic trust, hook adapter, command adapter, agent adapter, LSP
adapter, native extension runtime, or migration of another CLI's bundle. These
remain later work rather than implied capabilities.
