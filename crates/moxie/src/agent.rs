mod worker;

use bevy::prelude::*;
use moxie_ui::reactive::FynixSet;
use moxie_ui::widgets::dock::{DockNode, DockTree};

use crate::ProjectPath;
use worker::{Command, Event, ModelChoice, PROTOCOL, Worker};

pub(crate) struct AgentPlugin;

impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AgentState>()
            .add_systems(Update, tick.before(FynixSet));
    }
}

pub(crate) struct ChatMessage {
    pub role: &'static str,
    pub text: String,
    pub revision: u64,
}

#[derive(Resource, Default)]
pub(crate) struct AgentState {
    worker: Option<Worker>,
    attempted: bool,
    next_id: u64,
    reply: Option<usize>,
    pub active: Option<u64>,
    pub connected: bool,
    pub connecting: bool,
    pub cancelling: bool,
    pub models: Vec<ModelChoice>,
    pub selected: Option<ModelChoice>,
    pub messages: Vec<ChatMessage>,
    pub draft: String,
    pub error: Option<String>,
    pub stopped: bool,
    pub transcript_revision: u64,
    pub composer_revision: u64,
}

impl AgentState {
    pub fn can_send(&self) -> bool {
        self.connected
            && self.selected.is_some()
            && self.active.is_none()
            && !self.draft.trim().is_empty()
    }

    pub fn status(&self) -> &str {
        if let Some(error) = &self.error {
            error
        } else if self.connecting {
            "Connecting to Pi..."
        } else if self.cancelling {
            "Stopping..."
        } else if self.active.is_some() {
            "Pi is responding..."
        } else if self.connected && self.models.is_empty() {
            "No configured models. Set up Pi credentials or provider API keys, then start a new chat."
        } else if self.stopped {
            "Reply stopped."
        } else if self.connected {
            "Ready"
        } else {
            "Connect to Pi to start chatting."
        }
    }

    fn push(&mut self, role: &'static str, text: String) -> usize {
        self.messages.push(ChatMessage {
            role,
            text,
            revision: 0,
        });
        self.transcript_revision += 1;
        self.messages.len() - 1
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Ready {
                protocol,
                models,
                selected,
            } => {
                if protocol != PROTOCOL {
                    self.fail("Pi protocol version mismatch. Rebuild agent/.".into());
                    return;
                }
                self.connecting = false;
                self.connected = true;
                self.models = models;
                self.selected = selected;
            }
            Event::ModelChanged { model } => {
                self.selected = Some(model)
            }
            Event::AssistantStart { id }
                if self.active == Some(id) =>
            {
                self.reply = Some(self.push("Pi", String::new()));
            }
            Event::Text { id, delta } if self.active == Some(id) => {
                let index = match self.reply {
                    Some(index) => index,
                    None => {
                        let index = self.push("Pi", String::new());
                        self.reply = Some(index);
                        index
                    }
                };
                self.messages[index].text.push_str(&delta);
                self.messages[index].revision += 1;
            }
            Event::AssistantEnd { id, text }
                if self.active == Some(id) =>
            {
                if let Some(index) = self.reply.take() {
                    self.messages[index].text = text;
                    self.messages[index].revision += 1;
                } else if !text.is_empty() {
                    self.push("Pi", text);
                }
            }
            Event::Finished { id, cancelled }
                if self.active == Some(id) =>
            {
                self.active = None;
                self.reply = None;
                self.cancelling = false;
                self.stopped = cancelled;
            }
            Event::Error { id, message }
                if id.is_none() || id == self.active =>
            {
                if self.connecting {
                    self.fail(message);
                } else {
                    self.error = Some(message);
                }
            }
            _ => {}
        }
    }

    fn fail(&mut self, error: String) {
        self.worker = None;
        self.connected = false;
        self.connecting = false;
        self.cancelling = false;
        self.active = None;
        self.reply = None;
        self.error = Some(error);
    }
}

