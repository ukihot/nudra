//! タイトル・オプション・クレジット。状態変更はポートの指図だけで行う。

use bevy::{
    picking::{events::PointerClick, hover::Hovered},
    prelude::*,
};
use bevy_intl::I18n;
use domain::progress::Language;

use super::{
    keys::{self, Ui},
    select::label,
};
use crate::{
    port::{OptionsModel, OutgameScreen, Request, SubmitRequest},
    state::AppState,
};

#[derive(Component)]
pub struct MenuAction(pub OutgameScreen);

#[derive(Component)]
struct OptionsRoot;

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Title), spawn_title)
        .add_systems(OnEnter(AppState::Credits), spawn_credits)
        .add_systems(Update, render_options.run_if(in_state(AppState::Options)))
        .add_systems(Update, (sync_language, menu_keys))
        .add_systems(PostUpdate, apply_font.before(bevy::ui::UiSystems::Layout));
}

pub(super) fn panel(state: AppState) -> impl Bundle {
    (
        DespawnOnExit(state),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(16),
            padding: UiRect::all(px(24)),
            ..default()
        },
    )
}

pub(super) fn button(caption: impl Bundle, selected: bool) -> impl Bundle {
    (
        Hovered::default(),
        Node {
            min_width: px(200),
            padding: UiRect::all(px(12)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(if selected {
            Color::srgb(0.32, 0.38, 0.15)
        } else {
            Color::srgb(0.14, 0.19, 0.14)
        }),
        children![caption],
    )
}

pub(super) fn add_back(commands: &mut Commands, root: Entity) {
    let back = commands
        .spawn(button(label(Ui::Back.text(), 22.0, Color::WHITE), false))
        .observe(|_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
            submit.write(SubmitRequest(Request::Navigate(OutgameScreen::Title)));
        })
        .id();
    commands.entity(root).add_child(back);
}

fn spawn_title(mut commands: Commands) {
    let root = commands
        .spawn((
            panel(AppState::Title),
            children![label(Ui::Title.text(), 72.0, Color::srgb(0.95, 0.8, 0.3))],
        ))
        .id();
    for (caption, screen) in [
        (Ui::Prepare, OutgameScreen::Preparation),
        (Ui::Options, OutgameScreen::Options),
        (Ui::Credits, OutgameScreen::Credits),
    ] {
        let item = commands
            .spawn((
                MenuAction(screen),
                button(label(caption.text(), 28.0, Color::WHITE), false),
            ))
            .observe(move |_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
                submit.write(SubmitRequest(Request::Navigate(screen)));
            })
            .id();
        commands.entity(root).add_child(item);
    }
}

fn render_options(
    mut commands: Commands,
    model: Res<OptionsModel>,
    roots: Query<Entity, With<OptionsRoot>>,
) {
    if !model.is_changed() && !roots.is_empty() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let root = commands
        .spawn((
            OptionsRoot,
            panel(AppState::Options),
            children![
                label(Ui::Options.text(), 40.0, Color::WHITE),
                label(Ui::Language.text(), 24.0, Color::WHITE)
            ],
        ))
        .id();
    for (caption, language) in
        [(Ui::Japanese, Language::Japanese), (Ui::English, Language::English)]
    {
        let item = commands
            .spawn(button(
                label(caption.text(), 24.0, Color::WHITE),
                model.language == language,
            ))
            .observe(move |_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
                submit.write(SubmitRequest(Request::SetLanguage(language)));
            })
            .id();
        commands.entity(root).add_child(item);
    }
    add_back(&mut commands, root);
}

fn spawn_credits(mut commands: Commands) {
    let root = commands
        .spawn((
            panel(AppState::Credits),
            children![
                label(Ui::Credits.text(), 40.0, Color::WHITE),
                label(Ui::CreditsBody.text(), 20.0, Color::srgb(0.8, 0.83, 0.75))
            ],
        ))
        .id();
    add_back(&mut commands, root);
}

fn sync_language(model: Res<OptionsModel>, i18n: Option<ResMut<I18n>>) {
    if model.is_changed()
        && let Some(mut i18n) = i18n
    {
        i18n.set_lang(match model.language {
            Language::Japanese => keys::JAPANESE_LANG,
            Language::English => keys::ENGLISH_LANG,
        });
    }
}

fn apply_font(
    server: Res<AssetServer>,
    fonts: Option<Res<Assets<Font>>>,
    mut texts: Query<&mut TextFont, Added<TextFont>>,
) {
    if fonts.is_none() || texts.is_empty() {
        return;
    }
    let font = server.load::<Font>(keys::UI_FONT);
    for mut text in &mut texts {
        text.font = bevy::text::FontSource::Handle(font.clone());
    }
}

fn menu_keys(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<AppState>>,
    mut submit: MessageWriter<SubmitRequest>,
) {
    let screen = match *state.get() {
        AppState::Title
            if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Digit1) =>
        {
            Some(OutgameScreen::Preparation)
        },
        AppState::Title if keys.just_pressed(KeyCode::Digit2) => Some(OutgameScreen::Options),
        AppState::Title if keys.just_pressed(KeyCode::Digit3) => Some(OutgameScreen::Credits),
        AppState::CharacterSelect | AppState::Options | AppState::Credits
            if keys.just_pressed(KeyCode::Escape) =>
        {
            Some(OutgameScreen::Title)
        },
        _ => None,
    };
    if let Some(screen) = screen {
        submit.write(SubmitRequest(Request::Navigate(screen)));
    }
}
