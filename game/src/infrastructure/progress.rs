//! 進行記録（セーブデータ）とBevyの接続（オフラインファースト、基本設計 5.7, 6.4）。
//!
//! 書き込みは `IoTaskPool` のタスクで行う。タスクは投げっぱなしにせず `SaveJob` が保持し、
//! 毎フレーム完了を確認して終端（完了 / 失敗）を必ず受け取る。
//! 読込・保存の失敗はゲーム進行を止めない。記録はメモリに保持し、次の保存で再試行する。

use application::{ProgressStoreError, load_progress, save_progress};
use bevy::{
    prelude::{
        App, AppExit, Commands, Deref, DerefMut, IntoScheduleConfigs, Last, MessageReader, Res,
        ResMut, Resource, Startup, debug, error, warn,
    },
    tasks::{IoTaskPool, Task, block_on, futures::check_ready},
};
use domain::progress::PlayerProgress;

use super::save::{FileProgressRepository, SaveFileError};

/// 失敗した書き込みを再試行するまでのフレーム数（調整ノブ）。
const RETRY_AFTER_FRAMES: u32 = 60;

/// メモリ上の進行記録（正本はローカルのセーブファイル）。
#[derive(Resource, Debug, Default, Deref, DerefMut)]
pub struct Progress(pub PlayerProgress);

/// セーブのジョブ: `待機 → 書込中 → 完了 / 失敗`。
/// 書込中に届いた依頼は最新の内容1件に合流させ、書き終えたら続けて書く（同時に2つ書かない）。
#[derive(Resource, Default)]
pub struct SaveJob {
    /// 書込中のタスクと、その内容（失敗したら再試行に使う）
    in_flight: Option<(Task<Result<(), ProgressStoreError>>, PlayerProgress)>,
    /// まだ書いていない最新の内容
    queued: Option<PlayerProgress>,
    /// 失敗後、再試行までの残りフレーム
    retry_in: u32,
}

impl SaveJob {
    /// 保存を依頼する。書込中なら合流させる。
    pub fn request(&mut self, progress: &PlayerProgress) {
        self.queued = Some(progress.clone());
    }

    /// 書込中も未書き込みも無い。
    pub fn is_idle(&self) -> bool {
        self.in_flight.is_none() && self.queued.is_none()
    }
}

pub(super) fn plugin(app: &mut App) {
    // テストなどで保存先が先に与えられていれば、それを使う
    if !app
        .world()
        .contains_resource::<FileProgressRepository>()
    {
        app.insert_resource(FileProgressRepository::in_dir(FileProgressRepository::default_dir()));
    }
    app.init_resource::<Progress>()
        .init_resource::<SaveJob>()
        .add_systems(Startup, load_on_startup)
        .add_systems(Last, (drive_save_job, flush_on_exit).chain());
}

fn load_on_startup(repo: Res<FileProgressRepository>, mut commands: Commands) {
    let progress = load_progress(&*repo).unwrap_or_else(|e| {
        error!("{e:?}");
        error!(path = %repo.path().display(), "save data is unavailable; starting with empty progress");
        PlayerProgress::default()
    });
    commands.insert_resource(Progress(progress));
}

/// 毎フレーム: 書込中のタスクの終端を受け取り、未書き込みがあれば次を書き始める。
fn drive_save_job(mut job: ResMut<SaveJob>, repo: Res<FileProgressRepository>) {
    if let Some((task, _)) = job.in_flight.as_mut() {
        let Some(result) = check_ready(task) else {
            return;
        };
        let (_, attempted) = job.in_flight.take().expect("checked above");
        match result {
            Ok(()) => debug!("progress saved"),
            Err(e) if is_permanent(&e) => warn!("{e:?}"),
            Err(e) => {
                warn!("{e:?}");
                job.retry_in = RETRY_AFTER_FRAMES;
                // より新しい依頼が無ければ、失敗した内容を再試行する
                job.queued.get_or_insert(attempted);
            },
        }
    }
    if job.retry_in > 0 {
        job.retry_in -= 1;
        return;
    }
    if let Some(snapshot) = job.queued.take() {
        let mut repo = repo.clone();
        let content = snapshot.clone();
        let task = IoTaskPool::get().spawn(async move { save_progress(&mut repo, &content) });
        job.in_flight = Some((task, snapshot));
    }
}

/// 終了時: 書込中のタスクを待ち、未書き込みの依頼を書き切ってから終わる。
fn flush_on_exit(
    mut exits: MessageReader<AppExit>,
    mut job: ResMut<SaveJob>,
    repo: Res<FileProgressRepository>,
) {
    if exits.read().count() == 0 {
        return;
    }
    if let Some((task, _)) = job.in_flight.take()
        && let Err(e) = block_on(task)
    {
        warn!("{e:?}");
    }
    if let Some(snapshot) = job.queued.take()
        && let Err(e) = save_progress(&mut repo.clone(), &snapshot)
    {
        warn!("{e:?}");
    }
}

/// 再試行しても成功しない失敗（未来の版のファイルを上書きしない、など）。
fn is_permanent(error: &ProgressStoreError) -> bool {
    let ProgressStoreError::Infrastructure(inner) = error;
    matches!(inner.downcast_ref::<SaveFileError>(), Some(SaveFileError::ReadOnly))
}
