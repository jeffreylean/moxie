# Pi agent worker

Moxie's Agent tab uses the Pi Coding Agent SDK in a local Node.js
process. Bevy owns the tab and conversation display. The worker owns
the Pi session and provider requests.

## Development

Install Node.js 22.19 or newer, then run from the repository root:

```sh
npm ci --prefix agent
npm run build --prefix agent
cargo run -p moxie
```

Open **Agent** beside **Inspector**. The worker starts on first use.
Choose a configured model, type a message, and press Enter or Send.
Stop cancels the current reply. New chat starts a fresh Pi session.
Switching or closing tabs preserves the conversation and active run.
Closing Moxie stops the worker.

The worker uses Pi's credentials and custom models from
`~/.pi/agent`, or the directory selected by `PI_CODING_AGENT_DIR`.
Provider API key environment variables are also supported. To set up
credentials with the installed Pi CLI:

```sh
cd agent
npx --no-install pi
```

Use Pi's `/login` and `/model` commands, then open a new chat in
Moxie. See Pi's [provider authentication documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md).
Provider keys must be in the environment that launches Moxie when
using environment variables.

This first integration provides text chat. File and shell tools,
extensions, skills, prompt templates, and project context files are
disabled. The agent cannot inspect or edit the live Moxie scene.
Conversations are held in memory until New chat or application exit.
Model choices in Moxie do not change Pi's saved defaults.

The dependency is pinned to `@earendil-works/pi-coding-agent` 0.99.1.
Pi previously used the `@mariozechner` package scope.

## Distribution

The launcher checks `MOXIE_PI_WORKER`, then
`agent/dist/index.js` beside the Moxie executable, then the source
checkout's `agent/dist/index.js` for development.

Place these files beside the executable when packaging:

```text
moxie[.exe]
agent/
  package.json
  dist/
  node_modules/
  node[.exe]
```

Install production dependencies for each target platform with
`npm ci --omit=dev` after compiling the worker. Preserve dependency
assets, including WASM and native modules. Include a matching Node.js
runtime at `agent/node` or `agent/node.exe` so users do not need a
separate installation. This repository does not include those runtime
binaries or a desktop installer.

`MOXIE_NODE` overrides the runtime executable. Otherwise Moxie uses
the adjacent runtime, falling back to `node` on PATH.

## Protocol

Protocol version 1 uses UTF-8 JSON records separated by LF over
stdin/stdout. Diagnostics go to stderr. Initialize with an absolute
project folder before sending commands. The folder comes from the
open project's parent directory or Moxie's current directory.

Commands are `initialize`, `prompt`, `cancel`, and `set_model`.
Prompt and cancellation commands carry a request ID. Events are
`ready`, `model_changed`, `assistant_start`, `text`, `assistant_end`,
`finished`, and `error`. `assistant_end` supplies the final text for
one assistant message; `finished` completes the prompt. Errors can
precede `finished`. Only one prompt runs at a time.

The Rust reader and writer run off the UI thread. Bounded queues
carry messages between them and Bevy. The app ignores stale turn
events, reports worker failures, and ends a connection attempt after
30 seconds. The process lifetime belongs to the app, independently
of dock window entities.

## Checks

```sh
npm test --prefix agent
cargo test -p moxie agent::tests
```

The worker test runs the real Pi SDK against a local mock provider.
It checks streaming, conversation history, model selection,
cancellation, UTF-8 framing, and protocol errors without paid API
calls. Rust tests cover turn isolation and tab/composer state.
