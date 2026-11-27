# MCP server: design proposal

**Status: proposal. Nothing described here exists yet.** This page records what an MCP server for
skyship would do, what it would refuse to do, and how it would be kept safe, so the decision can be
reviewed before any code is written.

## Why

skyship is used by a person, through the desktop app or the terminal. An
[MCP](https://modelcontextprotocol.io) server would let an AI assistant (for example Claude Code)
look at the apps in the account and, later, act on them, reusing `skyship-core`. That also widens
the risk: an agent could create or remove cloud resources. The design below starts with what is
safe and treats each step further as a separate decision.

## Use cases and what already covers them

| Use case | Today |
|---|---|
| "List my apps", "what is the status of app X" | `skyship status` and the apps list |
| "Which permissions does skyship need" | `skyship policy` and the docs |
| "Publish this folder" | `skyship deploy` |
| "Remove app X" | `skyship destroy` |

An MCP server is only worth it where an assistant gains from calling it directly. Reading is the
clear case. Publishing is plausible. Removing is the one this page argues against.

## Where it lives

- A new crate `crates/mcp` (binary `skyship-mcp`) that calls only `skyship-core`, like the CLI and
  the app. The core keeps printing nothing and asking nothing.
- Transport: **stdio** only. No network listener.
- SDK: [`rmcp`](https://crates.io/crates/rmcp), the official Rust SDK (Apache-2.0), with only the
  server, macros and stdio features. Its dependency weight and `cargo audit` result have to be
  measured before it is adopted.

## Three levels, kept apart

The server never exposes more than the level the token allows, and each level is a separate
decision.

| Level | What it can do | Planned |
|---|---|---|
| **read** | list apps, status, list regions, print the permissions policy | first version |
| **write** | publish or update an app | later, own review |
| **delete** | remove an app | **not planned**: see below |

### Deleting is kept out of MCP

Removing an app is irreversible and, for EC2, also removes the security group, the ECR repository
and the role. A prompt-injected assistant with a delete tool is the worst case, and a human can
already do it in one click. The proposal is that **no MCP tool removes anything**: no `destroy`, no
flag that turns it on, no token scope for it. If it is ever added, it is a separate change that
reopens this page, with its own confirmation design.

### Write needs a human in the loop

The MCP specification says a human should be able to deny tool calls, and tool annotations such as
`readOnlyHint` and `destructiveHint` are
[hints, not enforcement](https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/).
So the write level would not rely on the client to ask. The server itself would split it in two
calls: a `plan` call that returns what would be created or replaced, and an `apply` call that only
runs with the identifier of that plan, and only if the setting "allow publishing through MCP" in
the app is on (off by default, like the other permissions).

## Credentials never go through MCP

- The server uses a profile **already saved** in `~/.aws/credentials`, chosen when the server is
  started. There is no tool that takes access keys, and "use only now" keys of the app are not
  shared with it.
- MCP elicitation must not be used to ask for sensitive data
  ([spec](https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation)), and this
  design does not need it.

## Access tokens

The person creates a skyship token, gives it to the assistant (as an environment variable in the
server's configuration), and the server validates it on every call.

- **Opaque and random**, not a JWT: there is no remote service to verify a signature.
- Stored **only as a hash**, in `<config>/mcp-tokens.json` with permission `600`, together with a
  name, a scope (`read`, later `write`) and an expiry. Shown **once**, when created.
- Created, listed and revoked from the app and from the CLI (`skyship mcp token ...`).
- Each call checks: the token exists, has not expired, and its scope covers the tool.
- The token is **not** an AWS key. Revoking it does not touch the AWS keys.

What the token does **not** do: the server runs as your user, so a program that can read the
assistant's environment can read the token, and one that already runs as you can read
`~/.aws/credentials` too. The token limits what the *assistant* can do through MCP. It does not
protect against malware on the computer, and a write token in the hands of a prompt-injected
assistant can still publish within its scope, which is why write keeps the plan and apply step.

## First version

- `crates/mcp` with the read tools only: `list_apps`, `app_status`, `list_regions`,
  `permissions_policy`.
- Token creation, listing and revocation (`read` scope).
- Tests: the tools against the core with a fake backend, token hashing and expiry, and a real MCP
  client talking to the binary over stdio.
- `docs/security.md` and the README describe it once it exists.

## Open questions

- Is the dependency weight of `rmcp` acceptable for the binary and for `cargo audit`?
- Should the server also be a subcommand of the existing CLI (`skyship mcp`) instead of a second
  binary, to keep a single download?
- Whether publishing is worth exposing at all, given the plan and apply step it needs.
