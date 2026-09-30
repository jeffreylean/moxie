use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Activate;
use bevy_fynix::WorldEntityMut as _;
use fynix::composer::Composer;
use fynix::prelude::*;
use moxie_ui::elements::{
    Button, Dropdown, DropdownList, DropdownMenu, Frame, Label,
    LabelCursor, Panel, ScrollArea, TextField, TextFieldCursor,
    menu_item,
};
use moxie_ui::reactive::{
    BevyUi, FynixHost, resource_changed, structure_changed,
    value_changed,
};

use crate::agent::{self, AgentState};

pub(super) struct AgentPanel;

impl Composer<FynixHost> for AgentPanel {
    type Element = Panel;

    fn compose(
        self,
        ui: &mut BevyUi,
    ) -> ElementHandle<FynixHost, Panel> {
        let pad = ui.theme.space.lg;
        ui.elem(elem!(
            Panel,
            direction = FlexDirection::Column,
            padding = UiRect::all(px(pad)),
            row_gap = px(8)
        ))
        .with(|ui| {
            ui.elem(elem!(Label, text = "Pi", bold = true));
            ui.elem(elem!(Frame, direction = FlexDirection::Column))
                .watch(
                    structure_changed::<AgentState, _>(|state| {
                        (
                            state.models.clone(),
                            state.selected.clone(),
                            state.active.is_some(),
                            state.connected,
                        )
                    }),
                    model_picker,
                );

            ui.elem(elem!(
                ScrollArea,
                width = percent(100),
                flex_grow = 1.0f32,
                scroll_x = false,
                row_gap = px(12)
            ))
            .insert((
                ScrollPosition::default(),
                ChatScroll::default(),
            ))
            .watch(
                structure_changed::<AgentState, _>(|state| {
                    state.transcript_revision
                }),
                transcript,
            );

            let muted = ui.theme.color.text_dim;
            ui.elem(elem!(Label, color = muted, size = 12.0f32))
                .bind(
                    |label| label.text(),
                    resource_changed::<AgentState>(),
                    |WorldNodeRef { world, .. }| {
                        world
                            .resource::<AgentState>()
                            .status()
                            .to_string()
                    },
                );
            composer(ui);
            ui.elem(elem!(
                Frame,
                direction = FlexDirection::Row,
                column_gap = px(8)
            ))
            .watch(
                structure_changed::<AgentState, _>(|state| {
                    (
                        state.can_send(),
                        state.active.is_some(),
                        state.connected,
                        state.connecting,
                        state.messages.is_empty(),
                    )
                }),
                controls,
            );
        })
        .handle()
    }
}

fn model_picker(ui: &mut BevyUi) {
    let state = ui.world.resource::<AgentState>();
    if state.models.is_empty() {
        return;
    }
    let models = state.models.clone();
    let selected = state
        .selected
        .as_ref()
        .map(|model| format!("{} / {}", model.provider, model.name))
        .unwrap_or_else(|| "Choose a model".into());
    let busy = state.active.is_some() || !state.connected;
    ui.elem(elem!(DropdownMenu)).with(move |ui| {
        let mut control = ui.elem(elem!(
            Dropdown,
            label = elem!(Label, text = selected, wrap = false),
            max_width = percent(100)
        ));
        if busy {
            control.insert(InteractionDisabled);
        }
        ui.elem(elem!(DropdownList, width = px(300))).with(
            move |ui| {
                let height = (models.len() as f32 * 26.0).min(260.0);
                ui.elem(elem!(
                    ScrollArea,
                    height = px(height),
                    scroll_x = false
                ))
                .with(move |ui| {
                    for model in models {
                        let label = format!(
                            "{} / {}",
                            model.provider, model.name
                        );
                        menu_item(ui, None, label, move |world| {
                            agent::select_model(
                                world,
                                &model.provider,
                                &model.id,
                            );
                        });
                    }
                });
            },
        );
    });
}

