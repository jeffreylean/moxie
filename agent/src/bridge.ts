import { isAbsolute } from "node:path";
import type { ModelChoice, Session } from "./session.js";

export const PROTOCOL_VERSION = 1;
export const MAX_RECORD_BYTES = 1024 * 1024;

export type Event =
  | { type: "ready"; protocol: number; models: ModelChoice[]; selected: ModelChoice | null }
  | { type: "model_changed"; model: ModelChoice }
  | { type: "assistant_start"; id: number }
  | { type: "text"; id: number; delta: string }
  | { type: "assistant_end"; id: number; text: string }
  | { type: "finished"; id: number; cancelled: boolean }
  | { type: "error"; id: number | null; message: string };

export class Bridge {
  private session?: Session;
  private unsubscribe?: () => void;
  private active?: number;
  private running?: Promise<void>;
  private cancelled = false;
  private lastError?: string;

  constructor(
    private readonly create: (cwd: string) => Promise<Session>,
    private readonly emit: (event: Event) => void,
  ) {}

  async dispatch(input: unknown): Promise<void> {
    let id: number | null = null;
    try {
      if (!input || typeof input !== "object" || Array.isArray(input)) {
        throw new Error("Expected a command object.");
      }
      const command = input as Record<string, unknown>;
      if (typeof command.id === "number" && Number.isSafeInteger(command.id)) id = command.id;
      switch (command.type) {
        case "initialize": {
          if (this.session) throw new Error("The worker is already initialized.");
          if (command.protocol !== PROTOCOL_VERSION) throw new Error("Agent protocol version mismatch.");
          if (typeof command.cwd !== "string" || !isAbsolute(command.cwd)) {
            throw new Error("The project folder must be an absolute path.");
          }
          this.session = await this.create(command.cwd);
          this.unsubscribe = this.session.subscribe((event) => {
            const id = this.active;
            if (id === undefined) return;
            if (event.type === "message_start" && event.message.role === "assistant") {
              this.emit({ type: "assistant_start", id });
            } else if (event.type === "message_update" && event.assistantMessageEvent.type === "text_delta") {
              this.emit({ type: "text", id, delta: event.assistantMessageEvent.delta });
            } else if (event.type === "message_end" && event.message.role === "assistant") {
              const text = event.message.content
                .filter((part) => part.type === "text")
                .map((part) => part.text)
                .join("");
              this.emit({ type: "assistant_end", id, text });
              this.lastError = event.message.stopReason === "error"
                ? event.message.errorMessage ?? "The provider returned an error."
                : undefined;
            }
          });
          this.emit({
            type: "ready",
            protocol: PROTOCOL_VERSION,
            models: this.session.models(),
            selected: this.session.selected() ?? null,
          });
          break;
        }
        case "prompt": {
          const session = this.requireSession();
          if (this.active !== undefined) throw new Error("A reply is already in progress.");
          if (id === null || id <= 0) throw new Error("A prompt needs a positive request ID.");
          if (typeof command.text !== "string" || !command.text.trim()) throw new Error("Enter a message first.");
          if (Buffer.byteLength(command.text) > MAX_RECORD_BYTES / 2) throw new Error("The message is too long.");
          if (!session.selected()) throw new Error("Configure Pi credentials and choose a model first.");
          this.active = id;
          this.cancelled = false;
          this.lastError = undefined;
          this.running = this.run(id, command.text);
          break;
        }
        case "cancel":
          if (id !== null && id === this.active) {
            this.cancelled = true;
            await this.requireSession().abort();
          }
          break;
        case "set_model": {
          if (this.active !== undefined) throw new Error("Stop the current reply before changing models.");
          if (typeof command.provider !== "string" || typeof command.model !== "string") {
            throw new Error("Select a provider and model.");
          }
          const model = await this.requireSession().select(command.provider, command.model);
          this.emit({ type: "model_changed", model });
          break;
        }
        default:
          throw new Error("Unknown agent command.");
      }
    } catch (error) {
      this.emit({ type: "error", id, message: error instanceof Error ? error.message : String(error) });
      if (id !== null && this.active !== id) this.emit({ type: "finished", id, cancelled: false });
    }
  }

  private requireSession(): Session {
    if (!this.session) throw new Error("Connect to the agent first.");
    return this.session;
  }

  private async run(id: number, text: string): Promise<void> {
    try {
      await this.requireSession().prompt(text);
    } catch (error) {
      this.lastError = error instanceof Error ? error.message : String(error);
    } finally {
      if (this.lastError && !this.cancelled) this.emit({ type: "error", id, message: this.lastError });
      this.emit({ type: "finished", id, cancelled: this.cancelled });
      this.active = undefined;
    }
  }

  async close(): Promise<void> {
    this.cancelled = true;
    await this.session?.abort();
    await this.running;
    this.unsubscribe?.();
    this.session?.dispose();
  }
}
