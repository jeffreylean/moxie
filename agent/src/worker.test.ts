import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { EventEmitter, once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import type { Event } from "./bridge.js";

test("the Pi SDK worker streams, retains context, switches models, and cancels", { timeout: 30_000 }, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "moxie-pi-"));
  const requests: Record<string, unknown>[] = [];
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk;
    const parsed = JSON.parse(body);
    requests.push(parsed);
    const lastMessage = JSON.stringify(parsed.messages.at(-1));
    if (lastMessage.includes("simulate provider failure")) {
      response.writeHead(401, { "Content-Type": "application/json" });
      response.end(JSON.stringify({ error: { message: "Fixture authentication failed", type: "authentication_error" } }));
      return;
    }
    response.writeHead(200, { "Content-Type": "text/event-stream" });
    const chunk = (delta: object, finish_reason: string | null = null) => response.write(
      `data: ${JSON.stringify({ id: "fixture", object: "chat.completion.chunk", created: 1, model: parsed.model,
        choices: [{ index: 0, delta, finish_reason }] })}\n\n`,
    );
    chunk({ role: "assistant", content: "Hello " });
    if (lastMessage.includes("wait for cancellation")) return;
    chunk({ content: "from Pi 🌟" });
    chunk({}, "stop");
    response.end("data: [DONE]\n\n");
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address !== "string");
  await writeFile(join(directory, "models.json"), JSON.stringify({
    providers: {
      "moxie-test": {
        baseUrl: `http://127.0.0.1:${address.port}/v1`,
        api: "openai-completions",
        apiKey: "test-key",
        models: ["fixture", "second"].map((id) => ({
          id, name: id, reasoning: false, input: ["text"],
          contextWindow: 128000, maxTokens: 2048,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
        })),
      },
    },
  }));
  await writeFile(join(directory, "settings.json"), JSON.stringify({
    defaultProvider: "moxie-test", defaultModel: "fixture",
  }));

  const child = spawn(process.execPath, [fileURLToPath(new URL("./index.js", import.meta.url))], {
    env: { ...process.env, PI_CODING_AGENT_DIR: directory },
    stdio: ["pipe", "pipe", "pipe"],
  });
  t.after(async () => {
    child.kill();
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(directory, { recursive: true, force: true });
  });
  let diagnostics = "";
  child.stderr.on("data", (chunk) => { diagnostics += chunk; });
  const events: Event[] = [];
  const updates = new EventEmitter();
  let output = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    output += chunk;
    let end: number;
    while ((end = output.indexOf("\n")) !== -1) {
      events.push(JSON.parse(output.slice(0, end)));
      output = output.slice(end + 1);
      updates.emit("event");
    }
  });
  async function waitFor(predicate: (event: Event) => boolean): Promise<Event> {
    const timeout = AbortSignal.timeout(15_000);
    try {
      for (;;) {
        const event = events.find(predicate);
        if (event) return event;
        await once(updates, "event", { signal: timeout });
      }
    } catch {
      throw new Error(`Worker timed out. Events: ${JSON.stringify(events)}. Stderr: ${diagnostics}`);
    }
  }
  const send = (command: object) => child.stdin.write(`${JSON.stringify(command)}\n`);

  send({ type: "initialize", protocol: 999, cwd: directory });
  await waitFor((event) => event.type === "error");
  send({ type: "initialize", protocol: 1, cwd: directory });
  const ready = await waitFor((event) => event.type === "ready");
  assert(ready.type === "ready");
  assert.equal(ready.selected?.id, "fixture");
  assert(ready.models.some((model) => model.provider === "moxie-test"));

  const record = Buffer.from(`${JSON.stringify({ type: "prompt", id: 1, text: "Hello 🌟\u2028Pi" })}\n`);
  const split = record.indexOf(Buffer.from("🌟")) + 1;
  child.stdin.write(record.subarray(0, split));
  child.stdin.write(record.subarray(split));
  await waitFor((event) => event.type === "finished" && event.id === 1);
  assert(events.some((event) => event.type === "text" && event.id === 1 && event.delta.includes("Hello")));
  assert(events.some((event) => event.type === "assistant_end" && event.id === 1 && event.text === "Hello from Pi 🌟"));
  assert.equal(requests.length, 1);
  assert(!requests[0]?.tools || (requests[0].tools as unknown[]).length === 0);

  send({ type: "set_model", provider: "moxie-test", model: "second" });
  await waitFor((event) => event.type === "model_changed" && event.model.id === "second");
  send({ type: "prompt", id: 2, text: "Remember our first exchange." });
  await waitFor((event) => event.type === "finished" && event.id === 2);
  assert.equal(requests[1]?.model, "second");
  assert(JSON.stringify(requests[1]?.messages).includes("Hello from Pi"));

  send({ type: "prompt", id: 3, text: "wait for cancellation" });
  await waitFor((event) => event.type === "text" && event.id === 3);
  send({ type: "cancel", id: 2 });
  send({ type: "cancel", id: 3 });
  const cancelled = await waitFor((event) => event.type === "finished" && event.id === 3);
  assert(cancelled.type === "finished" && cancelled.cancelled);

  send({ type: "prompt", id: 4, text: "Recover after stopping." });
  await waitFor((event) => event.type === "finished" && event.id === 4);
  assert(events.some((event) => event.type === "assistant_end" && event.id === 4 && event.text === "Hello from Pi 🌟"));

  send({ type: "prompt", id: 5, text: "simulate provider failure" });
  await waitFor((event) => event.type === "finished" && event.id === 5);
  assert(events.some((event) => event.type === "error" && event.id === 5));

  send({ type: "prompt", id: 6, text: "  " });
  await waitFor((event) => event.type === "finished" && event.id === 6);
  assert.equal(requests.length, 5);
  assert(events.some((event) => event.type === "error" && event.id === 6));

  child.stdin.write("not-json\n");
  await waitFor((event) => event.type === "error" && event.message === "Invalid JSON command.");

  const exit = once(child, "exit");
  child.stdin.end();
  const [code] = await exit;
  assert.equal(code, 0, diagnostics);
});
