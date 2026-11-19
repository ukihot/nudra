//! 進行記録（セーブデータ）とBevyの接続（オフラインファースト、基本設計 5.7）。
//! 読込・保存の失敗はゲーム進行を止めない。記録はメモリに保持し、次の保存で再試行する。

use application::{load_progress, record_result, save_progress};
use bevy::prelude::{
    App, Commands, Deref, DerefMut, IntoScheduleConfigs, MessageReader, Res, ResMut, Resource,
    Startup, Update, error, in_state, warn,
};
use domain::{
    progress::{PlayerProgress, RecordOutcome},
    session::GameSession,
};

use super::save::FileProgressRepository;
use crate::{port::StartRequested, state::AppState};

/// メモリ上の進行記録（正本はローカルのセーブファイル）。
#[derive(Resource, Debug, Default, Deref, DerefMut)]
pub struct Progress(pub PlayerProgress);

pub(super) fn plugin(app: &mut App) {
    // テストなどで保存先が先に与えられていれば、それを使う
    if !app
        .world()
        .contains_resource::<FileProgressRepository>()
    {
        app.insert_resource(FileProgressRepository::in_dir(FileProgressRepository::default_dir()));
    }
    app.init_resource::<Progress>()
        .add_systems(Startup, load_on_startup)
        .add_systems(Update, remember_character.run_if(in_state(AppState::CharacterSelect)));
}

fn load_on_startup(repo: Res<FileProgressRepository>, mut commands: Commands) {
    let progress = load_progress(&*repo).unwrap_or_else(|e| {
        error!("{e:?}");
        error!(path = %repo.path().display(), "save data is unavailable; starting with empty progress");
        PlayerProgress::default()
    });
    commands.insert_resource(Progress(progress));
}

fn remember_character(
    mut requests: MessageReader<StartRequested>,
    mut progress: ResMut<Progress>,
    mut repo: ResMut<FileProgressRepository>,
) {
    if let Some(request) = requests.read().last() {
        progress.select_character(request.character.clone());
        persist(&mut repo, &progress);
    }
}

/// 終了したセッションの結果を記録して保存する。
pub fn record_and_save(
    progress: &mut Progress,
    repo: &mut FileProgressRepository,
    session: &GameSession,
) -> Option<RecordOutcome> {
    let outcome = record_result(progress, session).ok()?;
    persist(repo, progress);
    Some(outcome)
}

/// 保存の失敗は警告に留め、ゲームを続ける。
fn persist(repo: &mut FileProgressRepository, progress: &PlayerProgress) {
    if let Err(e) = save_progress(repo, progress) {
        warn!("{e:?}");
    }
}