fn transcript(ui: &mut BevyUi) {
    let count = ui.world.resource::<AgentState>().messages.len();
    if count == 0 {
        let muted = ui.theme.color.text_dim;
        ui.elem(elem!(
            Label,
            text = "Ask about an animation or describe what you want to create.",
            color = muted
        ));
    }
    for index in 0..count {
        let role =
            ui.world.resource::<AgentState>().messages[index].role;
        let muted = ui.theme.color.text_dim;
        ui.elem(elem!(
            Frame,
            width = percent(100),
            min_width = px(0),
            flex_shrink = 0.0f32,
            direction = FlexDirection::Column,
            row_gap = px(4)
        ))
        .with(move |ui| {
            ui.elem(elem!(
                Label,
                text = role,
                bold = true,
                color = muted
            ));
            ui.elem(elem!(Label)).bind(
                |label| label.text(),
                value_changed(move |world, _| {
                    world
                        .resource::<AgentState>()
                        .messages
                        .get(index)
                        .map(|message| message.revision)
                }),
                move |WorldNodeRef { world, .. }| {
                    world
                        .resource::<AgentState>()
                        .messages
                        .get(index)
                        .map(|message| message.text.clone())
                        .unwrap_or_default()
                },
            );
        });
    }
}

fn composer(ui: &mut BevyUi) {
    let draft = ui.world.resource::<AgentState>().draft.clone();
    let mut field = ui.elem(elem!(
        TextField,
        value = draft,
        width = percent(100)
    ));
    field.insert(Name::new("Agent message")).bind(
        |field| field.value(),
        structure_changed::<AgentState, _>(|state| {
            state.composer_revision
        }),
        |WorldNodeRef { world, .. }| {
            world.resource::<AgentState>().draft.clone()
        },
    );
    let node = field.id();
    let Some(input) = TextField::text_input(ui.world, node) else {
        return;
    };
    ui.world.entity_mut(input).observe(
        |change: On<TextEditChange>,
         texts: Query<&EditableText>,
         mut state: ResMut<AgentState>| {
            if let Ok(text) = texts.get(change.event_target()) {
                let value = text.value().to_string();
                if state.draft != value {
                    state.draft = value;
                }
            }
        },
    );
    ui.world.entity_mut(input).observe(
        |mut key: On<FocusedInput<KeyboardInput>>,
         mut commands: Commands| {
            if key.input.state == ButtonState::Pressed
                && !key.input.repeat
                && key.input.logical_key == Key::Enter
            {
                key.propagate(false);
                commands.queue(agent::send);
            }
        },
    );
}

fn controls(ui: &mut BevyUi) {
    let state = ui.world.resource::<AgentState>();
    let busy = state.active.is_some();
    let connecting = state.connecting;
    let connected = state.connected;
    let can_send = state.can_send();
    let empty = state.messages.is_empty();
    if busy {
        button(ui, "Stop", true, agent::cancel);
    } else {
        button(ui, "Send", can_send, agent::send);
    }
    if !connected && !connecting && empty {
        button(ui, "Connect", true, agent::connect);
    }
    button(ui, "New chat", !busy && !connecting, agent::new_chat);
}

fn button(
    ui: &mut BevyUi,
    text: &str,
    enabled: bool,
    run: fn(&mut World),
) {
    let mut control = ui.elem(elem!(
        Button,
        label = elem!(Label, text = text),
        width = auto(),
        padding = UiRect::horizontal(px(10)),
        height = px(28)
    ));
    control.observe(
        move |_: On<Activate>, mut commands: Commands| {
            if enabled {
                commands.queue(run);
            }
        },
    );
    if !enabled {
        control.insert(InteractionDisabled);
    }
}

#[derive(Component, Default)]
pub(super) struct ChatScroll {
    maximum: f32,
}

pub(super) fn follow_output(
    mut areas: Query<(
        &ComputedNode,
        &mut ScrollPosition,
        &mut ChatScroll,
    )>,
) {
    for (node, mut scroll, mut state) in &mut areas {
        let maximum = ((node.content_size().y - node.size().y)
            * node.inverse_scale_factor())
        .max(0.0);
        if scroll.0.y >= state.maximum - 20.0 {
            scroll.0.y = maximum;
        }
        state.maximum = maximum;
    }
}
