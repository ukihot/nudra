//! Repeatable GPU screenshots: cargo run -p game --example art_review -- high
//! Uses a separate temporary save directory. Does not write to the player's save.

use avian3d::prelude::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};
use domain::{
    difficulty::Difficulty,
    id::{PusherCharacterId, StageId},
    progress::RenderQuality,
};
use game::{
    infrastructure::{progress::Progress, save::FileProgressRepository, session::GameClock},
    port::{OutgameScreen, Request, SubmitRequest},
    state::AppState,
};

#[derive(Resource)]
struct Review {
    quality: RenderQuality,
    name: String,
    phase: u8,
    frozen_at: std::time::Duration,
}

fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "standard".into());
    let quality = match name.as_str() {
        "mobile" => RenderQuality::Mobile,
        "high" => RenderQuality::High,
        _ => RenderQuality::Standard,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace");
    App::new()
        .insert_resource(Review {
            quality,
            name,
            phase: 0,
            frozen_at: std::time::Duration::ZERO,
        })
        .insert_resource(FileProgressRepository::in_dir(root.join("target/art-review-save")))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: root.join("game/assets").to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "NUDRA · Art review".into(),
                        resolution: (1440, 900).into(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(PhysicsPlugins::new(FixedPostUpdate))
        .add_plugins(game::plugin)
        .add_systems(Update, review)
        .run();
}

#[allow(clippy::too_many_arguments)]
fn review(
    mut review: ResMut<Review>,
    state: Res<State<AppState>>,
    mut submit: MessageWriter<SubmitRequest>,
    mut progress: ResMut<Progress>,
    clock: Option<Res<GameClock>>,
    mut time: ResMut<Time<Virtual>>,
    real: Res<Time<Real>>,
    mut commands: Commands,
) {
    match review.phase {
        0 if *state.get() == AppState::Title => {
            progress.set_render_quality(review.quality);
            submit.write(SubmitRequest(Request::Navigate(OutgameScreen::Preparation)));
            review.phase = 1;
        },
        1 if *state.get() == AppState::CharacterSelect => {
            submit.write(SubmitRequest(Request::Start {
                character: PusherCharacterId::new("ao_umiushi"),
                stage: StageId::new("stage_01"),
                difficulty: Difficulty::EASY,
            }));
            review.phase = 2;
        },
        2 if clock.is_some_and(|clock| clock.now().0 >= 300) => {
            time.pause();
            review.frozen_at = real.elapsed();
            review.phase = 3;
        },
        // Keep rendering the frozen world while each backend finishes pipeline compilation.
        3 if real
            .elapsed()
            .saturating_sub(review.frozen_at)
            .as_secs()
            >= 8 =>
        {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace");
            let path = root.join(format!("art/reviews/play-{}.png", review.name));
            std::fs::create_dir_all(path.parent().expect("review dir")).expect("review dir");
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path))
                .observe(|_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                });
            review.phase = 4;
        },
        _ => {},
    }
}
