import {
  type AgentSession,
  createAgentSession,
  DefaultResourceLoader,
  getAgentDir,
  ModelRuntime,
  SessionManager,
  SettingsManager,
} from "@earendil-works/pi-coding-agent";

export interface ModelChoice {
  provider: string;
  id: string;
  name: string;
}

export interface Session extends Pick<AgentSession, "prompt" | "subscribe" | "abort" | "dispose"> {
  models(): ModelChoice[];
  selected(): ModelChoice | undefined;
  select(provider: string, model: string): Promise<ModelChoice>;
}

export async function openSession(cwd: string): Promise<Session> {
  const agentDir = getAgentDir();
  const saved = SettingsManager.create(cwd, agentDir);
  const settingsManager = SettingsManager.inMemory({
    defaultProvider: saved.getDefaultProvider(),
    defaultModel: saved.getDefaultModel(),
  });
  const modelRuntime = await ModelRuntime.create();
  const resourceLoader = new DefaultResourceLoader({
    cwd,
    agentDir,
    settingsManager,
    noExtensions: true,
    noSkills: true,
    noPromptTemplates: true,
    noThemes: true,
    noContextFiles: true,
    systemPrompt:
      "You are Moxie's assistant. Help users understand animation and plan their work. " +
      "This chat has no access to the live scene, files, or editor tools. " +
      "Describe proposed changes without claiming to have inspected or edited the project.",
  });
  await resourceLoader.reload();
  const { session } = await createAgentSession({
    cwd,
    modelRuntime,
    settingsManager,
    resourceLoader,
    sessionManager: SessionManager.inMemory(cwd),
    noTools: "all",
    tools: [],
  });
  const choice = ({ provider, id, name }: ModelChoice): ModelChoice => ({ provider, id, name });
  return {
    prompt: session.prompt.bind(session),
    subscribe: session.subscribe.bind(session),
    abort: session.abort.bind(session),
    dispose: session.dispose.bind(session),
    models: () => modelRuntime.getAvailableSnapshot().map(choice),
    selected: () => {
      const model = session.model;
      return model && modelRuntime.getAvailableSnapshot().some(
        (available) => available.provider === model.provider && available.id === model.id,
      ) ? choice(model) : undefined;
    },
    async select(provider, id) {
      const model = modelRuntime.getAvailableSnapshot().find(
        (candidate) => candidate.provider === provider && candidate.id === id,
      );
      if (!model) throw new Error("This model has no configured credentials.");
      await session.setModel(model);
      return choice(model);
    },
  };
}
