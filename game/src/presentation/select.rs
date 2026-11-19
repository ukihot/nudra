//! プッシャー・ステージ・解禁済み難易度を選んでから開始する準備画面。

use bevy::{picking::events::PointerClick, prelude::*};
use bevy_intl::I18nText;
use domain::{
    difficulty::Difficulty,
    id::{PusherCharacterId, StageId},
};

use super::{
    keys::{self, Arg, Ui},
    outgame::{add_back, button, panel},
};
use crate::{
    port::{CharacterRoster, Request, StageRoster, SubmitRequest},
    state::AppState,
};

const NUMBER_KEYS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

#[derive(Resource, Default)]
struct Draft {
    character: Option<PusherCharacterId>,
    stage: Option<StageId>,
    difficulty: Option<Difficulty>,
}

#[derive(Component)]
struct PreparationRoot;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Draft>().add_systems(
        Update,
        (validate_draft, preparation_keys, render_preparation)
            .chain()
            .run_if(in_state(AppState::CharacterSelect)),
    );
}

pub(super) fn label(text: I18nText, size: f32, color: Color) -> impl Bundle {
    (
        text,
        TextFont { font_size: FontSize::Px(size), ..default() },
        TextColor(color),
    )
}

fn validate_draft(
    mut draft: ResMut<Draft>,
    roster: Res<CharacterRoster>,
    stages: Res<StageRoster>,
) {
    if !roster
        .0
        .iter()
        .any(|entry| Some(&entry.id) == draft.character.as_ref())
    {
        draft.character = roster.0.first().map(|entry| entry.id.clone());
    }
    if !stages
        .0
        .iter()
        .any(|entry| Some(&entry.id) == draft.stage.as_ref())
    {
        draft.stage = stages.0.first().map(|entry| entry.id.clone());
    }
    if draft.difficulty.is_none() {
        draft.difficulty = Some(Difficulty::EASY);
    }
    if let Some(stage) = stages
        .0
        .iter()
        .find(|entry| Some(&entry.id) == draft.stage.as_ref())
        && draft.difficulty.is_some_and(|d| d > stage.unlocked)
    {
        draft.difficulty = Some(stage.unlocked);
    }
}