fn tick(world: &mut World) {
    let visible = world.get_resource::<DockTree>().is_some_and(|tree| {
        tree.iter_dfs().into_iter().any(|(id, _)| {
            matches!(tree.get(id), Some(DockNode::Leaf(leaf)) if leaf.windows.iter().any(|window| window.window_id == "agent" && Some(window.id) == leaf.active))
        })
    });
    if visible && !world.resource::<AgentState>().attempted {
        connect(world);
    }
    let events = world
        .resource::<AgentState>()
        .worker
        .as_ref()
        .map(|worker| {
            worker.events.try_iter().take(512).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let timed_out = {
        let state = world.resource::<AgentState>();
        state.connecting
            && state
                .worker
                .as_ref()
                .is_some_and(Worker::startup_timed_out)
    };
    if events.is_empty() && !timed_out {
        return;
    }
    let mut state = world.resource_mut::<AgentState>();
    for event in events {
        match event {
            Ok(event) => state.apply(event),
            Err(error) => {
                state.fail(error);
                break;
            }
        }
    }
    if state.connecting && timed_out {
        state.fail(
            "Pi took too long to connect. Start a new chat to retry."
                .into(),
        );
    }
}

pub(crate) fn connect(world: &mut World) {
    let cwd = world
        .resource::<ProjectPath>()
        .0
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .and_then(std::fs::canonicalize);
    let mut state = world.resource_mut::<AgentState>();
    if state.worker.is_some() {
        return;
    }
    state.attempted = true;
    state.error = None;
    match cwd
        .map_err(|error| error.to_string())
        .and_then(|cwd| Worker::spawn(&cwd))
    {
        Ok(worker) => {
            state.worker = Some(worker);
            state.connecting = true;
        }
        Err(error) => state.fail(error),
    }
}

pub(crate) fn send(world: &mut World) {
    let mut state = world.resource_mut::<AgentState>();
    if !state.can_send() {
        return;
    }
    let text = state.draft.trim().to_string();
    state.next_id += 1;
    let id = state.next_id;
    let result =
        state.worker.as_ref().expect("connected worker").send(
            Command::Prompt {
                id,
                text: text.clone(),
            },
        );
    match result {
        Ok(()) => {
            state.push("You", text);
            state.draft.clear();
            state.composer_revision += 1;
            state.active = Some(id);
            state.error = None;
            state.stopped = false;
        }
        Err(error) => state.fail(error),
    }
}

pub(crate) fn cancel(world: &mut World) {
    let mut state = world.resource_mut::<AgentState>();
    if let Some(id) = state.active
        && let Some(worker) = &state.worker
    {
        match worker.send(Command::Cancel { id }) {
            Ok(()) => state.cancelling = true,
            Err(error) => state.fail(error),
        }
    }
}

pub(crate) fn new_chat(world: &mut World) {
    let mut state = world.resource_mut::<AgentState>();
    if state.active.is_some() {
        return;
    }
    let revision = state.transcript_revision + 1;
    let composer_revision = state.composer_revision + 1;
    *state = AgentState {
        transcript_revision: revision,
        composer_revision,
        ..default()
    };
}

pub(crate) fn select_model(
    world: &mut World,
    provider: &str,
    model: &str,
) {
    let mut state = world.resource_mut::<AgentState>();
    if state.active.is_some() {
        return;
    }
    if let Some(worker) = &state.worker {
        match worker.send(Command::SetModel {
            provider: provider.into(),
            model: model.into(),
        }) {
            Ok(()) => state.error = None,
            Err(error) => state.fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::harness::{Editor, SETTLE};
    use bevy::input_focus::InputFocus;
    use bevy::text::EditableText;
    use moxie_ui::elements::TextField;

    #[test]
    fn stale_events_cannot_finish_or_overwrite_another_turn() {
        let mut state = AgentState {
            active: Some(2),
            ..default()
        };
        state.apply(Event::Text {
            id: 1,
            delta: "old".into(),
        });
        state.apply(Event::Finished {
            id: 1,
            cancelled: false,
        });
        assert_eq!(state.active, Some(2));
        assert!(state.messages.is_empty());
        state.apply(Event::AssistantStart { id: 2 });
        state.apply(Event::Text {
            id: 2,
            delta: "partial".into(),
        });
        state.apply(Event::AssistantEnd {
            id: 2,
            text: "complete".into(),
        });
        state.apply(Event::Finished {
            id: 2,
            cancelled: false,
        });
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "complete");
        assert_eq!(state.active, None);
    }

    #[test]
    fn cancellation_keeps_the_partial_reply_and_releases_the_composer()
     {
        let mut state = AgentState {
            active: Some(1),
            cancelling: true,
            ..default()
        };
        state.apply(Event::Text {
            id: 1,
            delta: "So far".into(),
        });
        state.apply(Event::Finished {
            id: 1,
            cancelled: true,
        });
        assert_eq!(state.messages[0].text, "So far");
        assert!(state.stopped);
        assert!(!state.cancelling);
        assert_eq!(state.active, None);
    }

    #[test]
    fn incompatible_worker_cannot_enable_chat() {
        let mut state = AgentState {
            connecting: true,
            ..default()
        };
        state.apply(Event::Ready {
            protocol: PROTOCOL + 1,
            models: Vec::new(),
            selected: None,
        });
        assert!(!state.connected);
        assert!(!state.connecting);
        assert!(state.error.is_some());
    }

    #[test]
    fn streaming_and_tab_switches_preserve_the_composer_and_conversation()
     {
        let mut editor = Editor::new();
        assert!(!editor.world().resource::<AgentState>().attempted);
        {
            let mut state =
                editor.world().resource_mut::<AgentState>();
            state.attempted = true;
            state.active = Some(1);
        }
        let (leaf, agent_tab, inspector_tab) = {
            let tree = editor.world().resource::<DockTree>();
            let leaf = tree.find_leaf_with_window("agent").unwrap();
            let DockNode::Leaf(area) = tree.get(leaf).unwrap() else {
                panic!("agent leaf")
            };
            let tab = area
                .windows
                .iter()
                .find(|tab| tab.window_id == "agent")
                .unwrap()
                .id;
            (leaf, tab, area.active.unwrap())
        };
        editor
            .world()
            .resource_mut::<DockTree>()
            .set_active(leaf, agent_tab);
        editor.step(SETTLE);
        let composer = editor.named("Agent message");
        let input =
            TextField::text_input(editor.world(), composer).unwrap();
        editor
            .world()
            .insert_resource(InputFocus::from_entity(input));
        editor
            .world()
            .get_mut::<EditableText>(input)
            .unwrap()
            .editor_mut()
            .set_text("Next question");
        editor.step(SETTLE);
        {
            let mut state =
                editor.world().resource_mut::<AgentState>();
            state.apply(Event::AssistantStart { id: 1 });
            state.apply(Event::Text {
                id: 1,
                delta: "An answer".into(),
            });
        }
        editor.step(SETTLE);
        assert_eq!(editor.named("Agent message"), composer);
        assert_eq!(
            editor.world().resource::<InputFocus>().get(),
            Some(input)
        );
        assert_eq!(
            editor
                .world()
                .get::<EditableText>(input)
                .unwrap()
                .value()
                .to_string(),
            "Next question"
        );
        editor.text("An answer");
        editor
            .world()
            .resource_mut::<DockTree>()
            .set_active(leaf, inspector_tab);
        editor.step(SETTLE);
        editor
            .world()
            .resource_mut::<DockTree>()
            .set_active(leaf, agent_tab);
        editor.step(SETTLE);
        assert_eq!(
            editor.world().resource::<AgentState>().messages[0].text,
            "An answer"
        );
        assert_eq!(editor.named("Agent message"), composer);
    }
}
