import { Console } from "node:console";
import { Bridge, MAX_RECORD_BYTES } from "./bridge.js";
import { openSession } from "./session.js";

// Stdout is the desktop protocol; SDK diagnostics belong on stderr.
globalThis.console = new Console(process.stderr, process.stderr);

const bridge = new Bridge(openSession, (event) => {
  process.stdout.write(`${JSON.stringify(event)}\n`);
});

let closing = false;
async function close(): Promise<void> {
  if (closing) return;
  closing = true;
  await bridge.close();
}
process.once("SIGTERM", () => { void close().finally(() => process.exit(0)); });
process.once("SIGINT", () => { void close().finally(() => process.exit(0)); });

try {
  process.stdin.setEncoding("utf8");
  let pending = "";
  for await (const chunk of process.stdin) {
    pending += chunk;
    let end: number;
    while ((end = pending.indexOf("\n")) !== -1) {
      const record = pending.slice(0, end);
      pending = pending.slice(end + 1);
      if (Buffer.byteLength(record) > MAX_RECORD_BYTES) throw new Error("Agent command is too large.");
      if (!record.trim()) continue;
      try {
        await bridge.dispatch(JSON.parse(record));
      } catch {
        process.stdout.write(`${JSON.stringify({ type: "error", id: null, message: "Invalid JSON command." })}\n`);
      }
    }
    if (Buffer.byteLength(pending) > MAX_RECORD_BYTES) throw new Error("Agent command is too large.");
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
} finally {
  await close();
}