fn render_preparation(
    mut commands: Commands,
    draft: Res<Draft>,
    roster: Res<CharacterRoster>,
    stages: Res<StageRoster>,
    roots: Query<Entity, With<PreparationRoot>>,
) {
    if !draft.is_changed() && !roster.is_changed() && !stages.is_changed() && !roots.is_empty() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let root = commands
        .spawn((
            PreparationRoot,
            panel(AppState::CharacterSelect),
            children![label(Ui::Prepare.text(), 40.0, Color::WHITE)],
        ))
        .id();
    let columns = commands
        .spawn(Node {
            column_gap: px(32),
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .id();
    commands.entity(root).add_child(columns);
    let characters = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                ..default()
            },
            children![label(Ui::ChoosePusher.text(), 24.0, Color::WHITE)],
        ))
        .id();
    let stage_column = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                ..default()
            },
            children![label(Ui::ChooseStage.text(), 24.0, Color::WHITE)],
        ))
        .id();
    commands
        .entity(columns)
        .add_children(&[characters, stage_column]);
    for (index, entry) in roster.0.iter().enumerate() {
        let character = entry.id.clone();
        let item = commands
            .spawn(button(
                label(
                    Ui::CharacterEntry.with([
                        (Arg::Index, (index + 1).to_string()),
                        (Arg::Name, entry.name.clone()),
                    ]),
                    24.0,
                    Color::WHITE,
                ),
                draft.character.as_ref() == Some(&character),
            ))
            .observe(move |_: On<PointerClick>, mut draft: ResMut<Draft>| {
                draft.character = Some(character.clone());
            })
            .id();
        commands.entity(characters).add_child(item);
    }
    for entry in &stages.0 {
        let stage = entry.id.clone();
        let item = commands
            .spawn(button(
                label(keys::stage_name(&stage), 24.0, Color::WHITE),
                draft.stage.as_ref() == Some(&stage),
            ))
            .observe(move |_: On<PointerClick>, mut draft: ResMut<Draft>| {
                draft.stage = Some(stage.clone());
            })
            .id();
        commands.entity(stage_column).add_child(item);
    }
    let heading = commands
        .spawn(label(Ui::Difficulty.text(), 24.0, Color::WHITE))
        .id();
    commands.entity(root).add_child(heading);
    let row = commands
        .spawn(Node {
            column_gap: px(8),
            row_gap: px(8),
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .id();
    commands.entity(root).add_child(row);
    let unlocked = stages
        .0
        .iter()
        .find(|s| Some(&s.id) == draft.stage.as_ref())
        .map_or(Difficulty::EASY, |s| s.unlocked);
    for level in 1..=Difficulty::LEVELS {
        let Ok(difficulty) = Difficulty::new(level) else {
            continue;
        };
        let enabled = difficulty <= unlocked;
        let caption = if enabled {
            Ui::DifficultyLevel
        } else {
            Ui::LockedDifficulty
        };
        let item = commands
            .spawn(button(
                label(
                    caption.with([(Arg::Level, level.to_string())]),
                    20.0,
                    if enabled {
                        Color::WHITE
                    } else {
                        Color::srgb(0.4, 0.4, 0.4)
                    },
                ),
                draft.difficulty == Some(difficulty),
            ))
            .observe(move |_: On<PointerClick>, mut draft: ResMut<Draft>| {
                if enabled {
                    draft.difficulty = Some(difficulty);
                }
            })
            .id();
        commands.entity(row).add_child(item);
    }
    let ready = match (&draft.character, &draft.stage, draft.difficulty) {
        (Some(character), Some(stage), Some(difficulty)) => Some(Request::Start {
            character: character.clone(),
            stage: stage.clone(),
            difficulty,
        }),
        _ => None,
    };
    let start = commands
        .spawn(button(label(Ui::Start.text(), 26.0, Color::WHITE), true))
        .observe(move |_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
            if let Some(request) = &ready {
                submit.write(SubmitRequest(request.clone()));
            }
        })
        .id();
    commands.entity(root).add_child(start);
    add_back(&mut commands, root);
    let help = commands
        .spawn(label(
            Ui::PreparationControls.text(),
            16.0,
            Color::srgb(0.6, 0.65, 0.6),
        ))
        .id();
    commands.entity(root).add_child(help);
}

fn preparation_keys(
    keys: Res<ButtonInput<KeyCode>>,
    roster: Res<CharacterRoster>,
    stages: Res<StageRoster>,
    mut draft: ResMut<Draft>,
    mut submit: MessageWriter<SubmitRequest>,
) {
    if let Some(entry) = NUMBER_KEYS
        .iter()
        .position(|k| keys.just_pressed(*k))
        .and_then(|i| roster.0.get(i))
    {
        draft.character = Some(entry.id.clone());
    }
    if !stages.0.is_empty() && keys.any_just_pressed([KeyCode::ArrowLeft, KeyCode::ArrowRight]) {
        let current = stages
            .0
            .iter()
            .position(|s| Some(&s.id) == draft.stage.as_ref())
            .unwrap_or(0);
        let offset = if keys.just_pressed(KeyCode::ArrowLeft) {
            stages.0.len() - 1
        } else {
            1
        };
        draft.stage = Some(
            stages.0[(current + offset) % stages.0.len()]
                .id
                .clone(),
        );
        draft.difficulty = Some(Difficulty::EASY);
    }
    if let Some(stage) = stages
        .0
        .iter()
        .find(|s| Some(&s.id) == draft.stage.as_ref())
    {
        let level = draft.difficulty.unwrap_or(Difficulty::EASY).level();
        let next = if keys.just_pressed(KeyCode::ArrowUp) {
            (level + 1).min(stage.unlocked.level())
        } else if keys.just_pressed(KeyCode::ArrowDown) {
            level.saturating_sub(1).max(1)
        } else {
            level
        };
        if level != next {
            draft.difficulty = Difficulty::new(next).ok();
        }
    }
    if keys.just_pressed(KeyCode::Enter)
        && let (Some(character), Some(stage), Some(difficulty)) =
            (&draft.character, &draft.stage, draft.difficulty)
    {
        submit.write(SubmitRequest(Request::Start {
            character: character.clone(),
            stage: stage.clone(),
            difficulty,
        }));
    }
}
